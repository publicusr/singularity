//! Nightly library backups (#23, D20): Liked Songs and every playlist the account owns, in full; playlists it only
//! follows as name + link. One private-API request at a time, a second apart, through the Catalogue's queue (so
//! browsing isn't held up); a playlist whose revision hasn't changed is copied from the previous backup instead of
//! fetched again. A list that doesn't come back whole (a page with errors, or fewer tracks than Spotify says it has)
//! keeps its last whole copy, or without one is saved marked `incomplete` (#144). Written by `store.rs`, never sent
//! anywhere.

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

use crate::pathfinder::Fetch;

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

/// A list as fetched tonight: its rows, the length Spotify gave it, and why it isn't whole (if it isn't).
struct Listed {
    rows: Vec<Value>,
    total: u32,
    short: Option<String>,
}

/// Every track of a list, page by page. A page that comes back with errors, or not shaped like a list, or a list that
/// ends before its total, makes a short list, not a finished one (#144).
async fn tracks(f: &mut impl Fetch, uri: &str) -> anyhow::Result<Listed> {
    let mut rows = vec![];
    let mut offset = 0;
    let mut total = 0;
    loop {
        let Some((op, vars)) = library::tracks_query(uri, offset) else { anyhow::bail!("can't list {uri}") };
        let v = f.query(op, vars).await?;
        if v.get("errors").is_some() || library::stated_total(uri, &v).is_none() {
            let why = v["errors"][0]["message"].as_str().unwrap_or("not a track list");
            return Ok(Listed { rows, total, short: Some(format!("the page at {offset} came back with an error ({why})")) });
        }
        let (page, n) = library::parse_tracks(uri, &v);
        total = n;
        rows.extend(page.iter().map(row));
        offset += library::TRACK_PAGE;
        let want = total.min(MAX_TRACKS);
        if page.is_empty() || offset >= want {
            let short = (rows.len() < want as usize).then(|| format!("only {} of its {want} tracks came back", rows.len()));
            return Ok(Listed { rows, total, short });
        }
    }
}

