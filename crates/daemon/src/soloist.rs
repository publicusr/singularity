//! SoloistEngine (ADR-0001): supervises Spotify Soloist and drives it over its WebSocket.
//!
//! - where it runs: the Soloist runner (`runner.rs`, D89); on Linux a local process, its binary
//!   `$XDG_DATA_HOME/singularity/soloist/soloist`, downloaded from Spotify's CDN when missing (never redistributed)
//!   and kept current by `update` (#6): a new build is restarted into at a quiet moment, and playback picks up where
//!   it was
//! - API key: `SOLOIST_API_KEY`, else the keyring (`service singularity account soloist`), else the owner-only
//!   fallback file; the setup wizard stores it (`Command::SetSoloistKey`). Soloist only takes it as `--api-key`, so it
//!   shows in `ps` for local users: there's no other way in
//! - output: Soloist's verbose log is piped through the Daemon, which reads the stream format and build lifetime
//!   from it and writes it on to `soloist.log` (the previous one is kept as `soloist.log.1`, never truncated)
//! - Soloist's own state (pairing, cache) lives under `$XDG_DATA_HOME/singularity/soloist-state`
//! - On macOS (D55) the same supervision runs Soloist inside the Soloist VM (`soloist_vm.rs`): the build is kept current
//!   in the guest, Soloist is started over SSH, its WebSocket and audio come through forwarded ports, and pairing is
//!   announced on the LAN while it waits for it

pub mod runner;
mod session;

pub use runner::running_soloist;
pub use session::{Action, Handover, Input, Now, Session, Stop};

use crate::engine::{Engine, EngineEvent};
use runner::{Local, Runner};
use futures_util::{FutureExt, SinkExt, StreamExt};
use serde_json::Value;
use singularity_proto::{Command, EngineStatus};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::{Notify, mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

pub const DEVICE_NAME: &str = "Singularity";

#[derive(Clone)]
pub struct SoloistEngine {
    pub data_dir: PathBuf,
    /// its name on Connect
    pub device_name: String,
    /// where Soloist runs, picked by the Engine assembly
    pub runner: Local,
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
        let data_dir = base.join("singularity");
        SoloistEngine {
            runner: Local::new(data_dir.clone(), singularity_config::EngineChoice::default().soloist_cache_mb()),
            data_dir,
            device_name: DEVICE_NAME.into(),
            #[cfg(target_os = "macos")]
            vm: None,
        }
    }

    /// Soloist as a local process, as `device_name` on Connect, its cache capped at `cache_mb` (0: no limit).
    pub fn local(device_name: String, cache_mb: u32) -> SoloistEngine {
        let mut e = SoloistEngine::from_env();
        e.device_name = device_name;
        e.runner = Local::new(e.data_dir.clone(), cache_mb);
        e
    }

    /// Soloist in the Soloist VM, as `device_name` on Connect (the Engine assembly names it); its audio also goes to
    /// `feed`.
    #[cfg(target_os = "macos")]
    pub fn in_vm(device_name: String, feed: Arc<crate::tap::Feed>) -> SoloistEngine {
        let mut e = SoloistEngine::from_env();
        e.device_name = device_name;
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
        self.runner.update_waiting()
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
    /// playback runs again after running dry
    Flowing,
    /// the context it was told to play has nothing it can play (Spotify's DJ, a podcast-only or blocked context;
    /// verified 2026-10-31: `AdvanceStuck detected, reason: unplayable`, after which it stays silent)
    Unplayable,
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
        if line.contains("audioThreadMain: underrun") {
            return Some(LogNews::Stall);
        }
        // the recovery line: audio reaches the sink again
        if line.contains("updateInternalDriverToPlaying: underrun") {
            return Some(LogNews::Flowing);
        }
        if line.contains("AdvanceStuck detected, reason: unplayable") {
            return Some(LogNews::Unplayable);
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
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek() {
                // CSI: parameters and private markers, then one final byte in 0x40..0x7e (colour, cursor moves)
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC (a window title, OSC 52, a hyperlink): runs to BEL or ST (ESC \)
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.peek() == Some(&'\\')) {
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            },
            // every other control character (NUL, BEL, tab, CR…): a log line is text, not a terminal script
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

impl Engine for SoloistEngine {
    fn respawn(&self) -> Option<Box<dyn Engine>> {
        Some(Box::new(self.clone()))
    }

    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (ev_tx, ev_rx) = mpsc::channel(256);
        tokio::spawn(supervise(*self, cmd_rx, ev_tx));
        (cmd_tx, ev_rx)
    }
}

/// Run an update checker and keep it running: it loops forever, so only a panic ends it. The health view names it
/// while it's down (A1: it used to be a bare `tokio::spawn`, so a panic silently meant no Soloist updates again).
fn keep_checking<F, Fut>(name: &'static str, ev: mpsc::Sender<EngineEvent>, mut start: F) -> tokio::task::JoinHandle<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        loop {
            // the health row goes once the checker has run for a while (one that keeps dying keeps its row)
            let back = tokio::spawn({
                let ev = ev.clone();
                async move {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    let _ = ev.send(EngineEvent::TaskStarted(name)).await;
                }
            });
            let failed = std::panic::AssertUnwindSafe(start()).catch_unwind().await.is_err();
            back.abort();
            let _ = ev.send(EngineEvent::TaskStopped(name, if failed { "it panicked" } else { "it stopped" }.into())).await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    })
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
    /// what the last run left for the next (where playback was, the last restart for a stuck stream)
    handover: Handover,
}

