//! Library answers from Pathfinder → protocol types. Tolerant: unknown items are skipped, missing fields default.
//!
//! Shapes verified live on 2026-10-17 (`examples/probe.rs`): `libraryV3`, `fetchLibraryTracks` (Liked Songs),
//! `fetchPlaylist` and `getAlbum`.

use serde_json::{Value, json};
use singularity_proto::{ArtistPage, Card, LibraryItem, Link, ListHeader, Shelf, TrackRow};

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

fn uris(items: &Value) -> Vec<String> {
    items.as_array().into_iter().flatten().filter_map(|a| a["uri"].as_str().map(String::from)).collect()
}

fn links(items: &Value) -> Vec<Link> {
    items.as_array().into_iter().flatten().filter_map(|a| Some(Link { name: a["profile"]["name"].as_str()?.into(), uri: a["uri"].as_str()?.into() })).collect()
}

/// "1 234 567": thousands grouped by thin-ish spaces, as the desktop app shows them.
pub fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
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
    let mut artist_uris = uris(&t["firstArtist"]["items"]);
    artist_uris.extend(uris(&t["otherArtists"]["items"]));
    Some(singularity_proto::Item {
        artist_uris,
        album_uri: t["albumOfTrack"]["uri"].as_str().map(String::from),
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
    let album_uri = v["data"]["albumUnion"]["uri"].as_str();
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
                artist_uris: uris(&d["artists"]["items"]),
                album_uri: d["albumOfTrack"]["uri"].as_str().or(album_uri).map(String::from),
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

/// What a list is (album: artists, year, label; playlist: owner, likes), from its first page.
pub fn parse_header(uri: &str, v: &Value) -> Option<ListHeader> {
    match uri.split(':').nth(1)? {
        "album" => {
            let a = &v["data"]["albumUnion"];
            let kind = match a["type"].as_str().unwrap_or("ALBUM") {
                "SINGLE" => "Single",
                "EP" => "EP",
                "COMPILATION" => "Compilation",
                _ => "Album",
            };
            let year = a["date"]["isoString"].as_str().and_then(|d| d.get(..4)).map(String::from).or_else(|| a["date"]["year"].as_i64().map(|y| y.to_string())).unwrap_or_default();
            let mut parts = vec![kind.to_string()];
            parts.extend([year, a["label"].as_str().unwrap_or("").to_string()].into_iter().filter(|s| !s.is_empty()));
            Some(ListHeader { subtitle: parts.join(" · "), artists: links(&a["artists"]["items"]), cover_url: image(&a["coverArt"]["sources"]) })
        }
        "playlist" => {
            let p = &v["data"]["playlistV2"];
            let mut parts = vec!["Playlist".to_string()];
            if let Some(o) = p["ownerV2"]["data"]["name"].as_str() {
                parts.push(o.into());
            }
            if let Some(f) = p["followers"].as_u64().filter(|f| *f > 0) {
                parts.push(format!("{} likes", grouped(f)));
            }
            Some(ListHeader { subtitle: parts.join(" · "), artists: vec![], cover_url: image(&p["images"]["items"][0]["sources"]) })
        }
        _ => None,
    }
}

/// A release (album, single…) as a Card: name, "2023 · Single", cover.
fn release_card(r: &Value) -> Option<Card> {
    let kind = match r["type"].as_str().unwrap_or("ALBUM") {
        "SINGLE" => "Single",
        "EP" => "EP",
        "COMPILATION" => "Compilation",
        _ => "Album",
    };
    let year = r["date"]["year"].as_i64().map(|y| format!("{y} · ")).unwrap_or_default();
    Some(Card { uri: r["uri"].as_str()?.into(), name: r["name"].as_str()?.into(), subtitle: format!("{year}{kind}"), image_url: image(&r["coverArt"]["sources"]), kind: "album".into() })
}

pub fn search_vars(query: &str) -> Value {
    json!({
        "searchTerm": query, "offset": 0, "limit": 20, "numberOfTopResults": 5,
        "includeAudiobooks": false, "includeArtistHasConcertsField": false, "includePreReleases": true,
        "includeLocalConcertsField": false, "includeAuthors": false,
    })
}

/// A search hit (`{data: …}` or `{item: {data: …}}`) as a Card, using the Home parser's shapes.
fn hit_card(it: &Value) -> Option<Card> {
    let d = if it["data"].is_object() { &it["data"] } else { &it["item"]["data"] };
    crate::catalogue::card(d)
}

/// `searchDesktop` → results by type.
pub fn parse_search(query: &str, v: &Value) -> singularity_proto::SearchResults {
    let s = &v["data"]["searchV2"];
    let cards = |k: &str, items: &str| s[k][items].as_array().into_iter().flatten().filter_map(hit_card).collect::<Vec<_>>();
    let tracks = s["tracksV2"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|it| {
            let t = &it["item"]["data"];
            (t["__typename"] == "Track").then_some(())?;
            Some(TrackRow {
                uri: t["uri"].as_str()?.into(),
                name: t["name"].as_str().unwrap_or_default().into(),
                artists: names(&t["artists"]["items"]),
                artist_uris: uris(&t["artists"]["items"]),
                album: t["albumOfTrack"]["name"].as_str().unwrap_or_default().into(),
                album_uri: t["albumOfTrack"]["uri"].as_str().map(String::from),
                added_at: None,
                duration_ms: t["duration"]["totalMilliseconds"].as_u64().unwrap_or(0),
                playable: t["playability"]["playable"].as_bool().unwrap_or(true),
            })
        })
        .collect();
    singularity_proto::SearchResults {
        query: query.into(),
        top: cards("topResultsV2", "itemsV2"),
        tracks,
        artists: cards("artists", "items"),
        albums: cards("albumsV2", "items"),
        playlists: cards("playlists", "items"),
        podcasts: cards("podcasts", "items"),
    }
}

/// `queryArtistOverview` → an artist page.
pub fn parse_artist(v: &Value) -> Option<ArtistPage> {
    let a = &v["data"]["artistUnion"];
    let uri = a["uri"].as_str()?.to_string();
    let d = &a["discography"];
    let top_tracks = d["topTracks"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|it| {
            let t = &it["track"];
            Some(TrackRow {
                uri: t["uri"].as_str()?.into(),
                name: t["name"].as_str().unwrap_or_default().into(),
                artists: names(&t["artists"]["items"]),
                artist_uris: uris(&t["artists"]["items"]),
                album: t["albumOfTrack"]["name"].as_str().unwrap_or_default().into(),
                album_uri: t["albumOfTrack"]["uri"].as_str().map(String::from),
                added_at: None,
                duration_ms: t["duration"]["totalMilliseconds"].as_u64().unwrap_or(0),
                playable: t["playability"]["playable"].as_bool().unwrap_or(true),
            })
        })
        .collect();
    // albums, singles and appearances come wrapped as {releases: {items: [release]}}
    let wrapped = |x: &Value| x["items"].as_array().into_iter().flatten().filter_map(|w| release_card(&w["releases"]["items"][0])).collect::<Vec<_>>();
    let plain = |x: &Value| x["items"].as_array().into_iter().flatten().filter_map(release_card).collect::<Vec<_>>();
    let related: Vec<Card> = a["relatedContent"]["relatedArtists"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| Some(Card { uri: r["uri"].as_str()?.into(), name: r["profile"]["name"].as_str()?.into(), subtitle: "Artist".into(), image_url: image(&r["visuals"]["avatarImage"]["sources"]), kind: "artist".into() }))
        .collect();
    let shelves = [
        ("Popular releases", plain(&d["popularReleasesAlbums"])),
        ("Albums", wrapped(&d["albums"])),
        ("Singles and EPs", wrapped(&d["singles"])),
        ("Appears on", wrapped(&a["relatedContent"]["appearsOn"])),
        ("Fans also like", related),
    ]
    .into_iter()
    .filter(|(_, c)| !c.is_empty())
    .map(|(t, cards)| Shelf { title: t.into(), kind: "shelf".into(), cards })
    .collect();
    Some(ArtistPage {
        uri,
        name: a["profile"]["name"].as_str().unwrap_or_default().into(),
        monthly_listeners: a["stats"]["monthlyListeners"].as_u64(),
        image_url: image(&a["visuals"]["avatarImage"]["sources"]),
        top_tracks,
        shelves,
    })
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

    /// Against real answers saved by `examples/probe.rs --raw` (never committed):
    /// `SGL_LIVE_ALBUM=… SGL_LIVE_ARTIST=… cargo test -p singularityd live_shapes -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_shapes() {
        let read = |k: &str| std::env::var(k).ok().map(|p| serde_json::from_str::<Value>(&std::fs::read_to_string(p).unwrap()).unwrap());
        if let Some(v) = read("SGL_LIVE_ALBUM") {
            let uri = v["data"]["albumUnion"]["uri"].as_str().unwrap().to_string();
            let h = parse_header(&uri, &v).unwrap();
            let (rows, _) = parse_tracks(&uri, &v);
            println!("album header: {:?} · {} artists · cover {} · rows with artist uris {}/{}", h.subtitle.split(" · ").next(), h.artists.len(), h.cover_url.is_some(), rows.iter().filter(|r| !r.artist_uris.is_empty()).count(), rows.len());
            assert!(!h.artists.is_empty() && rows.iter().all(|r| r.album_uri.is_some()));
        }
        if let Some(v) = read("SGL_LIVE_SEARCH") {
            let r = parse_search("q", &v);
            println!("search: top {} · tracks {} · artists {} · albums {} · playlists {} · podcasts {}", r.top.len(), r.tracks.len(), r.artists.len(), r.albums.len(), r.playlists.len(), r.podcasts.len());
            assert!(!r.tracks.is_empty() && !r.artists.is_empty() && !r.top.is_empty());
        }
        if let Some(v) = read("SGL_LIVE_ARTIST") {
            let p = parse_artist(&v).unwrap();
            println!("artist: {} top tracks · shelves {:?} · listeners {}", p.top_tracks.len(), p.shelves.iter().map(|s| (s.title.clone(), s.cards.len())).collect::<Vec<_>>(), p.monthly_listeners.is_some());
            assert!(!p.top_tracks.is_empty() && !p.shelves.is_empty());
        }
    }

    #[test]
    fn queries_per_kind() {
        assert_eq!(tracks_query(LIKED, 0).unwrap().0, "fetchLibraryTracks");
        assert_eq!(tracks_query("spotify:playlist:x", 100).unwrap().1["offset"], 100);
        assert_eq!(tracks_query("spotify:album:x", 0).unwrap().0, "getAlbum");
        assert!(tracks_query("spotify:artist:x", 0).is_none());
    }
}
