//! The Daemon's own log file (Q170, #126): the last few are kept across starts, and none grows for ever.
//!
//! Whoever starts the Daemon gives it `daemon.log` as its stderr: a Client (appending, `conn.rs`) or launchd
//! (`StandardErrorPath`, appending). Once this Daemon holds the socket, it moves the earlier Daemon's log aside
//! (`daemon.log` → `daemon.log.1` → `daemon.log.2`) and writes a fresh one; a log that grows past `MAX_BYTES` is moved
//! aside the same way. A Daemon whose stderr is anything else (a terminal, systemd's journal) leaves it alone.

use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The log's name in the App folders' state dir.
pub const NAME: &str = "daemon.log";
/// Logs kept: `daemon.log`, and the two before it as `daemon.log.1` (the newer) and `daemon.log.2`.
pub const KEEP: usize = 3;
/// A log this big is moved aside, as at a start: a Daemon running for months can't fill the disk (a day of ordinary
/// use writes well under 1 MB).
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// How often the log's size is looked at.
const CHECK_EVERY: Duration = Duration::from_secs(60);

/// `path.n`
fn older(path: &Path, n: usize) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(format!(".{n}"));
    s.into()
}

/// Move `path` to `path.1`, `path.1` to `path.2` and so on; the oldest beyond `keep` files is dropped.
pub fn rotate(path: &Path, keep: usize) -> std::io::Result<()> {
    for n in (1..keep).rev() {
        let from = if n == 1 { path.to_path_buf() } else { older(path, n - 1) };
        match std::fs::rename(&from, older(path, n)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// The size of what `fd` writes to, if that is the regular file at `path` itself (not a copy of it, not a terminal).
fn size_if_same(fd: RawFd, path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let on_disk = std::fs::metadata(path).ok()?;
    // SAFETY: fstat fills a zeroed stat for `fd`; nothing else is touched
    let st = unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstat(fd, &mut st) != 0 {
            return None;
        }
        st
    };
    let same = (st.st_mode & libc::S_IFMT) == libc::S_IFREG && st.st_dev as u64 == on_disk.dev() && st.st_ino as u64 == on_disk.ino();
    same.then_some(st.st_size as u64)
}

/// Move the logs at `path` along and point `fd` at a fresh, owner-only `path`. What went wrong, said in full (once the
/// logs moved, `fd` still writes to the one now at `path.1`).
fn fresh(fd: RawFd, path: &Path) -> Result<(), String> {
    fresh_with(fd, path, || {})
}

/// `fresh`, with `between` run after the move (a test's way in).
fn fresh_with(fd: RawFd, path: &Path, between: impl FnOnce()) -> Result<(), String> {
    rotate(path, KEEP).map_err(|e| format!("couldn't move {NAME} aside: {e}"))?;
    between();
    reopen(fd, path).map_err(|e| format!("couldn't start a fresh {NAME} ({e}); this Daemon's lines go on in {NAME}.1"))
}

/// Point `fd` at `path`, made if it isn't there, owner-only.
fn reopen(fd: RawFd, path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let f = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    // SAFETY: both are open descriptors; dup2 swaps what `fd` writes to in one step, so no line is lost or split
    if unsafe { libc::dup2(f.as_raw_fd(), fd) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// At start, once this Daemon holds the socket: if `fd` writes to `path` and an earlier Daemon's lines are in it,
/// they're moved aside and `fd` starts a fresh log. Whether `fd` writes to `path` (then `watch` keeps it in bounds).
pub fn take_over(fd: RawFd, path: &Path) -> bool {
    match size_if_same(fd, path) {
        None => false,
        Some(0) => true,
        Some(_) => {
            if let Err(e) = fresh(fd, path) {
                tracing::warn!("log: {e}");
            }
            true
        }
    }
}

/// If the log `fd` writes to at `path` is past `max` bytes, move it aside and start a fresh one. Whether it did.
pub fn cap(fd: RawFd, path: &Path, max: u64) -> bool {
    if !size_if_same(fd, path).is_some_and(|n| n > max) {
        return false;
    }
    match fresh(fd, path) {
        Ok(()) => {
            tracing::info!("log: the last {} MB went on in {NAME}.1", max / (1024 * 1024));
            true
        }
        Err(e) => {
            tracing::warn!("log: {e}");
            false
        }
    }
}

/// If the log `fd` wrote to at `path` was removed, start a fresh one there. Whether it did.
pub fn recover(fd: RawFd, path: &Path) -> bool {
    if std::fs::symlink_metadata(path).is_ok() {
        return false;
    }
    match reopen(fd, path) {
        Ok(()) => {
            tracing::warn!("log: {NAME} was removed while the Daemon ran; this is a fresh one");
            true
        }
        Err(e) => {
            tracing::warn!("log: {NAME} was removed and couldn't be made again ({e}); nothing is kept from now on");
            false
        }
    }
}

/// Keep the log `fd` writes to at `path` under `MAX_BYTES`, and there, for as long as the Daemon runs.
pub async fn watch(fd: RawFd, path: PathBuf) {
    loop {
        tokio::time::sleep(CHECK_EVERY).await;
        if !recover(fd, &path) {
            cap(fd, &path, MAX_BYTES);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn read(p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }

    /// The log as a Client opens it for a new Daemon: appending, so the earlier lines are still there.
    fn opened(path: &Path) -> std::fs::File {
        std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap()
    }

    /// Q170: each start keeps the earlier logs as `.1` and `.2`, drops the one before, and the newest is fresh.
    #[test]
    fn each_start_keeps_the_last_logs_and_starts_a_fresh_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NAME);
        for start in 1..=4 {
            let mut f = opened(&path);
            assert!(take_over(f.as_raw_fd(), &path), "start {start}: it's the log");
            assert_eq!(read(&path).as_deref(), Some(""), "start {start}: the newest is fresh");
            writeln!(f, "start {start}").unwrap();
        }
        assert_eq!(read(&path).as_deref(), Some("start 4\n"), "this start's lines go to the fresh log");
        assert_eq!(read(&older(&path, 1)).as_deref(), Some("start 3\n"));
        assert_eq!(read(&older(&path, 2)).as_deref(), Some("start 2\n"));
        assert_eq!(read(&older(&path, 3)), None, "only {KEEP} are kept");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "owner-only: it names the account");
    }

    /// An empty log (the first start, or a Daemon that wrote nothing) isn't moved aside over a kept one.
    #[test]
    fn an_empty_log_is_kept_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NAME);
        std::fs::write(older(&path, 1), "the last start\n").unwrap();
        let f = opened(&path);
        assert!(take_over(f.as_raw_fd(), &path));
        assert_eq!(read(&older(&path, 1)).as_deref(), Some("the last start\n"));
        assert_eq!(read(&older(&path, 2)), None);
    }

    /// A Daemon writing to something else (a terminal, the journal, another file) leaves `daemon.log` alone.
    #[test]
    fn a_daemon_logging_elsewhere_leaves_the_log_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NAME);
        std::fs::write(&path, "another Daemon's\n").unwrap();
        let elsewhere = opened(&dir.path().join("elsewhere.log"));
        assert!(!take_over(elsewhere.as_raw_fd(), &path));
        assert!(!cap(elsewhere.as_raw_fd(), &path, 0));
        assert_eq!(read(&path).as_deref(), Some("another Daemon's\n"));
        assert_eq!(read(&older(&path, 1)), None);
    }

    /// Q170: a log past its size is moved aside while the Daemon runs, and the lines after it go to a fresh one.
    #[test]
    fn a_log_past_its_size_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NAME);
        let mut f = opened(&path);
        write!(f, "{}", "x".repeat(100)).unwrap();
        assert!(!cap(f.as_raw_fd(), &path, 100), "not past it yet");
        write!(f, "x").unwrap();
        assert!(cap(f.as_raw_fd(), &path, 100));
        writeln!(f, "after").unwrap();
        assert_eq!(read(&older(&path, 1)).map(|s| s.len()), Some(101));
        assert!(read(&path).unwrap().ends_with("after\n"), "the fresh log goes on: {:?}", read(&path));
    }

    /// Review: a log removed while the Daemon runs is started again (its lines don't go to a file no one can open).
    #[test]
    fn a_removed_log_is_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NAME);
        let mut f = opened(&path);
        assert!(!recover(f.as_raw_fd(), &path), "it's there");
        std::fs::remove_file(&path).unwrap();
        assert!(recover(f.as_raw_fd(), &path));
        writeln!(f, "after").unwrap();
        assert_eq!(read(&path).as_deref(), Some("after\n"));
    }

    /// Review: when no fresh log can be made after the move, the Daemon goes on writing to the one moved aside, and
    /// the warning says so.
    #[test]
    fn a_fresh_log_that_cant_be_made_says_where_the_lines_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NAME);
        let mut f = opened(&path);
        writeln!(f, "before").unwrap();
        // something that isn't a file where the fresh log goes, once the old one moved
        let blocked = path.clone();
        let err = fresh_with(f.as_raw_fd(), &path, || std::fs::create_dir(&blocked).unwrap()).unwrap_err();
        assert!(err.ends_with("go on in daemon.log.1"), "{err}");
        writeln!(f, "after").unwrap();
        assert_eq!(read(&older(&path, 1)).as_deref(), Some("before\nafter\n"));
    }
}
