//! Connection to the Daemon: start it if needed, handshake, forward messages, reconnect forever.

use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg, PROTOCOL_VERSION};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum ConnEvent {
    Connecting,
    Connected { daemon_version: String },
    /// the running Daemon is older than the `singularityd` on disk; it's being replaced
    Updating,
    Incompatible(String),
    Disconnected,
    Msg(DaemonMsg),
}

/// Spawn the connection task. Commands sent while disconnected are dropped (the UI shows why).
pub fn spawn() -> (mpsc::Sender<Command>, mpsc::Receiver<ConnEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (ev_tx, ev_rx) = mpsc::channel(256);
    tokio::spawn(run(cmd_rx, ev_tx));
    (cmd_tx, ev_rx)
}

async fn run(mut cmd_rx: mpsc::Receiver<Command>, ev_tx: mpsc::Sender<ConnEvent>) {
    let path = proto::socket_path();
    let mut started_daemon = false;
    let mut replaced = false;
    loop {
        let _ = ev_tx.send(ConnEvent::Connecting).await;
        let stream = match UnixStream::connect(&path).await {
            Ok(s) => s,
            Err(_) => {
                if !started_daemon {
                    started_daemon = true;
                    start_daemon();
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
                continue;
            }
        };
        // who's on the other end, in case it has to be stopped
        let peer = stream.peer_cred().ok().and_then(|c| c.pid());
        let (mut rd, mut wr) = stream.into_split();
        let hello = ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: format!("singularity {}", env!("CARGO_PKG_VERSION")) };
        if proto::write_msg(&mut wr, &hello).await.is_err() {
            continue;
        }
        match proto::read_msg::<_, DaemonMsg>(&mut rd).await {
            Ok(Some(DaemonMsg::Welcome { daemon_version, started_unix_ms, .. })) => {
                // after a rebuild the old Daemon keeps running; ask it to make way for the new binary (once)
                if !replaced && daemon_binary().is_some_and(|b| built_after(&b, started_unix_ms)) {
                    replaced = true;
                    let _ = ev_tx.send(ConnEvent::Updating).await;
                    let _ = proto::write_msg(&mut wr, &ClientMsg::Command(Command::Shutdown)).await;
                    drop((rd, wr));
                    // wait for the old one to let go of the socket, then start the new one; a Daemon too old to know
                    // `Shutdown` is stopped (only the process on the other end of the socket, and only a singularityd)
                    if !let_go(&path, 50).await
                        && let Some(pid) = peer
                    {
                        stop_daemon(pid).await;
                        let_go(&path, 30).await;
                    }
                    started_daemon = false;
                    continue;
                }
                let _ = ev_tx.send(ConnEvent::Connected { daemon_version }).await;
            }
            Ok(Some(DaemonMsg::Incompatible { message, .. })) => {
                let _ = ev_tx.send(ConnEvent::Incompatible(message)).await;
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
            _ => {
                tokio::time::sleep(Duration::from_millis(300)).await;
                continue;
            }
        }
        // connected: pump both directions until either side breaks
        loop {
            tokio::select! {
                msg = proto::read_msg::<_, DaemonMsg>(&mut rd) => match msg {
                    Ok(Some(m)) => {
                        if ev_tx.send(ConnEvent::Msg(m)).await.is_err() {
                            return;
                        }
                    }
                    _ => break,
                },
                cmd = cmd_rx.recv() => match cmd {
                    Some(c) => {
                        if proto::write_msg(&mut wr, &ClientMsg::Command(c)).await.is_err() {
                            break;
                        }
                    }
                    None => return,
                },
            }
        }
        let _ = ev_tx.send(ConnEvent::Disconnected).await;
        started_daemon = false;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Wait up to `tries` × 100 ms for nothing to answer on `path`; true once nothing does.
async fn let_go(path: &std::path::Path, tries: u32) -> bool {
    for _ in 0..tries {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if UnixStream::connect(path).await.is_err() {
            return true;
        }
    }
    false
}

/// Is process `pid` a `singularityd` (its binary may have been replaced on disk: "singularityd (deleted)")?
fn is_daemon(pid: i32) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().starts_with("singularityd"))).unwrap_or(false)
}

/// SIGTERM the Daemon `pid`, SIGKILL if it's still there 2 s later. Anything that isn't a singularityd is left alone.
async fn stop_daemon(pid: i32) {
    if pid <= 1 || !is_daemon(pid) {
        return;
    }
    // SAFETY: kill only sends a signal
    unsafe { libc::kill(pid, libc::SIGTERM) };
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !is_daemon(pid) {
            return;
        }
    }
    unsafe { libc::kill(pid, libc::SIGKILL) };
}

/// The `singularityd` next to this binary, if there is one.
fn daemon_binary() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("singularityd"))).filter(|p| p.exists())
}

/// Was `binary` written after a Daemon started at `started_unix_ms`? (2 s of slack for clock jitter)
fn built_after(binary: &std::path::Path, started_unix_ms: u64) -> bool {
    let Ok(m) = std::fs::metadata(binary).and_then(|m| m.modified()) else { return false };
    let mtime_ms = m.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    mtime_ms > started_unix_ms + 2000
}

/// Start `singularityd` detached (next to this binary, else from PATH). Its log goes to the state dir.
fn start_daemon() {
    let exe = daemon_binary().unwrap_or_else(|| "singularityd".into());
    let log_dir = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("singularity");
    let _ = std::fs::create_dir_all(&log_dir);
    let log = std::fs::File::create(log_dir.join("daemon.log")).ok();
    let mut cmd = std::process::Command::new(exe);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
    match log {
        Some(f) => cmd.stderr(f),
        None => cmd.stderr(std::process::Stdio::null()),
    };
    // own process group: closing the terminal must not take the Daemon (and the music) with it
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let _ = cmd.spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_a_singularityd_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        // a stand-in Daemon that ignores everything but signals: `sleep`, under the Daemon's name
        let fake = dir.path().join("singularityd");
        std::fs::copy("/bin/sleep", &fake).unwrap();
        let mut d = std::process::Command::new(&fake).arg("60").spawn().unwrap();
        let mut other = std::process::Command::new("/bin/sleep").arg("60").spawn().unwrap();
        // its binary replaced on disk, as after a rebuild
        std::fs::remove_file(&fake).unwrap();
        assert!(is_daemon(d.id() as i32) && !is_daemon(other.id() as i32));
        stop_daemon(other.id() as i32).await;
        assert!(other.try_wait().unwrap().is_none(), "not a singularityd: left alone");
        stop_daemon(d.id() as i32).await;
        assert!(d.wait().unwrap().code().is_none(), "stopped by a signal");
        other.kill().unwrap();
        other.wait().unwrap();
    }
}
