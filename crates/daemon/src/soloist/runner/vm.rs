//! Soloist in the Soloist VM (D55): macOS's Soloist runner. Soloist is Linux-only, so on the Mac it runs inside a
//! small Linux VM; what that VM is (vfkit, gvproxy, the audio over vsock) sits behind [`Guest`] (`soloist_vm.rs`), and
//! everything the runner asks of it is here, OS-free, so its contract is tested on Linux against a fake VM (Q95).
//!
//! - The guest keeps Soloist's build current itself (`sgl-soloist ensure | check | install`, written by the VM's setup):
//!   it asks Spotify's CDN, stages a newer build, and swaps it in while Soloist isn't running.
//! - Soloist is started by a shell session in the guest; its output comes back on it (the same log parsing as on
//!   Linux), and the API key goes in on its stdin, never on a command line on the Mac. Closing that stdin, or ending
//!   the session, stops Soloist.
//! - Its WebSocket is forwarded to a port on the Mac's loopback; while it waits for pairing, its ZeroConf endpoint is
//!   forwarded too and announced on the LAN.

use super::{Announcement, Launched, Runner};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, watch};

/// Soloist's WebSocket inside the guest.
pub const GUEST_WS: u16 = 7700;

/// What the guest's `sgl-soloist check` prints when it staged a newer build (the VM's setup writes the script: `vm.rs`).
pub const STAGED: &str = "staged";

/// The VM Soloist runs in.
pub trait Guest: Send + Sync + 'static {
    /// Boot it (downloading and setting up what's missing the first time), its audio coming to the Daemon. Progress
    /// the user should see goes to `say`.
    fn boot(&self, say: &(dyn Fn(String) + Sync)) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Power it off, and its audio with it.
    fn power_off(&self) -> impl Future<Output = ()> + Send;

    /// A shell `script` run in the guest's home, as a command to start: its stdin is empty unless set, and it is
    /// stopped when dropped.
    fn command(&self, script: &str) -> tokio::process::Command;

    /// Forward a free port on the Mac's loopback to the guest's `port`; the Mac's port. (The Guest picks it, so a fake
    /// can bind port 0 and hold it: a port picked and let go can be taken before it is bound, #118.)
    fn expose(&self, port: u16) -> impl Future<Output = anyhow::Result<u16>> + Send;

    /// Announce a Spotify Connect pairing endpoint, listening on the Mac's loopback `port`, on the LAN as `name`.
    fn publish(&self, port: u16, name: &str) -> impl Future<Output = anyhow::Result<Announcement>> + Send;
}

/// Soloist in the Soloist VM. Clones share the one VM.
pub struct InVm<G>(Arc<Shared<G>>);

impl<G> Clone for InVm<G> {
    fn clone(&self) -> Self {
        InVm(self.0.clone())
    }
}

struct Shared<G> {
    guest: G,
    data: PathBuf,
    /// the log and the last volume (Soloist's own state is in the guest)
    state: PathBuf,
    /// Soloist's WebSocket, on the Mac's loopback (0 until the VM is up and it is forwarded)
    ws_port: AtomicU16,
    /// a newer Soloist build waits in the guest
    staged: AtomicBool,
    /// the VM is up, its WebSocket forwarded
    up: tokio::sync::Mutex<bool>,
}

/// Starts Soloist in the guest: the key, name and volume come in on stdin. (A background job's stdin is /dev/null in
/// a script: the watcher gets the session's on fd 3 explicitly, and stops Soloist when it closes.)
const LAUNCH: &str = r#"read -r k; read -r n; read -r v; mkdir -p state/data state/cache
exec 3<&0
soloist/soloist --device-name "$n" --api-key "$k" --initial-volume "${v:-40}" --ws 0.0.0.0:7700 --verbose --data-dir state/data --cache-dir state/cache --cache-size 200 </dev/null &
p=$!
(cat <&3 >/dev/null; kill $p 2>/dev/null) >/dev/null 2>&1 &
wait $p"#;

impl<G: Guest> InVm<G> {
    /// Soloist in `guest`; Singularity's data folder is `data_dir` (the log and the volume go under it).
    pub fn new(guest: G, data_dir: PathBuf) -> InVm<G> {
        InVm(Arc::new(Shared {
            guest,
            state: data_dir.join("soloist-state"),
            data: data_dir,
            ws_port: AtomicU16::new(0),
            staged: AtomicBool::new(false),
            up: Default::default(),
        }))
    }

