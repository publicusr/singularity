//! Nightly library backups (#23, D20): Liked Songs and every playlist the account owns, in full; playlists it only
//! follows as name + link. One private-API request at a time, a second apart, through the Catalogue's queue (so
//! browsing isn't held up); a playlist whose revision hasn't changed is copied from the previous backup instead of
//! fetched again. Written by `store.rs`, never sent anywhere.

use crate::Hub;
use crate::catalogue::CatCmd;
use crate::library::{self, LIBRARY_PAGE, LIKED};
use serde_json::{Value, json};
use singularity_proto::{CatalogueStatus, TrackRow};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

/// Requests are spaced this far apart.
const PACE: Duration = Duration::from_secs(1);
/// A backup is due when the last one is this old.
const EVERY_MS: u64 = 24 * 3_600_000;
/// Longest track list fetched (as the Catalogue's own limit).
const MAX_TRACKS: u32 = 10_000;
/// Folders opened at most (they nest a few levels; guard anyway).
const MAX_FOLDERS: usize = 200;

/// Runs one private-API query.
pub trait Fetch {
    fn query(&mut self, op: &str, vars: Value) -> impl std::future::Future<Output = anyhow::Result<Value>> + Send;
}

/// Queries through the Catalogue task, paced.
pub struct CatalogueFetch {
    tx: mpsc::Sender<CatCmd>,
    last: Option<Instant>,
}

impl Fetch for CatalogueFetch {
    async fn query(&mut self, op: &str, vars: Value) -> anyhow::Result<Value> {
        if let Some(t) = self.last {
            tokio::time::sleep_until((t + PACE).into()).await;
        }
        let (reply, answer) = oneshot::channel();
        self.tx.send(CatCmd::Query { op: op.into(), vars, reply }).await.map_err(|_| anyhow::anyhow!("the Catalogue stopped"))?;
        let r = answer.await.map_err(|_| anyhow::anyhow!("the Catalogue stopped"))?;
        self.last = Some(Instant::now());
        r
    }
}

fn row(t: &TrackRow) -> Value {
    json!({ "uri": t.uri, "name": t.name, "artists": t.artists, "album": t.album, "added_at": t.added_at })
}

/// Every track of a list, page by page.
async fn tracks(f: &mut impl Fetch, uri: &str) -> anyhow::Result<Vec<Value>> {
    let mut rows = vec![];
    let mut offset = 0;
    loop {
        let Some((op, vars)) = library::tracks_query(uri, offset) else { anyhow::bail!("can't list {uri}") };
        let (page, total) = library::parse_tracks(uri, &f.query(op, vars).await?);
        rows.extend(page.iter().map(row));
        offset += library::TRACK_PAGE;
        if page.is_empty() || offset >= total.min(MAX_TRACKS) {
            return Ok(rows);
        }
    }
}

/// Take a backup of `username`'s library. `previous`: the last backup, whose unchanged playlists are reused.
pub async fn take(f: &mut impl Fetch, username: &str, previous: Option<&Value>, now_ms: u64) -> anyhow::Result<Value> {
    let mut playlists = vec![];
    let mut levels: Vec<Option<String>> = vec![None];
    let mut opened = 0;
    while let Some(folder) = levels.pop() {
        let mut offset = 0;
        loop {
            let v = f.query("libraryV3", library::library_vars(offset, folder.as_deref())).await?;
            let (pl, folders, total, n) = library::parse_backup_page(&v);
            playlists.extend(pl);
            for f in folders {
                if opened < MAX_FOLDERS {
                    opened += 1;
                    levels.push(Some(f));
                }
            }
            offset += LIBRARY_PAGE;
            if n == 0 || offset >= total {
                break;
            }
        }
    }
    let liked = tracks(f, LIKED).await?;
    // last night's playlists by URI: revision and tracks
    let before: HashMap<&str, (&Value, &Value)> = previous
        .and_then(|p| p["playlists"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["uri"].as_str()?, (&p["revision"], &p["tracks"]))))
        .collect();
    let (mut owned, mut followed) = (vec![], vec![]);
    let mut seen = std::collections::HashSet::new();
    for p in playlists {
        if !seen.insert(p.uri.clone()) {
            continue;
        }
        if p.owner_username.as_deref() != Some(username) {
            followed.push(json!({ "uri": p.uri, "name": p.name, "owner": p.owner }));
            continue;
        }
        let tracks = match before.get(p.uri.as_str()) {
            Some((rev, t)) if p.revision.is_some() && rev.as_str() == p.revision.as_deref() => (*t).clone(),
            _ => Value::Array(tracks(f, &p.uri).await?),
        };
        owned.push(json!({ "uri": p.uri, "name": p.name, "revision": p.revision, "tracks": tracks }));
    }
    Ok(json!({ "schema": 1, "taken_ms": now_ms, "liked": liked, "playlists": owned, "followed": followed }))
}

