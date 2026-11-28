//! The last Home and library, kept on disk (#81) so a fresh Daemon shows them at once while the Catalogue logs in and
//! fetches them again behind; the fresh ones replace them as they arrive.
//!
//! The file is the account's own data: it lives in the user's cache dir (App folders: `~/.cache/singularity`, or
//! `$XDG_CACHE_HOME/singularity`), owner-only, names the account it belongs to, and goes when the account logs out or
//! the listening data is deleted (#167). The copy is shown only once the Catalogue names the account it's logging in as (from the saved login, before
//! Spotify answers) and that is the file's account; another account's copy is deleted unseen. A file this build
//! can't read (an older shape, a broken write) is treated as none. Writes happen on a thread of their own, in order.

use singularity_proto::{HomeFeed, Library};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// What the file holds.
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Seen {
    /// the Spotify username Home and the library belong to
    pub account: Option<String>,
    pub home: Option<HomeFeed>,
    pub library: Option<Library>,
}

/// What naming the account did.
#[derive(Debug, Default, PartialEq)]
pub struct Claimed {
    /// the copy from disk, when it is this account's: show it
    pub seen: Option<Seen>,
    /// the account changed from another one: what the Daemon holds is that one's, and goes
    pub switched: bool,
}

enum Job {
    Write(Vec<u8>),
    Remove,
    Flush(std::sync::mpsc::Sender<()>),
}

pub struct LastSeen {
    jobs: std::sync::mpsc::Sender<Job>,
    path: PathBuf,
    /// the copy read at start, until the account is named
    waiting: Mutex<Option<Seen>>,
    /// the account logged in (or logging in), once named
    account: Mutex<Option<String>>,
    /// the listening data was deleted ([`LastSeen::delete`]): nothing is written until a logout and a login, or
    /// another account (read and set under `account`'s lock)
    deleted: AtomicBool,
}

impl LastSeen {
    /// The usual place: `home-and-library.json` in the user's cache dir.
    pub fn default_path() -> PathBuf {
        singularity_config::app_folders::AppFolders::this().cache.join("home-and-library.json")
    }