/// Keep Soloist running: start, connect, pump; on exit wait (with backoff) and start again.
async fn supervise(engine: SoloistEngine, mut cmd_rx: mpsc::Receiver<Command>, ev_tx: mpsc::Sender<EngineEvent>) {
    let mut backoff = Duration::from_secs(2);
    let runner = &engine.runner;
    let mut upd = Updates { staged: Arc::new(Notify::new()), days: watch::channel(None).0, handover: Handover::default() };
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
                let (days, staged, ev, vm) = (upd.days.clone(), upd.staged.clone(), ev_tx.clone(), v.clone());
                updater = Some(keep_checking("the Soloist update checker", ev, move || keep_current_in_vm(vm.clone(), days.subscribe(), staged.clone())));
            }
            let started = std::time::Instant::now();
            let exit = run_once(&engine, runner, &key, &mut cmd_rx, &ev_tx, &mut upd).await;
            if matches!(exit, Ok(Exit::Quit)) {
                if let Some(u) = updater {
                    u.abort();
                }
                v.down().await;
                return;
            }
            match exit {
                Ok(Exit::Update) | Ok(Exit::Expired) | Ok(Exit::Restart) => {
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
            if !wait_or_refuse(&mut cmd_rx, &ev_tx, restart_wait(&mut backoff, started.elapsed()), "Soloist is restarting").await {
                v.down().await;
                return;
            }
            continue;
        }
        // Soloist isn't running: the best moment to put a staged build in place
        if let Err(e) = runner.prepare().await {
            status(&ev_tx, EngineStatus::Failed { message: e.to_string() }).await;
            if !wait_or_refuse(&mut cmd_rx, &ev_tx, backoff, "Soloist isn't available").await {
                return;
            }
            backoff = (backoff * 2).min(Duration::from_secs(300));
            continue;
        }
        // there's a build: from now on keep it current
        if updater.is_none() {
            let (days, staged, ev, r) = (upd.days.clone(), upd.staged.clone(), ev_tx.clone(), runner.clone());
            updater = Some(keep_checking("the Soloist update checker", ev, move || r.keep_current(days.subscribe(), staged.clone())));
        }
        let started = std::time::Instant::now();
        match run_once(&engine, runner, &key, &mut cmd_rx, &ev_tx, &mut upd).await {
            Ok(Exit::Quit) => {
                if let Some(u) = updater {
                    u.abort();
                }
                return;
            }
            // straight back up with the new build, or because it was asked for
            Ok(Exit::Update) | Ok(Exit::Restart) => {
                backoff = Duration::from_secs(2);
                continue;
            }
            Ok(Exit::Expired) => {
                // (it shouldn't come to this: updates land well before)
                status(&ev_tx, EngineStatus::Failed { message: "this Soloist build has expired; updating".into() }).await;
                runner.expired();
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
        if !wait_or_refuse(&mut cmd_rx, &ev_tx, restart_wait(&mut backoff, started.elapsed()), "Soloist is restarting").await {
            return;
        }
    }
}

/// A run at least this long was healthy: if Soloist stops after it, the waits start again from the shortest.
const HEALTHY_RUN: Duration = Duration::from_secs(300);

/// The wait before starting Soloist again after a run of `ran` that ended by itself: 2 s, doubling up to a minute
/// while it keeps failing, and 2 s again after a healthy run (a crash days later isn't the fifth in a row).
fn restart_wait(backoff: &mut Duration, ran: Duration) -> Duration {
    if ran >= HEALTHY_RUN {
        *backoff = Duration::from_secs(2);
    }
    let wait = *backoff;
    *backoff = (wait * 2).min(Duration::from_secs(60));
    wait
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
        let wait = std::env::var("SINGULARITY_SOLOIST_CHECK_SECS").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or_else(|| crate::update::next_check(failures, *days.borrow()));
        tokio::time::sleep(wait).await;
    }
}