/// Keep the backups coming: whenever the Catalogue is logged in and the last backup is a day old (checked at start
/// and then every hour; `SINGULARITY_BACKUP_CHECK_SECS` for tests).
pub async fn run(hub: Arc<Hub>) {
    let (Some(store), Some(tx)) = (hub.store.clone(), hub.catalogue.clone()) else { return };
    let every = std::env::var("SINGULARITY_BACKUP_CHECK_SECS").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or(Duration::from_secs(3600));
    // the Catalogue logs in first
    tokio::time::sleep(every.min(Duration::from_secs(30))).await;
    let mut fetch = CatalogueFetch { tx, last: None };
    loop {
        let user = match hub.catalogue_status.lock().await.clone() {
            Some(CatalogueStatus::Ready { username, .. }) => Some(username),
            _ => None,
        };
        let now = singularity_proto::now_ms();
        let previous = store.lock().expect("store lock").latest_backup();
        let due = previous.as_ref().is_none_or(|(at, _)| now.saturating_sub(*at) >= EVERY_MS);
        if let Some(user) = user.filter(|_| due) {
            tracing::info!("taking the nightly library backup");
            let started = Instant::now();
            match take(&mut fetch, &user, previous.as_ref().map(|(_, v)| v), now).await {
                Ok(b) => match store.lock().expect("store lock").write_backup(now, &b) {
                    Ok(path) => tracing::info!("library backup written to {} in {:.0?}", path.display(), started.elapsed()),
                    Err(e) => tracing::warn!("couldn't write the library backup: {e}"),
                },
                // tried again at the next check
                Err(e) => tracing::warn!("library backup failed: {e}"),
            }
        }
        tokio::time::sleep(every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serves a library of three playlists (two owned by "me", one in a folder) and counts what's asked.
    struct Fake {
        asked: Vec<String>,
        revision: &'static str,
    }

    fn track_page(items: Vec<Value>, total: usize) -> Value {
        json!({ "data": { "playlistV2": { "content": { "totalCount": total, "items": items } } } })
    }

    fn track(i: usize) -> Value {
        json!({ "itemV2": { "data": { "__typename": "Track", "uri": format!("spotify:track:{i}"), "name": format!("T{i}"),
            "artists": { "items": [{ "profile": { "name": "Nova Drift" } }] }, "albumOfTrack": { "name": "Album" } } },
            "addedAt": { "isoString": "2026-01-01T00:00:00Z" } })
    }

    fn playlist(uri: &str, owner: &str, rev: &str) -> Value {
        json!({ "item": { "data": { "__typename": "Playlist", "uri": uri, "name": uri, "revisionId": rev,
            "ownerV2": { "data": { "name": owner, "username": owner } } } } })
    }

    impl Fetch for Fake {
        async fn query(&mut self, op: &str, vars: Value) -> anyhow::Result<Value> {
            let what = format!("{op} {}", vars["folderUri"].as_str().or(vars["uri"].as_str()).unwrap_or(""));
            self.asked.push(format!("{what} @{}", vars["offset"]));
            let lib = |items: Vec<Value>| json!({ "data": { "me": { "libraryV3": { "totalCount": items.len(), "items": items } } } });
            Ok(match (op, vars["folderUri"].as_str(), vars["uri"].as_str()) {
                ("libraryV3", None, _) => lib(vec![
                    playlist("spotify:playlist:mine", "me", self.revision),
                    playlist("spotify:playlist:theirs", "dj", "x"),
                    json!({ "item": { "data": { "__typename": "Folder", "uri": "spotify:user:me:folder:f" } } }),
                ]),
                ("libraryV3", Some(_), _) => lib(vec![playlist("spotify:playlist:nested", "me", "n1")]),
                ("fetchLibraryTracks", ..) => json!({ "data": { "me": { "library": { "tracks": { "totalCount": 2, "items": [
                    { "track": { "_uri": "spotify:track:l0", "data": { "name": "Liked", "artists": { "items": [] } } } },
                    { "track": { "_uri": "spotify:track:l1", "data": { "name": "Liked 2", "artists": { "items": [] } } } } ] } } } } }),
                // "mine" has 150 tracks: two pages
                ("fetchPlaylist", _, Some("spotify:playlist:mine")) => {
                    let o = vars["offset"].as_u64().unwrap() as usize;
                    track_page((o..(o + 100).min(150)).map(track).collect(), 150)
                }
                ("fetchPlaylist", ..) => track_page(vec![track(900)], 1),
                _ => anyhow::bail!("unexpected {op}"),
            })
        }
    }

    #[tokio::test]
    async fn backs_up_owned_playlists_in_full_and_followed_ones_by_name() {
        let mut f = Fake { asked: vec![], revision: "r1" };
        let b = take(&mut f, "me", None, 7).await.unwrap();
        assert_eq!(b["liked"].as_array().unwrap().len(), 2);
        let owned: Vec<(&str, usize)> = b["playlists"].as_array().unwrap().iter().map(|p| (p["uri"].as_str().unwrap(), p["tracks"].as_array().unwrap().len())).collect();
        assert_eq!(owned, [("spotify:playlist:mine", 150), ("spotify:playlist:nested", 1)]);
        assert_eq!(b["followed"], json!([{ "uri": "spotify:playlist:theirs", "name": "spotify:playlist:theirs", "owner": "dj" }]));
        assert_eq!(b["playlists"][0]["tracks"][0], json!({ "uri": "spotify:track:0", "name": "T0", "artists": ["Nova Drift"], "album": "Album", "added_at": "2026-01-01T00:00:00Z" }));
        assert!(!f.asked.iter().any(|a| a.contains("theirs")), "followed playlists aren't fetched: {:?}", f.asked);

        // the next night: "mine" unchanged is copied, not fetched; once it changes, it's fetched again
        let mut again = Fake { asked: vec![], revision: "r1" };
        let b2 = take(&mut again, "me", Some(&b), 8).await.unwrap();
        assert_eq!(b2["playlists"], b["playlists"]);
        assert!(!again.asked.iter().any(|a| a.contains("fetchPlaylist spotify:playlist:mine")), "{:?}", again.asked);
        let mut changed = Fake { asked: vec![], revision: "r2" };
        take(&mut changed, "me", Some(&b), 9).await.unwrap();
        assert_eq!(changed.asked.iter().filter(|a| a.contains("fetchPlaylist spotify:playlist:mine")).count(), 2);
    }
}
