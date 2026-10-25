//! The SoloistEngine's macOS host (D55): the same Soloist, supervised the same way, but inside the Soloist VM
//! (`vm.rs`). What differs from Linux lives here: booting the VM, keeping Soloist's build current inside it, starting
//! Soloist over SSH, the forwarded ports, the audio, and pairing.
//!
//! - Soloist is started by an SSH session; its output comes back on it (the same log parsing as on Linux), and the API
//!   key goes in on its stdin, never on a command line on the Mac. When the session ends (the Daemon stops, or the
//!   Engine restarts Soloist), the guest stops Soloist.
//! - Pairing (once, from a phone): Soloist's ZeroConf endpoint is announced on the LAN with `dns-sd` and reached
//!   through a relay run by Apple's own `python3`: macOS's firewall silently withholds LAN connections from programs
//!   it doesn't trust, which includes a locally built `singularityd` (spike, 2026-10-20). Only while pairing.

use crate::tap::Feed;
use crate::vm::{self, Vm};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::process::Child;

pub struct VmHost {
    pub vm: Vm,
    /// the AudioTap's side of the VM's audio
    pub feed: Arc<Feed>,
    /// Soloist's WebSocket, on the Mac's loopback
    pub ws_port: u16,
    /// a newer Soloist build waits in the guest
    pub staged: AtomicBool,
    audio: std::sync::Mutex<Option<crate::pcm::Player>>,
    up: tokio::sync::Mutex<bool>,
}

impl VmHost {
    pub fn new(data_dir: &std::path::Path, feed: Arc<Feed>) -> VmHost {
        VmHost {
            vm: Vm::new(data_dir),
            feed,
            ws_port: vm::free_port().unwrap_or(47_700),
            staged: AtomicBool::new(false),
            audio: Default::default(),
            up: Default::default(),
        }
    }

