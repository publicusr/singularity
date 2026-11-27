//! Seam for Soloist auto-update (#6): a fake CDN over HTTP serving stand-in builds (shell scripts that answer
//! `--version`), against the real check / download / verify / install code.

use singularityd::update::{self, Builds, Checked};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What the fake CDN serves right now.
#[derive(Clone)]
struct Release {
    status: u16,
    etag: String,
    body: Vec<u8>,
    /// send it without a Content-Length (chunked), as a broken or streaming CDN might
    chunked: bool,
}

/// A build: a tarball with a `soloist` script printing `version` (and failing `--version` if `broken`).
fn tarball(version: &str, broken: bool) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("soloist");
    let script = if broken { "#!/bin/sh\nexit 3\n".to_string() } else { format!("#!/bin/sh\necho \"{version}\"\n") };
    std::fs::write(&exe, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o555)).unwrap();
    std::fs::write(dir.path().join("CHANGELOG.md"), version).unwrap();
    let out = dir.path().join("b.tar.gz");
    assert!(std::process::Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(dir.path()).args(["soloist", "CHANGELOG.md"]).status().unwrap().success());
    std::fs::read(out).unwrap()
}

/// A build whose `build.json` is a symlink: an archive can carry links that point outside the directory it is
/// unpacked into.
fn tarball_with_linked_build_json(target: &std::path::Path) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("soloist");
    std::fs::write(&exe, "#!/bin/sh\necho 'soloist 1'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o555)).unwrap();
    std::os::unix::fs::symlink(target, dir.path().join("build.json")).unwrap();
    let out = dir.path().join("b.tar.gz");
    assert!(std::process::Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(dir.path()).args(["soloist", "build.json"]).status().unwrap().success());
    std::fs::read(out).unwrap()
}

/// Serve `release` on 127.0.0.1 (HEAD and GET, ETag, a fixed Last-Modified); returns the URL.
async fn fake_cdn(release: Arc<Mutex<Release>>) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/soloist_release_x86_64.tar.gz", l.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { return };
            let r = release.lock().unwrap().clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).starts_with("HEAD");
                let reason = if r.status == 200 { "OK" } else { "Server Error" };
                let length = if r.chunked { "Transfer-Encoding: chunked\r\n".to_string() } else { format!("Content-Length: {}\r\n", r.body.len()) };
                let hdr = format!(
                    "HTTP/1.1 {} {reason}\r\nETag: {}\r\nLast-Modified: Fri, 16 Oct 2026 18:10:28 GMT\r\n{length}Connection: close\r\n\r\n",
                    r.status, r.etag
                );
                let _ = s.write_all(hdr.as_bytes()).await;
                if !head {
                    if r.chunked {
                        for chunk in r.body.chunks(4096) {
                            let _ = s.write_all(format!("{:x}\r\n", chunk.len()).as_bytes()).await;
                            let _ = s.write_all(chunk).await;
                            let _ = s.write_all(b"\r\n").await;
                        }
                        let _ = s.write_all(b"0\r\n\r\n").await;
                    } else {
                        let _ = s.write_all(&r.body).await;
                    }
                }
            });
        }
    });
    url
}

fn version_on_disk(b: &Builds) -> String {
    String::from_utf8(std::process::Command::new(b.binary()).arg("--version").output().unwrap().stdout).unwrap().trim().into()
}

