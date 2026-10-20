//! The Soloist VM (D55): Spotify's Soloist, Linux-only, in a small Linux VM on macOS, so the Mac gets the same
//! lossless Engine as Linux (`research/2026-10-soloist-vm-spike.md`).
//!
//! - The VM: Debian's arm64 cloud image under Apple's Virtualization framework through `vfkit`, networked by
//!   `gvproxy` (user-mode: the guest's connections are made by a normal Mac process, so a VPN that drops the
//!   framework's NAT doesn't matter). Both helpers are downloaded once, pinned by checksum; the image is Debian's
//!   latest, checked against Debian's SHA512SUMS. Nothing of Spotify's is shipped: the guest downloads Soloist from
//!   Spotify's CDN itself.
//! - First boot sets the guest up with cloud-init: a `singularity` user reached over SSH (a key made for this VM,
//!   loopback only), PulseAudio with a null sink, a PCM server (`sgl-pcm`: the sink's monitor as raw float32 stereo
//!   44.1 kHz on TCP 5005) and an update script (`sgl-soloist`: ensure / check / install the arm64 build).
//! - Ports the Daemon reaches through gvproxy (`expose`): SSH, Soloist's WebSocket (7700), the PCM server, and while
//!   pairing Soloist's ZeroConf endpoint.
//! - It costs the RAM it's given (the framework keeps every touched page): 256 MB, 512 for the first boot's setup.

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::{Child, Command};

const VFKIT_URL: &str = "https://github.com/crc-org/vfkit/releases/download/v0.6.4/vfkit";
const VFKIT_SHA256: &str = "0ed83fc8ca7aa708598835480dba1362406aa7cd1dab3b27464eb76327d9652d";
const GVPROXY_URL: &str = "https://github.com/containers/gvisor-tap-vsock/releases/download/v0.8.9/gvproxy-darwin";
const GVPROXY_SHA256: &str = "c6f7b4bc7f21bf810b5cf54e04d979b014c5d96472a03a9e97fe62a00940067c";
const IMAGE_DIR: &str = "https://cloud.debian.org/images/cloud/trixie/latest";
const IMAGE: &str = "debian-13-genericcloud-arm64.tar.xz";
/// gvproxy's DHCP gives this MAC the guest address below
const MAC: &str = "5a:94:ef:e4:0c:ee";
pub const GUEST: &str = "192.168.127.2";
/// Soloist's WebSocket and the PCM server inside the guest
pub const GUEST_WS: u16 = 7700;
pub const GUEST_PCM: u16 = 5005;
/// bump when the guest's setup changes: a new instance id makes cloud-init run again
const SETUP_VERSION: u32 = 1;

/// The VM's files: `$XDG_DATA_HOME/singularity/vm/` (owner-only).
pub struct Vm {
    pub dir: PathBuf,
    /// where gvproxy's and vfkit's sockets go (Unix socket paths must be short)
    run: PathBuf,
    ssh_port: u16,
    children: tokio::sync::Mutex<Vec<Child>>,
}

impl Vm {
    pub fn new(data_dir: &Path) -> Vm {
        Vm { dir: data_dir.join("vm"), run: std::env::temp_dir().join("sgl-vm"), ssh_port: free_port().unwrap_or(2223), children: Default::default() }
    }

    fn bin(&self, name: &str) -> PathBuf {
        self.dir.join("bin").join(name)
    }