    /// Make sure the VM is up, its ports forwarded, its audio playing and a Soloist build in it. Progress goes to `say`.
    pub async fn ensure(&self, say: &(dyn Fn(String) + Sync)) -> anyhow::Result<()> {
        let mut up = self.up.lock().await;
        if !*up {
            self.vm.prepare(say).await?;
            // listening before the guest's audio service tries to connect
            *self.audio.lock().expect("audio lock") = Some(crate::pcm::listen(self.vm.pcm_socket(), self.feed.clone())?);
            self.vm.start(say).await?;
            self.vm.expose(&format!("127.0.0.1:{}", self.ws_port), vm::GUEST_WS).await?;
            *up = true;
        }
        drop(up);
        if self.vm.ssh("test -x soloist/soloist && echo ok").await.unwrap_or_default().trim() != "ok" {
            say("Downloading Soloist from Spotify into the VM…".into());
        }
        // a staged build goes in now: Soloist isn't running
        self.vm.ssh("sgl-soloist install >/dev/null; sgl-soloist ensure").await?;
        self.staged.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// Is the VM still running? (If it died, the next `ensure` boots it again.)
    pub async fn alive(&self) -> bool {
        self.vm.ssh("true").await.is_ok()
    }

    pub async fn down(&self) {
        *self.audio.lock().expect("audio lock") = None;
        self.vm.stop().await;
        *self.up.lock().await = false;
    }

    /// The installed build's version line.
    pub async fn version(&self) -> String {
        self.vm.ssh("cat soloist/version").await.unwrap_or_default().trim().to_string()
    }

    /// Ask the CDN for a newer build; stage it in the guest. True if one is waiting.
    pub async fn check(&self) -> anyhow::Result<bool> {
        let out = self.vm.ssh("sgl-soloist check").await?;
        let staged = out.trim() == "staged" || self.staged.load(Ordering::Relaxed);
        self.staged.store(staged, Ordering::Relaxed);
        Ok(staged)
    }

    /// Start Soloist in the guest as `name`: the SSH session is the process (its output is Soloist's). The key and
    /// name go in on its stdin, which the caller keeps open: closing it (or ending the session) stops Soloist.
    pub async fn launch(&self, key: &str, name: &str, volume: Option<u8>) -> anyhow::Result<(Child, tokio::process::ChildStdin)> {
        use tokio::io::AsyncWriteExt;
        // (a background job's stdin is /dev/null in a script: the watcher gets the session's on fd 3 explicitly)
        const SCRIPT: &str = r#"read -r k; read -r n; read -r v; mkdir -p state/data state/cache
exec 3<&0
soloist/soloist --device-name "$n" --api-key "$k" --initial-volume "${v:-40}" --ws 0.0.0.0:7700 --verbose --data-dir state/data --cache-dir state/cache --cache-size 200 </dev/null &
p=$!
(cat <&3 >/dev/null; kill $p 2>/dev/null) >/dev/null 2>&1 &
wait $p"#;
        let mut child = self.vm.ssh_command().arg(SCRIPT).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()?;
        let mut stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin for Soloist's session"))?;
        stdin.write_all(format!("{key}\n{}\n{}\n", name.replace('\n', " "), volume.unwrap_or(40)).as_bytes()).await?;
        stdin.flush().await?;
        Ok((child, stdin))
    }

    /// Announce Soloist's ZeroConf endpoint (guest `port`) on the LAN as `name`, for pairing from a phone.
    pub async fn announce(&self, port: u16, name: &str) -> anyhow::Result<Announcement> {
        let local = vm::free_port().ok_or_else(|| anyhow::anyhow!("no free port"))?;
        self.vm.expose(&format!("127.0.0.1:{local}"), port).await?;
        let lan_port = vm::free_port().ok_or_else(|| anyhow::anyhow!("no free port"))?;
        let ip = lan_ip().await.ok_or_else(|| anyhow::anyhow!("this Mac isn't on a network (wifi or ethernet)"))?;
        let relay = crate::programs::tokio_command("/usr/bin/python3").args(["-c", RELAY, &lan_port.to_string(), &local.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true).spawn()?;
        let host = format!("singularity-{}.local", std::process::id());
        let dns = crate::programs::tokio_command("/usr/bin/dns-sd")
            .args(["-P", name, "_spotify-connect._tcp", "local", &lan_port.to_string(), &host, &ip, "CPath=/zc", "VERSION=1.0", "Stack=SP"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        tracing::info!("pairing: {name:?} announced on {ip}:{lan_port}");
        Ok(Announcement { _children: vec![relay, dns] })
    }
}

/// Soloist announced for pairing, until dropped.
pub struct Announcement {
    _children: Vec<Child>,
}

/// The Mac's address on its LAN (wifi or ethernet; a VPN's tunnel can be the default route, so not that).
async fn lan_ip() -> Option<String> {
    for ifc in ["en0", "en1", "en2", "en3"] {
        let out = crate::programs::tokio_command("/usr/sbin/ipconfig").args(["getifaddr", ifc]).output().await.ok()?;
        let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !ip.is_empty() {
            return Some(ip);
        }
    }
    None
}

/// A TCP relay, LAN port → loopback port, in Apple's python (the firewall admits it).
const RELAY: &str = r#"
import socket, sys, threading
listen, target = int(sys.argv[1]), ("127.0.0.1", int(sys.argv[2]))
def pipe(a, b):
    try:
        while (d := a.recv(65536)):
            b.sendall(d)
    except OSError:
        pass
    for s in (a, b):
        try: s.shutdown(socket.SHUT_RDWR)
        except OSError: pass
srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); srv.bind(("0.0.0.0", listen)); srv.listen(16)
while True:
    c, _ = srv.accept()
    try: u = socket.create_connection(target)
    except OSError: c.close(); continue
    threading.Thread(target=pipe, args=(c, u), daemon=True).start()
    threading.Thread(target=pipe, args=(u, c), daemon=True).start()
"#;
