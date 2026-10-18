//! Stats follow the local calendar across a daylight-saving change. Its own test binary: it sets `TZ` for the process.

use singularity_proto::{Item, StatsPeriod};
use singularityd::store::{Play, Store, days_from_civil};

unsafe extern "C" {
    fn tzset();
}

/// Unix ms of a UTC date and time.
fn utc(y: i64, m: u32, d: u32, h: u64, min: u64) -> u64 {
    days_from_civil(y, m, d) as u64 * 86_400_000 + h * 3_600_000 + min * 60_000
}

#[test]
fn weeks_and_hours_follow_local_time_across_the_october_change() {
    // SAFETY: the only test in this binary, before any thread reads the environment
    unsafe {
        std::env::set_var("TZ", "Europe/Warsaw");
        tzset();
    }
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let rec = |at: u64, uri: &str, mins: u64| {
        let item = Item { uri: uri.into(), name: uri.into(), artists: vec!["Nova Drift".into()], duration_ms: 3_600_000, ..Default::default() };
        store.record(&Play { started_ms: at, item, context_uri: None, played_ms: mins * 60_000, device: "here".into() }).unwrap();
    };
    // 2026-10-25 is a Sunday; clocks go back at 03:00 CEST (UTC+2 → UTC+1)
    rec(utc(2026, 10, 24, 21, 30), "spotify:track:sat", 7); // Sat 23:30 CEST
    rec(utc(2026, 10, 25, 22, 30), "spotify:track:sun", 5); // Sun 23:30 CET
    rec(utc(2026, 10, 25, 23, 30), "spotify:track:mon", 3); // Mon 00:30 CET: the same UTC day, the next week
    let monday_noon = utc(2026, 10, 26, 11, 0);
    let w = store.stats(StatsPeriod::Week, monday_noon).unwrap();
    assert_eq!(w.from_day, days_from_civil(2026, 10, 26), "the week starts this Monday");
    assert_eq!((w.minutes, w.plays), (3, 1), "only Monday's play");
    assert_eq!(w.by_hour[0], 3, "00:30 local is hour 0");
    let sunday_late = utc(2026, 10, 25, 22, 59);
    let last = store.stats(StatsPeriod::Week, sunday_late).unwrap();
    assert_eq!(last.from_day, days_from_civil(2026, 10, 19));
    assert_eq!((last.minutes, last.by_hour[23]), (12, 12), "Saturday 23:30 CEST and Sunday 23:30 CET are both hour 23");
}
