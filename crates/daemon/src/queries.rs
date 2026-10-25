//! QueryRegistry: Pathfinder persisted-query hashes, discovered from the web player's public JavaScript.
//!
//! Spotify rotates these hashes; a retired one answers HTTP 412. Instead of shipping hashes in releases,
//! the Daemon reads them from open.spotify.com: the main bundle plus every lazily-loaded chunk
//! (≈160 files, ≈190 operations as of 2026-10). Results are cached on disk for a few days and refreshed
//! immediately when a query comes back 412.

use futures_util::StreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const WEB_PLAYER: &str = "https://open.spotify.com/";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36";
const TTL_MS: u64 = 3 * 24 * 3600 * 1000;
/// The most one of the web player's files may be (the main bundle is a few MB): a broken answer is refused instead of
/// held in memory whole (audit S7).
const MAX_BODY: u64 = 16 * 1024 * 1024;
/// A whole refresh's budget when something is waiting for it (a 412 after a hash was retired).
const REFRESH_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

static OP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""(\w+)","(?:query|mutation)","([0-9a-f]{64})""#).unwrap());
static MAIN_JS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"src="(https://[^"]+/web-player/web-player\.[0-9a-f]+\.js)""#).unwrap());
static CHUNK_MAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\(\{([^{}]*)\}\)\[e\]"#).unwrap());
static PAIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(\d+):"([^"]+)""#).unwrap());
static PUBLIC_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\.p="(https://[^"]+/web-player/)""#).unwrap());

/// Every `"name","query|mutation","hash"` triple in a bundle.
pub fn extract_ops(js: &str) -> HashMap<String, String> {
    OP.captures_iter(js).map(|c| (c[1].to_string(), c[2].to_string())).collect()
}