    /// Download what's missing (helpers, image) and write the guest's setup. `say` gets progress for the user.
    pub async fn prepare(&self, say: &(dyn Fn(String) + Sync)) -> anyhow::Result<()> {
        std::fs::create_dir_all(self.dir.join("bin"))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))?;
        for (name, url, sha) in [("vfkit", VFKIT_URL, VFKIT_SHA256), ("gvproxy", GVPROXY_URL, GVPROXY_SHA256)] {
            if self.bin(name).exists() {
                continue;
            }
            say(format!("Downloading {name} (runs the Soloist VM)…"));
            let part = self.bin(&format!("{name}.part"));
            download(url, &part).await?;
            let got = sha256(&part).await?;
            if got != sha {
                let _ = std::fs::remove_file(&part);
                anyhow::bail!("{name} didn't match its checksum (got {got})");
            }
            std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(&part, self.bin(name))?;
        }
        if !self.dir.join("disk.raw").exists() {
            say("Downloading a small Debian Linux for Soloist (~210 MB, once)…".into());
            let xz = self.dir.join(IMAGE);
            download(&format!("{IMAGE_DIR}/{IMAGE}"), &xz).await?;
            let sums = reqwest::get(format!("{IMAGE_DIR}/SHA512SUMS")).await?.error_for_status()?.text().await?;
            let want = sums.lines().find(|l| l.ends_with(IMAGE)).and_then(|l| l.split_whitespace().next()).ok_or_else(|| anyhow::anyhow!("Debian's SHA512SUMS doesn't list {IMAGE}"))?.to_string();
            let got = shasum(&xz, "512").await?;
            if got != want {
                let _ = std::fs::remove_file(&xz);
                anyhow::bail!("the Debian image didn't match Debian's checksum");
            }
            say("Unpacking it…".into());
            let ok = Command::new("/usr/bin/tar").arg("-xJf").arg(&xz).arg("-C").arg(&self.dir).status().await?.success();
            let _ = std::fs::remove_file(&xz);
            if !ok {
                anyhow::bail!("couldn't unpack the Debian image");
            }
            // the archive's disk is full-size on disk: copy it sparse (only the blocks with data take space)
            let dir = self.dir.clone();
            tokio::task::spawn_blocking(move || sparse_copy(&dir.join("disk.raw"), &dir.join("disk.sparse")).and_then(|_| std::fs::rename(dir.join("disk.sparse"), dir.join("disk.raw")))).await??;
        }
        if !self.dir.join("id_ed25519").exists() {
            let ok = Command::new("/usr/bin/ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", "singularity-vm", "-f"]).arg(self.dir.join("id_ed25519")).status().await?.success();
            if !ok {
                anyhow::bail!("couldn't make the VM's SSH key");
            }
        }
        let key = std::fs::read_to_string(self.dir.join("id_ed25519.pub"))?;
        let seed = self.dir.join("seed");
        std::fs::create_dir_all(&seed)?;
        std::fs::write(seed.join("meta-data"), format!("instance-id: singularity-vm-{SETUP_VERSION}\nlocal-hostname: singularity-vm\n"))?;
        std::fs::write(seed.join("user-data"), user_data(key.trim()))?;
        let iso = self.dir.join("seed.iso");
        let _ = std::fs::remove_file(&iso);
        let ok = Command::new("/usr/bin/hdiutil").args(["makehybrid", "-quiet", "-iso", "-joliet", "-default-volume-name", "cidata", "-o"]).arg(&iso).arg(&seed).status().await?.success();
        if !ok {
            anyhow::bail!("couldn't write the VM's setup disk");
        }
        Ok(())
    }

    /// Boot the VM and wait until its setup is done and SSH answers. `say` gets progress.
    pub async fn start(&self, say: &(dyn Fn(String) + Sync)) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.run)?;
        for f in ["gv.sock", "gvctl.sock"] {
            let _ = std::fs::remove_file(self.run.join(f));
        }
        let first = !self.dir.join("efi.vars").exists() || !self.dir.join(format!("setup-{SETUP_VERSION}")).exists();
        let log = |name: &str| std::fs::File::create(self.dir.join(name)).map(std::process::Stdio::from).unwrap_or_else(|_| std::process::Stdio::null());
        let gv = Command::new(self.bin("gvproxy"))
            .arg("-listen-vfkit")
            .arg(format!("unixgram://{}", self.run.join("gv.sock").display()))
            .arg("-listen")
            .arg(format!("unix://{}", self.run.join("gvctl.sock").display()))
            .args(["-ssh-port", &self.ssh_port.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(log("gvproxy.log"))
            .kill_on_drop(true)
            .spawn()?;
        for _ in 0..50 {
            if self.run.join("gv.sock").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let efi = format!("efi,variable-store={}{}", self.dir.join("efi.vars").display(), if self.dir.join("efi.vars").exists() { "" } else { ",create" });
        let memory = if first { "512" } else { "256" };
        let vm = Command::new(self.bin("vfkit"))
            .args(["--cpus", "2", "--memory", memory, "--bootloader", &efi])
            .arg("--device")
            .arg(format!("virtio-blk,path={}", self.dir.join("disk.raw").display()))
            .arg("--device")
            .arg(format!("virtio-blk,path={}", self.dir.join("seed.iso").display()))
            .arg("--device")
            .arg(format!("virtio-net,unixSocketPath={},mac={MAC}", self.run.join("gv.sock").display()))
            .arg("--device")
            .arg(format!("virtio-serial,logFilePath={}", self.dir.join("console.log").display()))
            .args(["--device", "virtio-rng"])
            .stdout(std::process::Stdio::null())
            .stderr(log("vfkit.log"))
            .kill_on_drop(true)
            .spawn()?;
        *self.children.lock().await = vec![vm, gv];
        say(if first { "Setting up the Soloist VM (first time: a minute or two)…".into() } else { "Starting the Soloist VM…".into() });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(if first { 600 } else { 90 });
        loop {
            if let Ok(out) = self.ssh("test -f /var/lib/singularity-ready && echo ready").await
                && out.trim() == "ready"
            {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("the Soloist VM didn't come up (see {})", self.dir.join("console.log").display());
            }
            if self.children.lock().await.iter_mut().any(|c| c.try_wait().ok().flatten().is_some()) {
                anyhow::bail!("the Soloist VM stopped (see {})", self.dir.join("vfkit.log").display());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let _ = std::fs::write(self.dir.join(format!("setup-{SETUP_VERSION}")), "");
        if first {
            // the setup needed 512 MB; from now on the VM runs in 256
            tracing::info!("Soloist VM set up; restarting it at its normal size");
            self.stop().await;
            return Box::pin(self.start(say)).await;
        }
        Ok(())
    }

    /// Power the VM off (it's only ever a cache of Soloist's state: a hard stop after 5 s is fine).
    pub async fn stop(&self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.ssh("sudo systemctl poweroff")).await;
        for mut c in self.children.lock().await.drain(..) {
            let _ = tokio::time::timeout(Duration::from_secs(5), c.wait()).await;
            let _ = c.kill().await;
        }
    }

    /// SSH into the guest: the base command (add the remote command, stdin, pipes).
    pub fn ssh_command(&self) -> Command {
        let mut c = Command::new("/usr/bin/ssh");
        c.arg("-i")
            .arg(self.dir.join("id_ed25519"))
            .args(["-p", &self.ssh_port.to_string(), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=5", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=10", "singularity@127.0.0.1"])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        c
    }

    /// Run `script` in the guest; its stdout.
    pub async fn ssh(&self, script: &str) -> anyhow::Result<String> {
        let out = self.ssh_command().arg(script).stderr(std::process::Stdio::piped()).output().await?;
        if !out.status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Forward `127.0.0.1:<local>` (or any address given) to the guest's `port`, through gvproxy.
    pub async fn expose(&self, local: &str, port: u16) -> anyhow::Result<()> {
        self.control("expose", &serde_json::json!({ "local": local, "remote": format!("{GUEST}:{port}") })).await
    }

    pub async fn unexpose(&self, local: &str) {
        let _ = self.control("unexpose", &serde_json::json!({ "local": local })).await;
    }

    /// One call to gvproxy's HTTP API on its control socket.
    async fn control(&self, what: &str, body: &serde_json::Value) -> anyhow::Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::UnixStream::connect(self.run.join("gvctl.sock")).await?;
        let body = body.to_string();
        s.write_all(format!("POST /services/forwarder/{what} HTTP/1.0\r\nHost: gvproxy\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await?;
        let mut answer = String::new();
        s.read_to_string(&mut answer).await?;
        if !answer.starts_with("HTTP/1.0 200") && !answer.starts_with("HTTP/1.1 200") {
            anyhow::bail!("gvproxy {what}: {}", answer.lines().last().unwrap_or_default());
        }
        Ok(())
    }
}

/// A port nobody uses on loopback right now.
pub fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok().map(|a| a.port())
}

async fn download(url: &str, to: &Path) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut r = reqwest::get(url).await?.error_for_status()?;
    let mut f = tokio::fs::File::create(to).await?;
    while let Some(chunk) = r.chunk().await? {
        f.write_all(&chunk).await?;
    }
    f.flush().await?;
    Ok(())
}

async fn sha256(p: &Path) -> anyhow::Result<String> {
    shasum(p, "256").await
}

/// `shasum -a <bits>` (macOS's own) of a file.
async fn shasum(p: &Path, bits: &str) -> anyhow::Result<String> {
    let out = Command::new("/usr/bin/shasum").args(["-a", bits]).arg(p).output().await?;
    Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or_default().to_string())
}

/// Copy `from` to `to` leaving all-zero blocks as holes, so the file takes only the space its data needs.
pub fn sparse_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut src = std::fs::File::open(from)?;
    let len = src.metadata()?.len();
    let mut dst = std::fs::File::create(to)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if buf[..n].iter().all(|b| *b == 0) {
            dst.seek(SeekFrom::Current(n as i64))?;
        } else {
            dst.write_all(&buf[..n])?;
        }
    }
    dst.set_len(len)?;
    dst.sync_all()
}

