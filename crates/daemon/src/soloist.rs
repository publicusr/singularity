//! SoloistEngine (ADR-0001): supervises Spotify Soloist and drives it over its local WebSocket.
//!
//! - binary: `$XDG_DATA_HOME/singularity/soloist/soloist`, downloaded from Spotify's CDN when missing (never
//!   redistributed) and kept current by `update` (#6): a new build is restarted into at a quiet moment, and playback
//!   picks up where it was
//! - API key: `SOLOIST_API_KEY`, else the keyring (`service singularity account soloist`), else the owner-only
//!   fallback file; the setup wizard stores it (`Command::SetSoloistKey`). Soloist only takes it as `--api-key`, so it
//!   shows in `ps` for local users: there's no other way in
//! - output: Soloist's verbose log is piped through the Daemon, which reads the stream format and build lifetime
//!   from it and writes it on to `soloist.log` (the previous one is kept as `soloist.log.1`, never truncated)
//! - Soloist's own state (pairing, cache) lives under `$XDG_DATA_HOME/singularity/soloist-state`
//! - On macOS (D55) the same supervision runs Soloist inside the Soloist VM (`soloist_vm.rs`): the build is kept current
//!   in the guest, Soloist is started over SSH, its WebSocket and audio come through forwarded ports, and pairing is
//!   announced on the LAN while it waits for it

use crate::engine::{Engine, EngineEvent, Resume};
use crate::update::{self, Builds, Checked};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use singularity_proto::{Command, EngineStatus, Item, PlayStatus, PlaybackState, QueueEntry, RepeatMode};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command as Proc};
use tokio::sync::{Notify, mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

pub const DEVICE_NAME: &str = "Singularity";

pub struct SoloistEngine {
    pub data_dir: PathBuf,
    /// its name on Connect
    pub device_name: String,
    /// macOS: Soloist runs in the Soloist VM (D55)
    #[cfg(target_os = "macos")]
    pub vm: Option<Arc<crate::soloist_vm::VmHost>>,
}

impl SoloistEngine {
    pub fn from_env() -> SoloistEngine {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir);
        SoloistEngine {
            data_dir: base.join("singularity"),
            device_name: DEVICE_NAME.into(),
            #[cfg(target_os = "macos")]
            vm: None,
        }
    }

    /// Soloist in the Soloist VM, as `device` on Connect (empty: "Singularity on <this computer> (lossless)", apart from a
    /// Linux machine's "Singularity" and this Mac's librespot, which can linger in the device list after a switch, as
    /// devices are found by name); its audio also goes to `feed`.
    #[cfg(target_os = "macos")]
    pub fn in_vm(device: &str, feed: Arc<crate::tap::Feed>) -> SoloistEngine {
        let mut e = SoloistEngine::from_env();
        e.device_name = if device.trim().is_empty() { format!("Singularity on {} (lossless)", crate::app_engine::this_computer()) } else { device.trim().into() };
        e.vm = Some(Arc::new(crate::soloist_vm::VmHost::new(&e.data_dir, feed)));
        e
    }

    /// The volume to start Soloist at: the last one it played at (it would start at its own 40 % otherwise).
    fn saved_volume(&self) -> Option<u8> {
        std::fs::read_to_string(self.state_dir().join("volume")).ok()?.trim().parse().ok().filter(|v| *v <= 100)
    }

    fn save_volume(&self, v: u8) {
        let _ = std::fs::write(self.state_dir().join("volume"), v.to_string());
    }

    #[cfg(target_os = "macos")]
    fn vm(&self) -> Option<&Arc<crate::soloist_vm::VmHost>> {
        self.vm.as_ref()
    }

    #[cfg(not(target_os = "macos"))]
    fn vm(&self) -> Option<&std::convert::Infallible> {
        None
    }

    /// Is a newer build waiting to be restarted into?
    fn update_waiting(&self) -> bool {
        #[cfg(target_os = "macos")]
        if let Some(v) = self.vm() {
            return v.staged.load(std::sync::atomic::Ordering::Relaxed);
        }
        self.builds().ready().is_some()
    }
    fn builds(&self) -> Builds {
        Builds { root: self.data_dir.clone() }
    }
    fn binary(&self) -> PathBuf {
        self.builds().binary()
    }
    fn state_dir(&self) -> PathBuf {
        self.data_dir.join("soloist-state")
    }
}

/// Soloist's API key: environment first, then the keyring (or its fallback file).
pub async fn api_key() -> Option<String> {
    if let Ok(k) = std::env::var("SOLOIST_API_KEY")
        && !k.trim().is_empty()
    {
        return Some(k.trim().into());
    }
    tokio::task::spawn_blocking(|| crate::secrets::get("soloist")).await.ok().flatten()
}

