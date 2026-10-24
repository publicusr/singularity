//! Soloist auto-update (#6). Soloist builds expire after ~90 days, so the Daemon keeps one current by itself.
//!
//! - The CDN is checked at start, then daily (every 6 h in the last week), by its ETag and Last-Modified.
//! - A new build is unpacked into `soloist.new/` and must answer `--version` before it's trusted.
//! - Installing swaps directories (`soloist/` → `soloist.old/`, `soloist.new/` → `soloist/`); the running Soloist keeps
//!   its open binary, and the SoloistEngine restarts it only at a quiet moment (paused, idle, or a track change).
//! - A failed check or download keeps the old build and retries with backoff.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command as Proc;

pub const CDN: &str = "https://soloist-builds.spotifycdn.com/soloist_release_x86_64.tar.gz";

/// The CDN to use (`SINGULARITY_SOLOIST_CDN` points tests and the pty harness at a fake one).
pub fn cdn_url() -> String {
    std::env::var("SINGULARITY_SOLOIST_CDN").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| CDN.into())
}

/// What a build directory holds, as the CDN described it (`build.json` next to the binary).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Build {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// what `soloist --version` said
    pub version: String,
}

impl Build {
    fn same_as(&self, etag: &Option<String>, last_modified: &Option<String>) -> bool {
        match (&self.etag, etag) {
            (Some(a), Some(b)) => a == b,
            _ => self.last_modified.is_some() && self.last_modified == *last_modified,
        }
    }

    /// The build date (`2026-10-16`), from Last-Modified.
    pub fn date(&self) -> Option<String> {
        let unix = http_date(self.last_modified.as_deref()?)?;
        Some(ymd(unix))
    }
}

/// Where builds live: `<data>/soloist/` (current), `soloist.new/` (staged), `soloist.old/` (the one before).
#[derive(Debug, Clone)]
pub struct Builds {
    pub root: PathBuf,
}

impl Builds {
    pub fn current(&self) -> PathBuf {
        self.root.join("soloist")
    }
    pub fn staged(&self) -> PathBuf {
        self.root.join("soloist.new")
    }
    fn old(&self) -> PathBuf {
        self.root.join("soloist.old")
    }
    pub fn binary(&self) -> PathBuf {
        self.current().join("soloist")
    }

    pub fn installed(&self) -> Option<Build> {
        read_build(&self.current())
    }

    /// A staged build that's ready to install.
    pub fn ready(&self) -> Option<Build> {
        self.staged().join("soloist").exists().then(|| read_build(&self.staged())).flatten()
    }

    /// Put the staged build in place (the previous one is kept as `soloist.old/`). Safe while Soloist runs.
    pub fn install(&self) -> std::io::Result<bool> {
        if self.ready().is_none() {
            return Ok(false);
        }
        let _ = std::fs::remove_dir_all(self.old());
        if self.current().exists() {
            std::fs::rename(self.current(), self.old())?;
        }
        std::fs::rename(self.staged(), self.current())?;
        Ok(true)
    }
}

fn read_build(dir: &Path) -> Option<Build> {
    serde_json::from_slice(&std::fs::read(dir.join("build.json")).ok()?).ok()
}

/// What a check found.
#[derive(Debug, Clone, PartialEq)]
pub enum Checked {
    UpToDate,
    /// a newer build is in `soloist.new/`, verified
    Staged(Build),
}

fn http() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(Duration::from_secs(300)).connect_timeout(Duration::from_secs(15)).user_agent(concat!("singularityd/", env!("CARGO_PKG_VERSION"))).build()?)
}

fn header(r: &reqwest::Response, name: &str) -> Option<String> {
    r.headers().get(name).and_then(|v| v.to_str().ok()).map(String::from)
}

/// Ask the CDN whether there's a newer build than the installed one, and stage it if so.
pub async fn check(builds: &Builds, url: &str) -> anyhow::Result<Checked> {
    let head = http()?.head(url).send().await?.error_for_status()?;
    let (etag, modified) = (header(&head, "etag"), header(&head, "last-modified"));
    if let Some(b) = builds.installed() {
        if b.same_as(&etag, &modified) {
            return Ok(Checked::UpToDate);
        }
    } else if builds.binary().exists() && adopt(builds, &modified) {
        // installed before builds were tracked: if it's at least as new as the CDN's, it's that build
        let version = version_of(&builds.binary()).await.unwrap_or_default();
        write_build(&builds.current(), &Build { etag, last_modified: modified, version })?;
        return Ok(Checked::UpToDate);
    }
    if let Some(b) = builds.ready()
        && b.same_as(&etag, &modified)
    {
        return Ok(Checked::Staged(b));
    }
    Ok(Checked::Staged(download(builds, url).await?))
}

/// The most a build archive may be (a build is ~35 MB): a few hundred MB means the CDN sent something else.
pub const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;

/// Fetch, unpack and verify a build into `soloist.new/`. On any failure nothing else is touched.
pub async fn download(builds: &Builds, url: &str) -> anyhow::Result<Build> {
    download_with_cap(builds, url, MAX_ARCHIVE).await
}

