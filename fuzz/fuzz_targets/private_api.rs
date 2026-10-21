//! Any JSON through every private-API and lyrics parser: no panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use singularityd::library as lib;

fuzz_target!(|data: &[u8]| {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) else { return };
    for uri in ["spotify:show:x", "spotify:album:x", "spotify:playlist:x", "spotify:collection"] {
        let _ = lib::parse_tracks(uri, &v);
        let _ = lib::parse_header(uri, &v);
        let _ = singularityd::lyrics::parse_spotify(uri, &v);
        let _ = singularityd::lyrics::parse_lrclib(uri, &v);
    }
    let _ = lib::parse_library(&v, 0, "me");
    let _ = lib::parse_backup_page(&v);
    let _ = lib::parse_track(&v);
    let _ = lib::episode_row(&v);
    let _ = lib::parse_show_header(&v);
    let _ = lib::parse_episode(&v);
    let _ = lib::parse_saved(&["a".into(), "b".into()], &v);
    let _ = lib::parse_search("q", &v);
    let _ = lib::parse_artist(&v);
    let _ = lib::book_subtitle(&v);
    let _ = singularityd::catalogue::parse_home(&v);
});
