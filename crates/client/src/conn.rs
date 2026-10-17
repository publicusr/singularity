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
                    // wait for the old one to let go of the socket, then start the new one
                    for _ in 0..50 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        if UnixStream::connect(&path).await.is_err() {
                            break;
                        }
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
