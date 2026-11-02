//! The Soloist VM (D55): Spotify's Soloist, Linux-only, in a small Linux VM on macOS, so the Mac gets the same
//! lossless Engine as Linux (`research/2026-10-soloist-vm-spike.md`).
//!
//! - The VM: Debian's arm64 cloud image under Apple's Virtualization framework through `vfkit`, networked by
//!   `gvproxy` (user-mode: the guest's connections are made by a normal Mac process, so a VPN that drops the
//!   framework's NAT doesn't matter). Both helpers are downloaded once, pinned by checksum; the image is Debian's
//!   latest, checked against Debian's SHA512SUMS. Nothing of Spotify's is shipped: the guest downloads Soloist from
//!   Spotify's CDN itself.
//! - First boot sets the guest up with cloud-init: a `singularity` user reached over SSH (a key made for this VM,
//!   loopback only), PulseAudio playing into a pipe sink (`sgl`, raw float32 stereo 44.1 kHz, its own clock; it sleeps
//!   whenever nothing plays, so a paused Soloist moves no data at all), `sgl-pcm` sending that pipe to the Mac over
//!   virtio-vsock (port 5005, the Daemon listening on `pcm_socket`), and an update script (`sgl-soloist`: ensure /
//!   check / install the arm64 build).
//! - Ports the Daemon reaches through gvproxy (`expose`): SSH, Soloist's WebSocket (7700), and while pairing Soloist's
//!   ZeroConf endpoint. The audio skips gvproxy's network stack (vsock): measured, it was most of the cost.
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
/// Soloist's WebSocket inside the guest, and the vsock port its audio comes to the Mac on
pub const GUEST_WS: u16 = 7700;
pub const GUEST_PCM: u16 = 5005;
/// bump when the guest's setup changes: a new instance id makes cloud-init run again (2: the pipe sink over vsock,
/// and #S7's size caps on the guest's `curl`: 3, the pinned host key: 4)
const SETUP_VERSION: u32 = 4;
/// The name the guest's host key is pinned under (`HostKeyAlias`): its port changes with every Daemon.
const HOST_ALIAS: &str = "singularity-vm";

