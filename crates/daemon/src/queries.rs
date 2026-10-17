//! QueryRegistry: Pathfinder persisted-query hashes, discovered from the web player's public JavaScript.
//!
//! Spotify rotates these hashes; a retired one answers HTTP 412. Instead of shipping hashes in releases,
//! the Daemon reads them from open.spotify.com: the main bundle plus every lazily-loaded chunk
//! (≈160 files, ≈190 operations as of 2026-10). Results are cached on disk for a few days and refreshed
//! immediately when a query comes back 412.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

const WEB_PLAYER: &str = "https://open.spotify.com/";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36";
const TTL_MS: u64 = 3 * 24 * 3600 * 1000;

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

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    fetched_unix_ms: u64,
    ops: HashMap<String, String>,
}

pub struct QueryRegistry {
    path: PathBuf,
    cache: Cache,
    http: reqwest::Client,
}

impl QueryRegistry {
    pub fn new(path: PathBuf) -> QueryRegistry {
        let cache = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let http = reqwest::Client::builder().user_agent(USER_AGENT).timeout(std::time::Duration::from_secs(20)).build().expect("http client");
        QueryRegistry { path, cache, http }
    }

    /// Hash for `op`, refreshing first if the cache is stale or doesn't know it.
    pub async fn hash(&mut self, op: &str) -> anyhow::Result<String> {
        let stale = singularity_proto::now_ms().saturating_sub(self.cache.fetched_unix_ms) > TTL_MS;
        if stale || !self.cache.ops.contains_key(op) {
            self.refresh().await?;
        }
        self.cache.ops.get(op).cloned().ok_or_else(|| anyhow::anyhow!("the web player no longer has a `{op}` query"))
    }

    /// Re-read every hash from the web player (called on a 412, or when stale).
    pub async fn refresh(&mut self) -> anyhow::Result<()> {
        let html = self.get(WEB_PLAYER).await?;
        let main_url = MAIN_JS.captures(&html).map(|c| c[1].to_string()).ok_or_else(|| anyhow::anyhow!("web player bundle not found"))?;
        let main_js = self.get(&main_url).await?;
        let mut ops = extract_ops(&main_js);
        let urls = chunk_urls(&main_js);
        // a handful at a time; this runs every few days at most
        for batch in urls.chunks(8) {
            let bodies = futures_util::future::join_all(batch.iter().map(|u| self.get(u))).await;
            for js in bodies.into_iter().flatten() {
                ops.extend(extract_ops(&js));
            }
        }
        anyhow::ensure!(ops.contains_key("home"), "couldn't find Pathfinder queries in the web player");
        tracing::info!("query registry refreshed: {} operations from {} chunks", ops.len(), urls.len() + 1);
        self.cache = Cache { fetched_unix_ms: singularity_proto::now_ms(), ops };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.path, serde_json::to_vec(&self.cache)?);
        Ok(())
    }

    async fn get(&self, url: &str) -> anyhow::Result<String> {
        Ok(self.http.get(url).send().await?.error_for_status()?.text().await?)
    }
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

    #[test]
    fn builds_chunk_urls_with_names_and_ids() {
        let urls = chunk_urls(FIXTURE);
        assert!(urls.contains(&"https://open.spotifycdn.com/cdn/build/web-player/xpui-pip-mini-player.91fe3fc9.js".to_string()));
        assert!(urls.contains(&"https://open.spotifycdn.com/cdn/build/web-player/9908.e091f4c0.js".to_string()));
        assert_eq!(urls.len(), 3);
    }
}
