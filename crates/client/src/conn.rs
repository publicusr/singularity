//! Connection to the Daemon: start it if needed, handshake, forward messages, reconnect forever.

// The Daemon is started with the Soloist key's variable (it is the one program that needs it); the Host's link opener
// starts without it (audit S10).

use singularity_config::host::Host;
use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg, PROTOCOL_VERSION};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

// Boxing the big variants would change every producer and consumer of these events.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum ConnEvent {
    Connecting,
    /// `demo`: the Daemon confirmed demo mode (D90), `account`: the saved login's username it gave a demo Client
    Connected { daemon_version: String, engine: proto::EngineKind, soloist_in_vm: bool, demo: bool, account: String },
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

async fn run(cmd_rx: mpsc::Receiver<Command>, ev_tx: mpsc::Sender<ConnEvent>) {
    run_at(proto::socket_path(), cmd_rx, ev_tx).await
}

/// How long a Client leaves the restart of a Daemon that systemd or launchd ran to them (#145), counted from the old
/// one's exit. Both start theirs again 2 s after it quits as replaced or crashed (`RestartSec=2`, `ThrottleInterval`
/// 2); a Daemon a Client started itself in that time took the socket, the service manager's copy found it taken and
/// quit cleanly, and the unit stayed inactive. A Daemon no service ran is started again at once, as before.
const SERVICE_GRACE: Duration = Duration::from_secs(5);

/// The connection to the Daemon on `path`.
async fn run_at(path: std::path::PathBuf, cmd_rx: mpsc::Receiver<Command>, ev_tx: mpsc::Sender<ConnEvent>) {
    let launch = Launch { start: Box::new(start_daemon), outdated: Box::new(|started| daemon_binary().is_some_and(|b| built_after(&b, started))), grace: SERVICE_GRACE };
    run_with(path, cmd_rx, ev_tx, launch).await
}

/// How the connection starts a Daemon and tells an outdated one (tests stand in for both).
struct Launch {
    /// start a Daemon detached
    start: Box<dyn Fn() -> Result<(), String> + Send>,
    /// is a Daemon started at these Unix ms older than the `singularityd` on disk?
    outdated: Box<dyn Fn(u64) -> bool + Send>,
    /// how long after a service-run Daemon quit to wait for the service manager to start one ([`SERVICE_GRACE`])
    grace: Duration,
}

/// When the Client may start a Daemon itself: once per lost connection, and after losing one that a service manager
/// ran only once the service manager had its chance ([`SERVICE_GRACE`]).
#[derive(Debug, Default)]
struct Starts {
    started: bool,
    /// a service-run Daemon quit: until then, one that appears on the socket is the service manager's
    wait_until: Option<tokio::time::Instant>,
}

impl Starts {
    /// Should a Daemon be started now (nothing answers on the socket)? True once, until [`Self::again`].
    fn due(&mut self, now: tokio::time::Instant) -> bool {
        if self.started || self.wait_until.is_some_and(|t| now < t) {
            return false;
        }
        self.started = true;
        self.wait_until = None;
        true
    }

    /// The connection was lost: the next failed connect may start one.
    fn again(&mut self) {
        self.started = false;
    }

    /// A Daemon a service manager ran quit at `now`: leave its restart to the service manager for `grace`.
    fn leave_to_service(&mut self, now: tokio::time::Instant, grace: Duration) {
        self.started = false;
        self.wait_until = Some(now + grace);
    }

    /// Connected: whoever started it, nothing is waited for any more.
    fn connected(&mut self) {
        self.wait_until = None;
    }
}