/// The most each download may be: the two helpers are tens of MB, Debian's image ~210 MB (a few hundred MB means the
/// server sent something else).
const MAX_HELPER: u64 = 128 * 1024 * 1024;
const MAX_IMAGE: u64 = 1024 * 1024 * 1024;
/// Debian's `SHA512SUMS` and a checksum tool's output are lines of text.
const MAX_TEXT: u64 = 1024 * 1024;

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

    /// Where the guest's audio arrives (the Daemon listens here; vfkit connects the guest's vsock to it).
    pub fn pcm_socket(&self) -> PathBuf {
        self.run.join("pcm.sock")
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
            download(url, &part, MAX_HELPER).await?;
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
            download(&format!("{IMAGE_DIR}/{IMAGE}"), &xz, MAX_IMAGE).await?;
            let sums = fetch_text(&format!("{IMAGE_DIR}/SHA512SUMS")).await?;
            let want = sums.lines().find(|l| l.ends_with(IMAGE)).and_then(|l| l.split_whitespace().next()).ok_or_else(|| anyhow::anyhow!("Debian's SHA512SUMS doesn't list {IMAGE}"))?.to_string();
            let got = shasum(&xz, "512").await?;
            if got != want {
                let _ = std::fs::remove_file(&xz);
                anyhow::bail!("the Debian image didn't match Debian's checksum");
            }
            say("Unpacking it…".into());
            let ok = Command::new(crate::programs::Tool::Tar.path()?).env_remove(crate::programs::SECRET_ENV).arg("-xJf").arg(&xz).arg("-C").arg(&self.dir).status().await?.success();
            let _ = std::fs::remove_file(&xz);
            if !ok {
                anyhow::bail!("couldn't unpack the Debian image");
            }
            // the archive's disk is full-size on disk: copy it sparse (only the blocks with data take space)
            let dir = self.dir.clone();
            tokio::task::spawn_blocking(move || sparse_copy(&dir.join("disk.raw"), &dir.join("disk.sparse")).and_then(|_| std::fs::rename(dir.join("disk.sparse"), dir.join("disk.raw")))).await??;
        }
        // our key, and the guest's host key: made here and handed to the guest, so `ssh` accepts only that guest (a
        // local user who took the loopback port while gvproxy was down can't pose as it and be sent the Soloist key)
        for (file, what) in [("id_ed25519", "SSH key"), ("host_ed25519", "host key")] {
            if !self.dir.join(file).exists() {
                let ok = Command::new("/usr/bin/ssh-keygen").env_remove(crate::programs::SECRET_ENV).args(["-q", "-t", "ed25519", "-N", "", "-C", "singularity-vm", "-f"]).arg(self.dir.join(file)).status().await?.success();
                if !ok {
                    anyhow::bail!("couldn't make the VM's {what}");
                }
            }
        }
        let key = std::fs::read_to_string(self.dir.join("id_ed25519.pub"))?;
        let host = HostKey { private: std::fs::read_to_string(self.dir.join("host_ed25519"))?, public: std::fs::read_to_string(self.dir.join("host_ed25519.pub"))? };
        std::fs::write(self.dir.join("known_hosts"), known_hosts(&host.public))?;
        let seed = self.dir.join("seed");
        std::fs::create_dir_all(&seed)?;
        std::fs::write(seed.join("meta-data"), format!("instance-id: singularity-vm-{SETUP_VERSION}\nlocal-hostname: singularity-vm\n"))?;
        std::fs::write(seed.join("user-data"), user_data(key.trim(), &host))?;
        let iso = self.dir.join("seed.iso");
        let _ = std::fs::remove_file(&iso);
        let ok = Command::new("/usr/bin/hdiutil").env_remove(crate::programs::SECRET_ENV).args(["makehybrid", "-quiet", "-iso", "-joliet", "-default-volume-name", "cidata", "-o"]).arg(&iso).arg(&seed).status().await?.success();
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
        let gv = crate::programs::tokio_command(self.bin("gvproxy"))
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
        // first setup: 2 CPUs and 512 MB for apt; then 1 and 256 (each idle vCPU costs the Mac timer ticks)
        let (cpus, memory) = if first { ("2", "512") } else { ("1", "256") };
        let vm = crate::programs::tokio_command(self.bin("vfkit"))
            .args(["--cpus", cpus, "--memory", memory, "--bootloader", &efi])
            .arg("--device")
            .arg(format!("virtio-blk,path={}", self.dir.join("disk.raw").display()))
            .arg("--device")
            .arg(format!("virtio-blk,path={}", self.dir.join("seed.iso").display()))
            .arg("--device")
            .arg(format!("virtio-net,unixSocketPath={},mac={MAC}", self.run.join("gv.sock").display()))
            .arg("--device")
            .arg(format!("virtio-serial,logFilePath={}", self.dir.join("console.log").display()))
            .args(["--device", "virtio-rng"])
            .arg("--device")
            .arg(format!("virtio-vsock,port={GUEST_PCM},socketURL={}", self.pcm_socket().display()))
            .stdout(std::process::Stdio::null())
            .stderr(log("vfkit.log"))
            .kill_on_drop(true)
            .spawn()?;
        *self.children.lock().await = vec![vm, gv];
        say(if first { "Setting up the Soloist VM (first time: a minute or two)…".into() } else { "Starting the Soloist VM…".into() });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(if first { 600 } else { 90 });
        loop {
            // gvproxy and vfkit still ours before anything is asked of whoever answers on the port
            if self.children.lock().await.iter_mut().any(|c| c.try_wait().ok().flatten().is_some()) {
                anyhow::bail!("the Soloist VM stopped (see {})", self.dir.join("vfkit.log").display());
            }
            if let Ok(out) = self.ssh(&format!("test -f /var/lib/singularity-ready-{SETUP_VERSION} && echo ready")).await
                && out.trim() == "ready"
            {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("the Soloist VM didn't come up (see {})", self.dir.join("console.log").display());
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

    /// SSH into the guest: the base command (add the remote command, stdin, pipes). Only the guest's pinned host key
    /// is accepted (`known_hosts`, written by `prepare`).
    pub fn ssh_command(&self) -> Command {
        let mut c = crate::programs::tokio_command("/usr/bin/ssh");
        c.arg("-i")
            .arg(self.dir.join("id_ed25519"))
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", self.dir.join("known_hosts").display()))
            .args(["-p", &self.ssh_port.to_string(), "-o", "StrictHostKeyChecking=yes", "-o", &format!("HostKeyAlias={HOST_ALIAS}"), "-o", "HostKeyAlgorithms=ssh-ed25519", "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=5", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=10", "singularity@127.0.0.1"])
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

/// An HTTP client with timeouts (a stalled answer must not wait forever).
fn http() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(Duration::from_secs(600)).connect_timeout(Duration::from_secs(15)).user_agent(concat!("singularityd/", env!("CARGO_PKG_VERSION"))).build()?)
}

/// Stream `url` to `to`, giving up past `cap` bytes and leaving no partial file behind (audit S7: before this, a
/// broken answer could fill the disk and the checksum was only compared afterwards).
async fn download(url: &str, to: &Path, cap: u64) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let r = async {
        const MB: u64 = 1024 * 1024;
        let resp = http()?.get(url).send().await?.error_for_status()?;
        if let Some(len) = resp.content_length() {
            anyhow::ensure!(len <= cap, "the download is {} MB, over the {} MB cap", len.div_ceil(MB), cap.div_ceil(MB));
        }
        let mut f = tokio::fs::File::create(to).await?;
        let mut body = resp.bytes_stream();
        let mut written = 0u64;
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            written += chunk.len() as u64;
            anyhow::ensure!(written <= cap, "the download is over the {} MB cap", cap.div_ceil(MB));
            f.write_all(&chunk).await?;
        }
        f.flush().await?;
        anyhow::Ok(())
    }
    .await;
    if r.is_err() {
        let _ = tokio::fs::remove_file(to).await;
    }
    r
}

/// A small text answer (Debian's `SHA512SUMS`), capped too.
async fn fetch_text(url: &str) -> anyhow::Result<String> {
    use futures_util::StreamExt;
    let resp = http()?.get(url).send().await?.error_for_status()?;
    let mut body = resp.bytes_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        anyhow::ensure!((buf.len() + chunk.len()) as u64 <= MAX_TEXT, "the answer is over the {} KB cap", MAX_TEXT / 1024);
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

async fn sha256(p: &Path) -> anyhow::Result<String> {
    shasum(p, "256").await
}

/// `shasum -a <bits>` (macOS's own) of a file.
async fn shasum(p: &Path, bits: &str) -> anyhow::Result<String> {
    let out = crate::programs::tokio_command("/usr/bin/shasum").args(["-a", bits]).arg(p).output().await?;
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

/// The guest's SSH host key (OpenSSH files), made on the Mac.
pub struct HostKey {
    pub private: String,
    pub public: String,
}

/// The one line `ssh` checks the guest against.
fn known_hosts(host_public: &str) -> String {
    let mut parts = host_public.split_whitespace();
    format!("{HOST_ALIAS} {} {}\n", parts.next().unwrap_or_default(), parts.next().unwrap_or_default())
}

/// The guest's setup (cloud-init): the user, its pinned host key, the packages, the PCM server and Soloist's update
/// script.
pub fn user_data(ssh_key: &str, host: &HostKey) -> String {
    let private: String = host.private.trim().lines().map(|l| format!("    {l}\n")).collect();
    let public = host.public.trim();
    format!(
        r#"#cloud-config
ssh_deletekeys: true
ssh_genkeytypes: []
ssh_keys:
  ed25519_private: |
{private}  ed25519_public: {public}
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
  - path: /etc/pulse/default.pa.d/singularity.pa
    content: |
      # Soloist plays into a pipe on its own clock; it sleeps (writes nothing) whenever nothing plays
      load-module module-pipe-sink file=/tmp/sgl-pcm sink_name=sgl format=float32le rate=44100 channels=2 use_system_clock_for_timing=yes
      set-default-sink sgl
  - path: /usr/local/bin/sgl-pcm
    permissions: "0755"
    content: |
      #!/bin/sh
      # the pipe sink's audio (raw float32 stereo 44.1 kHz) to the Mac over vsock (host CID 2), where the Daemon listens
      until [ -p /tmp/sgl-pcm ]; do sleep 1; done
      exec socat -u OPEN:/tmp/sgl-pcm,rdonly VSOCK-CONNECT:2:{GUEST_PCM}
  - path: /etc/systemd/user/sgl-pcm.service
    content: |
      [Unit]
      Description=Singularity: Soloist's audio to the Mac
      After=pulseaudio.service
      Wants=pulseaudio.service
      [Service]
      ExecStart=/usr/local/bin/sgl-pcm
      Restart=always
      RestartSec=2
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
        # capped like the Daemon's update (update.rs): a broken or hostile CDN answer must not fill the guest's disk
        curl -sf --max-filesize 268435456 --max-time 900 "$url" | tar -xz -C soloist.new
        # a regular executable and a build-sized tree, never a link or a bomb
        [ -f soloist.new/soloist ] && [ ! -L soloist.new/soloist ] || {{ rm -rf soloist.new; return 1; }}
        [ "$(du -sb soloist.new | cut -f1)" -le 536870912 ] || {{ rm -rf soloist.new; return 1; }}
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
  - touch /var/lib/singularity-ready-{SETUP_VERSION}
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

    /// S7: a download over its cap fails, and no partial file is left behind (before the fix `vm::download` had no
    /// cap and no timeout, and the checksum came only after the whole body was on disk).
    #[tokio::test]
    async fn a_download_over_its_cap_leaves_no_file() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/x", l.local_addr().unwrap());
        let announce = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let a = announce.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf).await;
                let body = vec![7u8; 4096];
                // first a Content-Length over the cap, then (no length) the bytes counted as they arrive
                let hdr = if a.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                } else {
                    "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_string()
                };
                let _ = s.write_all(hdr.as_bytes()).await;
                let _ = s.write_all(&body).await;
            }
        });
        let dir = tempfile::tempdir().unwrap();
        for attempt in 0..3 {
            let to = dir.path().join(format!("part{attempt}"));
            let err = download(&url, &to, 1024).await.unwrap_err().to_string();
            assert!(err.contains("cap"), "{err}");
            assert!(!to.exists(), "a partial file is left behind");
        }
    }

    #[test]
    fn the_setup_names_the_ports_the_daemon_uses() {
        let host = HostKey { private: "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaA\n-----END OPENSSH PRIVATE KEY-----\n".into(), public: "ssh-ed25519 AAAAhost singularity-vm\n".into() };
        let u = user_data("ssh-ed25519 AAAA test", &host);
        // the host key the Mac pins, as cloud-init wants it (a block under ed25519_private), and nothing generated
        assert!(u.contains("ssh_keys:\n  ed25519_private: |\n    -----BEGIN OPENSSH PRIVATE KEY-----\n    b3BlbnNzaA\n    -----END OPENSSH PRIVATE KEY-----\n  ed25519_public: ssh-ed25519 AAAAhost singularity-vm\nusers:"), "{u}");
        assert!(u.contains("ssh_genkeytypes: []") && u.contains("touch /var/lib/singularity-ready-4"));
        assert_eq!(known_hosts("ssh-ed25519 AAAAhost singularity-vm\n"), "singularity-vm ssh-ed25519 AAAAhost\n");
        assert!(u.starts_with("#cloud-config\n") && u.contains("ssh-ed25519 AAAA test") && u.contains("VSOCK-CONNECT:2:5005") && u.contains("module-pipe-sink") && u.contains("soloist_release_arm64"));
        assert!(!u.contains("{{") && !u.contains("}}"), "format escapes resolved");
        // S4/S7: the guest's own download is capped and its tar output checked
        assert!(u.contains("--max-filesize") && u.contains("--max-time") && u.contains("[ ! -L soloist.new/soloist ]"), "the guest script caps what it downloads and unpacks");
    }
}
