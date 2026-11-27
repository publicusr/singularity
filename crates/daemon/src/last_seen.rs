//! The last Home and library, kept on disk (#81) so a fresh Daemon shows them to the first Client at once while the
//! Catalogue logs in and fetches them again behind; the fresh ones replace them as they arrive.
//!
//! The file is the account's own data: it lives in the user's cache dir (App folders: `~/.cache/singularity`, or
//! `$XDG_CACHE_HOME/singularity`), owner-only, and goes when the account logs out. A file this build can't read
//! (an older shape, a broken write) is treated as none.

use singularity_proto::{HomeFeed, Library};
use std::path::{Path, PathBuf};

/// What the file holds.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Seen {
    pub home: Option<HomeFeed>,
    pub library: Option<Library>,
}

/// The file, and which of the Daemon's Home and library still came from it.
pub struct LastSeen {
    path: PathBuf,
    from_disk: std::sync::Mutex<(bool, bool)>,
}

impl LastSeen {
    /// The usual place: `home-and-library.json` in the user's cache dir.
    pub fn default_path() -> PathBuf {
        singularity_config::app_folders::AppFolders::this().cache.join("home-and-library.json")
    }

    pub fn at(path: PathBuf) -> LastSeen {
        LastSeen { path, from_disk: Default::default() }
    }

    /// What the last Daemon saw; nothing when there's no file or it can't be read.
    pub fn read(&self) -> Seen {
        let began = std::time::Instant::now();
        let seen: Seen = std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        *self.from_disk.lock().expect("from_disk lock") = (seen.home.is_some(), seen.library.is_some());
        if seen.home.is_some() || seen.library.is_some() {
            tracing::info!("timing: the last Home and library read from disk in {} ms", began.elapsed().as_millis());
        }
        seen
    }

    /// Keep these as the last seen. A fresh Home or library no longer counts as from disk.
    pub fn write(&self, seen: &Seen, fresh_home: bool) {
        {
            let mut from = self.from_disk.lock().expect("from_disk lock");
            if fresh_home {
                from.0 = false;
            } else {
                from.1 = false;
            }
        }
        if let Err(e) = write_private(&self.path, &serde_json::to_vec(seen).unwrap_or_default()) {
            tracing::warn!("couldn't keep the Home and library for the next start: {e}");
        }
    }

    /// Logged out: the file goes. Returns which of Home and the library the Daemon holds are still the disk copies
    /// (they belong to the account that left, and go too).
    pub fn forget(&self) -> (bool, bool) {
        let _ = std::fs::remove_file(&self.path);
        std::mem::take(&mut *self.from_disk.lock().expect("from_disk lock"))
    }
}

/// Write `bytes` to `path` owner-only, all at once (a crash mid-write leaves the old file). Not synced: it is a cache.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.part");
    let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::{LibraryItem, Shelf};

    fn seen() -> Seen {
        let item = LibraryItem { uri: "spotify:playlist:p".into(), name: "Night Drive".into(), subtitle: String::new(), kind: "playlist".into(), image_url: None, depth: 0, own: true, authors: vec![] };
        Seen { home: Some(HomeFeed { fetched_unix_ms: 1, shelves: vec![Shelf { title: "Made for you".into(), kind: "shelf".into(), cards: vec![] }] }), library: Some(Library { items: vec![item] }) }
    }

    #[test]
    fn what_is_written_is_read_back_owner_only_and_goes_on_logout() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache/home-and-library.json");
        let l = LastSeen::at(path.clone());
        assert!(l.read().home.is_none(), "no file: nothing");
        l.write(&seen(), true);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let back = LastSeen::at(path.clone()).read();
        assert_eq!(back.home, seen().home);
        assert_eq!(back.library, seen().library);
        l.forget();
        assert!(!path.exists());
    }

    #[test]
    fn a_file_this_build_cannot_read_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home-and-library.json");
        std::fs::write(&path, b"{\"home\": 3").unwrap();
        let s = LastSeen::at(path).read();
        assert!(s.home.is_none() && s.library.is_none());
    }

    #[test]
    fn only_what_is_still_from_disk_goes_with_a_logout() {
        let dir = tempfile::tempdir().unwrap();
        let l = LastSeen::at(dir.path().join("f.json"));
        l.write(&seen(), true);
        let l = LastSeen::at(dir.path().join("f.json"));
        l.read();
        // a fresh Home came in; the library is still the one from disk
        l.write(&seen(), true);
        assert_eq!(l.forget(), (false, true));
    }
}