/// [`run_at`] with its way of starting a Daemon.
async fn run_with(path: std::path::PathBuf, mut cmd_rx: mpsc::Receiver<Command>, ev_tx: mpsc::Sender<ConnEvent>, launch: Launch) {
    let mut starts = Starts::default();
    let mut replaced = false;
    // whether the Daemon of the last connection runs under systemd or launchd (it says so in its welcome)
    let mut supervised;
    loop {
        let _ = ev_tx.send(ConnEvent::Connecting).await;
        let stream = match UnixStream::connect(&path).await {
            Ok(s) => s,
            Err(_) => {
                if starts.due(tokio::time::Instant::now())
                    && let Err(message) = (launch.start)()
                {
                    // say it once, where the UI shows Daemon errors (retried on the next disconnect)
                    let _ = ev_tx.send(ConnEvent::Msg(DaemonMsg::Event(singularity_proto::Event::Error { message }))).await;
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
                continue;
            }
        };
        // only a Daemon of our own user's (another user's process could be listening on a path we expect)
        let me = singularity_config::host::uid();
        if !stream.peer_cred().is_ok_and(|c| c.uid() == me) {
            let _ = ev_tx.send(ConnEvent::Incompatible(format!("{} belongs to another user: not connecting", path.display()))).await;
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        }
        // who's on the other end, in case it has to be stopped
        let peer = stream.peer_cred().ok().and_then(|c| c.pid());
        let (mut rd, mut wr) = stream.into_split();
        let hello = ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: format!("singularity {}", env!("CARGO_PKG_VERSION")), demo: crate::demo::on() };
        if proto::write_msg(&mut wr, &hello).await.is_err() {
            continue;
        }
        match proto::read_msg::<_, DaemonMsg>(&mut rd).await {
            Ok(Some(DaemonMsg::Welcome { daemon_version, started_unix_ms, engine, soloist_in_vm, demo, account, service, .. })) => {
                supervised = service;
                // after a rebuild the old Daemon keeps running; ask it to make way for the new binary (once)
                if !replaced && (launch.outdated)(started_unix_ms) {
                    replaced = true;
                    let _ = ev_tx.send(ConnEvent::Updating).await;
                    let _ = proto::write_msg(&mut wr, &ClientMsg::Command(Command::Shutdown)).await;
                    drop((rd, wr));
                    // wait for the old one to let go of the socket; a Daemon too old to know `Shutdown` is stopped
                    // (only the process on the other end of the socket, and only a singularityd)
                    let mut signalled = false;
                    if !let_go(&path, 50).await
                        && let Some(pid) = peer
                    {
                        stop_daemon(pid).await;
                        let_go(&path, 30).await;
                        signalled = true;
                    }
                    // then the new one: systemd's or launchd's if they ran the old one (it quit as replaced, so they
                    // start it again), else started here. One stopped by a signal quit cleanly and isn't restarted
                    // by them: started here at once.
                    lost(&mut starts, peer, supervised && !signalled, launch.grace).await;
                    continue;
                }
                starts.connected();
                let _ = ev_tx.send(ConnEvent::Connected { daemon_version, engine, soloist_in_vm, demo, account }).await;
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
        if pump(rd, wr, &mut cmd_rx, &ev_tx).await == Pumped::Quit {
            return;
        }
        let _ = ev_tx.send(ConnEvent::Disconnected).await;
        // every Client follows the same rule (another one may have replaced it, or it crashed): a service-run Daemon
        // is left to the service manager
        lost(&mut starts, peer, supervised, launch.grace).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The Daemon `peer` went away: if a service manager ran it, wait for it to exit (the service manager's restart
/// counts from then) and leave the next Daemon to the service manager for `grace`; else the next failed connect starts
/// one at once.
async fn lost(starts: &mut Starts, peer: Option<i32>, supervised: bool, grace: Duration) {
    if !supervised {
        starts.again();
        return;
    }
    if let Some(pid) = peer {
        for _ in 0..50 {
            if !is_daemon(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    starts.leave_to_service(tokio::time::Instant::now(), grace);
}

/// How a connection's pump ended.
#[derive(Debug, PartialEq)]
enum Pumped {
    /// the connection broke: connect again
    Broke,
    /// the App is gone: stop
    Quit,
}

/// Messages from the Daemon to the App, and the App's commands to the Daemon, until either side breaks.
///
/// Frames are read on a task of their own (D82): reading one takes several awaits, and inside the `select!` a
/// command arriving between them dropped the half-read frame. Both branches here only take from a channel, which
/// loses nothing when the other one wins. The channel is bounded: a slow App makes the reader wait, as before.
async fn pump<R, W>(mut rd: R, mut wr: W, cmd_rx: &mut mpsc::Receiver<Command>, ev_tx: &mpsc::Sender<ConnEvent>) -> Pumped
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin,
{
    let (frame_tx, mut frames) = mpsc::channel::<DaemonMsg>(64);
    let reader = tokio::spawn(async move {
        while let Ok(Some(m)) = proto::read_msg::<_, DaemonMsg>(&mut rd).await {
            if frame_tx.send(m).await.is_err() {
                return;
            }
        }
    });
    // (the reader goes with the connection, whichever way the pump ends)
    let _reader = AbortOnDrop(reader);
    loop {
        tokio::select! {
            msg = frames.recv() => match msg {
                Some(m) => {
                    if ev_tx.send(ConnEvent::Msg(m)).await.is_err() {
                        return Pumped::Quit;
                    }
                }
                None => return Pumped::Broke,
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(c) => {
                    // (frames read but not yet handed on go with the connection; the reconnect sends fresh state)
                    if proto::write_msg(&mut wr, &ClientMsg::Command(c)).await.is_err() {
                        return Pumped::Broke;
                    }
                }
                None => return Pumped::Quit,
            },
        }
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
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
    exe_name(pid).is_some_and(|n| n.starts_with("singularityd"))
}

/// The file name of the binary process `pid` runs, as the Host tells it.
fn exe_name(pid: i32) -> Option<String> {
    Host::this().exe_name(u32::try_from(pid).ok()?)
}

/// SIGTERM the Daemon `pid`, SIGKILL if it's still there 2 s later. Anything that isn't a singularityd is left alone.
async fn stop_daemon(pid: i32) {
    if pid <= 1 || !is_daemon(pid) {
        return;
    }
    let host = Host::this();
    host.stop(pid as u32, false);
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !is_daemon(pid) {
            return;
        }
    }
    host.stop(pid as u32, true);
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

/// Start `singularityd` detached (next to this binary). Its log goes to the state dir.
///
/// The two are installed together (`cargo install` puts both in `~/.cargo/bin`, a build puts both in `target/…`), so
/// the Daemon is never looked up on `PATH`: a planted `singularityd` there would run as the user (audit S3).
fn start_daemon() -> Result<(), String> {
    let Some(exe) = daemon_binary() else {
        return Err(format!("can't find `singularityd` next to {}: install or build both together", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "singularity".into())));
    };
    let log_dir = singularity_config::app_folders::AppFolders::from_env().map_err(|e| e.to_string())?.state;
    let _ = std::fs::create_dir_all(&log_dir);
    // owner-only: the log names the account and what plays. Appended to: the new Daemon moves the last one's lines
    // aside once it holds the socket (Q170, `daemon_log`), and one that finds a Daemon running adds only its line
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = log_dir.join("daemon.log");
    let log = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(&path).ok();
    // (one made by an older build is 0644)
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let mut cmd = std::process::Command::new(&exe);
    // the Daemon keeps SOLOIST_API_KEY: it reads the key from there first (soloist.rs)
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
    match log {
        Some(f) => cmd.stderr(f),
        None => cmd.stderr(std::process::Stdio::null()),
    };
    // own process group: closing the terminal must not take the Daemon (and the music) with it
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    // the service manager's marks of the terminal this Client runs in aren't the Daemon's (#145: the Daemon tells
    // Clients whether systemd or launchd runs it)
    cmd.env_remove("INVOCATION_ID").env_remove("XPC_SERVICE_NAME");
    // reaped when it quits (replaced, crashed), or it stays behind as a zombie for as long as this Client runs (#143)
    let child = cmd.spawn().map_err(|e| format!("couldn't start {}: {e}", exe.display()))?;
    singularity_config::host::reap(child);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Q94 end to end: a real Daemon whose Soloist runs in the Soloist VM says so in its welcome, and the App takes it.
    #[tokio::test]
    async fn the_daemons_word_on_the_soloist_vm_reaches_the_app() {
        /// An Engine that says its Soloist runs in the VM, and does nothing else.
        struct SoloistInVm;
        impl singularityd::engine::Engine for SoloistInVm {
            fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<singularityd::engine::EngineEvent>) {
                let ((cmd_tx, cmd_rx), (ev_tx, ev_rx)) = (mpsc::channel(8), mpsc::channel(8));
                // kept for the test's length: a closed Engine would be reported, not the point here
                std::mem::forget((cmd_rx, ev_tx));
                (cmd_tx, ev_rx)
            }
            fn soloist_in_vm(&self) -> bool {
                true
            }
        }
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let path = dir.path().join("s").join("daemon.sock");
        let listener = singularityd::bind(&path).unwrap();
        tokio::spawn(singularityd::serve(listener, Box::new(SoloistInVm), None, None));
        let (_cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(64);
        tokio::spawn(run_at(path, cmd_rx, ev_tx));
        let mut app = crate::app::App::new(None);
        assert!(!app.soloist_in_vm);
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(10), ev_rx.recv()).await.expect("connected").expect("the connection runs");
            let connected = matches!(ev, ConnEvent::Connected { .. });
            app.receive(ev);
            if connected {
                break;
            }
        }
        assert!(app.soloist_in_vm, "the setup wizard words the Soloist step for the VM");
    }

    /// The pump (D82): a frame used to be read inside the `select!`, so a command sent while a frame was half
    /// in threw the half away, and the stream fell out of step ("frame too large", garbage, then a reconnect).
    #[tokio::test]
    async fn a_command_sent_while_a_frame_is_half_in_does_not_cut_the_frame() {
        use tokio::io::AsyncWriteExt;
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let (rd, wr) = tokio::io::split(ours);
        let (mut daemon_rd, mut daemon_wr) = tokio::io::split(theirs);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let (ev_tx, mut ev_rx) = mpsc::channel(4);
        let pumping = tokio::spawn(async move { pump(rd, wr, &mut cmd_rx, &ev_tx).await });
        let message = "a notice long enough to be sent in two halves".to_string();
        let body = serde_json::to_vec(&DaemonMsg::Event(proto::Event::Notice { message: message.clone() })).unwrap();
        let (first, rest) = body.split_at(body.len() / 2);
        daemon_wr.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
        daemon_wr.write_all(first).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        // the user does something now
        cmd_tx.send(Command::Next).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(2), proto::read_msg::<_, ClientMsg>(&mut daemon_rd)).await;
        assert!(matches!(got, Ok(Ok(Some(ClientMsg::Command(Command::Next))))), "{got:?}");
        daemon_wr.write_all(rest).await.unwrap();
        let ev = tokio::time::timeout(Duration::from_secs(2), ev_rx.recv()).await;
        match ev {
            Ok(Some(ConnEvent::Msg(DaemonMsg::Event(proto::Event::Notice { message: m })))) => assert_eq!(m, message),
            other => panic!("the frame didn't arrive whole: {other:?}; the pump ended {:?}", pumping.is_finished()),
        }
        drop(cmd_tx);
        assert_eq!(pumping.await.unwrap(), Pumped::Quit);
    }

    #[tokio::test]
    async fn only_a_singularityd_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        // a stand-in Daemon that ignores everything but signals: this test binary waiting, under the Daemon's name
        // (not a copy of /bin/sleep: macOS kills copies of its own signed binaries)
        let fake = dir.path().join("singularityd");
        std::fs::copy(std::env::current_exe().unwrap(), &fake).unwrap();
        let mut d = std::process::Command::new(&fake)
            .args(["--ignored", "--exact", "conn::tests::stand_in_waits", "--quiet"])
            .env("SINGULARITY_STAND_IN", "1")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let mut other = std::process::Command::new("/bin/sleep").arg("60").spawn().unwrap();
        // Some sandboxes can't show a child's binary: a PID namespace over the host's /proc, or non-dumpable
        // processes (whose /proc/<pid>/exe is unreadable). There nothing can tell a Daemon apart, so say so and stop.
        if exe_name(d.id() as i32).as_deref() != Some("singularityd") {
            eprintln!("skipped: this sandbox doesn't show which binary a child process runs");
            for c in [&mut d, &mut other] {
                c.kill().unwrap();
                c.wait().unwrap();
            }
            return;
        }
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

    /// The stand-in above: waits for a signal when asked to (`SINGULARITY_STAND_IN=1`), else passes at once.
    #[test]
    #[ignore]
    fn stand_in_waits() {
        if std::env::var_os("SINGULARITY_STAND_IN").is_some() {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    /// How a stand-in Daemon goes away.
    enum Quits {
        /// on `Shutdown`, as a replaced Daemon does
        OnShutdown,
        /// when this fires, as one replaced by another Client (or one that crashed) does for this Client
        When(tokio::sync::oneshot::Receiver<()>),
    }

    /// A stand-in Daemon on `path` that started at `started_unix_ms` (0: older than the binary on disk) and says whether
    /// a service manager runs it. It welcomes one Client; when it `quits`, it lets go of the socket and says when on
    /// `gone`.
    async fn stand_in_daemon(path: std::path::PathBuf, started_unix_ms: u64, service: bool, quits: Quits, gone: Option<tokio::sync::oneshot::Sender<tokio::time::Instant>>) {
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let (s, _) = listener.accept().await.unwrap();
        let (mut rd, mut wr) = s.into_split();
        let _hello = proto::read_msg::<_, ClientMsg>(&mut rd).await;
        let welcome = DaemonMsg::Welcome { protocol: PROTOCOL_VERSION, daemon_version: "stand-in".into(), started_unix_ms, engine: proto::EngineKind::default(), soloist_in_vm: false, demo: false, account: String::new(), service };
        proto::write_msg(&mut wr, &welcome).await.unwrap();
        match quits {
            Quits::OnShutdown => {
                while let Ok(Some(m)) = proto::read_msg::<_, ClientMsg>(&mut rd).await {
                    if matches!(m, ClientMsg::Command(Command::Shutdown)) {
                        break;
                    }
                }
            }
            Quits::When(rx) => {
                let _ = rx.await;
            }
        }
        drop((listener, rd, wr));
        let _ = std::fs::remove_file(&path);
        if let Some(g) = gone {
            let _ = g.send(tokio::time::Instant::now());
        }
    }

    /// A [`Launch`] whose Daemon started at 0 is outdated, and whose starts are counted (and timed) instead of made.
    fn counted_launch(grace: Duration) -> (Launch, std::sync::Arc<std::sync::Mutex<Vec<tokio::time::Instant>>>) {
        let starts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = starts.clone();
        let launch = Launch {
            start: Box::new(move || {
                s.lock().unwrap().push(tokio::time::Instant::now());
                Ok(())
            }),
            outdated: Box::new(|started| started == 0),
            grace,
        };
        (launch, starts)
    }

    /// A Client on `path` with `launch`, its events drained (connections counted).
    fn client(path: std::path::PathBuf, launch: Launch) -> (mpsc::Sender<Command>, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(256);
        tokio::spawn(run_with(path, cmd_rx, ev_tx, launch));
        let connected = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = connected.clone();
        tokio::spawn(async move {
            while let Some(ev) = ev_rx.recv().await {
                if matches!(ev, ConnEvent::Connected { .. }) {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
        (cmd_tx, connected)
    }

    async fn until(what: &str, f: impl Fn() -> bool) {
        for _ in 0..100 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {what}");
    }

    fn scratch() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let path = dir.path().join("daemon.sock");
        (dir, path)
    }

    /// #145: after replacing a Daemon that systemd or launchd ran, the Client leaves its restart to them. One that
    /// appears on the socket within the wait (theirs, 2 s after the old one quit) is connected to, and the Client
    /// starts none of its own: before, it started one within 0.3 s, and the service manager's copy found the socket
    /// taken and quit.
    #[tokio::test]
    async fn after_a_replace_the_service_managers_daemon_is_waited_for() {
        let (_dir, path) = scratch();
        let (gone_tx, gone_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(stand_in_daemon(path.clone(), 0, true, Quits::OnShutdown, Some(gone_tx)));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (launch, starts) = counted_launch(SERVICE_GRACE);
        let (_cmd, connected) = client(path.clone(), launch);
        tokio::time::timeout(Duration::from_secs(10), gone_rx).await.expect("the outdated Daemon is asked to make way").unwrap();
        // the service manager's restart
        tokio::time::sleep(Duration::from_secs(2)).await;
        tokio::spawn(stand_in_daemon(path, 1, true, Quits::OnShutdown, None));
        until("a connection to the new Daemon", || connected.load(std::sync::atomic::Ordering::SeqCst) == 1).await;
        assert!(starts.lock().unwrap().is_empty(), "the Client started a Daemon of its own while the service manager's was coming");
    }

    /// #145: every Client follows the rule, not only the one that replaced the Daemon: another Client connected to the
    /// same service-run Daemon sees it go away and waits for the service manager too.
    #[tokio::test]
    async fn a_second_client_also_waits_for_the_service_manager() {
        let (_dir, path) = scratch();
        let (quit_tx, quit_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(stand_in_daemon(path.clone(), 1, true, Quits::When(quit_rx), None));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (launch, starts) = counted_launch(SERVICE_GRACE);
        let (_cmd, connected) = client(path.clone(), launch);
        until("the first connection", || connected.load(std::sync::atomic::Ordering::SeqCst) == 1).await;
        // replaced by another Client (or crashed); the service manager brings the next one 2 s later
        quit_tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        tokio::spawn(stand_in_daemon(path, 1, true, Quits::OnShutdown, None));
        until("a connection to the new Daemon", || connected.load(std::sync::atomic::Ordering::SeqCst) == 2).await;
        assert!(starts.lock().unwrap().is_empty(), "the second Client started a Daemon of its own while the service manager's was coming");
    }

    /// #145: a Daemon no service manager runs (a Client started it) is started again at once after a replace, with no
    /// wait.
    #[tokio::test]
    async fn a_daemon_no_service_runs_is_started_again_at_once() {
        let (_dir, path) = scratch();
        let (gone_tx, gone_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(stand_in_daemon(path.clone(), 0, false, Quits::OnShutdown, Some(gone_tx)));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (launch, starts) = counted_launch(Duration::from_secs(30));
        let (_cmd, _connected) = client(path, launch);
        let gone = tokio::time::timeout(Duration::from_secs(10), gone_rx).await.expect("the outdated Daemon is asked to make way").unwrap();
        until("a Daemon started by the Client", || !starts.lock().unwrap().is_empty()).await;
        let started = starts.lock().unwrap()[0];
        assert!(started - gone < Duration::from_secs(2), "started {:?} after the old one went", started - gone);
    }

    /// #145: when the service manager doesn't bring one up (stopped by hand), the Client starts the new Daemon itself,
    /// once, when the wait is over.
    #[tokio::test]
    async fn after_a_replace_with_no_restart_the_client_starts_the_daemon_after_the_wait() {
        let (_dir, path) = scratch();
        let (gone_tx, gone_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(stand_in_daemon(path.clone(), 0, true, Quits::OnShutdown, Some(gone_tx)));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let grace = Duration::from_millis(1500);
        let (launch, starts) = counted_launch(grace);
        let (_cmd, _connected) = client(path, launch);
        let gone = tokio::time::timeout(Duration::from_secs(10), gone_rx).await.expect("the outdated Daemon is asked to make way").unwrap();
        tokio::time::sleep(grace + Duration::from_secs(1)).await;
        let starts = starts.lock().unwrap().clone();
        assert_eq!(starts.len(), 1, "one Daemon started, once the wait is over");
        assert!(starts[0] >= gone + grace, "started {:?} after the old one went, before the {grace:?} wait was over", starts[0] - gone);
    }

    #[test]
    fn a_daemon_is_started_once_per_lost_connection_and_not_during_the_wait() {
        let t = tokio::time::Instant::now();
        let mut s = Starts::default();
        assert!(s.due(t) && !s.due(t), "once");
        s.again();
        assert!(s.due(t));
        s.leave_to_service(t, Duration::from_secs(5));
        assert!(!s.due(t + Duration::from_secs(1)) && !s.due(t + Duration::from_millis(4999)), "the service manager's turn");
        assert!(s.due(t + Duration::from_secs(5)) && !s.due(t + Duration::from_secs(6)));
        // a Daemon answered during the wait: a later lost connection starts one at once again
        s.leave_to_service(t, Duration::from_secs(5));
        s.connected();
        s.again();
        assert!(s.due(t + Duration::from_secs(1)));
    }
}