#[tokio::test]
async fn new_builds_are_staged_verified_and_installed_and_failures_keep_the_old_one() {
    let release = Arc::new(Mutex::new(Release { status: 200, etag: "\"v1\"".into(), body: tarball("soloist 1", false), chunked: false }));
    let url = fake_cdn(release.clone()).await;
    let root = tempfile::tempdir().unwrap();
    let builds = Builds { root: root.path().into() };

    // first start: download, verify, install
    update::ensure(&builds, &url).await.unwrap();
    assert_eq!(version_on_disk(&builds), "soloist 1");
    let installed = builds.installed().unwrap();
    assert_eq!((installed.etag.as_deref(), installed.version.as_str()), (Some("\"v1\""), "soloist 1"));
    assert_eq!(installed.date().as_deref(), Some("2026-10-16"));
    assert_eq!(update::check(&builds, &url).await.unwrap(), Checked::UpToDate);

    // a new build appears: staged next to the running one, which stays as it is
    *release.lock().unwrap() = Release { status: 200, etag: "\"v2\"".into(), body: tarball("soloist 2", false), chunked: false };
    match update::check(&builds, &url).await.unwrap() {
        Checked::Staged(b) => assert_eq!(b.version, "soloist 2"),
        other => panic!("expected a staged build, got {other:?}"),
    }
    assert_eq!(version_on_disk(&builds), "soloist 1", "nothing changes under the running Soloist");
    // asking again doesn't download again
    assert!(matches!(update::check(&builds, &url).await.unwrap(), Checked::Staged(_)));
    assert!(builds.install().unwrap());
    assert_eq!(version_on_disk(&builds), "soloist 2");
    assert!(root.path().join("soloist.old/soloist").exists(), "the previous build is kept");
    assert_eq!(update::check(&builds, &url).await.unwrap(), Checked::UpToDate);

    // the CDN fails: the build stays
    *release.lock().unwrap() = Release { status: 500, etag: "\"v3\"".into(), body: vec![], chunked: false };
    assert!(update::check(&builds, &url).await.is_err());
    // a build that doesn't run is never staged
    *release.lock().unwrap() = Release { status: 200, etag: "\"v3\"".into(), body: tarball("soloist 3", true), chunked: false };
    let err = update::check(&builds, &url).await.unwrap_err().to_string();
    assert!(err.contains("--version"), "{err}");
    assert!(builds.ready().is_none() && !builds.staged().exists());
    // garbage instead of a tarball
    *release.lock().unwrap() = Release { status: 200, etag: "\"v4\"".into(), body: b"<html>not a build</html>".to_vec(), chunked: false };
    assert!(update::check(&builds, &url).await.is_err());
    assert!(!builds.install().unwrap(), "nothing to install");
    assert_eq!(version_on_disk(&builds), "soloist 2");
}

#[tokio::test]
async fn an_archive_over_the_cap_fails_cleanly_and_keeps_the_build() {
    // the real cap is 256 MB; the test's is 4 KB, and the stand-in CDN serves twice that
    let release = Arc::new(Mutex::new(Release { status: 200, etag: "\"v1\"".into(), body: tarball("soloist 1", false), chunked: false }));
    let url = fake_cdn(release.clone()).await;
    let root = tempfile::tempdir().unwrap();
    let builds = Builds { root: root.path().into() };
    update::ensure(&builds, &url).await.unwrap();
    assert_eq!(version_on_disk(&builds), "soloist 1");

    // announced with a Content-Length: refused before anything is written
    *release.lock().unwrap() = Release { status: 200, etag: "\"huge\"".into(), body: vec![7u8; 8 * 1024], chunked: false };
    let err = update::download_with_cap(&builds, &url, 4 * 1024).await.unwrap_err().to_string();
    assert!(err.contains("cap"), "{err}");
    assert_eq!(version_on_disk(&builds), "soloist 1", "the current build stays");
    assert!(!builds.staged().exists(), "soloist.new/ is removed");

    // chunked, no Content-Length: the bytes are counted as they arrive
    *release.lock().unwrap() = Release { status: 200, etag: "\"huge\"".into(), body: vec![7u8; 8 * 1024], chunked: true };
    let err = update::download_with_cap(&builds, &url, 4 * 1024).await.unwrap_err().to_string();
    assert!(err.contains("cap"), "{err}");
    assert_eq!(version_on_disk(&builds), "soloist 1");
    assert!(!builds.staged().exists() && builds.ready().is_none());
}

#[tokio::test]
async fn an_archive_cannot_write_outside_its_directory_through_a_symlink() {
    let victim_dir = tempfile::tempdir().unwrap();
    let victim = victim_dir.path().join("victim");
    std::fs::write(&victim, "not to be touched").unwrap();
    let release = Arc::new(Mutex::new(Release { status: 200, etag: "\"v1\"".into(), body: tarball_with_linked_build_json(&victim), chunked: false }));
    let url = fake_cdn(release).await;
    let root = tempfile::tempdir().unwrap();
    let builds = Builds { root: root.path().into() };
    let _ = update::download_with_cap(&builds, &url, 1 << 20).await;
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "not to be touched", "the archive's build.json symlink was followed outside soloist.new/");
}