/// Store a key from the setup wizard, and say where it went.
async fn store_key(key: String, ev_tx: &mpsc::Sender<EngineEvent>) {
    let key = key.trim().to_string();
    match tokio::task::spawn_blocking(move || crate::secrets::set("soloist", &key)).await {
        Ok(Ok(crate::secrets::Stored::Keyring)) => {
            let _ = ev_tx.send(EngineEvent::Notice(format!("Soloist key saved in your {}", crate::secrets::KEYRING))).await;
        }
        Ok(Ok(crate::secrets::Stored::File(p))) => {
            let _ = ev_tx.send(EngineEvent::Notice(format!("No {} available: the key is in {} (owner-only)", crate::secrets::KEYRING, p.display()))).await;
        }
        _ => {
            let _ = ev_tx.send(EngineEvent::Error("Couldn't store the Soloist key".into())).await;
        }
    }
}

/// Spotify refused the key: retrying won't help until there's a new one.
#[derive(Debug)]
struct BadKey;

impl std::fmt::Display for BadKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Spotify rejected the Soloist API key")
    }
}

impl std::error::Error for BadKey {}

/// What a line of Soloist's verbose log tells us.
#[derive(Debug, Clone, PartialEq)]
pub enum LogNews {
    Lifetime(u32),
    Stream(singularity_proto::StreamInfo),
    /// another Soloist already runs on this data directory
    Locked,
    /// Spotify refused the API key
    BadKey,
    /// a download speed sample, kB/s
    Bandwidth(u32),
    /// playback ran dry
    Stall,
    /// its ZeroConf endpoint (pairing from a phone) listens on this port
    ZeroConf(u16),
}

/// Reads Soloist's verbose log line by line. Per track it logs `FileStreamer file average bitrate: N`, then a block
/// with `codec:`, `bitrate: N bits/s`, `sample rate: N Hz` and `sample format:` (verified 2026-10-17). While it
/// streams: `FileStreamer bandwidth sample: N kBps`, and `audioThreadMain: underrun` when playback runs dry
/// (verified 2026-10-18).
#[derive(Default)]
pub struct LogParser {
    average: Option<u32>,
    block: Option<singularity_proto::StreamInfo>,
}

impl LogParser {
    pub fn line(&mut self, raw: &str) -> Option<LogNews> {
        let line = strip_ansi(raw);
        let after = |key: &str| line.split(key).nth(1).map(str::trim);
        let num = |s: &str| s.split_whitespace().next().and_then(|n| n.parse::<f64>().ok()).map(|n| n as u32);
        if line.contains("another session is running for data directory") {
            return Some(LogNews::Locked);
        }
        if line.contains("make sure --api-key is valid") {
            return Some(LogNews::BadKey);
        }
        if let Some(rest) = after("Started ZeroConf service on port") {
            return rest.split_whitespace().next().and_then(|p| p.parse().ok()).map(LogNews::ZeroConf);
        }
        if let Some(rest) = after("FileStreamer bandwidth sample:") {
            return rest.split_whitespace().next().and_then(|n| n.parse::<f64>().ok()).map(|k| LogNews::Bandwidth(k.round() as u32));
        }
        // (the recovery line, "updateInternalDriverToPlaying: underrun", isn't a new stall)
        if line.contains("audioThreadMain: underrun") {
            return Some(LogNews::Stall);
        }
        if let Some(rest) = after("client expires in") {
            return num(rest).map(LogNews::Lifetime);
        }
        if let Some(rest) = after("file average bitrate:") {
            self.average = num(rest).filter(|n| *n > 0).or(self.average);
            return None;
        }
        if let Some(rest) = after("codec:") {
            self.block = Some(singularity_proto::StreamInfo { codec: rest.to_string(), average_bitrate: self.average.take(), ..Default::default() });
            return None;
        }
        let b = self.block.as_mut()?;
        if let Some(rest) = after("sample rate:") {
            b.sample_rate = num(rest).unwrap_or(0);
        } else if let Some(rest) = after("sample format:") {
            b.sample_format = rest.to_string();
            return self.block.take().map(LogNews::Stream);
        } else if let Some(rest) = after("bitrate:") {
            b.bitrate = num(rest).unwrap_or(0);
        }
        None
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut esc = false;
    for c in s.chars() {
        match c {
            '\x1b' => esc = true,
            c if esc => esc = !c.is_ascii_alphabetic(),
            '\0' => {}
            c => out.push(c),
        }
    }
    out
}

impl Engine for SoloistEngine {
    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (ev_tx, ev_rx) = mpsc::channel(256);
        tokio::spawn(supervise(*self, cmd_rx, ev_tx));
        (cmd_tx, ev_rx)
    }
}

