//! Library answers from Pathfinder → protocol types. Tolerant: unknown items are skipped, missing fields default.
//!
//! Shapes verified live on 2026-10-17 (`examples/probe.rs`): `libraryV3`, `fetchLibraryTracks` (Liked Songs),
//! `fetchPlaylist` and `getAlbum`.

use serde_json::{Value, json};
use singularity_proto::{LibraryItem, TrackRow};

/// Liked Songs as the Client names it; the Daemon swaps in `spotify:user:<name>:collection` to play it.
pub const LIKED: &str = "spotify:collection";

/// Page sizes per request: the web player's own limits.
pub const LIBRARY_PAGE: u32 = 50;
pub const TRACK_PAGE: u32 = 100;
pub const ALBUM_PAGE: u32 = 50;

pub fn library_vars(offset: u32, folder: Option<&str>) -> Value {
    json!({
        "filters": [], "order": null, "textFilter": "",
        "features": ["LIKED_SONGS", "YOUR_EPISODES_V2", "PRERELEASES", "PRERELEASES_V2", "EVENTS"],
        "limit": LIBRARY_PAGE, "offset": offset, "flatten": false, "expandedFolders": [],
        "folderUri": folder, "includeFoldersWhenFlattening": true,
    })
}

/// Which query lists the tracks of `uri`, with its variables for one page.
pub fn tracks_query(uri: &str, offset: u32) -> Option<(&'static str, Value)> {
    match uri.split(':').nth(1)? {
        _ if uri == LIKED => Some(("fetchLibraryTracks", json!({ "offset": offset, "limit": TRACK_PAGE }))),
        "playlist" => Some((
            "fetchPlaylist",
            json!({ "uri": uri, "offset": offset, "limit": TRACK_PAGE, "enableWatchFeedEntrypoint": false, "includeEpisodeContentRatingsV2": false }),
        )),
        "album" => Some(("getAlbum", json!({ "uri": uri, "locale": "", "offset": offset, "limit": ALBUM_PAGE }))),
        _ => None,
    }
}

fn image(sources: &Value) -> Option<String> {
    // ~300 px: plenty for a sidebar thumbnail or a card
    let list = sources.as_array()?;
    list.iter().min_by_key(|s| (s["width"].as_i64().unwrap_or(300) - 300).abs()).and_then(|s| s["url"].as_str().map(String::from))
}

fn names(items: &Value) -> Vec<String> {
    items.as_array().into_iter().flatten().filter_map(|a| a["profile"]["name"].as_str().map(String::from)).collect()
}

/// One `libraryV3` page: its items and the total count (folders' contents are fetched separately).
pub struct LibraryPage {
    pub items: Vec<LibraryItem>,
    pub total: u32,
}

pub fn parse_library(v: &Value, depth: u8) -> LibraryPage {
    let lib = &v["data"]["me"]["libraryV3"];
    let mut page = LibraryPage { items: vec![], total: lib["totalCount"].as_u64().unwrap_or(0) as u32 };
    for it in lib["items"].as_array().into_iter().flatten() {
        let d = &it["item"]["data"];
        let Some(uri) = d["uri"].as_str().or_else(|| it["item"]["_uri"].as_str()).map(String::from) else { continue };
        let (kind, name, subtitle, img) = match d["__typename"].as_str().unwrap_or("") {
            "PseudoPlaylist" if uri == LIKED => {
                let n = d["count"].as_u64().unwrap_or(0);
                ("liked", "Liked Songs".to_string(), format!("{n} songs"), image(&d["image"]["sources"]))
            }
            "Playlist" => (
                "playlist",
                d["name"].as_str().unwrap_or_default().to_string(),
                d["ownerV2"]["data"]["name"].as_str().map(|o| format!("Playlist · {o}")).unwrap_or_else(|| "Playlist".into()),
                image(&d["images"]["items"][0]["sources"]),
            ),
            "Album" => ("album", d["name"].as_str().unwrap_or_default().to_string(), names(&d["artists"]["items"]).join(", "), image(&d["coverArt"]["sources"])),
            "Artist" => ("artist", d["profile"]["name"].as_str().unwrap_or_default().to_string(), "Artist".into(), image(&d["visuals"]["avatarImage"]["sources"])),
            t if t.contains("Folder") => ("folder", d["name"].as_str().unwrap_or("Folder").to_string(), String::new(), None),
            _ => continue,
        };
        page.items.push(LibraryItem { uri, name, subtitle, kind: kind.into(), image_url: img, depth });
    }
    page
}

/// A `getTrack` answer → an Item (for queue entries Soloist sent without metadata).
pub fn parse_track(v: &Value) -> Option<singularity_proto::Item> {
    let t = &v["data"]["trackUnion"];
    let uri = t["uri"].as_str()?.to_string();
    let mut artists = names(&t["firstArtist"]["items"]);
    artists.extend(names(&t["otherArtists"]["items"]));
    Some(singularity_proto::Item {
        uri,
        name: t["name"].as_str().unwrap_or_default().to_string(),
        artists,
        album: t["albumOfTrack"]["name"].as_str().map(String::from),
        cover_url: image(&t["albumOfTrack"]["coverArt"]["sources"]),
        duration_ms: t["duration"]["totalMilliseconds"].as_u64().unwrap_or(0),
        explicit: t["contentRating"]["label"] == "EXPLICIT",
    })
}