/// Soloist's state directory and the two below it, owner-only (it holds its pairing and cache: audit S1).
fn private_state_dirs(state: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for dir in [state.to_path_buf(), state.join("data"), state.join("cache")] {
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// This run's `soloist.log`, owner-only, with the previous one kept as `soloist.log.1` (a Soloist that's still running
/// keeps writing to its old file, so it's never cut short). `mode(0600)` only applies on creation: an older build's
/// log is tightened as well.
fn open_log(state: &std::path::Path) -> std::io::Result<tokio::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = state.join("soloist.log");
    let previous = state.join("soloist.log.1");
    let _ = std::fs::rename(&path, &previous);
    let _ = std::fs::set_permissions(&previous, std::fs::Permissions::from_mode(0o600));
    let f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&path)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(tokio::fs::File::from_std(f))
}

enum Exit {
    Quit,
    Died,
    /// stopped because it was asked to (Restart the player, a new key): straight back up
    Restart,
    Expired,
    /// restarting into a new build
    Update,
}

/// A Soloist that has been started, before its WebSocket is up.
struct Started {
    child: Child,
    /// what its log says
    news: mpsc::Receiver<LogNews>,
    version: String,
    /// the VM's Soloist: its WebSocket, through the forwarded port
    vm_ws: Option<String>,
    #[cfg(target_os = "macos")]
    in_vm: Option<Arc<crate::soloist_vm::VmHost>>,
    /// (the VM's Soloist session: closing its stdin stops Soloist, so it's held for the run)
    _vm_stdin: Option<tokio::process::ChildStdin>,
}

/// Start Soloist, its output parsed for news and written on to the log.
async fn start(engine: &SoloistEngine, runner: &impl Runner, key: &str) -> anyhow::Result<Started> {
    let state = engine.state_dir();
    // another Daemon's Soloist owns these files: touch nothing (its port file, its log)
    if engine.vm().is_none() {
        runner.check_free()?;
    }
    private_state_dirs(&state)?;
    let mut log = open_log(&state)?;
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
        None => runner.launch(key, &engine.device_name, engine.saved_volume()).await?,
    };
    // Soloist's output: parsed for news, then written on to the log
    let (news_tx, news_rx) = mpsc::channel::<LogNews>(16);
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
            // written raw, the log would be a terminal-escape sink for hostile track names (audit S1)
            let _ = log.write_all(strip_ansi(&l).as_bytes()).await;
            let _ = log.write_all(b"\n").await;
            if let Some(n) = parser.line(&l) {
                let _ = news_tx.send(n).await;
            }
        }
    });
    #[cfg(target_os = "macos")]
    let version = match &in_vm {
        Some(v) => v.version().await,
        None => runner.version().await,
    };
    #[cfg(not(target_os = "macos"))]
    let version = runner.version().await;
    // the VM's Soloist: its WebSocket through the forwarded port
    #[cfg(target_os = "macos")]
    let vm_ws = in_vm.as_ref().map(|v| format!("ws://127.0.0.1:{}", v.ws_port));
    #[cfg(not(target_os = "macos"))]
    let vm_ws: Option<String> = None;
    Ok(Started {
        child,
        news: news_rx,
        version,
        vm_ws,
        #[cfg(target_os = "macos")]
        in_vm,
        _vm_stdin: session_stdin,
    })
}

/// One run of Soloist: started, then its process, its log, its WebSocket, our commands and the update side pumped
/// through a [`Session`]. What the run leaves for the next one goes back into `upd`.
async fn run_once(engine: &SoloistEngine, runner: &impl Runner, key: &str, cmd_rx: &mut mpsc::Receiver<Command>, ev_tx: &mpsc::Sender<EngineEvent>, upd: &mut Updates) -> anyhow::Result<Exit> {
    let mut run = start(engine, runner, key).await?;
    let mut s = Session::new(engine.device_name.clone(), run.version.clone(), engine.saved_volume(), std::mem::take(&mut upd.handover));
    let exit = pump(engine, runner, &mut run, &mut s, cmd_rx, ev_tx, upd).await;
    upd.handover = s.into_handover();
    exit
}