async fn status(tx: &mpsc::Sender<EngineEvent>, s: EngineStatus) {
    let _ = tx.send(EngineEvent::Status(s)).await;
}

/// What the update side and the running Soloist tell each other.
struct Updates {
    /// a new build is staged: restart into it at the next quiet moment
    staged: Arc<Notify>,
    /// the running build's days left (the checks get closer near the end)
    days: watch::Sender<Option<u32>>,
    /// where playback was when we restarted for an update
    resume: Option<Resume>,
}

/// Keep Soloist running: start, connect, pump; on exit wait (with backoff) and start again.
async fn supervise(engine: SoloistEngine, mut cmd_rx: mpsc::Receiver<Command>, ev_tx: mpsc::Sender<EngineEvent>) {
    let mut backoff = Duration::from_secs(2);
    let (builds, url) = (engine.builds(), update::cdn_url());
    let mut upd = Updates { staged: Arc::new(Notify::new()), days: watch::channel(None).0, resume: None };
    let mut updater = None;
    loop {
        let Some(key) = api_key().await else {
            status(&ev_tx, EngineStatus::NotConfigured { reason: "no Soloist API key yet".into() }).await;
            if !wait_or_refuse(&mut cmd_rx, &ev_tx, Duration::from_secs(10), "No Soloist API key yet").await {
                return;
            }
            continue;
        };
        status(&ev_tx, EngineStatus::Starting).await;
        // macOS: the VM up, a current build in it (the guest keeps its own builds; D55)
        #[cfg(target_os = "macos")]
        if let Some(v) = engine.vm().cloned() {
            let tx = ev_tx.clone();
            let say = move |m: String| {
                tracing::info!("{m}");
                let _ = tx.try_send(EngineEvent::Notice(m));
            };
            if let Err(e) = v.ensure(&say).await {
                status(&ev_tx, EngineStatus::Failed { message: format!("the Soloist VM: {e}") }).await;
                v.down().await;
                if !wait_or_refuse(&mut cmd_rx, &ev_tx, backoff, "the Soloist VM isn't up").await {
                    v.down().await;
                    return;
                }
                backoff = (backoff * 2).min(Duration::from_secs(300));
                continue;
            }
            if updater.is_none() {
                updater = Some(tokio::spawn(keep_current_in_vm(v.clone(), upd.days.subscribe(), upd.staged.clone())));
            }
            let exit = run_once(&engine, &key, &mut cmd_rx, &ev_tx, &mut upd).await;
            if matches!(exit, Ok(Exit::Quit)) {
                if let Some(u) = updater {
                    u.abort();
                }
                v.down().await;
                return;
            }
            match exit {
                Ok(Exit::Update) | Ok(Exit::Expired) => {
                    backoff = Duration::from_secs(2);
                    continue;
                }
                Err(e) if e.downcast_ref::<BadKey>().is_some() => {
                    status(&ev_tx, EngineStatus::Failed { message: e.to_string() }).await;
                    if !wait_or_refuse(&mut cmd_rx, &ev_tx, Duration::from_secs(3600), "Spotify rejected the Soloist API key").await {
                        v.down().await;
                        return;
                    }
                    continue;
                }
                Err(e) => status(&ev_tx, EngineStatus::Failed { message: e.to_string() }).await,
                _ => {}
            }
            // the VM itself may be what went away: the next round boots it again
            if !v.alive().await {
                v.down().await;
            }
            if !wait_or_refuse(&mut cmd_rx, &ev_tx, backoff, "Soloist is restarting").await {
                v.down().await;
                return;
            }
            backoff = (backoff * 2).min(Duration::from_secs(60));
            continue;
        }
        // Soloist isn't running: the best moment to put a staged build in place
        match builds.install() {
            Ok(true) => tracing::info!("Soloist updated to {}", builds.installed().map(|b| b.version).unwrap_or_default()),
            Ok(false) => {}
            Err(e) => tracing::warn!("couldn't install the new Soloist build: {e}"),
        }
        if let Err(e) = update::ensure(&builds, &url).await {
            status(&ev_tx, EngineStatus::Failed { message: format!("couldn't download Soloist: {e}") }).await;
            if !wait_or_refuse(&mut cmd_rx, &ev_tx, backoff, "Soloist isn't available").await {
                return;
            }
            backoff = (backoff * 2).min(Duration::from_secs(300));
            continue;
        }
        // there's a build: from now on keep it current
        if updater.is_none() {
            updater = Some(tokio::spawn(keep_current(builds.clone(), url.clone(), upd.days.subscribe(), upd.staged.clone())));
        }
        match run_once(&engine, &key, &mut cmd_rx, &ev_tx, &mut upd).await {
            Ok(Exit::Quit) => {
                if let Some(u) = updater {
                    u.abort();
                }
                return;
            }
            // straight back up with the new build
            Ok(Exit::Update) => {
                backoff = Duration::from_secs(2);
                continue;
            }
            Ok(Exit::Expired) => {
                // (it shouldn't come to this: updates land well before)
                status(&ev_tx, EngineStatus::Failed { message: "this Soloist build has expired; updating".into() }).await;
                if builds.ready().is_none() {
                    let _ = std::fs::remove_dir_all(builds.current());
                }
                backoff = Duration::from_secs(2);
            }
            Ok(Exit::Died) => {}
            Err(e) if e.downcast_ref::<BadKey>().is_some() => {
                status(&ev_tx, EngineStatus::Failed { message: e.to_string() }).await;
                // a new key (SetSoloistKey) ends the wait at once
                if !wait_or_refuse(&mut cmd_rx, &ev_tx, Duration::from_secs(3600), "Spotify rejected the Soloist API key").await {
                    return;
                }
                continue;
            }
            Err(e) => status(&ev_tx, EngineStatus::Failed { message: e.to_string() }).await,
        }
        if !wait_or_refuse(&mut cmd_rx, &ev_tx, backoff, "Soloist is restarting").await {
            return;
        }
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

/// Sleep for `d`, answering commands with an explanation meanwhile. False if the Daemon is shutting down.
async fn wait_or_refuse(cmd_rx: &mut mpsc::Receiver<Command>, ev_tx: &mpsc::Sender<EngineEvent>, d: Duration, why: &str) -> bool {
    let deadline = tokio::time::sleep(d);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return true,
            c = cmd_rx.recv() => match c {
                // a new key: try it right away
                Some(Command::SetSoloistKey { key }) => {
                    store_key(key, ev_tx).await;
                    return true;
                }
                Some(_) => { let _ = ev_tx.send(EngineEvent::Error(why.into())).await; }
                None => return false,
            },
        }
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

/// The same, for the build inside the Soloist VM (the guest asks the CDN itself).
#[cfg(target_os = "macos")]
async fn keep_current_in_vm(v: Arc<crate::soloist_vm::VmHost>, days: watch::Receiver<Option<u32>>, staged: Arc<Notify>) {
    let mut failures = 0;
    loop {
        match v.check().await {
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
        let wait = std::env::var("SINGULARITY_SOLOIST_CHECK_SECS").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or_else(|| update::next_check(failures, *days.borrow()));
        tokio::time::sleep(wait).await;
    }
}

/// Is now a good moment to restart into a new build? Not in the middle of a playing track.
pub fn quiet_moment(p: &PlaybackState, track_changed: bool) -> bool {
    track_changed || !matches!(p.status, PlayStatus::Playing | PlayStatus::Buffering)
}

/// Where to pick up after the restart (a track that just started resumes from its start).
pub fn resume_point(p: &PlaybackState, track_changed: bool) -> Option<Resume> {
    let item = p.item.as_ref()?;
    let position_ms = if track_changed { 0 } else { p.position_at(singularity_proto::now_ms()) };
    Some(Resume { context_uri: p.context_uri.clone(), track_uri: item.uri.clone(), position_ms, paused: p.status == PlayStatus::Paused })
}

enum Exit {
    Quit,
    Died,
    Expired,
    /// restarting into a new build
    Update,
}

/// Is a Soloist already running on this state directory? (Its pid file names a live `soloist` process.)
pub fn running_soloist(state: &std::path::Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(state.join("data/soloist.pid")).ok()?.trim().parse().ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    (comm.trim() == "soloist").then_some(pid)
}

// (the ZeroConf port is only used on macOS, to announce the VM's Soloist for pairing)
#[cfg_attr(not(target_os = "macos"), allow(unused_assignments, unused_variables))]
async fn run_once(engine: &SoloistEngine, key: &str, cmd_rx: &mut mpsc::Receiver<Command>, ev_tx: &mpsc::Sender<EngineEvent>, upd: &mut Updates) -> anyhow::Result<Exit> {
    let state = engine.state_dir();
    // another Daemon's Soloist owns these files: touch nothing (its port file, its log)
    if engine.vm().is_none()
        && let Some(pid) = running_soloist(&state)
    {
        anyhow::bail!("Soloist is already running (pid {pid}) with Singularity's data: is a second singularityd running?");
    }
    tokio::fs::create_dir_all(state.join("data")).await?;
    tokio::fs::create_dir_all(state.join("cache")).await?;
    let _ = tokio::fs::remove_file(state.join("data/ws.port")).await;
    // keep the previous log (a Soloist that's still running keeps writing to it, so it's never cut short)
    let log_path = state.join("soloist.log");
    let _ = std::fs::rename(&log_path, state.join("soloist.log.1"));
    let mut log = tokio::fs::File::create(&log_path).await?;
    // (the VM's Soloist session: closing its stdin stops Soloist, so it's held for this run)
    #[allow(unused_mut, unused_assignments)]
    let mut session_stdin: Option<tokio::process::ChildStdin> = None;
    #[cfg(target_os = "macos")]
    let in_vm = engine.vm().cloned();
    #[cfg(not(target_os = "macos"))]
    let in_vm: Option<std::convert::Infallible> = None;
    let mut child: Child = match &in_vm {
        #[cfg(target_os = "macos")]
        Some(v) => {
            let (c, stdin) = v.launch(key, &engine.device_name, engine.saved_volume()).await?;
            session_stdin = Some(stdin);
            c
        }
        #[allow(unreachable_patterns)]
        Some(_) => unreachable!(),
        None => {
            let mut cmd = Proc::new(engine.binary());
            cmd.args(["--device-name", &engine.device_name, "--api-key", key, "--ws", "127.0.0.1:0", "--verbose"])
                .args(engine.saved_volume().map(|v| ["--initial-volume".to_string(), v.to_string()]).into_iter().flatten())
                .arg("--data-dir")
                .arg(state.join("data"))
                .arg("--cache-dir")
                .arg(state.join("cache"))
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
            cmd.spawn()?
        }
    };
    let _session = session_stdin;
    // Soloist's output: parsed for news, then written on to the log
    let (news_tx, mut news_rx) = mpsc::channel::<LogNews>(16);
    let (line_tx, mut line_rx) = mpsc::channel::<String>(256);
    for out in [child.stdout.take().map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|e| Box::new(e) as _)].into_iter().flatten() {
        let tx = line_tx.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if tx.send(l).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(line_tx);
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut parser = LogParser::default();
        while let Some(l) = line_rx.recv().await {
            let _ = log.write_all(l.as_bytes()).await;
            let _ = log.write_all(b"\n").await;
            if let Some(n) = parser.line(&l) {
                let _ = news_tx.send(n).await;
            }
        }
    });
    #[cfg(target_os = "macos")]
    let version = match &in_vm {
        Some(v) => v.version().await,
        None => Proc::new(engine.binary()).arg("--version").output().await.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default(),
    };
    #[cfg(not(target_os = "macos"))]
    let version = Proc::new(engine.binary()).arg("--version").output().await.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    // the VM's Soloist: its WebSocket through the forwarded port, and its ZeroConf port for pairing
    #[cfg(target_os = "macos")]
    let vm_ws = in_vm.as_ref().map(|v| format!("ws://127.0.0.1:{}", v.ws_port));
    #[cfg(not(target_os = "macos"))]
    let vm_ws: Option<String> = None;
    let mut zeroconf: Option<u16> = None;

    // wait for the WebSocket endpoint files
    let mut addr = None;
    let mut expires_in_days = None;
    // the stream's download speed and recent stalls (#45)
    let mut delivery = singularity_proto::Delivery::default();
    for _ in 0..100 {
        if let Some(st) = child.try_wait()? {
            // let the output catch up: it says why
            tokio::time::sleep(Duration::from_millis(100)).await;
            while let Ok(n) = news_rx.try_recv() {
                match n {
                    LogNews::BadKey => return Err(BadKey.into()),
                    LogNews::Locked => anyhow::bail!("another Soloist is already running with Singularity's data (is a second singularityd running?)"),
                    _ => {}
                }
            }
            return Ok(if st.code() == Some(10) { Exit::Expired } else { Exit::Died });
        }
        while let Ok(n) = news_rx.try_recv() {
            match n {
                LogNews::Lifetime(d) => {
                    expires_in_days = Some(d);
                    upd.days.send_replace(Some(d));
                }
                LogNews::Locked => anyhow::bail!("another Soloist is already running with Singularity's data (is a second singularityd running?)"),
                LogNews::BadKey => return Err(BadKey.into()),
                LogNews::ZeroConf(p) => zeroconf = Some(p),
                LogNews::Stream(_) | LogNews::Bandwidth(_) | LogNews::Stall => {}
            }
        }
        // the VM's Soloist: ready once its WebSocket answers through the forward
        if let Some(a) = &vm_ws {
            if let Ok(Ok(_)) = tokio::time::timeout(Duration::from_secs(2), tokio_tungstenite::connect_async(a.as_str())).await {
                addr = Some(a.clone());
                break;
            }
        } else if let (Ok(p), Ok(a)) = (std::fs::read_to_string(state.join("data/ws.port")), std::fs::read_to_string(state.join("data/ws.addr"))) {
            addr = Some(format!("ws://{}:{}", a.trim(), p.trim()));
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let Some(addr) = addr else { anyhow::bail!("Soloist never opened its WebSocket") };
    let (ws, _) = tokio_tungstenite::connect_async(addr).await?;
    let (mut ws_tx, mut ws_rx) = ws.split();
    let mut st = SoloistState::default();
    let mut logged_in = false;
    // macOS: Soloist's pairing endpoint announced on the LAN while it waits for a phone
    #[cfg(target_os = "macos")]
    let mut announced: Option<crate::soloist_vm::Announcement> = None;
    let mut active = false;
    // a build staged while this one runs waits for a quiet moment
    let mut staged = false;

    loop {
        tokio::select! {
            exit = child.wait() => {
                let exit = exit?;
                return Ok(if exit.code() == Some(10) { Exit::Expired } else { Exit::Died });
            }
            msg = ws_rx.next() => {
                let Some(Ok(Message::Text(t))) = msg else {
                    if msg.is_none() { return Ok(Exit::Died); }
                    continue;
                };
                let Ok(ev) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                match ev["type"].as_str() {
                    Some("auth_state") | Some("device_changed") => {
                        if let Some(l) = ev["logged_in"].as_bool() { logged_in = l; }
                        if let Some(a) = ev["is_active"].as_bool() { active = a; }
                        // back after an update: pick up where playback was
                        if logged_in && let Some(r) = upd.resume.take() {
                            let _ = ev_tx.send(EngineEvent::Resume(r)).await;
                        }
                        #[cfg(target_os = "macos")]
                        if let Some(v) = &in_vm {
                            if logged_in {
                                announced = None;
                            } else if announced.is_none()
                                && let Some(p) = zeroconf
                            {
                                match v.announce(p, &engine.device_name).await {
                                    Ok(a) => announced = Some(a),
                                    Err(e) => tracing::warn!("pairing: couldn't announce Soloist on the LAN: {e}"),
                                }
                            }
                        }
                        let s = if logged_in {
                            // (the lifetime may still arrive below; status is re-sent then)
                            EngineStatus::Ready { device_name: engine.device_name.clone(), active, version: version.clone(), expires_in_days }
                        } else {
                            EngineStatus::NeedsPairing { device_name: engine.device_name.clone() }
                        };
                        status(ev_tx, s).await;
                    }
                    Some("error") => {
                        let _ = ev_tx.send(EngineEvent::Error(ev["message"].as_str().unwrap_or("Soloist error").to_string())).await;
                    }
                    _ => {
                        let track_changed = ev["type"] == "track_changed";
                        let volume = st.playback.volume;
                        if st.apply(&ev) {
                            let _ = ev_tx.send(EngineEvent::Playback(st.playback.clone())).await;
                        }
                        if st.playback.volume != volume && ev["type"] == "volume_changed" {
                            engine.save_volume(st.playback.volume);
                        }
                        if track_changed {
                            let _ = ws_tx.send(Message::text(json!({"type":"command","command":"get_queue","limit":20}).to_string())).await;
                        }
                        if staged && quiet_moment(&st.playback, track_changed) {
                            return restart_for_update(&mut child, &st.playback, track_changed, upd).await;
                        }
                    }
                }
            }
            // (a notice from before this build started may be stale: only a build still waiting counts)
            _ = upd.staged.notified(), if engine.update_waiting() => {
                staged = true;
                if quiet_moment(&st.playback, false) {
                    return restart_for_update(&mut child, &st.playback, false, upd).await;
                }
            }
            Some(n) = news_rx.recv() => match n {
                LogNews::Stream(info) => { let _ = ev_tx.send(EngineEvent::Stream(info)).await; }
                LogNews::Bandwidth(k) => {
                    delivery.bandwidth_kbps = Some(k);
                    delivery.measured_ms = singularity_proto::now_ms();
                    let _ = ev_tx.send(EngineEvent::Delivery(delivery.clone())).await;
                }
                LogNews::Stall => {
                    let now = singularity_proto::now_ms();
                    delivery.stalls_ms.retain(|t| now.saturating_sub(*t) <= 120_000);
                    delivery.stalls_ms.push(now);
                    let _ = ev_tx.send(EngineEvent::Delivery(delivery.clone())).await;
                }
                LogNews::Lifetime(d) => {
                    expires_in_days = Some(d);
                    upd.days.send_replace(Some(d));
                    if logged_in {
                        status(ev_tx, EngineStatus::Ready { device_name: engine.device_name.clone(), active, version: version.clone(), expires_in_days }).await;
                    }
                }
                LogNews::ZeroConf(p) => zeroconf = Some(p),
                LogNews::Locked | LogNews::BadKey => {}
            },
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { let _ = child.kill().await; return Ok(Exit::Quit) };
                if cmd == Command::RestartEngine {
                    let _ = ev_tx.send(EngineEvent::Notice("Restarting Soloist…".into())).await;
                    let _ = child.kill().await;
                    return Ok(Exit::Died);
                }
                if let Command::SetSoloistKey { key } = cmd {
                    // a new key: restart Soloist with it
                    store_key(key, ev_tx).await;
                    let _ = child.kill().await;
                    return Ok(Exit::Died);
                }
                for m in st.to_soloist(cmd) {
                    ws_tx.send(Message::text(m.to_string())).await?;
                }
            }
        }
    }
}

/// Stop Soloist so the new build takes over, remembering where playback was.
async fn restart_for_update(child: &mut Child, p: &PlaybackState, track_changed: bool, upd: &mut Updates) -> anyhow::Result<Exit> {
    upd.resume = resume_point(p, track_changed);
    tracing::info!("restarting Soloist into the new build ({})", if track_changed { "between tracks" } else { "while quiet" });
    let _ = child.kill().await;
    Ok(Exit::Update)
}

/// Translates Soloist's WebSocket events into our `PlaybackState`, and our commands into Soloist's.
/// Pure (no I/O) so it can be tested against recorded transcripts.
#[derive(Default)]
pub struct SoloistState {
    pub playback: PlaybackState,
}

fn entity(v: &Value) -> Option<Item> {
    let uri = v["uri"].as_str().filter(|u| !u.is_empty())?.to_string();
    let d = &v["decorations"];
    let artists: Vec<String> = d["creators"].as_array().map(|a| a.iter().filter_map(|c| c["entity"]["decorations"]["identity"]["name"].as_str().map(String::from)).collect()).unwrap_or_default();
    let covers = d["visual_identity"]["cover"].as_array().cloned().unwrap_or_default();
    let cover_url = ["large", "xlarge", "default", "small"]
        .iter()
        .find_map(|size| covers.iter().find(|c| c["size"] == *size))
        .or(covers.first())
        .and_then(|c| c["url"].as_str().map(String::from));
    // an episode's creator may be missing: its show (the parent) stands in
    let parent = d["parent"]["entity"]["decorations"]["identity"]["name"].as_str().map(String::from);
    let artists = if artists.is_empty() && uri.starts_with("spotify:episode:") { parent.clone().into_iter().collect() } else { artists };
    Some(Item {
        uri,
        name: d["identity"]["name"].as_str().unwrap_or_default().to_string(),
        artists,
        album: d["parent"]["entity"]["decorations"]["identity"]["name"].as_str().map(String::from),
        cover_url,
        duration_ms: d["playback"]["duration_ms"].as_u64().unwrap_or(0),
        explicit: d["playback"]["content_ratings"].as_array().is_some_and(|r| r.iter().any(|x| x == "explicit")),
        artist_uris: d["creators"].as_array().map(|a| a.iter().filter_map(|c| c["entity"]["uri"].as_str().map(String::from)).collect()).unwrap_or_default(),
        album_uri: d["parent"]["entity"]["uri"].as_str().map(String::from),
    })
}

fn status_of(s: &str) -> PlayStatus {
    match s {
        "playing" => PlayStatus::Playing,
        "paused" => PlayStatus::Paused,
        "buffering" => PlayStatus::Buffering,
        _ => PlayStatus::Idle,
    }
}

impl SoloistState {
    /// Apply one Soloist event. Returns true when our playback state changed.
    pub fn apply(&mut self, ev: &Value) -> bool {
        let p = &mut self.playback;
        match ev["type"].as_str().unwrap_or("") {
            "playback_state" => {
                p.status = status_of(ev["status"].as_str().unwrap_or(""));
                p.item = entity(&ev["item"]);
                p.context_uri = ev["context"]["uri"].as_str().filter(|u| !u.is_empty()).map(String::from);
                p.context_name = ev["context"]["decorations"]["identity"]["name"].as_str().map(String::from);
                self.position(&ev["position"]);
                let p = &mut self.playback;
                p.volume = ev["volume"].as_u64().unwrap_or(p.volume as u64).min(100) as u8;
                self.options(&ev["options"]);
            }
            "track_changed" => p.item = entity(&ev["item"]),
            "playback_changed" => {
                p.status = status_of(ev["status"].as_str().unwrap_or(""));
                if p.status != PlayStatus::Playing {
                    p.speed = 0.0;
                }
            }
            "volume_changed" => p.volume = ev["volume"].as_u64().unwrap_or(0).min(100) as u8,
            "context_changed" => {
                p.context_uri = ev["context"]["uri"].as_str().filter(|u| !u.is_empty()).map(String::from);
                p.context_name = ev["context"]["decorations"]["identity"]["name"].as_str().map(String::from);
            }
            "options_changed" => self.options(&ev["options"]),
            "position_sync" => self.position(&ev["position"]),
            "queue_changed" => {
                p.queue = ev["upcoming"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|q| {
                                let item = entity(&q["item"]).unwrap_or_else(|| Item { uri: q["uid"].as_str().unwrap_or_default().into(), ..Default::default() });
                                Some(QueueEntry { item, source: q["source"].as_str().unwrap_or("context").into(), uid: q["uid"].as_str().unwrap_or_default().into() })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            _ => return false,
        }
        true
    }

    fn position(&mut self, pos: &Value) {
        let p = &mut self.playback;
        if let (Some(ms), Some(ts)) = (pos["position_ms"].as_u64(), pos["timestamp_ms"].as_u64()) {
            p.position_ms = ms;
            p.timestamp_ms = ts;
            p.speed = pos["speed"].as_f64().unwrap_or(0.0) as f32;
        }
    }

    fn options(&mut self, o: &Value) {
        let p = &mut self.playback;
        if let Some(s) = o["shuffle"].as_bool() {
            p.shuffle = s;
        }
        p.repeat = match o["repeat"].as_str() {
            Some("context") => RepeatMode::Context,
            Some("track") => RepeatMode::Track,
            Some(_) => RepeatMode::Off,
            None => p.repeat,
        };
    }

    /// Our command → Soloist WebSocket command(s).
    pub fn to_soloist(&self, cmd: Command) -> Vec<Value> {
        let c = |name: &str| json!({"type":"command","command":name});
        match cmd {
            Command::Play { uri: Some(u) } => vec![json!({"type":"command","command":"play","uri":u})],
            Command::Play { uri: None } => vec![c("play")],
            Command::Pause => vec![c("pause")],
            Command::TogglePlay => vec![c(if self.playback.status == PlayStatus::Playing { "pause" } else { "play" })],
            Command::Next => vec![c("skip_next")],
            Command::Previous => vec![c("skip_prev")],
            Command::Seek { position_ms } => vec![json!({"type":"command","command":"seek","position_ms":position_ms})],
            Command::SetVolume { percent } => vec![json!({"type":"command","command":"set_volume","volume":percent.min(100)})],
            Command::SetShuffle { enabled } => vec![json!({"type":"command","command":"set_shuffle","enabled":enabled})],
            Command::SetRepeat { mode } => {
                let (t, ctx) = match mode {
                    RepeatMode::Off => (false, false),
                    RepeatMode::Context => (false, true),
                    RepeatMode::Track => (true, false),
                };
                // order matters (Soloist docs): disable the other mode first
                if t {
                    vec![json!({"type":"command","command":"set_repeat_context","enabled":false}), json!({"type":"command","command":"set_repeat_track","enabled":true})]
                } else {
                    vec![json!({"type":"command","command":"set_repeat_track","enabled":false}), json!({"type":"command","command":"set_repeat_context","enabled":ctx})]
                }
            }
            Command::AddToQueue { uri } => vec![json!({"type":"command","command":"add_to_queue","uri":uri})],
            Command::GetState => vec![c("get_state")],
            // Soloist can't start a context at an offset (tested 2026-10-17); ConnectRemote does that. Here, as a
            // last resort, the track plays on its own.
            Command::PlayFrom { track_uri: Some(u), .. } => vec![json!({"type":"command","command":"play","uri":u})],
            Command::PlayFrom { .. } | Command::GetLibrary | Command::GetTracks { .. } | Command::MoreTracks { .. } | Command::FindShow { .. } | Command::Spectrum { .. } | Command::Shutdown | Command::SetSoloistKey { .. } | Command::GetHealth | Command::GetLyrics { .. } | Command::GetArtist { .. } | Command::Search { .. } | Command::GetDevices | Command::Transfer { .. } | Command::SetSaved { .. } | Command::CheckSaved { .. } | Command::GetListening | Command::GetStats { .. } | Command::ExportLocalData | Command::DeleteLocalData | Command::CreatePlaylist { .. } | Command::DeletePlaylist { .. } | Command::EditPlaylist { .. } | Command::AddToPlaylist { .. } | Command::RemoveFromPlaylist { .. } | Command::MoveInPlaylist { .. } | Command::SetPlaylistCover { .. } => vec![],
            // Soloist only appends; the rest goes through ConnectRemote
            Command::PlayNext { uri } => vec![json!({"type":"command","command":"add_to_queue","uri":uri})],
            Command::RemoveFromQueue { .. } | Command::ClearQueue | Command::MoveInQueue { .. } => vec![],
            Command::Login | Command::Logout | Command::GetHome | Command::RestartEngine | Command::Unknown => vec![],
        }
    }
}