/// Can a list from an earlier backup stand in for tonight's? Not if it was marked incomplete, or doesn't hold as many
/// tracks as Spotify said it had (a backup from before #144 doesn't say, so it's fetched again).
fn whole(tracks: &Value, total: &Value, incomplete: &Value) -> bool {
    incomplete.is_null() && tracks.as_array().zip(total.as_u64()).is_some_and(|(t, n)| t.len() as u64 == n.min(MAX_TRACKS as u64))
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
    // Liked Songs: tonight's, or when that comes back short the last whole copy, or else the short one, marked
    let mut liked = tracks(f, LIKED).await?;
    let (mut liked_total, mut liked_incomplete) = (json!(liked.total), Value::Null);
    if let Some(why) = liked.short.take() {
        match previous.filter(|p| whole(&p["liked"], &p["liked_total"], &p["liked_incomplete"])) {
            Some(p) => {
                tracing::warn!("library backup: Liked Songs: {why}; keeping the copy from the last backup");
                liked.rows = p["liked"].as_array().cloned().unwrap_or_default();
                liked_total = p["liked_total"].clone();
            }
            None => {
                tracing::warn!("library backup: Liked Songs: {why}; backed up as incomplete");
                liked_incomplete = json!(why);
            }
        }
    }
    // the last backup's whole playlists by URI
    let before: HashMap<&str, &Value> = previous
        .and_then(|p| p["playlists"].as_array())
        .into_iter()
        .flatten()
        .filter(|p| whole(&p["tracks"], &p["total"], &p["incomplete"]))
        .filter_map(|p| Some((p["uri"].as_str()?, p)))
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
        let last = before.get(p.uri.as_str());
        let entry = |revision: &Value, total: &Value, tracks: Value| json!({ "uri": p.uri, "name": p.name, "revision": revision, "total": total, "tracks": tracks });
        if let Some(e) = last.filter(|e| p.revision.is_some() && e["revision"].as_str() == p.revision.as_deref()) {
            owned.push(entry(&e["revision"], &e["total"], e["tracks"].clone()));
            continue;
        }
        let l = tracks(f, &p.uri).await?;
        owned.push(match (l.short, last) {
            (None, _) => entry(&json!(p.revision), &json!(l.total), Value::Array(l.rows)),
            // the last whole copy, under its own revision, so the next backup tries again
            (Some(why), Some(e)) => {
                tracing::warn!("library backup: {}: {why}; keeping the copy from the last backup", p.uri);
                entry(&e["revision"], &e["total"], e["tracks"].clone())
            }
            (Some(why), None) => {
                tracing::warn!("library backup: {}: {why}; backed up as incomplete", p.uri);
                let mut e = entry(&json!(p.revision), &json!(l.total), Value::Array(l.rows));
                e["incomplete"] = json!(why);
                e
            }
        });
    }
    let mut b = json!({ "schema": 1, "taken_ms": now_ms, "liked": liked.rows, "liked_total": liked_total, "playlists": owned, "followed": followed });
    if !liked_incomplete.is_null() {
        b["liked_incomplete"] = liked_incomplete;
    }
    Ok(b)
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
        /// What "mine"'s second page comes back as instead of its last 50 tracks.
        trouble: Option<Trouble>,
    }

    #[derive(Clone, Copy)]
    enum Trouble {
        /// An answer with `errors` and no list.
        Errors,
        /// An empty page, as if the list ended there.
        Empty,
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
                    match self.trouble.filter(|_| o > 0) {
                        Some(Trouble::Errors) => json!({ "errors": [{ "message": "Something went wrong" }], "data": null }),
                        Some(Trouble::Empty) => track_page(vec![], 150),
                        None => track_page((o..(o + 100).min(150)).map(track).collect(), 150),
                    }
                }
                ("fetchPlaylist", ..) => track_page(vec![track(900)], 1),
                _ => anyhow::bail!("unexpected {op}"),
            })
        }
    }

    #[tokio::test]
    async fn backs_up_owned_playlists_in_full_and_followed_ones_by_name() {
        let mut f = Fake { asked: vec![], revision: "r1", trouble: None };
        let b = take(&mut f, "me", None, 7).await.unwrap();
        assert_eq!(b["liked"].as_array().unwrap().len(), 2);
        let owned: Vec<(&str, usize)> = b["playlists"].as_array().unwrap().iter().map(|p| (p["uri"].as_str().unwrap(), p["tracks"].as_array().unwrap().len())).collect();
        assert_eq!(owned, [("spotify:playlist:mine", 150), ("spotify:playlist:nested", 1)]);
        assert_eq!(b["followed"], json!([{ "uri": "spotify:playlist:theirs", "name": "spotify:playlist:theirs", "owner": "dj" }]));
        assert_eq!(b["playlists"][0]["tracks"][0], json!({ "uri": "spotify:track:0", "name": "T0", "artists": ["Nova Drift"], "album": "Album", "added_at": "2026-01-01T00:00:00Z" }));
        assert!(!f.asked.iter().any(|a| a.contains("theirs")), "followed playlists aren't fetched: {:?}", f.asked);

        // the next night: "mine" unchanged is copied, not fetched; once it changes, it's fetched again
        let mut again = Fake { asked: vec![], revision: "r1", trouble: None };
        let b2 = take(&mut again, "me", Some(&b), 8).await.unwrap();
        assert_eq!(b2["playlists"], b["playlists"]);
        assert!(!again.asked.iter().any(|a| a.contains("fetchPlaylist spotify:playlist:mine")), "{:?}", again.asked);
        let mut changed = Fake { asked: vec![], revision: "r2", trouble: None };
        take(&mut changed, "me", Some(&b), 9).await.unwrap();
        assert_eq!(changed.asked.iter().filter(|a| a.contains("fetchPlaylist spotify:playlist:mine")).count(), 2);
    }

    fn mine(b: &Value) -> &Value {
        b["playlists"].as_array().unwrap().iter().find(|p| p["uri"] == "spotify:playlist:mine").unwrap()
    }

    #[tokio::test]
    async fn a_page_that_comes_back_wrong_makes_an_incomplete_list_not_a_short_whole_one() {
        for trouble in [Trouble::Errors, Trouble::Empty] {
            // no earlier copy: what came back is kept, marked incomplete
            let mut f = Fake { asked: vec![], revision: "r1", trouble: Some(trouble) };
            let b = take(&mut f, "me", None, 7).await.unwrap();
            let m = mine(&b);
            assert_eq!(m["tracks"].as_array().unwrap().len(), 100);
            assert!(m["incomplete"].is_string(), "{m}");
            // the next night, same revision: the marked copy isn't carried forward but fetched again
            let mut next = Fake { asked: vec![], revision: "r1", trouble: None };
            let b2 = take(&mut next, "me", Some(&b), 8).await.unwrap();
            assert_eq!(next.asked.iter().filter(|a| a.contains("fetchPlaylist spotify:playlist:mine")).count(), 2);
            assert_eq!(mine(&b2)["tracks"].as_array().unwrap().len(), 150);
            assert!(mine(&b2).get("incomplete").is_none());

            // a whole earlier copy: kept, under its own revision so the next night tries again
            let mut bad = Fake { asked: vec![], revision: "r2", trouble: Some(trouble) };
            let b3 = take(&mut bad, "me", Some(&b2), 9).await.unwrap();
            assert_eq!(mine(&b3), mine(&b2));
            let mut fine = Fake { asked: vec![], revision: "r2", trouble: None };
            take(&mut fine, "me", Some(&b3), 10).await.unwrap();
            assert_eq!(fine.asked.iter().filter(|a| a.contains("fetchPlaylist spotify:playlist:mine")).count(), 2);
        }
    }

    #[tokio::test]
    async fn a_short_copy_from_before_isnt_carried_forward() {
        let mut f = Fake { asked: vec![], revision: "r1", trouble: None };
        let mut b = take(&mut f, "me", None, 7).await.unwrap();
        // a backup from before #144 that stopped early: no total, 100 of 150 tracks
        let m = b["playlists"].as_array_mut().unwrap().iter_mut().find(|p| p["uri"] == "spotify:playlist:mine").unwrap();
        m.as_object_mut().unwrap().remove("total");
        m["tracks"].as_array_mut().unwrap().truncate(100);
        let mut next = Fake { asked: vec![], revision: "r1", trouble: None };
        let b2 = take(&mut next, "me", Some(&b), 8).await.unwrap();
        assert_eq!(mine(&b2)["tracks"].as_array().unwrap().len(), 150);
        // or one whose length doesn't match the count it was saved with
        let mut b3 = b2.clone();
        b3["playlists"][0]["tracks"].as_array_mut().unwrap().truncate(100);
        let mut again = Fake { asked: vec![], revision: "r1", trouble: None };
        let b4 = take(&mut again, "me", Some(&b3), 9).await.unwrap();
        assert_eq!(mine(&b4)["tracks"].as_array().unwrap().len(), 150);
    }

    #[test]
    fn an_answer_without_a_list_isnt_an_empty_list() {
        let v = json!({ "errors": [{ "message": "x" }], "data": null });
        assert_eq!(library::stated_total("spotify:playlist:x", &v), None);
        assert_eq!(library::stated_total(LIKED, &json!({ "data": { "me": { "library": { "tracks": { "totalCount": 0, "items": [] } } } } })), Some(0));
    }
}
