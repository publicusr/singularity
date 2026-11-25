//! The Soloist runner (D89): where Soloist runs and how it is kept alive. It starts Soloist, says its version and
//! WebSocket address, and keeps its build current; the Soloist session (`soloist.rs`, `session.rs`) talks to Soloist
//! through it and holds no OS-specific code.
//!
//! [`Local`] is Linux's: Soloist as a local process that dies with the Daemon, its builds under the data directory. The
//! Soloist VM (macOS, `soloist_vm.rs`) moves behind the same seam next (#94); pairing's announcement and the VM's
//! up/down join the trait with it.

use crate::update::{self, Builds, Checked};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::{Notify, watch};

/// Where Soloist runs.
pub trait Runner: Send + Sync {
    /// Make sure there is a build to start (a staged one goes in now: Soloist isn't running). The error says what's
    /// wrong, for the Engine's status.
    fn prepare(&self) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Keep the build current, for as long as the Daemon runs: `staged` is told when a newer build waits to be
    /// restarted into; `days` is the running build's days left (the checks get closer near the end).
    fn keep_current(&self, days: watch::Receiver<Option<u32>>, staged: Arc<Notify>) -> impl Future<Output = ()> + Send + use<Self>;

    /// Is a newer build waiting to be restarted into?
    fn update_waiting(&self) -> bool;

    /// The running build said it has expired: the next [`prepare`](Runner::prepare) must not start it again.
    fn expired(&self);

    /// Fails if another Soloist already runs on this data (another Daemon's): then none of its files may be touched.
    fn check_free(&self) -> anyhow::Result<()>;

    /// Start Soloist as `name` with this API key, at `volume` if one was saved. Its stdout and stderr are piped (its
    /// log), and it stops with the Daemon, even a killed one.
    fn launch(&self, key: &str, name: &str, volume: Option<u8>) -> impl Future<Output = anyhow::Result<Child>> + Send;

    /// The build's version line (empty if it can't be read).
    fn version(&self) -> impl Future<Output = String> + Send;

    /// The started Soloist's WebSocket address, once it is listening (asked again until it is).
    fn ws_addr(&self) -> impl Future<Output = Option<String>> + Send;
}

/// Soloist as a local process (Linux; on macOS only the pty harness's stand-in, `SINGULARITY_SOLOIST=local`). Its
/// binary is downloaded from Spotify's CDN when missing and kept current by `update` (#6).
#[derive(Clone)]
pub struct Local {
    builds: Builds,
    /// Soloist's own state (pairing, cache, its WebSocket's port files)
    state: PathBuf,
    /// `--cache-size` in MB (0: no limit), from `engine.cache_gb`
    cache_mb: u32,
}

impl Local {
    pub fn new(data_dir: PathBuf, cache_mb: u32) -> Local {
        Local { state: data_dir.join("soloist-state"), builds: Builds { root: data_dir }, cache_mb }
    }

    /// Singularity's data folder, which holds Soloist's builds and state.
    pub fn data_dir(&self) -> &std::path::Path {
        &self.builds.root
    }

    /// Soloist's own state (pairing, cache, its log).
    pub fn state_dir(&self) -> &std::path::Path {
        &self.state
    }

    fn binary(&self) -> PathBuf {
        self.builds.binary()
    }
}

impl Runner for Local {
    async fn prepare(&self) -> anyhow::Result<()> {
        let builds = &self.builds;
        match builds.install() {
            Ok(true) => tracing::info!("Soloist updated to {}", builds.installed().map(|b| b.version).unwrap_or_default()),
            Ok(false) => {}
            Err(e) => tracing::warn!("couldn't install the new Soloist build: {e}"),
        }
        update::ensure(builds, &update::cdn_url()).await.map_err(|e| anyhow::anyhow!("couldn't download Soloist: {e}"))
    }

    fn keep_current(&self, days: watch::Receiver<Option<u32>>, staged: Arc<Notify>) -> impl Future<Output = ()> + Send + use<> {
        keep_current(self.builds.clone(), update::cdn_url(), days, staged)
    }

    fn update_waiting(&self) -> bool {
        self.builds.ready().is_some()
    }

    fn expired(&self) {
        if self.builds.ready().is_none() {
            let _ = std::fs::remove_dir_all(self.builds.current());
        }
    }

    fn check_free(&self) -> anyhow::Result<()> {
        if let Some(pid) = running_soloist(&self.state) {
            anyhow::bail!("Soloist is already running (pid {pid}) with Singularity's data: is a second singularityd running?");
        }
        Ok(())
    }

    async fn launch(&self, key: &str, name: &str, volume: Option<u8>) -> anyhow::Result<Child> {
        let state = &self.state;
        let _ = tokio::fs::remove_file(state.join("data/ws.port")).await;
        let mut cmd = crate::programs::tokio_command(self.binary());
        cmd.args(["--device-name", name, "--api-key", key, "--ws", "127.0.0.1:0", "--verbose"])
            .args(volume.map(|v| ["--initial-volume".to_string(), v.to_string()]).into_iter().flatten())
            .arg("--data-dir")
            .arg(state.join("data"))
            .arg("--cache-dir")
            .arg(state.join("cache"))
            .args(["--cache-size", &self.cache_mb.to_string()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Soloist must never outlive the Daemon, even if the Daemon is SIGKILLed: ask the kernel to
        // SIGTERM it when its parent dies. (Linux-only, like Soloist itself.)
        #[cfg(target_os = "linux")]
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        Ok(cmd.spawn()?)
    }

    async fn version(&self) -> String {
        crate::programs::tokio_command(self.binary()).arg("--version").output().await.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    }

    /// Soloist writes the address it listens on into two files in its data directory.
    async fn ws_addr(&self) -> Option<String> {
        let data = self.state.join("data");
        let (p, a) = (std::fs::read_to_string(data.join("ws.port")).ok()?, std::fs::read_to_string(data.join("ws.addr")).ok()?);
        Some(format!("ws://{}:{}", a.trim(), p.trim()))
    }
}

/// Check the CDN now and then; when a new build is staged, say so.
async fn keep_current(builds: Builds, url: String, days: watch::Receiver<Option<u32>>, staged: Arc<Notify>) {
    let (mut failures, mut told) = (0, None);
    loop {
        match update::check(&builds, &url).await {
            Ok(Checked::UpToDate) => failures = 0,
            Ok(Checked::Staged(b)) => {
                failures = 0;
                if told.as_ref() != Some(&b) {
                    tracing::info!("new Soloist build staged: {}", b.version);
                    staged.notify_one();
                    told = Some(b);
                }
            }
            Err(e) => {
                failures += 1;
                tracing::warn!("Soloist update check failed (keeping the current build): {e}");
            }
        }
        // SINGULARITY_SOLOIST_CHECK_SECS: a short interval for the pty harness
        let wait = std::env::var("SINGULARITY_SOLOIST_CHECK_SECS").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or_else(|| update::next_check(failures, *days.borrow()));
        tokio::time::sleep(wait).await;
    }
}

/// Is a Soloist already running on this state directory? (Its pid file names a live `soloist` process.)
pub fn running_soloist(state: &std::path::Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(state.join("data/soloist.pid")).ok()?.trim().parse().ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    (comm.trim() == "soloist").then_some(pid)
}