/// Carry out what a step of the session asks for. Returns the stop among it, if any (Soloist isn't stopped yet).
async fn carry_out<S>(actions: Vec<Action>, ws: &mut S, engine: &SoloistEngine, ev_tx: &mpsc::Sender<EngineEvent>) -> anyhow::Result<Option<Stop>>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    for a in actions {
        match a {
            Action::Send(m) => ws.send(Message::text(m.to_string())).await?,
            Action::Event(e) => {
                let _ = ev_tx.send(e).await;
            }
            Action::SaveVolume(v) => engine.save_volume(v),
            Action::StoreKey(k) => store_key(k, ev_tx).await,
            Action::Stop(s) => return Ok(Some(s)),
        }
    }
    Ok(None)
}

fn locked() -> anyhow::Error {
    anyhow::anyhow!("another Soloist is already running with Singularity's data (is a second singularityd running?)")
}

/// Soloist stopped by itself (or closed its WebSocket): its last lines may say why (a rejected key), so they get a
/// moment to arrive, and the session the last word.
async fn died(child: &mut Child, news_rx: &mut mpsc::Receiver<LogNews>, s: &mut Session) -> anyhow::Result<Exit> {
    tokio::time::sleep(Duration::from_millis(100)).await;
    while let Ok(n) = news_rx.try_recv() {
        // (the run is over: only why it ended still counts)
        if let Some(Action::Stop(stop)) = s.step(Input::Log(n), Now::real()).into_iter().find(|a| matches!(a, Action::Stop(_))) {
            return stopped(child, stop).await;
        }
    }
    Ok(match child.try_wait()? {
        Some(st) if st.code() == Some(10) => Exit::Expired,
        _ => Exit::Died,
    })
}

/// Stop Soloist for the session's reason.
async fn stopped(child: &mut Child, s: Stop) -> anyhow::Result<Exit> {
    let _ = child.kill().await;
    Ok(match s {
        Stop::Restart => Exit::Restart,
        Stop::Update => Exit::Update,
        Stop::BadKey => return Err(BadKey.into()),
        Stop::Locked => return Err(locked()),
    })
}