/// The guest's setup (cloud-init): the user, the packages, the PCM server and Soloist's update script.
pub fn user_data(ssh_key: &str) -> String {
    format!(
        r#"#cloud-config
users:
  - name: singularity
    shell: /bin/bash
    lock_passwd: true
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
      - {ssh_key}
package_update: true
packages: [pulseaudio, pulseaudio-utils, libatomic1, socat, curl, ca-certificates]
write_files:
  - path: /usr/local/bin/sgl-pcm
    permissions: "0755"
    content: |
      #!/bin/sh
      # what Soloist plays, as raw float32 stereo 44.1 kHz, to whoever connects (the Daemon, through gvproxy)
      exec socat TCP-LISTEN:{GUEST_PCM},reuseaddr,fork EXEC:"parec --raw --format=float32le --rate=44100 --channels=2 --latency-msec=30 -d auto_null.monitor"
  - path: /etc/systemd/user/sgl-pcm.service
    content: |
      [Unit]
      Description=Singularity: Soloist's audio to the Mac
      After=pulseaudio.service
      Wants=pulseaudio.service
      [Service]
      ExecStart=/usr/local/bin/sgl-pcm
      Restart=always
      [Install]
      WantedBy=default.target
  - path: /usr/local/bin/sgl-soloist
    permissions: "0755"
    content: |
      #!/bin/sh
      # Soloist's arm64 build from Spotify's CDN, kept like the Daemon keeps it on Linux (update.rs):
      #   ensure: download if there's none · check: stage a newer one (prints "staged") · install: swap in a staged one
      set -e
      cd "$HOME"
      url=https://soloist-builds.spotifycdn.com/soloist_release_arm64.tar.gz
      stage() {{
        etag=$(curl -sfI "$url" | tr -d '\r' | awk 'tolower($1)=="etag:"{{print $2}}')
        [ -n "$etag" ] && [ "$etag" = "$(cat soloist/etag 2>/dev/null)" ] && {{ echo current; return; }}
        rm -rf soloist.new && mkdir soloist.new
        curl -sf "$url" | tar -xz -C soloist.new
        soloist.new/soloist --version > soloist.new/version
        echo "$etag" > soloist.new/etag
        echo staged
      }}
      install() {{ [ -d soloist.new ] || return 0; rm -rf soloist.old; [ -d soloist ] && mv soloist soloist.old; mv soloist.new soloist; echo installed; }}
      case "$1" in
        ensure) [ -x soloist/soloist ] || {{ stage >/dev/null; install; }}; cat soloist/version ;;
        check) stage ;;
        install) install ;;
      esac
runcmd:
  - loginctl enable-linger singularity
  - systemctl --global enable sgl-pcm.service
  # the disk file on the Mac only takes what's in use: drop the package caches and tell it what's free
  - apt-get clean
  - rm -rf /var/lib/apt/lists/*
  - fstrim -a
  - touch /var/lib/singularity-ready
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sparse_copy_keeps_the_data_and_drops_the_zeros() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let mut data = vec![0u8; 8 << 20];
        data[3 << 20] = 7;
        data[(8 << 20) - 1] = 9;
        std::fs::write(&a, &data).unwrap();
        sparse_copy(&a, &b).unwrap();
        assert_eq!(std::fs::read(&b).unwrap(), data);
    }

    #[test]
    fn the_setup_names_the_ports_the_daemon_uses() {
        let u = user_data("ssh-ed25519 AAAA test");
        assert!(u.starts_with("#cloud-config\n") && u.contains("ssh-ed25519 AAAA test") && u.contains("TCP-LISTEN:5005") && u.contains("soloist_release_arm64"));
        assert!(!u.contains("{{") && !u.contains("}}"), "format escapes resolved");
    }
}