#[tokio::test]
async fn an_install_from_before_tracking_is_adopted_not_downloaded_again() {
    let release = Arc::new(Mutex::new(Release { status: 200, etag: "\"v1\"".into(), body: vec![], chunked: false }));
    let url = fake_cdn(release).await;
    let root = tempfile::tempdir().unwrap();
    let builds = Builds { root: root.path().into() };
    // unpacked yesterday-ish (after the CDN's Last-Modified), no build.json
    std::fs::create_dir_all(builds.current()).unwrap();
    std::fs::write(builds.binary(), "#!/bin/sh\necho 'soloist old'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(builds.binary(), std::fs::Permissions::from_mode(0o555)).unwrap();
    // (the CDN serves an empty body: a download would fail, so UpToDate proves nothing was fetched)
    assert_eq!(update::check(&builds, &url).await.unwrap(), Checked::UpToDate);
    assert_eq!(builds.installed().unwrap().version, "soloist old");
}

/// #16: a build is unpacked by this process and then run (`--version`). A process another thread starts in that
/// moment briefly holds a copy of the file still open for writing, and running a file open for writing fails with
/// "Text file busy". The Daemon starts processes on other threads all the time; here four threads do nothing else.
#[tokio::test]
async fn a_build_is_verified_while_other_threads_start_processes() {
    let release = Arc::new(Mutex::new(Release { status: 200, etag: "\"v1\"".into(), body: tarball("soloist 1", false), chunked: false }));
    let url = fake_cdn(release).await;
    let root = tempfile::tempdir().unwrap();
    let builds = Builds { root: root.path().into() };
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let starters: Vec<_> = (0..4)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = std::process::Command::new("/bin/sh").args(["-c", ":"]).status();
                }
            })
        })
        .collect();
    let mut failed = None;
    for _ in 0..100 {
        if let Err(e) = update::download_with_cap(&builds, &url, 1 << 20).await {
            failed = Some(e.to_string());
            break;
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for s in starters {
        s.join().unwrap();
    }
    assert_eq!(failed, None);
}

/// Serve `body` (with its Content-Length) in pieces of `piece` bytes, `gap` apart; after `stall_after` bytes (if any)
/// it sends nothing more and keeps the connection open. Returns the URL.
async fn trickling_cdn(body: Vec<u8>, piece: usize, gap: std::time::Duration, stall_after: Option<usize>) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/soloist_release_x86_64.tar.gz", l.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let _ = s.read(&mut buf).await;
                let hdr = format!("HTTP/1.1 200 OK\r\nETag: \"slow\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(hdr.as_bytes()).await;
                let mut sent = 0;
                for chunk in body.chunks(piece) {
                    if stall_after.is_some_and(|n| sent >= n) {
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        return;
                    }
                    tokio::time::sleep(gap).await;
                    if s.write_all(chunk).await.is_err() {
                        return;
                    }
                    sent += chunk.len();
                }
            });
        }
    });
    url
}

/// #142: a download has no deadline as a whole, only a pace (the real one: 1 KB/s over 30 s; here 20 bytes a second
/// over 0.5 s). A build that trickles in at that pace for many windows finishes; one that stops arriving, or arrives
/// slower than the pace (a byte now and then, never long enough apart to look stalled), is given up after about one
/// window, and leaves nothing staged.
#[tokio::test]
async fn a_slow_download_finishes_and_a_stalled_or_too_slow_one_is_given_up() {
    use std::time::{Duration, Instant};
    let pace = update::Pace { window: Duration::from_millis(500), min_rate: 20 };
    let body = tarball("soloist slow", false);
    let piece = body.len().div_ceil(16);
    assert!(piece * 3 > 10, "a window holds more than the pace asks for");
    let root = tempfile::tempdir().unwrap();
    let builds = Builds { root: root.path().into() };

    // 16 pieces 150 ms apart: about 2.4 s in all, almost five windows
    let url = trickling_cdn(body.clone(), piece, Duration::from_millis(150), None).await;
    let start = Instant::now();
    let build = update::download_with(&builds, &url, 1 << 20, pace).await.unwrap();
    assert!(start.elapsed() > pace.window * 4, "the download took {:?}, not long enough to prove anything", start.elapsed());
    assert_eq!(build.version, "soloist slow");
    assert!(builds.ready().is_some());

    // nothing more after half of it; one byte every 300 ms (2 or 3 bytes a second)
    for (name, url) in [
        ("stalled", trickling_cdn(body.clone(), piece, Duration::from_millis(50), Some(piece * 8)).await),
        ("too slow", trickling_cdn(body.clone(), 1, Duration::from_millis(300), None).await),
    ] {
        let start = Instant::now();
        let r = tokio::time::timeout(Duration::from_secs(10), update::download_with(&builds, &url, 1 << 20, pace)).await;
        let waited = start.elapsed();
        assert!(r.expect("a stalled download must not wait forever").is_err(), "{name}");
        assert!(waited >= pace.window && waited < pace.window * 4, "{name}: given up after {waited:?}, the window is {:?}", pace.window);
        assert!(!builds.staged().exists() && builds.ready().is_none(), "{name}: leaves nothing staged");
    }
}
