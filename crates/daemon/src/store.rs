//! LocalStore (#23, D20): the private listening log (SQLite) and the nightly library backups (JSON), under
//! `$XDG_DATA_HOME/singularity/`. Owner-only files, never uploaded: nothing here touches the network (backups are
//! fetched by `backup.rs` and only written here). Export writes one JSON file to the Downloads folder; delete removes
//! the log and every backup.

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use singularity_proto::{Item, LISTENING_DAYS, ListeningSummary, PlayStatus, PlaybackState};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const DB: &str = "listening.db";
const BACKUPS: &str = "backups";
/// Nightly backups kept as they are; older ones survive only as the first of their month, for a year.
const KEEP_DAYS: i64 = 14;
const KEEP_MONTHS: i64 = 12;

/// One logged play.
#[derive(Debug, Clone, PartialEq)]
pub struct Play {
    pub started_ms: u64,
    pub item: Item,
    pub context_uri: Option<String>,
    pub played_ms: u64,
    /// where it played: this computer's Engine, or the Connect device Singularity followed
    pub device: String,
}

pub struct Store {
    dir: PathBuf,
    db: Connection,
}

impl Store {
    /// Open (or create) the store in `dir` (normally `$XDG_DATA_HOME/singularity`).
    pub fn open(dir: &Path) -> anyhow::Result<Store> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(DB);
        let db = Connection::open(&path)?;
        // owner-only, also for a file an older run left behind; SQLite's journal copies these permissions
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS plays (
                id INTEGER PRIMARY KEY,
                started_ms INTEGER NOT NULL,
                day INTEGER NOT NULL,
                track_uri TEXT NOT NULL,
                name TEXT NOT NULL,
                artists TEXT NOT NULL,
                album TEXT,
                context_uri TEXT,
                duration_ms INTEGER NOT NULL,
                played_ms INTEGER NOT NULL,
                device TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS plays_day ON plays(day);
            PRAGMA user_version = 1;",
        )?;
        Ok(Store { dir: dir.to_path_buf(), db })
    }

    /// The default place: `$XDG_DATA_HOME/singularity`.
    pub fn default_dir() -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("singularity")
    }

    pub fn record(&self, p: &Play) -> anyhow::Result<()> {
        self.db.execute(
            "INSERT INTO plays (started_ms, day, track_uri, name, artists, album, context_uri, duration_ms, played_ms, device)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                p.started_ms as i64,
                local_day(p.started_ms),
                p.item.uri,
                p.item.name,
                serde_json::to_string(&p.item.artists)?,
                p.item.album,
                p.context_uri,
                p.item.duration_ms as i64,
                p.played_ms as i64,
                p.device
            ],
        )?;
        Ok(())
    }

    /// The listening Pane's numbers, as of `now_ms`.
    pub fn summary(&self, now_ms: u64) -> anyhow::Result<ListeningSummary> {
        let today = local_day(now_ms);
        let first = today - LISTENING_DAYS as i64 + 1;
        let mut minutes = vec![0u32; LISTENING_DAYS];
        let mut q = self.db.prepare("SELECT day, SUM(played_ms) FROM plays WHERE day >= ?1 AND day <= ?2 GROUP BY day")?;
        for r in q.query_map(params![first, today], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (day, ms) = r?;
            minutes[(day - first) as usize] = (ms / 60_000) as u32;
        }
        let (y, _, _) = civil(today);
        let year_start = days_from_civil(y, 1, 1);
        let year_ms: i64 = self.db.query_row("SELECT COALESCE(SUM(played_ms), 0) FROM plays WHERE day >= ?1", [year_start], |r| r.get(0))?;
        let top_artist = self
            .db
            .query_row(
                "SELECT json_extract(artists, '$[0]') AS a FROM plays WHERE day >= ?1 AND a IS NOT NULL GROUP BY a ORDER BY SUM(played_ms) DESC LIMIT 1",
                [year_start],
                |r| r.get::<_, String>(0),
            )
            .ok();
        let mut q = self.db.prepare("SELECT DISTINCT day FROM plays WHERE day <= ?1 ORDER BY day DESC")?;
        let days: Vec<i64> = q.query_map([today], |r| r.get(0))?.collect::<Result<_, _>>()?;
        let plays: i64 = self.db.query_row("SELECT COUNT(*) FROM plays", [], |r| r.get(0))?;
        Ok(ListeningSummary {
            minutes,
            today_weekday: weekday(today),
            hours_this_year: year_ms as f32 / 3_600_000.0,
            top_artist,
            streak_days: streak(&days, today),
            plays: plays as u64,
        })
    }

    fn plays_json(&self) -> anyhow::Result<Vec<Value>> {
        let mut q = self.db.prepare("SELECT started_ms, track_uri, name, artists, album, context_uri, duration_ms, played_ms, device FROM plays ORDER BY started_ms")?;
        let rows = q.query_map([], |r| {
            Ok(json!({
                "started_ms": r.get::<_, i64>(0)?,
                "track_uri": r.get::<_, String>(1)?,
                "name": r.get::<_, String>(2)?,
                "artists": serde_json::from_str::<Value>(&r.get::<_, String>(3)?).unwrap_or(Value::Null),
                "album": r.get::<_, Option<String>>(4)?,
                "context_uri": r.get::<_, Option<String>>(5)?,
                "duration_ms": r.get::<_, i64>(6)?,
                "played_ms": r.get::<_, i64>(7)?,
                "device": r.get::<_, String>(8)?,
            }))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // ── backups ───────────────────────────────────────────────────────

    fn backups_dir(&self) -> anyhow::Result<PathBuf> {
        let d = self.dir.join(BACKUPS);
        std::fs::create_dir_all(&d)?;
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
        Ok(d)
    }

    /// Backups on disk, oldest first: (day, path).
    fn backups(&self) -> Vec<(i64, PathBuf)> {
        let mut v: Vec<(i64, PathBuf)> = std::fs::read_dir(self.dir.join(BACKUPS))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let date = name.strip_prefix("library-")?.strip_suffix(".json")?;
                Some((parse_date(date)?, e.path()))
            })
            .collect();
        v.sort();
        v
    }

    /// The newest backup, and when it was written (Unix ms).
    pub fn latest_backup(&self) -> Option<(u64, Value)> {
        let (_, path) = self.backups().pop()?;
        let at = std::fs::metadata(&path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
        Some((at, serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?))
    }

    /// Write today's backup (replacing one already taken today), then drop the ones retention doesn't keep.
    pub fn write_backup(&self, now_ms: u64, backup: &Value) -> anyhow::Result<PathBuf> {
        let today = local_day(now_ms);
        let path = self.backups_dir()?.join(format!("library-{}.json", date(today)));
        write_private(&path, &serde_json::to_vec_pretty(backup)?)?;
        let days: Vec<i64> = self.backups().iter().map(|(d, _)| *d).collect();
        let keep = retained(&days, today);
        for (d, p) in self.backups() {
            if !keep.contains(&d) {
                let _ = std::fs::remove_file(p);
            }
        }
        Ok(path)
    }

    // ── export / delete ───────────────────────────────────────────────

    /// The whole log and the latest backup in one file in `to` (normally the Downloads folder).
    pub fn export(&self, to: &Path, now_ms: u64) -> anyhow::Result<PathBuf> {
        let doc = json!({
            "exported_ms": now_ms,
            "plays": self.plays_json()?,
            "library_backup": self.latest_backup().map(|(_, v)| v),
        });
        std::fs::create_dir_all(to)?;
        let path = to.join(format!("singularity-listening-{}.json", date(local_day(now_ms))));
        write_private(&path, &serde_json::to_vec_pretty(&doc)?)?;
        Ok(path)
    }

    /// Forget every play and remove every backup.
    pub fn delete_all(&self) -> anyhow::Result<()> {
        self.db.execute("DELETE FROM plays", [])?;
        // free the pages too, so nothing lingers in the file
        self.db.execute_batch("VACUUM")?;
        match std::fs::remove_dir_all(self.dir.join(BACKUPS)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

/// Write `data` to `path` with owner-only permissions, atomically (temp file + rename).
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// The Downloads folder: `$XDG_DOWNLOAD_DIR`, else `user-dirs.dirs`, else `~/Downloads` (or home if that's missing).
pub fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let expand = |v: &str| PathBuf::from(v.trim_matches('"').replace("$HOME", &home.to_string_lossy()));
    if let Some(v) = std::env::var_os("XDG_DOWNLOAD_DIR") {
        return expand(&v.to_string_lossy());
    }
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(config.join("user-dirs.dirs"))
        && let Some(v) = text.lines().find_map(|l| l.trim().strip_prefix("XDG_DOWNLOAD_DIR="))
    {
        return expand(v);
    }
    Some(home.join("Downloads")).filter(|d| d.is_dir()).unwrap_or(home)
}

// ── what counts as a play ─────────────────────────────────────────────

/// Follows playback and says when a track has been played: at least 30 s of it (half, if it's shorter), logged
/// when the track changes or playback stops.
#[derive(Default)]
pub struct Recorder {
    cur: Option<Current>,
}

struct Current {
    item: Item,
    context_uri: Option<String>,
    device: String,
    started_ms: u64,
    played_ms: u64,
    /// playing since (Unix ms); None while paused
    since: Option<u64>,
}

impl Recorder {
    pub fn observe(&mut self, s: &PlaybackState, device: &str, now_ms: u64) -> Option<Play> {
        if let Some(c) = self.cur.as_mut()
            && let Some(t) = c.since.take()
        {
            c.played_ms += now_ms.saturating_sub(t);
        }
        let same = matches!((&self.cur, &s.item), (Some(c), Some(i)) if c.item.uri == i.uri) && s.status != PlayStatus::Idle;
        if same {
            let c = self.cur.as_mut().expect("same track");
            c.since = (s.status == PlayStatus::Playing).then_some(now_ms);
            return None;
        }
        let done = self.cur.take().and_then(Current::finish);
        if let Some(item) = s.item.clone().filter(|_| s.status != PlayStatus::Idle) {
            self.cur = Some(Current {
                item,
                context_uri: s.context_uri.clone(),
                device: device.into(),
                started_ms: now_ms,
                played_ms: 0,
                since: (s.status == PlayStatus::Playing).then_some(now_ms),
            });
        }
        done
    }
}

impl Current {
    fn finish(self) -> Option<Play> {
        let d = self.item.duration_ms;
        let played = if d > 0 { self.played_ms.min(d) } else { self.played_ms };
        let needed = if d > 0 { (d / 2).min(30_000) } else { 30_000 };
        (played >= needed).then_some(Play { started_ms: self.started_ms, item: self.item, context_uri: self.context_uri, played_ms: played, device: self.device })
    }
}

// ── calendar (local days, no time-zone crate) ─────────────────────────

/// The local calendar day of a Unix-ms instant, as days since 1970-01-01.
pub fn local_day(ms: u64) -> i64 {
    let secs = (ms / 1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r only writes the tm we pass
    let off = if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() { 0 } else { tm.tm_gmtoff };
    (secs as i64 + off).div_euclid(86_400)
}

/// 0 = Monday (1970-01-01 was a Thursday).
pub fn weekday(day: i64) -> u8 {
    (day + 3).rem_euclid(7) as u8
}

/// Days since 1970-01-01 → (year, month, day). (H. Hinnant's algorithm.)
pub fn civil(day: i64) -> (i64, u32, u32) {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn date(day: i64) -> String {
    let (y, m, d) = civil(day);
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_date(s: &str) -> Option<i64> {
    let mut it = s.split('-');
    let (y, m, d) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    Some(days_from_civil(y, m, d))
}

/// Days in a row with listening, ending today, or yesterday when nothing has played yet today. `days`: newest first.
fn streak(days: &[i64], today: i64) -> u32 {
    let Some(&first) = days.first().filter(|d| **d >= today - 1) else { return 0 };
    days.iter().enumerate().take_while(|(i, d)| **d == first - *i as i64).count() as u32
}

/// Which backup days to keep: the last `KEEP_DAYS` days, plus the first backup of each of the last `KEEP_MONTHS`
/// months. `days`: any order.
fn retained(days: &[i64], today: i64) -> Vec<i64> {
    let (ty, tm, _) = civil(today);
    let month_no = |d: i64| {
        let (y, m, _) = civil(d);
        y * 12 + m as i64
    };
    let now = ty * 12 + tm as i64;
    let mut sorted = days.to_vec();
    sorted.sort();
    let mut keep = vec![];
    let mut months = std::collections::HashSet::new();
    for d in sorted {
        let first_of_month = months.insert(month_no(d));
        if d > today - KEEP_DAYS || (first_of_month && now - month_no(d) < KEEP_MONTHS) {
            keep.push(d);
        }
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(uri: &str, ms: u64) -> Item {
        Item { uri: uri.into(), name: uri.into(), artists: vec!["A".into()], duration_ms: ms, ..Default::default() }
    }

    fn state(status: PlayStatus, uri: Option<&str>, ms: u64) -> PlaybackState {
        PlaybackState { status, item: uri.map(|u| item(u, ms)), ..Default::default() }
    }

    #[test]
    fn a_play_counts_after_30_s_or_half_a_short_track() {
        let mut r = Recorder::default();
        assert_eq!(r.observe(&state(PlayStatus::Playing, Some("a"), 200_000), "here", 0), None);
        // 29 s of a, then b: not a play
        let p = r.observe(&state(PlayStatus::Playing, Some("b"), 40_000), "here", 29_000);
        assert_eq!(p, None);
        // 20 s of a 40 s track: half of it, so it counts, logged when playback stops
        let p = r.observe(&state(PlayStatus::Idle, None, 0), "here", 49_000).expect("half of a short track");
        assert_eq!((p.item.uri.as_str(), p.played_ms, p.started_ms), ("b", 20_000, 29_000));
    }

    #[test]
    fn paused_time_does_not_count_and_the_device_is_kept() {
        let mut r = Recorder::default();
        r.observe(&state(PlayStatus::Playing, Some("a"), 200_000), "Kitchen", 0);
        r.observe(&state(PlayStatus::Paused, Some("a"), 200_000), "Kitchen", 20_000);
        // paused for an hour, then 15 s more
        r.observe(&state(PlayStatus::Playing, Some("a"), 200_000), "Kitchen", 3_620_000);
        let p = r.observe(&state(PlayStatus::Playing, Some("b"), 200_000), "Kitchen", 3_635_000).unwrap();
        assert_eq!((p.played_ms, p.device.as_str()), (35_000, "Kitchen"));
    }

    #[test]
    fn calendar_round_trips_and_knows_weekdays() {
        for d in [-1000, 0, 59, 365, 11_016, 20_744, 30_000] {
            let (y, m, dd) = civil(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(civil(20_744), (2026, 10, 18));
        assert_eq!(weekday(20_744), 6, "2026-10-18 is a Sunday");
        assert_eq!(parse_date("2026-10-18"), Some(20_744));
    }

    #[test]
    fn streaks_end_today_or_yesterday() {
        assert_eq!(streak(&[10, 9, 8, 6], 10), 3);
        assert_eq!(streak(&[9, 8], 10), 2, "nothing yet today keeps yesterday's streak");
        assert_eq!(streak(&[8, 7], 10), 0);
        assert_eq!(streak(&[], 10), 0);
    }

    #[test]
    fn retention_keeps_two_weeks_and_monthly_firsts_for_a_year() {
        let today = days_from_civil(2026, 10, 18);
        let all: Vec<i64> = (today - 500..=today).collect();
        let keep = retained(&all, today);
        assert!((today - 13..=today).all(|d| keep.contains(&d)));
        assert!(!keep.contains(&(today - 14)));
        for (y, m) in [(2026, 9), (2026, 1), (2025, 11)] {
            assert!(keep.contains(&days_from_civil(y, m, 1)), "{y}-{m}");
            assert!(!keep.contains(&days_from_civil(y, m, 2)));
        }
        assert!(!keep.contains(&days_from_civil(2025, 10, 1)), "a year ago is gone");
        assert_eq!(keep.len(), 14 + 12, "two weeks, and the 1st of November 2025 … October 2026");
    }

    #[test]
    fn the_store_sums_up_exports_and_deletes_with_private_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let now = 1_792_000_000_000u64; // 2026-10-14
        let day = 86_400_000;
        for (i, (back, artist, mins)) in [(0, "Nova Drift", 30), (1, "Nova Drift", 20), (1, "Mira Vale", 10), (3, "Mira Vale", 5)].into_iter().enumerate() {
            let mut it = item(&format!("spotify:track:{i}"), 3_600_000);
            it.artists = vec![artist.into()];
            store.record(&Play { started_ms: now - back * day, item: it, context_uri: None, played_ms: mins * 60_000, device: "here".into() }).unwrap();
        }
        let s = store.summary(now).unwrap();
        assert_eq!(s.minutes.len(), LISTENING_DAYS);
        assert_eq!(&s.minutes[LISTENING_DAYS - 4..], &[5, 0, 30, 30]);
        assert_eq!((s.top_artist.as_deref(), s.streak_days, s.plays), (Some("Nova Drift"), 2, 4));
        assert!((s.hours_this_year - 65.0 / 60.0).abs() < 1e-4);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.path().join(DB)), 0o600);

        let b = store.write_backup(now, &json!({ "liked": [] })).unwrap();
        assert_eq!((mode(&b), mode(b.parent().unwrap())), (0o600, 0o700));
        let out = dir.path().join("downloads");
        let e = store.export(&out, now).unwrap();
        assert_eq!(mode(&e), 0o600);
        let doc: Value = serde_json::from_slice(&std::fs::read(&e).unwrap()).unwrap();
        assert_eq!(doc["plays"].as_array().unwrap().len(), 4);
        assert_eq!(doc["library_backup"], json!({ "liked": [] }));

        store.delete_all().unwrap();
        let s = store.summary(now).unwrap();
        assert_eq!((s.plays, s.minutes.iter().sum::<u32>()), (0, 0));
        assert!(store.latest_backup().is_none() && !dir.path().join(BACKUPS).exists());
    }
}
