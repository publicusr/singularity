//! Cover art: fetched lazily (only what's drawn) from Spotify's public image CDN, cached on disk under
//! `$XDG_CACHE_HOME/singularity/covers` (pruned to [`DISK_BUDGET`]), decoded off the UI thread, kept in memory LRU.

use image::RgbImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

const MEM_ENTRIES: usize = 400;
const DISK_BUDGET: u64 = 200 * 1024 * 1024;
const PARALLEL: usize = 4;

enum Entry {
    Loading,
    Ready(Arc<RgbImage>, u64),
    Failed,
}

pub struct Covers {
    mem: HashMap<String, Entry>,
    /// scaled copies for the cell tier, which would otherwise be resized every frame
    scaled: HashMap<(String, u32, u32), (Arc<RgbImage>, u64)>,
    requests: Option<mpsc::UnboundedSender<String>>,
    done: Option<mpsc::UnboundedReceiver<(String, Option<RgbImage>)>>,
    tick: u64,
}

impl Covers {
    /// No fetching: images only appear through [`Covers::insert`] (tests, snapshots).
    pub fn offline() -> Covers {
        Covers { mem: HashMap::new(), scaled: HashMap::new(), requests: None, done: None, tick: 0 }
    }

    /// Start the fetch workers (needs a tokio runtime).
    pub fn spawn(dir: PathBuf) -> Covers {
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<String>();
        let (done_tx, done_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let _ = std::fs::create_dir_all(&dir);
            let dir2 = dir.clone();
            tokio::task::spawn_blocking(move || prune(&dir2, DISK_BUDGET));
            let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().ok();
            let gate = Arc::new(tokio::sync::Semaphore::new(PARALLEL));
            while let Some(url) = req_rx.recv().await {
                let (dir, http, done, gate) = (dir.clone(), http.clone(), done_tx.clone(), gate.clone());
                tokio::spawn(async move {
                    let _permit = gate.acquire().await;
                    let img = load(&dir, http.as_ref(), &url).await;
                    let _ = done.send((url, img));
                });
            }
        });
        Covers { mem: HashMap::new(), scaled: HashMap::new(), requests: Some(req_tx), done: Some(done_rx), tick: 0 }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn insert(&mut self, url: &str, img: RgbImage) {
        self.mem.insert(url.into(), Entry::Ready(Arc::new(img), self.tick));
    }

    /// Take in finished downloads; returns true if anything new can be drawn.
    pub fn poll(&mut self) -> bool {
        self.tick += 1;
        let mut fresh = false;
        while let Some((url, img)) = self.done.as_mut().and_then(|d| d.try_recv().ok()) {
            self.mem.insert(url, img.map(|i| Entry::Ready(Arc::new(i), self.tick)).unwrap_or(Entry::Failed));
            fresh = true;
        }
        if self.mem.len() > MEM_ENTRIES {
            let mut ready: Vec<(u64, String)> = self.mem.iter().filter_map(|(k, e)| if let Entry::Ready(_, t) = e { Some((*t, k.clone())) } else { None }).collect();
            ready.sort();
            for (_, k) in ready.into_iter().take(self.mem.len() - MEM_ENTRIES) {
                self.mem.remove(&k);
            }
        }
        if self.scaled.len() > MEM_ENTRIES {
            let cutoff = self.tick.saturating_sub(300);
            self.scaled.retain(|_, (_, t)| *t >= cutoff);
        }
        fresh
    }

    /// The cover if it's ready; otherwise starts fetching it (once) and returns None.
    pub fn get(&mut self, url: &str) -> Option<Arc<RgbImage>> {
        match self.mem.get_mut(url) {
            Some(Entry::Ready(img, t)) => {
                *t = self.tick;
                Some(img.clone())
            }
            Some(_) => None,
            None => {
                if let Some(tx) = &self.requests
                    && url.starts_with("https://")
                    && tx.send(url.to_string()).is_ok()
                {
                    self.mem.insert(url.into(), Entry::Loading);
                }
                None
            }
        }
    }

    /// The cover cropped and scaled to exactly `w`×`h` pixels (memoised).
    pub fn scaled(&mut self, url: &str, w: u32, h: u32) -> Option<Arc<RgbImage>> {
        let key = (url.to_string(), w, h);
        if let Some((img, t)) = self.scaled.get_mut(&key) {
            *t = self.tick;
            return Some(img.clone());
        }
        let src = self.get(url)?;
        let img = Arc::new(crate::gfx::fit(&src, w, h));
        self.scaled.insert(key, (img.clone(), self.tick));
        Some(img)
    }
}

/// Cache file for a URL: a stable hash of it (FNV-1a), so no URL characters reach the filesystem.
fn cache_file(dir: &Path, url: &str) -> PathBuf {
    let h = url.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    dir.join(format!("{h:016x}"))
}

async fn load(dir: &Path, http: Option<&reqwest::Client>, url: &str) -> Option<RgbImage> {
    let path = cache_file(dir, url);
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => {
            let resp = http?.get(url).send().await.ok()?.error_for_status().ok()?;
            let b = resp.bytes().await.ok()?.to_vec();
            // write-then-rename, so a half-written file is never read back as a cover
            let tmp = path.with_extension("part");
            if tokio::fs::write(&tmp, &b).await.is_ok() {
                let _ = tokio::fs::rename(&tmp, &path).await;
            }
            b
        }
    };
    tokio::task::spawn_blocking(move || image::load_from_memory(&bytes).ok().map(|i| i.to_rgb8())).await.ok()?
}

/// Delete the least recently modified cache files until the directory fits the budget.
fn prune(dir: &Path, budget: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, p) in files {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_the_newest_files_within_budget() {
        let dir = std::env::temp_dir().join(format!("sgl-covers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..5 {
            std::fs::write(dir.join(format!("f{i}")), vec![0u8; 100]).unwrap();
            let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i);
            std::fs::File::options().write(true).open(dir.join(format!("f{i}"))).unwrap().set_modified(t).unwrap();
        }
        prune(&dir, 250);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["f3", "f4"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cache_files_are_stable_and_opaque() {
        let d = Path::new("/c");
        assert_eq!(cache_file(d, "https://i.scdn.co/image/ab67"), cache_file(d, "https://i.scdn.co/image/ab67"));
        assert_ne!(cache_file(d, "https://i.scdn.co/image/a"), cache_file(d, "https://i.scdn.co/image/b"));
        assert!(!cache_file(d, "https://x/../../etc").to_string_lossy().contains(".."));
    }

    #[test]
    fn offline_covers_come_from_insert_and_scale_once() {
        let mut c = Covers::offline();
        assert!(c.get("https://i.scdn.co/image/x").is_none());
        c.insert("https://i.scdn.co/image/x", RgbImage::new(300, 300));
        let a = c.scaled("https://i.scdn.co/image/x", 8, 8).unwrap();
        let b = c.scaled("https://i.scdn.co/image/x", 8, 8).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
