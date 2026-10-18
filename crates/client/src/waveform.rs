//! The waveform seek bar (#46): the playing track's loudness over time, built up from the captured audio as it plays.
//! A track is cut into `SLOTS` stretches; each keeps the loudest level heard there (0 = not heard yet). Once a track
//! has been heard (nearly) to the end, its slots go to a small file in the cache, named by a hash of its URI, so the
//! next time it shows whole from the start.

use std::path::{Path, PathBuf};

pub const SLOTS: usize = 200;
/// Heard this much of a track and it's worth keeping.
const COMPLETE: usize = SLOTS * 95 / 100;
/// Waveforms kept in the cache (200 bytes each).
const MAX_FILES: usize = 2000;

#[derive(Default)]
pub struct Waveform {
    uri: String,
    duration_ms: u64,
    slots: Vec<u8>,
    saved: bool,
    dir: Option<PathBuf>,
}

impl Waveform {
    /// Kept in `dir` (no directory: nothing is kept, as in tests).
    pub fn with_cache(dir: PathBuf) -> Waveform {
        Waveform { dir: Some(dir), ..Default::default() }
    }

    /// The playing track; a new one starts from its cached waveform, or empty.
    pub fn follow(&mut self, uri: &str, duration_ms: u64) {
        if uri == self.uri && duration_ms == self.duration_ms {
            return;
        }
        self.keep();
        self.uri = uri.to_string();
        self.duration_ms = duration_ms;
        self.saved = false;
        self.slots = self.dir.as_ref().and_then(|d| std::fs::read(file(d, uri)).ok()).filter(|b| b.len() == SLOTS).unwrap_or_else(|| vec![0; SLOTS]);
        if self.slots.iter().filter(|v| **v > 0).count() >= COMPLETE {
            self.saved = true;
        }
    }

    /// The audio at `pos_ms` was this loud (0–255).
    pub fn hear(&mut self, pos_ms: u64, level: u8) {
        if self.duration_ms == 0 || self.slots.len() != SLOTS {
            return;
        }
        let i = ((pos_ms as u128 * SLOTS as u128) / self.duration_ms as u128).min(SLOTS as u128 - 1) as usize;
        // heard silence still counts as heard
        self.slots[i] = self.slots[i].max(level.max(1));
        if !self.saved && self.slots.iter().filter(|v| **v > 0).count() >= COMPLETE {
            self.keep();
        }
    }

    /// The loudest level heard over the fraction `from..to` of the track (0..1), if any of it was heard.
    pub fn level(&self, from: f64, to: f64) -> Option<u8> {
        if self.slots.len() != SLOTS {
            return None;
        }
        let a = ((from * SLOTS as f64) as usize).min(SLOTS - 1);
        let b = ((to * SLOTS as f64).ceil() as usize).clamp(a + 1, SLOTS);
        self.slots[a..b].iter().copied().filter(|v| *v > 0).max()
    }

    /// The quietest and loudest levels heard in the track, so a bar can use its own range (the audio's automatic gain
    /// keeps most music near the top).
    pub fn range(&self) -> Option<(u8, u8)> {
        let heard = self.slots.iter().copied().filter(|v| *v > 0);
        Some((heard.clone().min()?, heard.max()?))
    }

    /// Write a fully heard track's waveform to the cache (once), keeping the cache to `MAX_FILES`.
    fn keep(&mut self) {
        let Some(dir) = &self.dir else { return };
        if self.saved || self.uri.is_empty() || self.slots.iter().filter(|v| **v > 0).count() < COMPLETE {
            return;
        }
        self.saved = true;
        if std::fs::create_dir_all(dir).is_ok() && std::fs::write(file(dir, &self.uri), &self.slots).is_ok() {
            prune(dir, MAX_FILES);
        }
    }
}

/// The cache file for a URI: a stable hash of it (FNV-1a), so no URI characters reach the filesystem.
fn file(dir: &Path, uri: &str) -> PathBuf {
    let h = uri.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    dir.join(format!("{h:016x}"))
}

/// Drop the oldest files beyond `max`.
fn prune(dir: &Path, max: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= max {
        return;
    }
    files.sort();
    for (_, p) in files.iter().take(files.len() - max) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fills_as_the_track_plays_and_comes_back_whole() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Waveform::with_cache(dir.path().to_path_buf());
        w.follow("spotify:track:a", 200_000);
        assert_eq!(w.level(0.0, 1.0), None, "nothing heard yet");
        w.hear(0, 40);
        w.hear(500, 90);
        assert_eq!(w.level(0.0, 0.01), Some(90), "the loudest in the stretch");
        assert_eq!(w.level(0.5, 0.6), None);
        w.hear(100_000, 0);
        assert_eq!(w.level(0.5, 0.505), Some(1), "silence heard is still heard");
        assert_eq!(w.range(), Some((1, 90)));
        // play it through: kept in the cache
        for ms in (0..200_000).step_by(500) {
            w.hear(ms, (ms / 1000 % 200) as u8 + 10);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        // another track, then back: whole from the start
        w.follow("spotify:track:b", 180_000);
        assert_eq!(w.level(0.0, 1.0), None);
        w.follow("spotify:track:a", 200_000);
        assert!(w.level(0.9, 1.0).is_some(), "the end shows before it's played");
    }

    #[test]
    fn a_partly_heard_track_isnt_kept_and_the_cache_stays_small() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Waveform::with_cache(dir.path().to_path_buf());
        w.follow("spotify:track:a", 200_000);
        for ms in (0..100_000).step_by(500) {
            w.hear(ms, 50);
        }
        w.follow("spotify:track:b", 200_000);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "half a track isn't worth keeping");
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("{i:016x}")), [1u8; SLOTS]).unwrap();
        }
        prune(dir.path(), 3);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    }
}