/// URLs of the lazily-loaded chunks: `publicPath + (name || id) + "." + hash + ".js"`.
pub fn chunk_urls(main_js: &str) -> Vec<String> {
    let Some(end) = main_js.find(r#"+".js""#) else { return vec![] };
    let Some(start) = main_js[..end].rfind(".u=e=>") else { return vec![] };
    let seg = &main_js[start..end];
    let maps: Vec<&str> = CHUNK_MAP.captures_iter(seg).map(|c| c.get(1).unwrap().as_str()).collect();
    let (names, hashes) = match maps.as_slice() {
        [hashes] => (HashMap::new(), *hashes),
        [names, hashes, ..] => (PAIR.captures_iter(names).map(|c| (c[1].to_string(), c[2].to_string())).collect(), *hashes),
        _ => return vec![],
    };
    let base = PUBLIC_PATH.captures(main_js).map(|c| c[1].to_string()).unwrap_or_else(|| "https://open.spotifycdn.com/cdn/build/web-player/".into());
    PAIR.captures_iter(hashes)
        .map(|c| {
            let id = &c[1];
            format!("{base}{}.{}.js", names.get(id).map(String::as_str).unwrap_or(id), &c[2])
        })
        .collect()
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Cache {
    fetched_unix_ms: u64,
    ops: HashMap<String, String>,
}

pub struct QueryRegistry {
    path: PathBuf,
    cache: Cache,
    http: reqwest::Client,
    /// where the web player is (a test points it at a stand-in)
    base: String,
    /// a refresh running off the Catalogue's command loop (audit C8): browsing carries on with the old hashes until
    /// it finishes
    refreshing: Option<tokio::task::JoinHandle<anyhow::Result<Cache>>>,
}

impl QueryRegistry {
    pub fn new(path: PathBuf) -> QueryRegistry {
        let cache = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let http = reqwest::Client::builder().user_agent(USER_AGENT).timeout(std::time::Duration::from_secs(20)).build().expect("http client");
        QueryRegistry { path, cache, http, base: WEB_PLAYER.into(), refreshing: None }
    }

    /// Hash for `op`. A stale cache is refreshed *in the background*: browsing keeps working with the hashes it has
    /// (a retired one answers 412 and gets [`QueryRegistry::refresh`]ed then). Only a hash the cache never had waits.
    pub async fn hash(&mut self, op: &str) -> anyhow::Result<String> {
        self.collect().await;
        let stale = singularity_proto::now_ms().saturating_sub(self.cache.fetched_unix_ms) > TTL_MS;
        if !self.cache.ops.contains_key(op) {
            self.refresh().await?;
        } else if stale && self.refreshing.is_none() {
            tracing::info!("query registry: refreshing the Pathfinder hashes in the background; browsing carries on");
            self.start_refresh();
        }
        self.cache.ops.get(op).cloned().ok_or_else(|| anyhow::anyhow!("the web player no longer has a `{op}` query"))
    }

    /// Re-read every hash from the web player and wait for it (called on a 412, when the old hash is known dead).
    /// Bounded by [`REFRESH_BUDGET`]: a stalled CDN fails this query instead of the whole Catalogue.
    pub async fn refresh(&mut self) -> anyhow::Result<()> {
        self.collect().await;
        if self.refreshing.is_none() {
            self.start_refresh();
        }
        let handle = self.refreshing.take().expect("started above");
        let result = match tokio::time::timeout(REFRESH_BUDGET, handle).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => Err(anyhow::anyhow!("the query registry refresh stopped: {e}")),
            Err(_) => Err(anyhow::anyhow!("refreshing the query registry took over {} s", REFRESH_BUDGET.as_secs())),
        };
        if let Ok(cache) = &result {
            self.cache = cache.clone();
        }
        result.map(|_| ())
    }

    /// Apply a background refresh that has finished (if one has).
    async fn collect(&mut self) {
        if !self.refreshing.as_ref().is_some_and(|h| h.is_finished()) {
            return;
        }
        let handle = self.refreshing.take().expect("checked above");
        match handle.await {
            Ok(Ok(cache)) => self.cache = cache,
            Ok(Err(e)) => tracing::warn!("query registry refresh failed: {e}"),
            Err(e) => tracing::warn!("query registry refresh stopped: {e}"),
        }
    }

    /// Crawl the web player on its own task, so a slow one doesn't hold up the Catalogue.
    fn start_refresh(&mut self) {
        if self.refreshing.is_some() {
            return;
        }
        let (http, path, base) = (self.http.clone(), self.path.clone(), self.base.clone());
        self.refreshing = Some(tokio::spawn(async move { refresh_http(&http, &path, &base).await }));
    }

}

/// Crawl the web player and write the cache file: the background half of [`QueryRegistry::refresh`].
async fn refresh_http(http: &reqwest::Client, path: &Path, base: &str) -> anyhow::Result<Cache> {
    let get = |url: &str| {
        let http = http.clone();
        let url = url.to_string();
        async move {
            let resp = http.get(&url).send().await?.error_for_status()?;
            let mut body = resp.bytes_stream();
            let mut buf = Vec::new();
            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                anyhow::ensure!((buf.len() + chunk.len()) as u64 <= MAX_BODY, "a web player file is over the {} MB cap", MAX_BODY / (1024 * 1024));
                buf.extend_from_slice(&chunk);
            }
            Ok::<String, anyhow::Error>(String::from_utf8_lossy(&buf).into_owned())
        }
    };
    let html = get(base).await?;
    let main_url = MAIN_JS.captures(&html).map(|c| c[1].to_string()).ok_or_else(|| anyhow::anyhow!("web player bundle not found"))?;
    let main_js = get(&main_url).await?;
    let mut ops = extract_ops(&main_js);
    let urls = chunk_urls(&main_js);
    // a handful at a time; this runs every few days at most
    for (i, batch) in urls.chunks(8).enumerate() {
        let bodies = futures_util::future::join_all(batch.iter().map(|u| get(u))).await;
        for js in bodies.into_iter().flatten() {
            ops.extend(extract_ops(&js));
        }
        tracing::debug!("query registry: {} of {} chunks read", ((i + 1) * 8).min(urls.len()), urls.len());
    }
    anyhow::ensure!(ops.contains_key("home"), "couldn't find Pathfinder queries in the web player");
    tracing::info!("query registry refreshed: {} operations from {} chunks", ops.len(), urls.len() + 1);
    let cache = Cache { fetched_unix_ms: singularity_proto::now_ms(), ops };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::to_vec(&cache)?);
    anyhow::Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A miniature bundle with the same shape as the real one (no Spotify code is committed).
    const FIXTURE: &str = r#"var a=new o.l("home","query","5fb7da7a03ee2776c0670856609cb85a189a3cecc36cc160b91cde79063b9cf1",null),b=new o.l("addToLibrary","mutation","0f3e5b4b1a9e2a8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c",null);u.p="https://open.spotifycdn.com/cdn/build/web-player/",u.u=e=>""+(({1328:"xpui-pip-mini-player",1872:"xpui-routes-playlist"})[e]||e)+"."+({1328:"91fe3fc9",1872:"aaaa1111",9908:"e091f4c0"})[e]+".js""#;

    #[test]
    fn extracts_queries_and_mutations() {
        let ops = extract_ops(FIXTURE);
        assert_eq!(ops["home"], "5fb7da7a03ee2776c0670856609cb85a189a3cecc36cc160b91cde79063b9cf1");
        assert!(ops.contains_key("addToLibrary"));
    }

    /// C8: a stale cache is refreshed in the background: a hanging web player must not hold up the queries browsing
    /// needs (before this, `refresh` ran on the Catalogue's own task, one 20 s timeout per batch).
    #[tokio::test]
    async fn a_stale_cache_does_not_block_a_hash_it_already_has() {
        // a web player that accepts and then says nothing
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener_addr = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let mut open = Vec::new();
            while let Ok((s, _)) = listener.accept().await {
                open.push(s); // never answered
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pathfinder-queries.json");
        std::fs::write(&path, serde_json::to_vec(&Cache { fetched_unix_ms: 1, ops: HashMap::from([("home".to_string(), "h".to_string())]) }).unwrap()).unwrap();
        let mut registry = QueryRegistry::new(path);
        // the stand-in web player: a stale cache, and it never answers
        registry.base = format!("http://{}/", listener_addr);
        let hash = tokio::time::timeout(std::time::Duration::from_secs(2), registry.hash("home")).await.expect("a stale cache must not block the hash it has").unwrap();
        assert_eq!(hash, "h");
        assert!(registry.refreshing.is_some(), "the refresh is running off the command loop");
        held.abort();
    }

    #[test]
    fn builds_chunk_urls_with_names_and_ids() {
        let urls = chunk_urls(FIXTURE);
        assert!(urls.contains(&"https://open.spotifycdn.com/cdn/build/web-player/xpui-pip-mini-player.91fe3fc9.js".to_string()));
        assert!(urls.contains(&"https://open.spotifycdn.com/cdn/build/web-player/9908.e091f4c0.js".to_string()));
        assert_eq!(urls.len(), 3);
    }
}