    pub fn at(path: PathBuf) -> LastSeen {
        let (jobs, rx) = std::sync::mpsc::channel();
        let file = path.clone();
        std::thread::Builder::new()
            .name("last-seen writer".into())
            .spawn(move || {
                for job in rx {
                    match job {
                        Job::Write(bytes) => {
                            if let Err(e) = write_private(&file, &bytes) {
                                tracing::warn!("couldn't keep the Home and library for the next start: {e}");
                            }
                        }
                        Job::Remove => {
                            // (a write cut short leaves its temp file)
                            for f in [file.clone(), file.with_extension("json.part")] {
                                match std::fs::remove_file(&f) {
                                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => tracing::warn!("couldn't remove {}: {e}", f.display()),
                                    _ => {}
                                }
                            }
                        }
                        Job::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .expect("a thread for the last-seen file");
        LastSeen { jobs, path, waiting: Default::default(), account: Default::default(), deleted: Default::default() }
    }

    /// Read the last Daemon's copy, to be shown once its account is named ([`LastSeen::claim`]).
    pub fn read(&self) {
        let began = std::time::Instant::now();
        let seen: Option<Seen> = std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok());
        if seen.is_some() {
            tracing::info!("timing: the last Home and library read from disk in {} ms", began.elapsed().as_millis());
        }
        *self.waiting.lock().expect("waiting lock") = seen;
    }

    /// The Catalogue names the account it is logging in as, or has logged in as.
    pub fn claim(&self, account: &str) -> Claimed {
        let switched = {
            let mut current = self.account.lock().expect("account lock");
            let switched = current.as_deref().is_some_and(|a| a != account);
            // another account's Home and library are kept again; the same one named again (a reconnect) changes nothing
            if switched {
                self.deleted.store(false, Ordering::SeqCst);
            }
            *current = Some(account.to_string());
            switched
        };
        let seen = match self.waiting.lock().expect("waiting lock").take() {
            Some(s) if s.account.as_deref() == Some(account) => Some(s),
            Some(_) => {
                // another account's: never shown
                let _ = self.jobs.send(Job::Remove);
                None
            }
            None => None,
        };
        Claimed { seen, switched }
    }

    /// Keep the Daemon's Home and library as the current account's. Nothing is written before an account is named.
    pub fn write(&self, home: Option<HomeFeed>, library: Option<Library>) {
        // (held until the write is queued: a `forget` meanwhile queues its removal after it)
        let held = self.account.lock().expect("account lock");
        let Some(account) = held.clone() else { return };
        if self.deleted.load(Ordering::SeqCst) {
            return;
        }
        match serde_json::to_vec(&Seen { account: Some(account), home, library }) {
            Ok(bytes) => {
                let _ = self.jobs.send(Job::Write(bytes));
            }
            Err(e) => tracing::warn!("couldn't keep the Home and library for the next start: {e}"),
        }
    }

    /// Logged out: the file goes, and nothing is written until an account is named again.
    pub fn forget(&self) {
        *self.waiting.lock().expect("waiting lock") = None;
        let mut account = self.account.lock().expect("account lock");
        *account = None;
        self.deleted.store(false, Ordering::SeqCst);
        let _ = self.jobs.send(Job::Remove);
    }

    /// The listening data was deleted (#167): the file goes, and nothing is written again in this run until a logout
    /// and a login, or another account. The same account named again (a reconnect) doesn't bring it back.
    pub fn delete(&self) {
        *self.waiting.lock().expect("waiting lock") = None;
        // (under `account`'s lock: a write under way is queued before the removal)
        let _held = self.account.lock().expect("account lock");
        self.deleted.store(true, Ordering::SeqCst);
        let _ = self.jobs.send(Job::Remove);
    }

    /// Wait until every write and removal asked for so far is done (tests).
    pub fn flush(&self) {
        let (done, wait) = std::sync::mpsc::channel();
        if self.jobs.send(Job::Flush(done)).is_ok() {
            let _ = wait.recv();
        }
    }
}

/// Write `bytes` to `path` owner-only, all at once: a temp file, synced, then renamed over it (a crash mid-write
/// leaves the old file).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.part");
    let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::{LibraryItem, Shelf};

    fn home(t: &str) -> HomeFeed {
        HomeFeed { fetched_unix_ms: 1, shelves: vec![Shelf { id: String::new(), title: t.into(), kind: "shelf".into(), cards: vec![] }] }
    }

    fn library(n: &str) -> Library {
        Library { items: vec![LibraryItem { uri: "spotify:playlist:p".into(), name: n.into(), subtitle: String::new(), kind: "playlist".into(), image_url: None, depth: 0, own: true, authors: vec![] }] }
    }

    fn written(path: &Path) -> Option<Seen> {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    #[test]
    fn what_is_written_names_its_account_is_owner_only_and_shown_to_that_account() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache/home-and-library.json");
        let l = LastSeen::at(path.clone());
        l.write(Some(home("Mine")), None);
        l.flush();
        assert!(!path.exists(), "nothing written before an account is named");
        l.claim("alice");
        l.write(Some(home("Mine")), Some(library("Mine")));
        l.flush();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(written(&path).unwrap().account.as_deref(), Some("alice"));
        let next = LastSeen::at(path.clone());
        next.read();
        let c = next.claim("alice");
        assert_eq!(c.seen.unwrap().home, Some(home("Mine")));
        assert!(!c.switched);
        assert_eq!(next.claim("alice"), Claimed::default(), "shown once");
    }

    #[test]
    fn another_accounts_copy_is_never_shown_and_goes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        let a = LastSeen::at(path.clone());
        a.claim("alice");
        a.write(Some(home("Alice's")), Some(library("Alice's")));
        a.flush();
        let b = LastSeen::at(path.clone());
        b.read();
        assert_eq!(b.claim("bob").seen, None);
        b.flush();
        assert!(!path.exists());
    }

    #[test]
    fn a_logout_removes_the_file_and_stops_writes_until_the_next_account() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        let l = LastSeen::at(path.clone());
        l.claim("alice");
        l.write(Some(home("Alice's")), None);
        l.forget();
        l.write(Some(home("late")), None);
        l.flush();
        assert!(!path.exists());
        assert!(!l.claim("bob").switched, "logged out in between: nothing of alice's is held");
        l.claim("bob");
        assert!(l.claim("carol").switched);
    }

    /// #167: after a delete nothing is written, through reconnects (the same account named again), until a logout and
    /// a login, or another account; a delete before the account is first named holds too.
    #[test]
    fn a_delete_stops_writes_until_a_logout_and_login_or_another_account() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        std::fs::write(path.with_extension("json.part"), b"{").unwrap();
        let l = LastSeen::at(path.clone());
        l.claim("alice");
        l.write(Some(home("Alice's")), None);
        l.delete();
        l.flush();
        assert!(!path.exists() && !path.with_extension("json.part").exists(), "the file and a cut-short write's temp file go");
        let kept = |l: &LastSeen| {
            l.write(Some(home("again")), Some(library("again")));
            l.flush();
            path.exists()
        };
        l.claim("alice");
        assert!(!kept(&l), "a reconnect doesn't bring it back");
        l.forget();
        l.claim("alice");
        assert!(kept(&l), "a logout and a login do");
        l.delete();
        l.flush();
        l.claim("bob");
        assert!(kept(&l), "another account does");

        // deleted before the account is first named: its first naming doesn't bring it back either
        let path = dir.path().join("g.json");
        let l = LastSeen::at(path.clone());
        l.delete();
        l.claim("alice");
        l.write(Some(home("Alice's")), None);
        l.flush();
        assert!(!path.exists());
    }

    #[test]
    fn a_file_this_build_cannot_read_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home-and-library.json");
        std::fs::write(&path, b"{\"home\": 3").unwrap();
        let l = LastSeen::at(path);
        l.read();
        assert_eq!(l.claim("alice").seen, None);
    }
}