/// [`download`] with an explicit cap (tests serve a small one).
pub async fn download_with_cap(builds: &Builds, url: &str, cap: u64) -> anyhow::Result<Build> {
    let dir = builds.staged();
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await?;
    let r = async {
        let resp = http()?.get(url).send().await?.error_for_status()?;
        let (etag, last_modified) = (header(&resp, "etag"), header(&resp, "last-modified"));
        let tgz = dir.join("soloist.tar.gz");
        save_archive(resp, &tgz, cap).await?;
        let tar = crate::programs::Tool::Tar.path()?;
        let ok = Proc::new(tar).arg("-xzf").arg(&tgz).arg("-C").arg(&dir).status().await?.success();
        let _ = tokio::fs::remove_file(&tgz).await;
        anyhow::ensure!(ok, "couldn't unpack the build");
        let version = version_of(&dir.join("soloist")).await?;
        let build = Build { etag, last_modified, version };
        write_build(&dir, &build)?;
        anyhow::Ok(build)
    }
    .await;
    if r.is_err() {
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
    r
}

/// Stream the response body to `path`, giving up once it passes `cap` bytes: a broken CDN answer must not fill the
/// disk (and it is never held in memory whole).
async fn save_archive(resp: reqwest::Response, path: &Path, cap: u64) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    const MB: u64 = 1024 * 1024;
    if let Some(len) = resp.content_length() {
        anyhow::ensure!(len <= cap, "the build is {} MB, over the {} MB cap", len.div_ceil(MB), cap.div_ceil(MB));
    }
    let mut file = tokio::fs::File::create(path).await?;
    let mut body = resp.bytes_stream();
    let mut written = 0u64;
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        written += chunk.len() as u64;
        anyhow::ensure!(written <= cap, "the build is over the {} MB cap", cap.div_ceil(MB));
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(())
}

/// First start: put a build in place (one already staged, else a fresh download).
pub async fn ensure(builds: &Builds, url: &str) -> anyhow::Result<()> {
    if builds.binary().exists() {
        return Ok(());
    }
    if builds.ready().is_none() {
        tracing::info!("downloading Soloist from Spotify's CDN");
        download(builds, url).await?;
    }
    builds.install()?;
    anyhow::ensure!(builds.binary().exists(), "the build has no soloist binary");
    Ok(())
}

/// `soloist --version`, which a build must answer before it's used.
async fn version_of(exe: &Path) -> anyhow::Result<String> {
    let out = tokio::time::timeout(Duration::from_secs(15), Proc::new(exe).arg("--version").kill_on_drop(true).output()).await??;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    anyhow::ensure!(out.status.success() && !v.is_empty(), "the new build doesn't run (`--version` failed)");
    Ok(v)
}

fn write_build(dir: &Path, b: &Build) -> std::io::Result<()> {
    std::fs::write(dir.join("build.json"), serde_json::to_vec_pretty(b).expect("plain struct"))
}

/// Was the untracked install made after the CDN's build appeared? (Then it is that build.)
fn adopt(builds: &Builds, modified: &Option<String>) -> bool {
    let cdn = modified.as_deref().and_then(http_date);
    let installed = std::fs::metadata(builds.current()).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
    matches!((cdn, installed), (Some(c), Some(i)) if i >= c)
}

/// When to look again: daily, every 6 h in the build's last week, sooner after failures (15 min doubling, ≤ 6 h).
pub fn next_check(failures: u32, days_left: Option<u32>) -> Duration {
    const H: u64 = 3600;
    if failures > 0 {
        return Duration::from_secs((15 * 60 * 2u64.pow(failures.min(10) - 1)).min(6 * H));
    }
    match days_left {
        Some(d) if d <= 7 => Duration::from_secs(6 * H),
        _ => Duration::from_secs(24 * H),
    }
}

/// `Fri, 16 Oct 2026 18:10:28 GMT` → Unix seconds.
pub fn http_date(s: &str) -> Option<u64> {
    let mut p = s.split_whitespace().skip(1);
    let day: i64 = p.next()?.parse().ok()?;
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|m| Some(*m) == p.clone().next())? as i64 + 1;
    p.next();
    let year: i64 = p.next()?.parse().ok()?;
    let hms: Vec<i64> = p.next()?.split(':').map(|n| n.parse().ok()).collect::<Option<_>>()?;
    let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hms.first()? * 3600 + hms.get(1)? * 60 + hms.get(2)?).ok()
}

/// Unix seconds → `YYYY-MM-DD`.
pub fn ymd(unix: u64) -> String {
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{m:02}-{d:02}", yoe + era * 400 + i64::from(m <= 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_dates_round_trip() {
        let t = http_date("Fri, 16 Oct 2026 18:10:28 GMT").unwrap();
        assert_eq!(ymd(t), "2026-10-16");
        assert_eq!(t % 86_400, 18 * 3600 + 10 * 60 + 28);
        assert_eq!(http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(http_date("garbage"), None);
    }

    #[test]
    fn checks_get_closer_near_expiry_and_back_off_after_failures() {
        assert_eq!(next_check(0, Some(60)), Duration::from_secs(86_400));
        assert_eq!(next_check(0, Some(5)), Duration::from_secs(6 * 3600));
        assert_eq!(next_check(1, None), Duration::from_secs(15 * 60));
        assert_eq!(next_check(3, None), Duration::from_secs(60 * 60));
        assert_eq!(next_check(30, None), Duration::from_secs(6 * 3600), "capped");
    }

    #[test]
    fn builds_match_by_etag_else_date() {
        let b = Build { etag: Some("\"a\"".into()), last_modified: Some("x".into()), version: "1".into() };
        assert!(b.same_as(&Some("\"a\"".into()), &None));
        assert!(!b.same_as(&Some("\"b\"".into()), &Some("x".into())), "a new ETag wins over an equal date");
        let d = Build { etag: None, last_modified: Some("x".into()), version: "1".into() };
        assert!(d.same_as(&None, &Some("x".into())));
    }
}
