//! Nightly library backups (#23, D20): Liked Songs and every playlist the account owns, in full; playlists it only
//! follows as name + link. One private-API request at a time, a second apart, through the Catalogue's queue (so
//! browsing isn't held up); a playlist whose revision hasn't changed is copied from the previous backup instead of
//! fetched again. A list that doesn't come back whole (a page without its tracks, or fewer than Spotify says it has)
//! keeps its last whole copy with tonight's short one dated beside it, or without one is saved marked `incomplete`
//! (#144). Written by `store.rs`, never sent anywhere.

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

/// Every track of a list, page by page. A page that comes back without its list (an error, or a changed shape), or a
/// list that ends before its total, makes a short list, not a finished one (#144). Errors beside a whole page don't.
async fn tracks(f: &mut impl Fetch, uri: &str) -> anyhow::Result<Listed> {
    let mut rows = vec![];
    let mut offset = 0;
    let mut total = 0;
    loop {
        let Some((op, vars)) = library::tracks_query(uri, offset) else { anyhow::bail!("can't list {uri}") };
        let v = f.query(op, vars).await?;
        if library::stated_total(uri, &v).is_none() {
            let why = v["errors"][0]["message"].as_str().unwrap_or("not a track list");
            return Ok(Listed { rows, total, short: Some(format!("the page at {offset} came back without its tracks ({why})")) });
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

/// A list's fields tonight: `tracks` and `total`. When it came back short: with a whole copy from an earlier backup
/// (`last`: its tracks, total, and when it was fetched), that copy with `from_ms`, and tonight's short one beside it
/// in `short` (its `taken_ms`, `incomplete` saying why), so nothing whole is overwritten and nothing new is lost;
/// without one, tonight's marked `incomplete`.
fn settle(l: Listed, last: Option<(&Value, &Value, u64)>, now_ms: u64, what: &str) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    match (l.short, last) {
        (None, _) => {
            m.insert("tracks".into(), Value::Array(l.rows));
            m.insert("total".into(), json!(l.total));
        }
        (Some(why), Some((tracks, total, from_ms))) => {
            tracing::warn!("library backup: {what}: {why}; keeping the whole copy from before beside it");
            m.insert("tracks".into(), tracks.clone());
            m.insert("total".into(), total.clone());
            m.insert("from_ms".into(), json!(from_ms));
            m.insert("short".into(), json!({ "taken_ms": now_ms, "incomplete": why, "total": l.total, "tracks": l.rows }));
        }
        (Some(why), None) => {
            tracing::warn!("library backup: {what}: {why}; backed up as incomplete");
            m.insert("tracks".into(), Value::Array(l.rows));
            m.insert("total".into(), json!(l.total));
            m.insert("incomplete".into(), json!(why));
        }
    }
    m
}

/// The lists in backup `b` that came back short: Liked Songs and owned playlists, and how many of them kept their last
/// whole copy beside the short one (`short`) rather than being saved marked `incomplete`. `None` when all were whole.
pub fn short_lists(b: &Value) -> Option<singularity_proto::BackupShort> {
    let marks = std::iter::once((&b["liked_short"], &b["liked_incomplete"])).chain(b["playlists"].as_array().into_iter().flatten().map(|p| (&p["short"], &p["incomplete"])));
    let (mut lists, mut kept) = (0, 0);
    for (short, incomplete) in marks {
        if !short.is_null() {
            kept += 1;
        }
        if !short.is_null() || !incomplete.is_null() {
            lists += 1;
        }
    }
    (lists > 0).then_some(singularity_proto::BackupShort { lists, kept })
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
    // when the previous backup was taken: a copy it carried from further back says its own
    let then = previous.and_then(|p| p["taken_ms"].as_u64()).unwrap_or(0);
    // Liked Songs is every account's: only this account's own copy can stand in (a backup from before #144 names none)
    let liked_last = previous
        .filter(|p| p["account"].as_str() == Some(username))
        .filter(|p| whole(&p["liked"], &p["liked_total"], &p["liked_incomplete"]))
        .map(|p| (&p["liked"], &p["liked_total"], p["liked_from_ms"].as_u64().unwrap_or(then)));
    let liked = settle(tracks(f, LIKED).await?, liked_last, now_ms, "Liked Songs");
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
        let mut entry = json!({ "uri": p.uri, "name": p.name, "revision": p.revision });
        if let Some(e) = last.filter(|e| p.revision.is_some() && e["revision"].as_str() == p.revision.as_deref()) {
            entry["total"] = e["total"].clone();
            entry["tracks"] = e["tracks"].clone();
            owned.push(entry);
            continue;
        }
        let l = tracks(f, &p.uri).await?;
        let fields = settle(l, last.map(|e| (&e["tracks"], &e["total"], e["from_ms"].as_u64().unwrap_or(then))), now_ms, &p.uri);
        // a kept copy keeps its own revision, so the next backup fetches the list again
        if fields.contains_key("from_ms") {
            entry["revision"] = last.map(|e| e["revision"].clone()).unwrap_or_default();
        }
        entry.as_object_mut().expect("an object").extend(fields);
        owned.push(entry);
    }
    let mut b = json!({ "schema": 1, "taken_ms": now_ms, "account": username, "playlists": owned, "followed": followed });
    for (k, v) in liked {
        // Liked Songs' fields at the top: `liked`, `liked_total`, `liked_short`, …
        b[if k == "tracks" { "liked".to_string() } else { format!("liked_{k}") }] = v;
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
        let previous = {
            let mut s = store.lock().expect("store lock");
            let previous = s.latest_backup();
            // what came back short in it, learnt here rather than when the Daemon starts (#144); Clients are told
            if previous.as_ref().is_some_and(|(_, b)| s.note_backup(b))
                && let Ok(summary) = s.summary(now)
            {
                let _ = hub.events.send(singularity_proto::Event::Listening(summary));
            }
            previous
        };
        let due = previous.as_ref().is_none_or(|(at, _)| now.saturating_sub(*at) >= EVERY_MS);
        if let Some(user) = user.filter(|_| due) {
            tracing::info!("taking the nightly library backup");
            let started = Instant::now();
            match take(&mut fetch, &user, previous.as_ref().map(|(_, v)| v), now).await {
                Ok(b) => {
                    let mut s = store.lock().expect("store lock");
                    match s.write_backup(now, &b) {
                        Ok(path) => {
                            tracing::info!("library backup written to {} in {:.0?}", path.display(), started.elapsed());
                            // Clients learn whether it came out whole (Settings, #144)
                            if let Ok(summary) = s.summary(now) {
                                let _ = hub.events.send(singularity_proto::Event::Listening(summary));
                            }
                        }
                        Err(e) => tracing::warn!("couldn't write the library backup: {e}"),
                    }
                }
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
        /// What goes wrong: with "mine"'s second page, or with Liked Songs.
        trouble: Option<Trouble>,
    }

    #[derive(Clone, Copy)]
    enum Trouble {
        /// An answer with `errors` and no list.
        Errors,
        /// An empty page, as if the list ended there.
        Empty,
        /// `errors` beside a whole page (Spotify does that): not a short list.
        ErrorsBesideData,
        /// A song liked while Liked Songs is read: its count is one more than the rows that come back.
        LikedGrows,
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
                ("fetchLibraryTracks", ..) => json!({ "data": { "me": { "library": { "tracks": {
                    "totalCount": if matches!(self.trouble, Some(Trouble::LikedGrows)) { 3 } else { 2 }, "items": [
                    { "track": { "_uri": "spotify:track:l0", "data": { "name": "Liked", "artists": { "items": [] } } } },
                    { "track": { "_uri": "spotify:track:l1", "data": { "name": "Liked 2", "artists": { "items": [] } } } } ] } } } } }),
                // "mine" has 150 tracks: two pages
                ("fetchPlaylist", _, Some("spotify:playlist:mine")) => {
                    let o = vars["offset"].as_u64().unwrap() as usize;
                    let mut page = track_page((o..(o + 100).min(150)).map(track).collect(), 150);
                    match self.trouble.filter(|_| o > 0) {
                        Some(Trouble::Errors) => json!({ "errors": [{ "message": "Something went wrong" }], "data": null }),
                        Some(Trouble::Empty) => track_page(vec![], 150),
                        Some(Trouble::ErrorsBesideData) => {
                            page["errors"] = json!([{ "message": "a minor complaint" }]);
                            page
                        }
                        _ => page,
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

            // a whole earlier copy: kept, under its own revision so the next night tries again, with when it was
            // whole, and tonight's short one beside it, dated
            let mut bad = Fake { asked: vec![], revision: "r2", trouble: Some(trouble) };
            let b3 = take(&mut bad, "me", Some(&b2), 9).await.unwrap();
            let m = mine(&b3);
            assert_eq!((&m["tracks"], &m["total"], &m["revision"]), (&mine(&b2)["tracks"], &mine(&b2)["total"], &mine(&b2)["revision"]));
            assert_eq!(m["from_ms"], 8);
            assert_eq!((&m["short"]["taken_ms"], m["short"]["tracks"].as_array().unwrap().len()), (&json!(9), 100));
            assert!(m["short"]["incomplete"].is_string() && m.get("incomplete").is_none(), "{m}");
            // short again the night after: the whole copy stays (still dated 8), the short one is the newest
            let mut worse = Fake { asked: vec![], revision: "r2", trouble: Some(trouble) };
            let b4 = take(&mut worse, "me", Some(&b3), 10).await.unwrap();
            let m = mine(&b4);
            assert_eq!((m["tracks"].as_array().unwrap().len(), &m["from_ms"], &m["short"]["taken_ms"]), (150, &json!(8), &json!(10)));
            // whole again: tonight's copy, nothing kept beside it
            let mut fine = Fake { asked: vec![], revision: "r2", trouble: None };
            let b5 = take(&mut fine, "me", Some(&b4), 11).await.unwrap();
            assert_eq!(fine.asked.iter().filter(|a| a.contains("fetchPlaylist spotify:playlist:mine")).count(), 2);
            let m = mine(&b5);
            assert!(m.get("from_ms").is_none() && m.get("short").is_none() && m["revision"] == "r2", "{m}");
        }
    }

    #[tokio::test]
    async fn short_lists_counts_what_came_back_short_and_what_kept_a_whole_copy() {
        use singularity_proto::BackupShort;
        let mut f = Fake { asked: vec![], revision: "r1", trouble: None };
        let b = take(&mut f, "me", None, 7).await.unwrap();
        assert_eq!(short_lists(&b), None, "a whole backup");
        // "mine" short without an earlier copy: marked incomplete
        let mut bad = Fake { asked: vec![], revision: "r1", trouble: Some(Trouble::Errors) };
        let b2 = take(&mut bad, "me", None, 8).await.unwrap();
        assert_eq!(short_lists(&b2), Some(BackupShort { lists: 1, kept: 0 }));
        // "mine" short beside its whole copy, and Liked Songs too
        let mut worse = Fake { asked: vec![], revision: "r2", trouble: Some(Trouble::Errors) };
        let mut b3 = take(&mut worse, "me", Some(&b), 9).await.unwrap();
        b3["liked_incomplete"] = json!("only 2 of its 3 tracks came back");
        assert_eq!(short_lists(&b3), Some(BackupShort { lists: 2, kept: 1 }));
    }

    #[tokio::test]
    async fn errors_beside_a_whole_page_dont_make_it_short() {
        let mut f = Fake { asked: vec![], revision: "r1", trouble: Some(Trouble::ErrorsBesideData) };
        let b = take(&mut f, "me", None, 7).await.unwrap();
        let m = mine(&b);
        assert_eq!(m["tracks"].as_array().unwrap().len(), 150);
        assert!(m.get("incomplete").is_none() && m.get("short").is_none(), "{m}");
    }

    #[tokio::test]
    async fn a_like_during_the_backup_keeps_the_whole_liked_songs_and_the_new_one() {
        let mut f = Fake { asked: vec![], revision: "r1", trouble: None };
        let b = take(&mut f, "me", None, 7).await.unwrap();
        // the next night a song is liked while Liked Songs is read: 2 rows of 3
        let mut liking = Fake { asked: vec![], revision: "r1", trouble: Some(Trouble::LikedGrows) };
        let b2 = take(&mut liking, "me", Some(&b), 8).await.unwrap();
        assert_eq!((&b2["liked"], &b2["liked_total"], &b2["liked_from_ms"]), (&b["liked"], &json!(2), &json!(7)));
        assert_eq!((&b2["liked_short"]["taken_ms"], &b2["liked_short"]["total"]), (&json!(8), &json!(3)));
        assert!(b2["liked_short"]["incomplete"].is_string() && b2.get("liked_incomplete").is_none(), "{b2}");
        // without an earlier backup: tonight's, marked
        let mut first = Fake { asked: vec![], revision: "r1", trouble: Some(Trouble::LikedGrows) };
        let b3 = take(&mut first, "me", None, 9).await.unwrap();
        assert!(b3["liked_incomplete"].is_string() && b3.get("liked_short").is_none(), "{b3}");
    }

    #[tokio::test]
    async fn another_accounts_liked_songs_never_stand_in() {
        let mut f = Fake { asked: vec![], revision: "r1", trouble: None };
        let b = take(&mut f, "me", None, 7).await.unwrap();
        assert_eq!(b["account"], "me");
        // the next night another account is logged in, and its Liked Songs come back short: saved marked incomplete,
        // not "kept" from the first account's
        let mut other = Fake { asked: vec![], revision: "r1", trouble: Some(Trouble::LikedGrows) };
        let b2 = take(&mut other, "you", Some(&b), 8).await.unwrap();
        assert!(b2["liked_incomplete"].is_string() && b2.get("liked_short").is_none() && b2.get("liked_from_ms").is_none(), "{b2}");
        assert_eq!(b2["account"], "you");
        // nor does a backup from before #144, which names no account
        let mut unnamed = b.clone();
        unnamed.as_object_mut().unwrap().remove("account");
        let mut again = Fake { asked: vec![], revision: "r1", trouble: Some(Trouble::LikedGrows) };
        let b3 = take(&mut again, "me", Some(&unnamed), 9).await.unwrap();
        assert!(b3["liked_incomplete"].is_string() && b3.get("liked_short").is_none(), "{b3}");
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
