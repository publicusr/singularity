//! The Soloist VM on the Mac (D55): the [`Guest`] the VM's Soloist runner (`soloist/runner/vm.rs`) runs Soloist in.
//! What's macOS's lives here: booting the VM (`vm.rs`), playing its audio, reaching the guest over SSH and gvproxy's
//! forwards, and announcing pairing on the LAN. (It builds on any OS; only a Mac can run it: D49's seam.)
//!
//! - Commands in the guest go over SSH, to the guest's pinned host key only.
//! - Pairing (once, from a phone): Soloist's ZeroConf endpoint is announced on the LAN with `dns-sd` and reached
//!   through a relay run by Apple's own `python3`: macOS's firewall silently withholds LAN connections from programs
//!   it doesn't trust, which includes a locally built `singularityd` (spike, 2026-10-20). Only while pairing.

use crate::soloist::runner::Announcement;
use crate::soloist::runner::vm::Guest;
use crate::tap::Feed;
use crate::vm::{self, Vm};
use std::sync::Arc;

pub struct Mac {
    vm: Vm,
    /// the AudioTap's side of the VM's audio
    feed: Arc<Feed>,
    audio: std::sync::Mutex<Option<crate::pcm::Player>>,
}

impl Mac {
    /// The VM whose files are under `data_dir`; its audio also goes to `feed`.
    pub fn new(data_dir: &std::path::Path, feed: Arc<Feed>) -> Mac {
        Mac { vm: Vm::new(data_dir), feed, audio: Default::default() }
    }
}

impl Guest for Mac {
    async fn boot(&self, say: &(dyn Fn(String) + Sync)) -> anyhow::Result<()> {
        anyhow::ensure!(cfg!(target_os = "macos"), "it runs on macOS only");
        self.vm.prepare(say).await?;
        // listening before the guest's audio service tries to connect
        *self.audio.lock().expect("audio lock") = Some(crate::pcm::listen(self.vm.pcm_socket(), self.feed.clone())?);
        self.vm.start(say).await
    }

    async fn power_off(&self) {
        *self.audio.lock().expect("audio lock") = None;
        self.vm.stop().await;
    }

    fn command(&self, script: &str) -> tokio::process::Command {
        let mut c = self.vm.ssh_command();
        c.arg(script);
        c
    }

    async fn expose(&self, port: u16) -> anyhow::Result<u16> {
        // gvproxy binds it, so it is picked here, just before
        let local = vm::free_port().ok_or_else(|| anyhow::anyhow!("no free port"))?;
        self.vm.expose(&format!("127.0.0.1:{local}"), port).await?;
        Ok(local)
    }

    async fn publish(&self, port: u16, name: &str) -> anyhow::Result<Announcement> {
        let lan_port = vm::free_port().ok_or_else(|| anyhow::anyhow!("no free port"))?;
        let ip = lan_ip().await.ok_or_else(|| anyhow::anyhow!("this Mac isn't on a network (wifi or ethernet)"))?;
        let relay = crate::programs::tokio_command("/usr/bin/python3").args(["-c", RELAY, &lan_port.to_string(), &port.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true).spawn()?;
        let host = format!("singularity-{}.local", std::process::id());
        let dns = crate::programs::tokio_command("/usr/bin/dns-sd")
            .args(["-P", name, "_spotify-connect._tcp", "local", &lan_port.to_string(), &host, &ip, "CPath=/zc", "VERSION=1.0", "Stack=SP"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        tracing::info!("pairing: {name:?} announced on {ip}:{lan_port}");
        Ok(Announcement::held(vec![relay, dns]))
    }
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
