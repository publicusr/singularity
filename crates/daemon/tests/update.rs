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
                let hdr = format!(
                    "HTTP/1.1 {} {reason}\r\nETag: {}\r\nLast-Modified: Fri, 16 Oct 2026 18:10:28 GMT\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    r.status,
                    r.etag,
                    r.body.len()
                );
                let _ = s.write_all(hdr.as_bytes()).await;
                if !head {
                    let _ = s.write_all(&r.body).await;
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
    let release = Arc::new(Mutex::new(Release { status: 200, etag: "\"v1\"".into(), body: tarball("soloist 1", false) }));
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
    *release.lock().unwrap() = Release { status: 200, etag: "\"v2\"".into(), body: tarball("soloist 2", false) };
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
    *release.lock().unwrap() = Release { status: 500, etag: "\"v3\"".into(), body: vec![] };
    assert!(update::check(&builds, &url).await.is_err());
    // a build that doesn't run is never staged
    *release.lock().unwrap() = Release { status: 200, etag: "\"v3\"".into(), body: tarball("soloist 3", true) };
    let err = update::check(&builds, &url).await.unwrap_err().to_string();
    assert!(err.contains("--version"), "{err}");
    assert!(builds.ready().is_none() && !builds.staged().exists());
    // garbage instead of a tarball
    *release.lock().unwrap() = Release { status: 200, etag: "\"v4\"".into(), body: b"<html>not a build</html>".to_vec() };
    assert!(update::check(&builds, &url).await.is_err());
    assert!(!builds.install().unwrap(), "nothing to install");
    assert_eq!(version_on_disk(&builds), "soloist 2");
}

#[tokio::test]
async fn an_install_from_before_tracking_is_adopted_not_downloaded_again() {
    let release = Arc::new(Mutex::new(Release { status: 200, etag: "\"v1\"".into(), body: vec![] }));
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