/// One page of a track list, in any of the three shapes, plus the list's total length.
pub fn parse_tracks(uri: &str, v: &Value) -> (Vec<TrackRow>, u32) {
    let (items, total) = match uri.split(':').nth(1) {
        _ if uri == LIKED => {
            let t = &v["data"]["me"]["library"]["tracks"];
            (&t["items"], t["totalCount"].as_u64())
        }
        Some("playlist") => {
            let c = &v["data"]["playlistV2"]["content"];
            (&c["items"], c["totalCount"].as_u64())
        }
        Some("album") => {
            let t = &v["data"]["albumUnion"]["tracksV2"];
            (&t["items"], t["totalCount"].as_u64())
        }
        _ => return (vec![], 0),
    };
    let album_name = v["data"]["albumUnion"]["name"].as_str().unwrap_or_default();
    let rows = items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|it| {
            // Liked Songs: {track:{_uri,data}}, playlists: {itemV2:{data}}, albums: {track:{uri,…}}
            let (d, uri) = if let Some(d) = it["itemV2"]["data"].as_object() {
                (Value::Object(d.clone()), d.get("uri")?.as_str()?.to_string())
            } else if it["track"]["data"].is_object() {
                (it["track"]["data"].clone(), it["track"]["_uri"].as_str()?.to_string())
            } else {
                (it["track"].clone(), it["track"]["uri"].as_str()?.to_string())
            };
            if d["__typename"].as_str().is_some_and(|t| t != "Track") {
                return None; // episodes and local files: later
            }
            let ms = d["trackDuration"]["totalMilliseconds"].as_u64().or_else(|| d["duration"]["totalMilliseconds"].as_u64()).unwrap_or(0);
            Some(TrackRow {
                uri,
                name: d["name"].as_str().unwrap_or_default().to_string(),
                artists: names(&d["artists"]["items"]),
                album: d["albumOfTrack"]["name"].as_str().unwrap_or(album_name).to_string(),
                added_at: it["addedAt"]["isoString"].as_str().map(String::from),
                duration_ms: ms,
                playable: d["playability"]["playable"].as_bool().unwrap_or(true),
            })
        })
        .collect();
    (rows, total.unwrap_or(0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
    }

    #[test]
    fn library_items_kinds_and_folders() {
        let p = parse_library(&fixture("library.json"), 0);
        let kinds: Vec<&str> = p.items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["liked", "playlist", "folder", "album", "artist"]);
        assert_eq!(p.items[0].subtitle, "321 songs");
        assert_eq!(p.items[1].subtitle, "Playlist · Nova Drift");
        assert_eq!(p.items[2].uri, "spotify:user:someone:folder:f00");
        assert_eq!(p.total, 5);
        assert!(p.items[3].image_url.as_deref().unwrap().ends_with("300"));
    }

    #[test]
    fn three_track_shapes() {
        let (liked, n) = parse_tracks(LIKED, &fixture("liked.json"));
        assert_eq!((liked.len(), n), (2, 321));
        assert_eq!(liked[0].uri, "spotify:track:l0");
        assert_eq!(liked[0].album, "Event Horizon");
        assert_eq!(liked[0].duration_ms, 201_000);
        assert!(liked[0].added_at.is_some());

        let (pl, n) = parse_tracks("spotify:playlist:p", &fixture("playlist.json"));
        assert_eq!((pl.len(), n), (2, 100), "the episode is skipped");
        assert_eq!(pl[1].artists, ["Mira Vale", "Lumen Fields"]);
        assert!(!pl[1].playable);

        let (al, n) = parse_tracks("spotify:album:a", &fixture("album.json"));
        assert_eq!((al.len(), n), (2, 19));
        assert_eq!(al[0].album, "Quiet Orbit", "album tracks take the album's name");
        assert_eq!(al[1].duration_ms, 185_500);
    }

    #[test]
    fn get_track_fills_an_item() {
        let v = serde_json::json!({"data": {"trackUnion": {"__typename": "Track", "uri": "spotify:track:t", "name": "Perihelion",
            "firstArtist": {"items": [{"profile": {"name": "Lumen Fields"}}]}, "otherArtists": {"items": [{"profile": {"name": "Mira Vale"}}]},
            "albumOfTrack": {"name": "Quiet Orbit", "coverArt": {"sources": [{"url": "https://i.scdn.co/image/c300", "width": 300}]}},
            "duration": {"totalMilliseconds": 210000}, "contentRating": {"label": "NONE"}}}});
        let i = parse_track(&v).unwrap();
        assert_eq!((i.name.as_str(), i.artists.len(), i.duration_ms, i.album.as_deref()), ("Perihelion", 2, 210_000, Some("Quiet Orbit")));
    }

    #[test]
    fn queries_per_kind() {
        assert_eq!(tracks_query(LIKED, 0).unwrap().0, "fetchLibraryTracks");
        assert_eq!(tracks_query("spotify:playlist:x", 100).unwrap().1["offset"], 100);
        assert_eq!(tracks_query("spotify:album:x", 0).unwrap().0, "getAlbum");
        assert!(tracks_query("spotify:artist:x", 0).is_none());
    }
}
