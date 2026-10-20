//! (#71) Spotify's private API, LRCLIB and Clients are treated as hostile: random JSON trees built from the key names
//! the parsers look for, random LRC text and random frames must never panic. A stable stand-in for `cargo fuzz`.

use serde_json::{Map, Value, json};
use singularityd::library as lib;

/// xorshift: deterministic, so a failure reproduces from its seed
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Keys the parsers read (and some typenames), so random trees get deep into them.
const KEYS: &[&str] = &[
    "data", "me", "libraryV3", "items", "item", "_uri", "uri", "name", "__typename", "totalCount", "podcastUnionV2", "episodesV2", "chaptersV2", "entity",
    "playedState", "state", "playPositionMilliseconds", "duration", "totalMilliseconds", "playability", "playable", "searchV2", "tracksV2", "albumsV2",
    "artists", "profile", "albumOfTrack", "coverArt", "sources", "url", "width", "height", "trackUnion", "albumUnion", "playlistV2", "content", "itemV2",
    "track", "addedAt", "isoString", "uid", "ownerV2", "username", "authorsV2", "narrators", "accessInfo", "signifier", "text", "episodeUnionV2",
    "podcastV2", "audiobookV2", "artistUnion", "discography", "stats", "monthlyListeners", "visuals", "avatarImage", "lyrics", "lines", "startTimeMs",
    "words", "syncType", "syncedLyrics", "plainLyrics", "home", "sectionContainer", "sections", "sectionItems", "lookup", "saved", "description",
];
const STRINGS: &[&str] = &["Track", "Episode", "Chapter", "Audiobook", "Podcast", "Album", "Playlist", "Artist", "PseudoPlaylist", "LINE_SYNCED", "COMPLETED", "spotify:track:x", "spotify:show:x", "", "ü", "\u{1b}[2J", "9999999999999999999999"];

fn value(r: &mut Rng, depth: usize) -> Value {
    match r.below(if depth > 6 { 5 } else { 8 }) {
        0 => Value::Null,
        1 => Value::Bool(r.below(2) == 0),
        2 => json!(r.next() as i64 >> r.below(64)),
        3 => json!((r.next() as f64) / 7.0 - 1e18),
        4 => Value::String(STRINGS[r.below(STRINGS.len())].into()),
        5 => Value::Array((0..r.below(4)).map(|_| value(r, depth + 1)).collect()),
        _ => {
            let mut m = Map::new();
            for _ in 0..r.below(6) {
                m.insert(KEYS[r.below(KEYS.len())].into(), value(r, depth + 1));
            }
            Value::Object(m)
        }
    }
}

#[test]
fn private_api_parsers_never_panic_on_random_answers() {
    let mut r = Rng(0x5eed_1234_abcd_0071);
    for _ in 0..20_000 {
        let v = value(&mut r, 0);
        let uri = ["spotify:show:x", "spotify:album:x", "spotify:playlist:x", "spotify:collection", "x"][r.below(5)];
        let _ = lib::parse_library(&v, 0, "me");
        let _ = lib::parse_backup_page(&v);
        let _ = lib::parse_track(&v);
        let _ = lib::parse_tracks(uri, &v);
        let _ = lib::episode_row(&v);
        let _ = lib::parse_show_header(&v);
        let _ = lib::parse_episode(&v);
        let _ = lib::parse_header(uri, &v);
        let _ = lib::parse_saved(&["a".into(), "b".into()], &v);
        let _ = lib::parse_search("q", &v);
        let _ = lib::parse_artist(&v);
        let _ = lib::book_subtitle(&v);
        let _ = singularityd::catalogue::parse_home(&v);
        let _ = singularityd::lyrics::parse_spotify(uri, &v);
        let _ = singularityd::lyrics::parse_lrclib(uri, &v);
    }
}

#[test]
fn lrc_text_never_panics() {
    let mut r = Rng(0x00c0_ffee_0071);
    let pieces = ["[", "]", ":", ".", "00", "99999999999999999999", "-1", "1e308", "ü", "\n", " ", "words", "[00:12.34]", "[ar:x]"];
    for _ in 0..20_000 {
        let text: String = (0..r.below(30)).map(|_| pieces[r.below(pieces.len())]).collect();
        let _ = singularityd::lyrics::parse_lrc(&text);
    }
}

#[tokio::test]
async fn random_frames_never_panic_the_reader() {
    let mut r = Rng(0xf4a3_e5ee_0071);
    for _ in 0..5_000 {
        let n = r.below(64);
        let mut bytes: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
        // half of them carry a plausible small length prefix
        if r.below(2) == 0 && bytes.len() >= 4 {
            let len = (bytes.len() - 4 + r.below(3)) as u32;
            bytes[..4].copy_from_slice(&len.to_be_bytes());
        }
        let mut rd = bytes.as_slice();
        let _ = singularity_proto::read_msg::<_, singularity_proto::ClientMsg>(&mut rd).await;
        let mut rd = bytes.as_slice();
        let _ = singularity_proto::read_msg::<_, singularity_proto::DaemonMsg>(&mut rd).await;
    }
}
