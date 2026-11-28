//! The Soloist runner (D89): where Soloist runs and how it is kept alive. It starts Soloist, says its version and
//! WebSocket address, keeps its build current, announces it for pairing, and stops it with the Daemon; the Soloist
//! session (`soloist.rs`, `session.rs`) talks to Soloist through it and holds no OS-specific code.
//!
//! - [`Local`]: Soloist as a local process that dies with the Daemon, its builds under the data directory (Linux, and
//!   Apple Silicon Macs since Q188, which replaced the Soloist VM of D55: Q189).

use crate::update::{self, Builds, Checked};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, ChildStdin};
use tokio::sync::{Notify, watch};

/// Where Soloist runs.
pub trait Runner: Clone + Send + Sync + 'static {
    /// Singularity's data folder: Soloist's builds are under it.
    fn data_dir(&self) -> &Path;

    /// Soloist's state on this computer: its log and the last volume (a local Soloist's pairing and cache too).
    fn state_dir(&self) -> &Path;

    /// What a command is answered with while Soloist can't be started (its [`prepare`](Runner::prepare) failed).
    fn unavailable(&self) -> &'static str;

    /// Make sure there is a build to start (a staged one goes in now: Soloist isn't running), and whatever Soloist
    /// runs in is up. Progress the user should see goes to `say`. The error says what's wrong, for the Engine's status.
    fn prepare(&self, say: &(dyn Fn(String) + Sync)) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Keep the build current, for as long as the Daemon runs: `staged` is told when a newer build waits to be
    /// restarted into; `days` is the running build's days left (the checks get closer near the end).
    fn keep_current(&self, days: watch::Receiver<Option<u32>>, staged: Arc<Notify>) -> impl Future<Output = ()> + Send + use<Self>;

    /// Is a newer build waiting to be restarted into?
    fn update_waiting(&self) -> bool;

    /// The running build said it has expired: the next [`prepare`](Runner::prepare) must not start it again. True if
    /// that takes a new build first (the Daemon says so and waits a moment); false if there's nothing to do here.
    fn expired(&self) -> bool;

    /// Fails if another Soloist already runs on this data (another Daemon's): then none of its files may be touched.
    fn check_free(&self) -> anyhow::Result<()>;

    /// Start Soloist as `name` with this API key, at `volume` if one was saved. Its stdout and stderr are piped (its
    /// log), and it stops with the Daemon, even a killed one.
    fn launch(&self, key: &str, name: &str, volume: Option<u8>) -> impl Future<Output = anyhow::Result<Launched>> + Send;

    /// The build's version line (empty if it can't be read).
    fn version(&self) -> impl Future<Output = String> + Send;

    /// The started Soloist's WebSocket address, once it is listening (asked again until it is).
    fn ws_addr(&self) -> impl Future<Output = Option<String>> + Send;

    /// Announce Soloist's pairing endpoint (its ZeroConf `port`) on the LAN as `name`, until the answer is dropped.
    fn announce(&self, port: u16, name: &str) -> impl Future<Output = anyhow::Result<Announcement>> + Send;

    /// After a run ended: is what Soloist runs in still there? If not, [`down`](Runner::down) clears it away and the
    /// next [`prepare`](Runner::prepare) brings it up again.
    fn alive(&self) -> impl Future<Output = bool> + Send;

    /// Stop what Soloist runs in (the Daemon quits, or it didn't come up).
    fn down(&self) -> impl Future<Output = ()> + Send;
}

/// A started Soloist.
pub struct Launched {
    pub child: Child,
    /// the VM's Soloist runs while this stays open: closing it stops Soloist, so it is held for the run
    pub hold: Option<ChildStdin>,
}

/// Soloist announced for pairing, until dropped.
pub struct Announcement {
    _held: Box<dyn Send + Sync>,
}

impl Announcement {
    /// An announcement kept up by `what` (the processes announcing it), withdrawn when it is dropped.
    pub fn held(what: impl Send + Sync + 'static) -> Announcement {
        Announcement { _held: Box::new(what) }
    }

    /// Nothing to withdraw: Soloist announces itself.
    pub fn none() -> Announcement {
        Announcement::held(())
    }
}

/// Soloist as a local process (Linux; Apple Silicon Macs, Q188). Its binary is downloaded from Spotify's CDN when
/// missing and kept current by `update` (#6).
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

    fn binary(&self) -> PathBuf {
        self.builds.binary()
    }
}

impl Runner for Local {
    fn data_dir(&self) -> &Path {
        &self.builds.root
    }

    fn state_dir(&self) -> &Path {
        &self.state
    }

    fn unavailable(&self) -> &'static str {
        "Soloist isn't available"
    }

    async fn prepare(&self, _say: &(dyn Fn(String) + Sync)) -> anyhow::Result<()> {
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

    fn expired(&self) -> bool {
        if self.builds.ready().is_none() {
            let _ = std::fs::remove_dir_all(self.builds.current());
        }
        true
    }

    fn check_free(&self) -> anyhow::Result<()> {
        if let Some(pid) = running_soloist(&self.state) {
            // macOS has no PR_SET_PDEATHSIG: a Soloist whose Daemon was killed outright runs on, handed to launchd.
            // Nobody drives it any more, so it makes way for this Daemon's.
            if orphaned(pid) {
                stop_orphan(pid);
                return Ok(());
            }
            anyhow::bail!("Soloist is already running (pid {pid}) with Singularity's data: is a second singularityd running?");
        }
        Ok(())
    }

    async fn launch(&self, key: &str, name: &str, volume: Option<u8>) -> anyhow::Result<Launched> {
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
        // SIGTERM it when its parent dies. (Linux only: on macOS an orphan is stopped at the next start, `check_free`.)
        #[cfg(target_os = "linux")]
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        Ok(Launched { child: cmd.spawn()?, hold: None })
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

    /// (A local Soloist announces itself.)
    async fn announce(&self, _port: u16, _name: &str) -> anyhow::Result<Announcement> {
        Ok(Announcement::none())
    }

    async fn alive(&self) -> bool {
        true
    }

    /// (A local Soloist stops with its run: nothing runs around it.)
    async fn down(&self) {}
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
        tokio::time::sleep(check_wait(failures, &days)).await;
    }
}

/// The wait before the next update check (`SINGULARITY_SOLOIST_CHECK_SECS`: a short interval for the pty harness).
fn check_wait(failures: u32, days: &watch::Receiver<Option<u32>>) -> Duration {
    std::env::var("SINGULARITY_SOLOIST_CHECK_SECS").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or_else(|| update::next_check(failures, *days.borrow()))
}

/// Has this process lost the process that started it (its parent is now init or launchd)?
fn orphaned(pid: u32) -> bool {
    singularity_config::host::Host::this().parent_of(pid) == Some(1)
}

/// Stop an orphaned Soloist: asked first, then made to, within about 3 s.
fn stop_orphan(pid: u32) {
    let host = singularity_config::host::Host::this();
    tracing::warn!("stopping Soloist {pid}, left running by a Daemon that was killed");
    host.stop(pid, false);
    for _ in 0..30 {
        if host.name_of(pid).as_deref() != Some("soloist") {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    host.stop(pid, true);
}

/// Is a Soloist already running on this state directory? (Its pid file names a live `soloist` process.)
pub fn running_soloist(state: &std::path::Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(state.join("data/soloist.pid")).ok()?.trim().parse().ok()?;
    (singularity_config::host::Host::this().name_of(pid)? == "soloist").then_some(pid)
}