    /// Run `script` in the guest; its stdout.
    async fn run(&self, script: &str) -> anyhow::Result<String> {
        let out = self.0.guest.command(script).stderr(std::process::Stdio::piped()).output().await?;
        if !out.status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The VM up, its WebSocket forwarded, and a current build in it.
    async fn ensure(&self, say: &(dyn Fn(String) + Sync)) -> anyhow::Result<()> {
        let mut up = self.0.up.lock().await;
        if !*up {
            self.0.guest.boot(say).await?;
            let ws = self.0.guest.expose(GUEST_WS).await?;
            self.0.ws_port.store(ws, Ordering::Relaxed);
            *up = true;
        }
        drop(up);
        if self.run("test -x soloist/soloist && echo ok").await.unwrap_or_default().trim() != "ok" {
            say("Downloading Soloist from Spotify into the VM…".into());
        }
        // a staged build goes in now: Soloist isn't running
        self.run("sgl-soloist install >/dev/null; sgl-soloist ensure").await?;
        self.0.staged.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// Ask the CDN for a newer build; the guest stages it. True if one is waiting.
    async fn check(&self) -> anyhow::Result<bool> {
        let out = self.run("sgl-soloist check").await?;
        let staged = out.trim() == STAGED || self.0.staged.load(Ordering::Relaxed);
        self.0.staged.store(staged, Ordering::Relaxed);
        Ok(staged)
    }
}

impl<G: Guest> Runner for InVm<G> {
    fn in_vm(&self) -> bool {
        true
    }

    fn data_dir(&self) -> &Path {
        &self.0.data
    }

    fn state_dir(&self) -> &Path {
        &self.0.state
    }

    fn unavailable(&self) -> &'static str {
        "the Soloist VM isn't up"
    }

    async fn prepare(&self, say: &(dyn Fn(String) + Sync)) -> anyhow::Result<()> {
        self.ensure(say).await.map_err(|e| anyhow::anyhow!("the Soloist VM: {e}"))
    }

    fn keep_current(&self, days: watch::Receiver<Option<u32>>, staged: Arc<Notify>) -> impl Future<Output = ()> + Send + use<G> {
        let vm = self.clone();
        async move {
            let mut failures = 0;
            loop {
                match vm.check().await {
                    Ok(true) => {
                        failures = 0;
                        tracing::info!("new Soloist build staged in the VM");
                        staged.notify_one();
                    }
                    Ok(false) => failures = 0,
                    Err(e) => {
                        failures += 1;
                        tracing::warn!("Soloist update check in the VM failed (keeping the current build): {e}");
                    }
                }
                tokio::time::sleep(super::check_wait(failures, &days)).await;
            }
        }
    }

    fn update_waiting(&self) -> bool {
        self.0.staged.load(Ordering::Relaxed)
    }

    /// (The guest's next `ensure` takes care of it: Soloist starts again at once.)
    fn expired(&self) -> bool {
        false
    }

    /// (The VM's Soloist keeps its data in the guest, where only this Daemon's VM reaches it.)
    fn check_free(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn launch(&self, key: &str, name: &str, volume: Option<u8>) -> anyhow::Result<Launched> {
        use tokio::io::AsyncWriteExt;
        let mut child = self.0.guest.command(LAUNCH).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()?;
        let mut stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin for Soloist's session"))?;
        stdin.write_all(format!("{key}\n{}\n{}\n", name.replace('\n', " "), volume.unwrap_or(40)).as_bytes()).await?;
        stdin.flush().await?;
        Ok(Launched { child, hold: Some(stdin) })
    }

    async fn version(&self) -> String {
        self.run("cat soloist/version").await.unwrap_or_default().trim().to_string()
    }

    /// Its WebSocket through the forwarded port, once it answers there.
    async fn ws_addr(&self) -> Option<String> {
        let port = self.0.ws_port.load(Ordering::Relaxed);
        if port == 0 {
            return None;
        }
        let a = format!("ws://127.0.0.1:{port}");
        matches!(tokio::time::timeout(Duration::from_secs(2), tokio_tungstenite::connect_async(a.as_str())).await, Ok(Ok(_))).then_some(a)
    }

    /// The guest's ZeroConf `port` forwarded to one on the Mac's loopback, and that one announced on the LAN.
    async fn announce(&self, port: u16, name: &str) -> anyhow::Result<Announcement> {
        let local = self.0.guest.expose(port).await?;
        self.0.guest.publish(local, name).await
    }

    /// Does the guest still answer?
    async fn alive(&self) -> bool {
        self.run("true").await.is_ok()
    }

    async fn down(&self) {
        self.0.guest.power_off().await;
        let mut up = self.0.up.lock().await;
        *up = false;
        // its forward went with it
        self.0.ws_port.store(0, Ordering::Relaxed);
    }
}