/// The I/O around a [`Session`]: Soloist's process, its log, its WebSocket, our commands, the update side and the
/// session's deadline in; what the session asks for out.
// (the ZeroConf port is only used on macOS, to announce the VM's Soloist for pairing)
#[cfg_attr(not(target_os = "macos"), allow(unused_assignments, unused_variables))]
async fn pump(engine: &SoloistEngine, runner: &impl Runner, run: &mut Started, s: &mut Session, cmd_rx: &mut mpsc::Receiver<Command>, ev_tx: &mpsc::Sender<EngineEvent>, upd: &Updates) -> anyhow::Result<Exit> {
    let (child, news_rx) = (&mut run.child, &mut run.news);
    // what the session doesn't need from the log: the build's days left go to the update checker, the ZeroConf port
    // to pairing
    let mut zeroconf: Option<u16> = None;
    let tap = |n: &LogNews, zeroconf: &mut Option<u16>| match n {
        LogNews::Lifetime(d) => {
            upd.days.send_replace(Some(*d));
        }
        LogNews::ZeroConf(p) => *zeroconf = Some(*p),
        _ => {}
    };
    // wait for the WebSocket endpoint files
    let mut addr = None;
    for _ in 0..100 {
        if child.try_wait()?.is_some() {
            return died(child, news_rx, s).await;
        }
        while let Ok(n) = news_rx.try_recv() {
            tap(&n, &mut zeroconf);
            // before the WebSocket is up, only the build's lifetime and why it can't start count (as ever); log news
            // sends nothing to Soloist either way
            if !matches!(n, LogNews::Lifetime(_) | LogNews::BadKey | LogNews::Locked) {
                continue;
            }
            if let Some(stop) = carry_out(s.step(Input::Log(n), Now::real()), &mut futures_util::sink::drain(), engine, ev_tx).await? {
                return stopped(child, stop).await;
            }
        }
        // the VM's Soloist: ready once its WebSocket answers through the forward
        if let Some(a) = &run.vm_ws {
            if let Ok(Ok(_)) = tokio::time::timeout(Duration::from_secs(2), tokio_tungstenite::connect_async(a.as_str())).await {
                addr = Some(a.clone());
                break;
            }
        } else if let Some(a) = runner.ws_addr().await {
            addr = Some(a);
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let Some(addr) = addr else { anyhow::bail!("Soloist never opened its WebSocket") };
    let (ws, _) = tokio_tungstenite::connect_async(addr).await?;
    let (mut ws_tx, mut ws_rx) = ws.split();
    // macOS: Soloist's pairing endpoint announced on the LAN while it waits for a phone
    #[cfg(target_os = "macos")]
    let mut announced: Option<crate::soloist_vm::Announcement> = None;

    loop {
        let deadline = s.deadline();
        let actions = tokio::select! {
            exit = child.wait() => {
                exit?;
                return died(child, news_rx, s).await;
            }
            msg = ws_rx.next() => {
                let Some(Ok(Message::Text(t))) = msg else {
                    if msg.is_none() { return died(child, news_rx, s).await; }
                    continue;
                };
                let Ok(ev) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                let pairing = matches!(ev["type"].as_str(), Some("auth_state") | Some("device_changed"));
                let actions = s.step(Input::Ws(ev), Now::real());
                #[cfg(target_os = "macos")]
                if pairing && let Some(v) = &run.in_vm {
                    if s.logged_in() {
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
                actions
            }
            // (a notice from before this build started may be stale: only a build still waiting counts)
            _ = upd.staged.notified(), if engine.update_waiting() => s.step(Input::Staged, Now::real()),
            // the time the session asked to be woken at: a resume after a restart, a dry spell gone on too long
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(tokio::time::Instant::now)), if deadline.is_some() => s.step(Input::Tick, Now::real()),
            Some(n) = news_rx.recv() => {
                tap(&n, &mut zeroconf);
                s.step(Input::Log(n), Now::real())
            }
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { let _ = child.kill().await; return Ok(Exit::Quit) };
                s.step(Input::Command(cmd), Now::real())
            }
        };
        if let Some(stop) = carry_out(actions, &mut ws_tx, engine, ev_tx).await? {
            return stopped(child, stop).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unplayable_context_is_news() {
        let mut p = LogParser::default();
        assert_eq!(p.line("19:48:31.990 I [f:2197] AdvanceStuck detected, reason: unplayable"), Some(LogNews::Unplayable));
        assert_eq!(p.line("19:48:31.990 I [f:289] Prefetch: looks like new context"), None);
    }

    /// The lines that start and end a dry spell (2026-11-02: Soloist sat "playing" at 0:09 for minutes with no data).
    #[test]
    fn an_underrun_and_its_recovery_are_news() {
        let mut p = LogParser::default();
        let stall = p.line("01:41:05.291 I [f:776                           ] audioThreadMain: underrun, _is_processing_audio now false");
        assert_eq!(stall, Some(LogNews::Stall));
        let flowing = p.line("01:42:46.421 I [f:972                           ] updateInternalDriverToPlaying: underrun, _is_processing_audio now true");
        assert_eq!(flowing, Some(LogNews::Flowing));
    }

    /// S1: the state directory (pairing, cache) and both logs are owner-only, and an older build's 0644 log is
    /// tightened in place.
    #[test]
    fn the_state_directory_and_logs_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("soloist-state");
        std::fs::create_dir_all(state.join("cache")).unwrap();
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
        // an older build's log, loose
        std::fs::write(state.join("soloist.log"), "old").unwrap();
        std::fs::set_permissions(state.join("soloist.log"), std::fs::Permissions::from_mode(0o644)).unwrap();
        private_state_dirs(&state).unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&state), mode(&state.join("data")), mode(&state.join("cache"))), (0o700, 0o700, 0o700));
        drop(open_log(&state).unwrap());
        assert_eq!(mode(&state.join("soloist.log")), 0o600);
        assert_eq!(mode(&state.join("soloist.log.1")), 0o600, "the previous log is tightened too");
        assert_eq!(std::fs::read_to_string(state.join("soloist.log.1")).unwrap(), "old");
    }

    /// S1: Soloist's lines are third-party text (track names, another person's playlist): no control characters reach
    /// the log or a terminal showing it.
    #[test]
    fn log_lines_carry_no_control_characters() {
        let hostile = "\u{1b}]0;pwned\u{7}\u{1b}]52;c;ZXZpbA==\u{7}\u{1b}[2Jtrack\u{0}\tname";
        assert_eq!(strip_ansi(hostile), "trackname");
        // the parser still reads the lines it needs
        assert!(strip_ansi("Streaming: a\tb").contains("Streaming"));
    }

    #[test]
    fn restart_waits_grow_while_failing_and_start_over_after_a_healthy_run() {
        let mut backoff = Duration::from_secs(2);
        let quick = Duration::from_secs(1);
        let waits: Vec<u64> = (0..7).map(|_| restart_wait(&mut backoff, quick).as_secs()).collect();
        assert_eq!(waits, [2, 4, 8, 16, 32, 60, 60]);
        // days later it stops again: that isn't the eighth failure in a row
        assert_eq!(restart_wait(&mut backoff, Duration::from_secs(86_400)).as_secs(), 2);
        assert_eq!(restart_wait(&mut backoff, quick).as_secs(), 4);
    }
}
