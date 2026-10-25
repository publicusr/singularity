//! `singularityd`: owns playback (Engine), later the Catalogue session, the audio tap and local data.
//! Clients talk to it over the protocol in `singularity-proto` (ADR-0003).

pub mod app_engine;
pub mod librespot;
pub mod backup;
pub mod catalogue;
pub mod connect;
pub mod engine;
pub mod health;
pub mod library;
pub mod lyrics;
pub mod mpris;
pub mod nowplaying;
pub mod pcm;
#[cfg(target_os = "macos")]
pub mod process_tap;
pub mod playlists;
pub mod programs;
pub mod queries;
pub mod secrets;
pub mod soloist;
#[cfg(target_os = "macos")]
pub mod soloist_vm;
pub mod store;
pub mod tap;
pub mod update;
#[cfg(target_os = "macos")]
pub mod vm;

use catalogue::{CatCmd, CatEvent};
use engine::{Engine, EngineEvent};
use futures_util::FutureExt;
use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg, Event, PROTOCOL_VERSION, PlaybackState};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, broadcast, mpsc};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How to restart the Daemon by hand, as the OS's service manager says it (packaging/systemd, packaging/launchd).
pub const RESTART: &str = if cfg!(target_os = "macos") {
    "`launchctl kickstart -k gui/$(id -u)/io.github.publicusr.singularityd`, or quit it: the next Client starts it"
} else {
    "`systemctl --user restart singularityd`"
};

/// Shared Daemon state that every connection reads.
pub struct Hub {
    pub started_unix_ms: u64,
    /// which Engine this Daemon runs
    pub engine_kind: proto::EngineKind,
    pub playback: Mutex<PlaybackState>,
    pub engine_status: Mutex<Option<proto::EngineStatus>>,
    pub catalogue_status: Mutex<Option<proto::CatalogueStatus>>,
    pub home: Mutex<Option<proto::HomeFeed>>,
    pub library: Mutex<Option<proto::Library>>,
    pub stream: Mutex<Option<proto::StreamInfo>>,
    /// the Engine's download speed and stalls (#45)
    pub delivery: Mutex<Option<proto::Delivery>>,
    /// what the Engine reports, and another device's playback while that one is active (then it's what Clients see)
    pub engine_playback: Mutex<PlaybackState>,
    pub remote: Mutex<Option<PlaybackState>>,
    pub devices: Mutex<Option<Vec<proto::Device>>>,
    /// track metadata the Catalogue filled in, for queue entries Soloist sent bare; and what's being asked for
    pub described: Mutex<std::collections::HashMap<String, proto::Item>>,
    pub describing: Mutex<std::collections::HashSet<String>>,
    pub events: broadcast::Sender<Event>,
    pub commands: mpsc::Sender<Command>,
    pub catalogue: Option<mpsc::Sender<CatCmd>>,
    /// AudioTap frames, and how many Clients want them (the tap runs only while > 0)
    pub spectrum: broadcast::Sender<proto::SpectrumFrame>,
    pub spectrum_listeners: std::sync::atomic::AtomicUsize,
    pub spectrum_wake: tokio::sync::Notify,
    /// a Client asked this Daemon to make way for a newer one
    pub shutdown: tokio::sync::Notify,
    /// the listening log and library backups (LocalStore); None: off (tests, fake Daemons)
    pub store: Option<Arc<std::sync::Mutex<store::Store>>>,
    /// background tasks that stopped and haven't run for a while: the health view names them (A1)
    pub stopped_tasks: Mutex<BTreeSet<String>>,
}

/// How long a background task has to run before it counts as back (its health row goes).
const TASK_HEALTHY: Duration = Duration::from_secs(60);
/// The first pause before a stopped background task is started again, and the longest one.
const TASK_BACKOFF_MIN: Duration = Duration::from_secs(2);
const TASK_BACKOFF_MAX: Duration = Duration::from_secs(300);

/// The next restart delay: doubling, up to `TASK_BACKOFF_MAX`.
fn next_backoff(backoff: &mut Duration) -> Duration {
    let wait = *backoff;
    *backoff = (*backoff * 2).min(TASK_BACKOFF_MAX);
    wait
}

impl Hub {
    /// Tell the Clients about a problem; a problem nobody hears about is at least in the log (Q11).
    pub fn report(&self, message: impl Into<String>) {
        let message = message.into();
        if self.events.send(Event::Error { message: message.clone() }).is_err() {
            tracing::debug!("nobody was told: {message}");
        }
    }

    /// A background task stopped: log it, name it in the health view, and tell the Clients once per outage (A1).
    pub async fn task_stopped(&self, name: &str, why: &str) {
        tracing::error!("{name} stopped: {why}");
        if self.stopped_tasks.lock().await.insert(name.to_string()) {
            self.report(format!("{name} stopped ({why}); the Daemon is starting it again"));
        }
    }

    /// A task that had stopped has run for a while: it's back, so its health row goes (A1).
    pub async fn task_running(&self, name: &str) {
        if self.stopped_tasks.lock().await.remove(name) {
            tracing::info!("{name} is running again");
        }
    }
}

/// What `serve` ended with.
#[derive(Debug, PartialEq, Eq)]
pub enum Served {
    /// asked to make way for a newer Daemon (exit non-zero, so systemd's `Restart=on-failure` starts the new one)
    Replaced,
}

/// A running Catalogue task (see `catalogue::spawn`); `None` disables it (tests, offline use).
pub type CatalogueHandle = (mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>);

/// Bind the socket (replacing a stale one), in a directory that is ours and owner-only.
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        socket_dir(dir)?;
    }
    if path.exists() {
        // A live Daemon answers; a dead one leaves a stale socket file behind.
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "another singularityd is already running"));
        }
        std::fs::remove_file(path)?;
    }
    UnixListener::bind(path)
}

/// The socket's directory: created owner-only from the start when it's missing, and never created or reached through
/// a symlink (A10/S5: a link anywhere on the way could put the socket where someone else chose). A directory that is
/// already there is accepted if it's ours, but its mode is left alone: the Daemon only tightens what it made itself.
fn socket_dir(dir: &Path) -> std::io::Result<()> {
    // before creating anything: creating directories follows a symlink, so it would end up behind one
    no_symlinks(dir)?;
    if !dir.exists() {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(dir)?;
    }
    private_dir(dir)
}

/// Every component of the way to `dir` that exists must be a real directory: a symlink anywhere could put the
/// socket (and its directory) where someone else chose.
fn no_symlinks(dir: &Path) -> std::io::Result<()> {
    let mut cur = PathBuf::new();
    for part in dir.components() {
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{} is a symlink: refusing to put the socket there", cur.display())));
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{} isn't a directory: refusing to put the socket there", cur.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The socket's directory must be ours, and so must the one holding it (or root's): in a shared directory like
/// `/tmp`, another user who made either first could swap in a socket of their own.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let me = unsafe { libc::getuid() }; // SAFETY: getuid has no preconditions and can't fail
    let m = std::fs::symlink_metadata(dir)?;
    let parent_ok = dir.parent().and_then(|p| std::fs::metadata(p).ok()).is_none_or(|p| p.uid() == me || p.uid() == 0);
    if !m.is_dir() || m.uid() != me || !parent_ok {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{} (or the directory holding it) isn't yours: refusing to put the socket there", dir.display())));
    }
    Ok(())
}

/// Only our own user's processes may talk to the Daemon (the directory's mode says so too; this doesn't rely on it).
fn same_user(stream: &UnixStream) -> bool {
    let me = unsafe { libc::getuid() }; // SAFETY: as above
    stream.peer_cred().is_ok_and(|c| c.uid() == me)
}

/// What a Daemon runs beyond playback and the Catalogue.
#[derive(Default)]
pub struct Extras {
    /// MPRIS (and notifications) on a D-Bus
    pub mpris: Option<mpris::Options>,
    /// Now Playing, media keys and notifications on macOS (#68)
    pub now_playing: Option<nowplaying::Options>,
    /// the LocalStore's directory: the listening log and nightly library backups
    pub store: Option<PathBuf>,
    /// the desktop-notification flag a background task keeps following `config.toml` (`notifications.track_change`)
    pub notify: Option<Arc<std::sync::atomic::AtomicBool>>,
}

/// Run the Daemon on `listener` with `engine` until the process ends.
pub async fn serve(listener: UnixListener, engine: Box<dyn Engine>, catalogue: Option<CatalogueHandle>, tap: Option<tap::TapSource>) -> anyhow::Result<Served> {
    serve_with(listener, engine, catalogue, tap, Extras::default()).await
}

/// `serve`, plus the `extras` asked for.
pub async fn serve_with(listener: UnixListener, engine: Box<dyn Engine>, catalogue: Option<CatalogueHandle>, tap: Option<tap::TapSource>, extras: Extras) -> anyhow::Result<Served> {
    let store = match extras.store.as_deref().map(store::Store::open) {
        Some(Ok(s)) => Some(Arc::new(std::sync::Mutex::new(s))),
        Some(Err(e)) => {
            tracing::warn!("the listening log is unavailable: {e}");
            None
        }
        None => None,
    };
    let (events, _) = broadcast::channel(256);
    let (commands, command_rx) = mpsc::channel(64);
    let (cat_tx, cat_rx) = match catalogue {
        Some((tx, rx)) => (Some(tx), Some(rx)),
        None => (None, None),
    };
    let hub = Arc::new(Hub {
        started_unix_ms: proto::now_ms(),
        engine_kind: engine.kind(),
        playback: Mutex::new(PlaybackState::default()),
        engine_status: Mutex::new(None),
        catalogue_status: Mutex::new(None),
        home: Mutex::new(None),
        library: Mutex::new(None),
        stream: Mutex::new(None),
        delivery: Mutex::new(None),
        engine_playback: Mutex::new(PlaybackState::default()),
        remote: Mutex::new(None),
        devices: Mutex::new(None),
        described: Default::default(),
        describing: Default::default(),
        events,
        commands,
        catalogue: cat_tx,
        spectrum: broadcast::channel(8).0,
        spectrum_listeners: Default::default(),
        spectrum_wake: Default::default(),
        shutdown: Default::default(),
        store,
        stopped_tasks: Default::default(),
    });
    if hub.store.is_some() {
        let h = hub.clone();
        supervise(hub.clone(), "the listening log", move || log_plays(h.clone()));
        // nothing to back up without the Catalogue
        if hub.catalogue.is_some() {
            let h = hub.clone();
            supervise(hub.clone(), "library backups", move || backup::run(h.clone()));
        }
    }
    if let Some(source) = tap {
        let h = hub.clone();
        supervise(hub.clone(), "the audio tap", move || tap::run(h.clone(), tap_again(&source)));
    }
    if let Some(notify) = extras.notify.clone() {
        supervise(hub.clone(), "the config.toml follower", move || follow_config(notify.clone()));
    }
    tokio::spawn(run_engine(engine, hub.clone(), command_rx));
    if let Some(rx) = cat_rx {
        tokio::spawn(run_catalogue(rx, hub.clone()));
    }
    if let Some(opts) = extras.mpris {
        let h = hub.clone();
        let again = opts.clone();
        supervise(hub.clone(), "MPRIS", move || mpris::run(h.clone(), again.clone()));
    }
    #[cfg(target_os = "macos")]
    match extras.now_playing {
        Some(opts) => {
            let h = hub.clone();
            let notify = opts.notify.clone();
            supervise(hub.clone(), "Now Playing", move || {
                let (h, notify) = (h.clone(), notify.clone());
                nowplaying::run(h, nowplaying::Options { notify })
            });
        }
        None => nowplaying::not_wanted(),
    }
    loop {
        let stream = tokio::select! {
            r = listener.accept() => r?.0,
            _ = hub.shutdown.notified() => {
                tracing::info!("a newer Client asked this Daemon to make way");
                return Ok(Served::Replaced);
            }
        };
        if !same_user(&stream) {
            tracing::warn!("refused a connection from another user");
            continue;
        }
        let hub = hub.clone();
        tokio::spawn(async move {
            if let Err(e) = client_session(stream, hub).await {
                tracing::debug!("client session ended: {e}");
            }
        });
    }
}

/// Run a background task, and keep it running: when it ends or panics, say so (log, Clients, the health view) and
/// start it again after a growing pause (A1). `start` builds a fresh future each time: a restarted task keeps none of
/// the old one's state.
fn supervise<F, Fut>(hub: Arc<Hub>, name: &'static str, mut start: F) -> tokio::task::JoinHandle<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        let mut backoff = TASK_BACKOFF_MIN;
        loop {
            let began = std::time::Instant::now();
            // the health row goes once the task has run for a while (a task that keeps dying keeps its row)
            let back = back_soon(hub.clone(), name);
            let done = std::panic::AssertUnwindSafe(start()).catch_unwind().await;
            back.abort();
            if began.elapsed() >= TASK_HEALTHY {
                backoff = TASK_BACKOFF_MIN;
            }
            hub.task_stopped(name, if done.is_err() { "it panicked" } else { "its task ended" }).await;
            tokio::time::sleep(next_backoff(&mut backoff)).await;
        }
    })
}

/// Drop a task's health row once it has been running for `TASK_HEALTHY`.
fn back_soon(hub: Arc<Hub>, name: &'static str) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(TASK_HEALTHY).await;
        hub.task_running(name).await;
    })
}

/// The tap's source for a restarted task: the Engine's `Feed` must stay the same one, or the tap hears nothing.
fn tap_again(source: &tap::TapSource) -> tap::TapSource {
    match source {
        tap::TapSource::PipeWire(of) => tap::TapSource::PipeWire(*of),
        tap::TapSource::Feed(feed) => tap::TapSource::Feed(feed.clone()),
        #[cfg(target_os = "macos")]
        tap::TapSource::App(feed) => tap::TapSource::App(feed.clone()),
        tap::TapSource::Synthetic => tap::TapSource::Synthetic,
    }
}

/// Follow `config.toml` (`notifications.track_change`) as it's saved, so a Settings change applies without a restart.
async fn follow_config(notify: Arc<std::sync::atomic::AtomicBool>) {
    let path = singularity_config::config_path();
    let mut watched = singularity_config::Watched::new(path.clone());
    let mut first = true;
    loop {
        if first || watched.changed() {
            first = false;
            let r = singularity_config::load_config(&path);
            for p in &r.problems {
                tracing::warn!("{p}");
            }
            notify.store(r.value.notifications.track_change, std::sync::atomic::Ordering::Relaxed);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Engine events update the shared state and fan out to Clients; Client commands go to the Engine.
async fn run_engine(engine: Box<dyn Engine>, hub: Arc<Hub>, mut commands: mpsc::Receiver<Command>) {
    // A fresh Engine for after this one's task ends. A stand-in that can't make one (a test) is only reported.
    let mut spare = engine.respawn();
    let mut up = Some(engine.spawn());
    let mut backoff = TASK_BACKOFF_MIN;
    let mut restart_at: Option<tokio::time::Instant> = None;
    loop {
        let Some((engine_tx, mut events)) = up.take() else {
            // down: commands fail loudly, and the next Engine starts when the pause is over (A1)
            let Some(at) = restart_at else {
                while commands.recv().await.is_some() {
                    hub.report("the player has stopped; that command wasn't delivered");
                }
                return;
            };
            tokio::select! {
                _ = tokio::time::sleep_until(at) => match spare.take() {
                    Some(next) => {
                        // another spare: an Engine that dies twice is started again as well
                        spare = next.respawn();
                        up = Some(next.spawn());
                        restart_at = None;
                        back_soon(hub.clone(), "the player");
                    }
                    None => {
                        hub.report("the player has stopped, and this Daemon can't start it again");
                        restart_at = None;
                    }
                },
                cmd = commands.recv() => {
                    if cmd.is_none() {
                        return;
                    }
                    hub.report("the player has stopped; that command wasn't delivered");
                }
            }
            continue;
        };
        let began = tokio::time::Instant::now();
        let died = tokio::select! {
            ev = events.recv() => match ev {
                Some(ev) => {
                    on_engine_event(&hub, ev, &engine_tx).await;
                    false
                }
                None => true,
            },
            cmd = commands.recv() => match cmd {
                Some(cmd) => {
                    // a failed send means the Engine's task is gone; the event channel closes too, so the next
                    // round reports it
                    let _ = engine_tx.send(cmd).await;
                    false
                }
                None => return,
            },
        };
        if died {
            // the Engine's task ended (a panic, or one of its own error paths): say so, then start it again
            if began.elapsed() >= TASK_HEALTHY {
                backoff = TASK_BACKOFF_MIN;
            }
            let message = "the player stopped; the Daemon is starting it again".to_string();
            *hub.engine_status.lock().await = Some(proto::EngineStatus::Failed { message: message.clone() });
            let _ = hub.events.send(Event::Engine(proto::EngineStatus::Failed { message }));
            hub.task_stopped("the player", "its task ended").await;
            restart_at = Some(tokio::time::Instant::now() + next_backoff(&mut backoff));
            // the engine's channels went with it
        } else {
            up = Some((engine_tx, events));
        }
    }
}

/// What the Engine reports (Soloist itself, or the Catalogue for the Spotify app, D51), into the shared state.
async fn on_engine_event(hub: &Arc<Hub>, ev: EngineEvent, engine_tx: &mpsc::Sender<Command>) {
    match ev {
        EngineEvent::Playback(mut state) => {
            let missing = fill_queue(hub, &mut state).await;
            if !missing.is_empty()
                && let Some(c) = &hub.catalogue
                && c.send(CatCmd::Describe(missing)).await.is_err()
            {
                hub.report("the Catalogue has stopped; those tracks won't be named");
            }
            *hub.engine_playback.lock().await = state.clone();
            // while another device plays, Clients follow that one
            if hub.remote.lock().await.is_none() {
                *hub.playback.lock().await = state.clone();
                let _ = hub.events.send(Event::Playback(state));
            }
        }
        EngineEvent::Status(status) => {
            // (the app's status comes with every Connect update: Clients hear only changes)
            let mut current = hub.engine_status.lock().await;
            if current.as_ref() != Some(&status) {
                *current = Some(status.clone());
                let _ = hub.events.send(Event::Engine(status));
            }
        }
        EngineEvent::Stream(info) => {
            let mut current = hub.stream.lock().await;
            if current.as_ref() != Some(&info) {
                *current = Some(info.clone());
                let _ = hub.events.send(Event::Stream(info));
            }
        }
        EngineEvent::Delivery(d) => {
            *hub.delivery.lock().await = Some(d.clone());
            let _ = hub.events.send(Event::Delivery(d));
        }
        EngineEvent::Error(message) => hub.report(message),
        EngineEvent::Notice(message) => {
            let _ = hub.events.send(Event::Notice { message });
        }
        // a task inside the Engine stopped (or came back): the health view follows it (A1)
        EngineEvent::TaskStopped(name, why) => hub.task_stopped(name, &why).await,
        EngineEvent::TaskStarted(name) => hub.task_running(name).await,
        // the Engine restarted for an update: the context goes back on through Connect (position, paused),
        // else the track alone through the Engine
        EngineEvent::Resume(r) => match &hub.catalogue {
            Some(c) => {
                if c.send(CatCmd::Resume(r)).await.is_err() {
                    hub.report("the Catalogue has stopped; playback wasn't picked up again");
                }
            }
            None => {
                engine_cmd(hub, engine_tx, Command::Play { uri: Some(r.track_uri) }).await;
                engine_cmd(hub, engine_tx, Command::Seek { position_ms: r.position_ms }).await;
                if r.paused {
                    engine_cmd(hub, engine_tx, Command::Pause).await;
                }
            }
        },
    }
}

/// A command straight to the Engine's channel; a dead Engine is reported, not swallowed (A1).
async fn engine_cmd(hub: &Hub, engine_tx: &mpsc::Sender<Command>, cmd: Command) {
    if engine_tx.send(cmd).await.is_err() {
        hub.report("the player has stopped; that command wasn't delivered");
    }
}

/// The listening log: plays as the Clients see them (this computer's Engine, or the Connect device being followed).
async fn log_plays(hub: Arc<Hub>) {
    let Some(store) = hub.store.clone() else { return };
    let mut events = hub.events.subscribe();
    let mut rec = store::Recorder::default();
    loop {
        let state = match events.recv().await {
            Ok(Event::Playback(s)) => s,
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => return,
        };
        let device = device_name(&hub).await;
        let now = proto::now_ms();
        if let Some(play) = rec.observe(&state, &device, now) {
            let summary = {
                let s = store.lock().expect("store lock");
                if let Err(e) = s.record(&play) {
                    tracing::warn!("couldn't log a play: {e}");
                }
                s.summary(now)
            };
            if let Ok(summary) = summary {
                let _ = hub.events.send(Event::Listening(summary));
            }
        }
    }
}

/// Where playback is happening, for the listening log.
async fn device_name(hub: &Hub) -> String {
    if hub.remote.lock().await.is_some()
        && let Some(d) = hub.devices.lock().await.iter().flatten().find(|d| d.active && !d.is_engine)
    {
        return d.name.clone();
    }
    match hub.engine_status.lock().await.clone() {
        Some(proto::EngineStatus::Ready { device_name, .. }) => device_name,
        _ => "this computer".into(),
    }
}

/// Answer a LocalStore command (`GetListening`, `GetStats`, `ExportLocalData`, `DeleteLocalData`).
async fn local_data(hub: &Hub, cmd: Command, out: &mpsc::Sender<DaemonMsg>) {
    let Some(store) = hub.store.clone() else {
        // questions go unanswered (Widgets keep waiting quietly); asking to export or delete is told
        if matches!(cmd, Command::ExportLocalData | Command::DeleteLocalData) {
            let _ = out.send(DaemonMsg::Event(Event::Error { message: "the listening log is off in this Daemon".into() })).await;
        }
        return;
    };
    let now = proto::now_ms();
    let (reply, everyone) = {
        let s = store.lock().expect("store lock");
        match cmd {
            Command::GetListening => (s.summary(now).map(Event::Listening), None),
            Command::GetStats { period } => {
                // plays logged before links were kept get them from the Catalogue, a few at a time (see Described)
                if let Some(c) = &hub.catalogue
                    && let Ok(uris) = s.unlinked_tracks(20)
                    && !uris.is_empty()
                {
                    let _ = c.try_send(CatCmd::Describe(uris));
                }
                (s.stats(period, now).map(Event::Stats), None)
            }
            Command::ExportLocalData => (s.export(&store::downloads_dir(), now).map(|p| Event::Notice { message: format!("Listening data exported to {}", p.display()) }), None),
            _ => match s.delete_all() {
                Ok(()) => (Ok(Event::Notice { message: "Deleted the listening log and every library backup".into() }), s.summary(now).ok().map(Event::Listening)),
                Err(e) => (Err(e), None),
            },
        }
    };
    let ev = reply.unwrap_or_else(|e| Event::Error { message: format!("Local data: {e}") });
    let _ = out.send(DaemonMsg::Event(ev)).await;
    if let Some(ev) = everyone {
        let _ = hub.events.send(ev);
    }
}

/// Fill bare queue entries from the Catalogue's answers; returns the URIs still unknown (and not yet asked for).
async fn fill_queue(hub: &Hub, state: &mut PlaybackState) -> Vec<String> {
    let described = hub.described.lock().await;
    let mut asking = hub.describing.lock().await;
    let mut missing = vec![];
    let bare = |i: &proto::Item| (i.name.is_empty() || i.artists.is_empty()) && i.uri.starts_with("spotify:track:");
    for item in state.item.iter_mut().chain(state.queue.iter_mut().map(|q| &mut q.item)).filter(|i| bare(i)) {
        match described.get(&item.uri) {
            Some(i) => *item = i.clone(),
            // (A8/S8: the "being asked" set is bounded like the Catalogue's caches; past that, rows stay bare)
            None if asking.len() < catalogue::MAX_DESCRIBED && asking.insert(item.uri.clone()) => missing.push(item.uri.clone()),
            None => {}
        }
    }
    missing
}

async fn run_catalogue(mut events: mpsc::Receiver<CatEvent>, hub: Arc<Hub>) {
    while let Some(ev) = events.recv().await {
        match ev {
            CatEvent::Described(items) => {
                // old plays of these tracks get their links; Clients showing stats then ask again
                if let Some(store) = hub.store.clone() {
                    let summary = {
                        let s = store.lock().expect("store lock");
                        s.backfill(&items).ok().filter(|n| *n > 0).and_then(|_| s.summary(proto::now_ms()).ok())
                    };
                    if let Some(summary) = summary {
                        let _ = hub.events.send(Event::Listening(summary));
                    }
                }
                {
                    let mut d = hub.described.lock().await;
                    let mut asking = hub.describing.lock().await;
                    // A8/S8: the Hub's cache is bounded like the Catalogue's; what falls out can be asked for again
                    for i in items {
                        asking.remove(&i.uri);
                        catalogue::trim_any(&mut d, catalogue::MAX_DESCRIBED);
                        d.insert(i.uri.clone(), i);
                    }
                }
                // show them at once
                let mut state = hub.playback.lock().await.clone();
                fill_queue(&hub, &mut state).await;
                *hub.playback.lock().await = state.clone();
                if let Some(r) = hub.remote.lock().await.as_mut() {
                    *r = state.clone();
                }
                let _ = hub.events.send(Event::Playback(state));
            }
            CatEvent::Saved(entries) => {
                let _ = hub.events.send(Event::Saved { entries });
            }
            CatEvent::Created { uri, name } => {
                let _ = hub.events.send(Event::PlaylistCreated { uri, name });
            }
            CatEvent::Added { playlist, name, uris, outcome } => {
                let _ = hub.events.send(Event::AddedToPlaylist { playlist, name, uris, outcome });
            }
            CatEvent::CoverSet { playlist, ok } => {
                let _ = hub.events.send(Event::PlaylistCoverSet { playlist, ok });
            }
            CatEvent::Moved { playlist, uids, ok } => {
                let _ = hub.events.send(Event::MovedInPlaylist { playlist, uids, ok });
            }
            CatEvent::Removed { playlist, uids, ok } => {
                let _ = hub.events.send(Event::RemovedFromPlaylist { playlist, uids, ok });
            }
            CatEvent::Edited { uri, name, description, ok } => {
                let _ = hub.events.send(Event::PlaylistEdited { uri, name, description, ok });
            }
            CatEvent::Devices(d) => {
                *hub.devices.lock().await = Some(d.clone());
                let _ = hub.events.send(Event::Devices { devices: d });
            }
            CatEvent::Remote(r) => {
                let state = match r {
                    Some(mut s) => {
                        let missing = fill_queue(&hub, &mut s).await;
                        if !missing.is_empty()
                            && let Some(c) = &hub.catalogue
                            && c.send(CatCmd::Describe(missing)).await.is_err()
                        {
                            hub.report("the Catalogue has stopped; those tracks won't be named");
                        }
                        *hub.remote.lock().await = Some(s.clone());
                        s
                    }
                    None => {
                        let was_remote = hub.remote.lock().await.take().is_some();
                        if !was_remote {
                            continue;
                        }
                        hub.engine_playback.lock().await.clone()
                    }
                };
                *hub.playback.lock().await = state.clone();
                let _ = hub.events.send(Event::Playback(state));
            }
            CatEvent::Status(s) => {
                // librespot plays as the Catalogue's login: waiting for one, it starts as soon as there is one
                let waiting = matches!(*hub.engine_status.lock().await, Some(proto::EngineStatus::NotConfigured { .. }));
                if hub.engine_kind == proto::EngineKind::Librespot && waiting && matches!(s, proto::CatalogueStatus::Ready { .. }) {
                    to_player(&hub, Command::RestartEngine).await;
                }
                *hub.catalogue_status.lock().await = Some(s.clone());
                let _ = hub.events.send(Event::Catalogue(s));
            }
            CatEvent::Home(h) => {
                *hub.home.lock().await = Some(h.clone());
                let _ = hub.events.send(Event::Home(h));
            }
            CatEvent::Library(l) => {
                *hub.library.lock().await = Some(l.clone());
                let _ = hub.events.send(Event::Library(l));
            }
            CatEvent::Search(r) => {
                let _ = hub.events.send(Event::Search(r));
            }
            CatEvent::Artist(p) => {
                let _ = hub.events.send(Event::Artist(p));
            }
            CatEvent::Lyrics(l) => {
                let _ = hub.events.send(Event::Lyrics(l));
            }
            CatEvent::Tracks(t) => {
                let _ = hub.events.send(Event::Tracks(t));
            }
            CatEvent::EpisodeShow { episode, show } => {
                let _ = hub.events.send(Event::EpisodeShow { episode, show });
            }
            CatEvent::Engine(e) => on_engine_event(&hub, e, &hub.commands).await,
            CatEvent::ToEngine(cmd) => to_player(&hub, cmd).await,
            CatEvent::Error(message) => hub.report(message),
            // tasks inside the Catalogue report their own deaths; the health view names them (A1)
            CatEvent::TaskStopped(name, why) => hub.task_stopped(name, &why).await,
            CatEvent::TaskStarted(name) => hub.task_running(name).await,
            // these couldn't be looked up: clear the marks so a later playback state asks again (A8; the Catalogue
            // keeps them from being asked about too often)
            CatEvent::DescribeFailed(uris) => {
                let mut asking = hub.describing.lock().await;
                for uri in uris {
                    asking.remove(&uri);
                }
            }
            // a change the Clients showed early didn't happen: the last real state puts it back
            CatEvent::Resync => {
                let state = hub.playback.lock().await.clone();
                let _ = hub.events.send(Event::Playback(state));
            }
        }
    }
}

/// Is this playback event older than the snapshot just sent? Such an event is already in the snapshot, so
/// forwarding it would put a freshly connected Client back in time (B6/C12). A state without an anchor is always
/// forwarded: it carries no time to compare.
fn superseded(state: &PlaybackState, snapshot_at: u64) -> bool {
    state.timestamp_ms > 0 && state.timestamp_ms < snapshot_at
}

/// The Catalogue command a Client command asks for, if the Catalogue is the one who answers it (Q3: this used to be
/// two hand-kept lists, with everything unmatched silently answering by fetching Home). `Err`: the command itself is
/// unusable (a cover that isn't base64 JPEG); `None`: a player command.
fn catalogue_cmd(cmd: &Command) -> Option<Result<CatCmd, &'static str>> {
    let c = match cmd {
        Command::Login => CatCmd::Login,
        Command::Logout => CatCmd::Logout,
        Command::GetHome => CatCmd::Home,
        Command::GetLibrary => CatCmd::Library,
        Command::GetTracks { uri } => CatCmd::Tracks(uri.clone()),
        Command::MoreTracks { uri, offset } => CatCmd::MoreTracks { uri: uri.clone(), offset: *offset },
        Command::FindShow { uri } => CatCmd::FindShow(uri.clone()),
        Command::PlayFrom { context_uri, index, track_uri } => CatCmd::PlayFrom { context_uri: context_uri.clone(), index: *index, track_uri: track_uri.clone() },
        Command::PlayNext { uri } => CatCmd::Queue(connect::QueueEdit::PlayNext(uri.clone())),
        Command::RemoveFromQueue { uri, uid } => CatCmd::Queue(connect::QueueEdit::Remove { uri: uri.clone(), uid: uid.clone() }),
        Command::ClearQueue => CatCmd::Queue(connect::QueueEdit::Clear),
        Command::MoveInQueue { uri, uid, to, into_queue } => CatCmd::Queue(connect::QueueEdit::Move { uri: uri.clone(), uid: uid.clone(), to: *to, into_queue: *into_queue }),
        Command::GetArtist { uri } => CatCmd::Artist(uri.clone()),
        Command::Search { query } => CatCmd::Search(query.clone()),
        Command::GetDevices => CatCmd::Devices,
        Command::Transfer { device_id } => CatCmd::Transfer(device_id.clone()),
        Command::SetSaved { uri, saved } => CatCmd::SetSaved { uri: uri.clone(), saved: *saved },
        Command::CheckSaved { uris } => CatCmd::CheckSaved(uris.clone()),
        Command::CreatePlaylist { name, uris } => CatCmd::CreatePlaylist { name: name.clone(), uris: uris.clone() },
        Command::AddToPlaylist { playlist, name, uris, anyway } => CatCmd::AddToPlaylist { playlist: playlist.clone(), name: name.clone(), uris: uris.clone(), anyway: *anyway },
        Command::RemoveFromPlaylist { playlist, uids } => CatCmd::RemoveFromPlaylist { playlist: playlist.clone(), uids: uids.clone() },
        Command::MoveInPlaylist { playlist, uids, before, after } => CatCmd::MoveInPlaylist { playlist: playlist.clone(), uids: uids.clone(), before: before.clone(), after: after.clone() },
        Command::SetPlaylistCover { playlist, jpeg } => {
            let jpeg = match jpeg.as_ref().map(|j| crate::playlists::base64_decode(j)) {
                Some(None) => return Some(Err("That cover wasn't readable")),
                jpeg => jpeg.flatten(),
            };
            CatCmd::SetCover { playlist: playlist.clone(), jpeg }
        }
        Command::DeletePlaylist { uri } => CatCmd::DeletePlaylist(uri.clone()),
        Command::EditPlaylist { uri, name, description } => CatCmd::EditPlaylist { uri: uri.clone(), name: name.clone(), description: description.clone() },
        _ => return None,
    };
    Some(Ok(c))
}

async fn client_session(stream: UnixStream, hub: Arc<Hub>) -> std::io::Result<()> {
    let (mut rd, mut wr) = stream.into_split();
    let hello: Option<ClientMsg> = proto::read_msg(&mut rd).await?;
    match hello {
        Some(ClientMsg::Hello { protocol, client }) if protocol == PROTOCOL_VERSION => {
            tracing::info!("client connected: {client}");
            proto::write_msg(&mut wr, &DaemonMsg::Welcome { protocol: PROTOCOL_VERSION, daemon_version: VERSION.into(), started_unix_ms: hub.started_unix_ms, engine: hub.engine_kind })
                .await?;
        }
        Some(ClientMsg::Hello { protocol, .. }) => {
            let message = format!("this singularityd speaks protocol {PROTOCOL_VERSION}, the Client speaks {protocol}. Restart the Daemon ({RESTART}) after updating.");
            proto::write_msg(&mut wr, &DaemonMsg::Incompatible { protocol: PROTOCOL_VERSION, message }).await?;
            return Ok(());
        }
        _ => return Ok(()),
    }
    // everything to this Client goes through one writer; a Client that takes nothing for WRITE_DEADLINE has stopped
    // reading, and its session ends (#71: it used to stay, blocked, for as long as it was connected)
    let (out_tx, mut out_rx) = mpsc::channel::<DaemonMsg>(64);
    let mut writer = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            match tokio::time::timeout(WRITE_DEADLINE, proto::write_msg(&mut wr, &m)).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => break,
                Err(_) => {
                    tracing::warn!("a Client stopped reading; closing its session");
                    break;
                }
            }
        }
    });
    // Subscribe before the snapshot is built (B6/C12): a change made while it's read is then queued here rather than
    // in neither the snapshot nor the subscription. Everything queued before the snapshot is old news, except a
    // playback state that's newer than the snapshot's anchor, which is forwarded below.
    let mut events = hub.events.subscribe();
    let mut snapshot = vec![];
    if let Some(st) = hub.engine_status.lock().await.clone() {
        snapshot.push(Event::Engine(st));
    }
    let playback = hub.playback.lock().await.clone();
    let mut newest = playback.timestamp_ms;
    snapshot.push(Event::Playback(playback));
    snapshot.extend(hub.catalogue_status.lock().await.clone().map(Event::Catalogue));
    snapshot.extend(hub.home.lock().await.clone().map(Event::Home));
    snapshot.extend(hub.library.lock().await.clone().map(Event::Library));
    snapshot.extend(hub.devices.lock().await.clone().map(|devices| Event::Devices { devices }));
    snapshot.extend(hub.stream.lock().await.clone().map(Event::Stream));
    snapshot.extend(hub.delivery.lock().await.clone().map(Event::Delivery));
    for ev in snapshot {
        let _ = out_tx.send(DaemonMsg::Event(ev)).await;
    }

    let fwd_tx = out_tx.clone();
    let forwarder = tokio::spawn(async move {
        loop {
            let ev = match events.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            if let Event::Playback(p) = &ev {
                // older than what the snapshot already showed: it would put the Client back in time (B6)
                if superseded(p, newest) {
                    continue;
                }
                newest = newest.max(p.timestamp_ms);
            }
            if fwd_tx.send(DaemonMsg::Event(ev)).await.is_err() {
                break;
            }
        }
    });
    // this Client's spectrum subscription: a forwarder task while it's on
    let mut spectrum: Option<tokio::task::JoinHandle<()>> = None;
    let reading = async {
        while let Some(msg) = proto::read_msg::<_, ClientMsg>(&mut rd).await? {
            match msg {
                ClientMsg::Command(Command::Spectrum { on }) => {
                    use std::sync::atomic::Ordering;
                    if on && spectrum.is_none() {
                        let mut frames = hub.spectrum.subscribe();
                        let tx = out_tx.clone();
                        spectrum = Some(tokio::spawn(async move {
                            loop {
                                match frames.recv().await {
                                    // frames are disposable: a slow Client skips some rather than queueing them
                                    Ok(f) => {
                                        if let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(DaemonMsg::Event(Event::Spectrum(f))) {
                                            break;
                                        }
                                    }
                                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                    Err(_) => break,
                                }
                            }
                        }));
                        hub.spectrum_listeners.fetch_add(1, Ordering::SeqCst);
                        hub.spectrum_wake.notify_waiters();
                    } else if !on && let Some(t) = spectrum.take() {
                        t.abort();
                        hub.spectrum_listeners.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                ClientMsg::Ping { id } => {
                    let _ = out_tx.send(DaemonMsg::Pong { id }).await;
                }
                // (SINGULARITY_IGNORE_SHUTDOWN: tests play a Daemon too old to know `Shutdown`)
                ClientMsg::Command(Command::Shutdown) if std::env::var_os("SINGULARITY_IGNORE_SHUTDOWN").is_none() => hub.shutdown.notify_one(),
                ClientMsg::Command(Command::Shutdown) => {}
                ClientMsg::Command(Command::GetLyrics { uri }) => match &hub.catalogue {
                    Some(c) => {
                        // what we know about the track helps the fallback search
                        let pb = hub.playback.lock().await.clone();
                        let meta = pb.item.into_iter().chain(pb.queue.into_iter().map(|q| q.item)).find(|i| i.uri == uri);
                        if c.send(CatCmd::Lyrics { uri, meta }).await.is_err() {
                            let _ = out_tx.send(DaemonMsg::Event(Event::Error { message: "the Catalogue has stopped; no lyrics".into() })).await;
                        }
                    }
                    None => {
                        let _ = out_tx.send(DaemonMsg::Event(Event::Lyrics(proto::Lyrics { uri, missing: "no lyrics: the Daemon runs without the Catalogue".into(), ..Default::default() }))).await;
                    }
                },
                ClientMsg::Command(Command::GetHealth) => {
                    let expires = match hub.engine_status.lock().await.clone() {
                        Some(proto::EngineStatus::Ready { expires_in_days, .. }) => expires_in_days,
                        _ => None,
                    };
                    let kind = hub.engine_kind;
                    let mut h = tokio::task::spawn_blocking(move || health::report(kind)).await.unwrap_or_default();
                    h.engine_expires_in_days = expires;
                    // background tasks that stopped and haven't run for a while (A1)
                    h.stopped_tasks = hub.stopped_tasks.lock().await.iter().cloned().collect();
                    let _ = out_tx.send(DaemonMsg::Event(Event::Health(h))).await;
                }
                ClientMsg::Command(cmd @ (Command::GetListening | Command::GetStats { .. } | Command::ExportLocalData | Command::DeleteLocalData)) => local_data(&hub, cmd, &out_tx).await,
                ClientMsg::Command(Command::GetState) => {
                    let s = hub.playback.lock().await.clone();
                    let _ = out_tx.send(DaemonMsg::Event(Event::Playback(s))).await;
                }
                // one list of what goes to the Catalogue (Q3): a command can't silently fall through to Home
                ClientMsg::Command(cmd) => match catalogue_cmd(&cmd) {
                    None => transport(&hub, cmd).await,
                    Some(Err(message)) => {
                        let _ = out_tx.send(DaemonMsg::Event(Event::Error { message: message.into() })).await;
                    }
                    Some(Ok(c)) => match &hub.catalogue {
                        Some(tx) => {
                            if tx.send(c).await.is_err() {
                                let _ = out_tx.send(DaemonMsg::Event(Event::Error { message: "the Catalogue has stopped; that did nothing".into() })).await;
                            }
                        }
                        // without a Catalogue the Engine still plays the track itself (and appends instead of "next")
                        None if matches!(cmd, Command::PlayFrom { .. } | Command::PlayNext { .. }) => to_player(&hub, cmd).await,
                        None => {
                            let _ = out_tx.send(DaemonMsg::Event(Event::Error { message: "the Catalogue is disabled in this Daemon".into() })).await;
                        }
                    },
                },
                ClientMsg::Hello { .. } | ClientMsg::Unknown => {}
            }
        }
        Ok::<(), std::io::Error>(())
    };
    // the session ends when the Client goes, or when its writer gave up on it
    let result = tokio::select! {
        r = reading => r,
        _ = &mut writer => Ok(()),
    };
    if let Some(t) = spectrum.take() {
        t.abort();
        hub.spectrum_listeners.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
    forwarder.abort();
    writer.abort();
    result
}

/// How long a Client may take to accept one message before its session is closed.
const WRITE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// A command for the player: to the Engine, or over Connect while another device plays.
pub(crate) async fn transport(hub: &Hub, cmd: Command) {
    let remote = hub.remote.lock().await.clone();
    match (remote, &hub.catalogue) {
        (Some(r), Some(c)) if is_transport(&cmd) => {
            let cmd = match cmd {
                Command::TogglePlay if r.status == proto::PlayStatus::Playing => Command::Pause,
                Command::TogglePlay => Command::Play { uri: None },
                other => other,
            };
            if c.send(CatCmd::Remote(cmd)).await.is_err() {
                hub.report("the Catalogue has stopped; that command wasn't delivered");
            }
        }
        _ => to_player(hub, cmd).await,
    }
}

/// A command for the Engine; one the Engine can't take is reported, never swallowed (A1).
async fn to_player(hub: &Hub, cmd: Command) {
    if hub.commands.send(cmd).await.is_err() {
        hub.report("the player has stopped; that command wasn't delivered");
    }
}


/// Commands that drive whichever device is playing.
fn is_transport(c: &Command) -> bool {
    matches!(
        c,
        Command::Play { .. } | Command::Pause | Command::TogglePlay | Command::Next | Command::Previous | Command::Seek { .. } | Command::SetVolume { .. } | Command::SetShuffle { .. } | Command::SetRepeat { .. }
    )
}

/// Default socket location (see `singularity_proto::socket_path`).
pub fn default_socket() -> PathBuf {
    proto::socket_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hub() -> Arc<Hub> {
        let (events, _) = broadcast::channel(8);
        let (commands, _) = mpsc::channel(4);
        Arc::new(Hub {
            started_unix_ms: 0,
            engine_kind: proto::EngineKind::Fake,
            playback: Mutex::new(PlaybackState::default()),
            engine_status: Mutex::new(None),
            catalogue_status: Mutex::new(None),
            home: Mutex::new(None),
            library: Mutex::new(None),
            stream: Mutex::new(None),
            delivery: Mutex::new(None),
            engine_playback: Mutex::new(PlaybackState::default()),
            remote: Mutex::new(None),
            devices: Mutex::new(None),
            described: Default::default(),
            describing: Default::default(),
            events,
            commands,
            catalogue: None,
            spectrum: broadcast::channel(1).0,
            spectrum_listeners: Default::default(),
            spectrum_wake: Default::default(),
            shutdown: Default::default(),
            store: None,
            stopped_tasks: Default::default(),
        })
    }

    /// Q3: every command the Catalogue answers has its own arm here, so a new one can't quietly answer by fetching
    /// Home (the old inner match ended in `_ => CatCmd::Home`).
    #[test]
    fn catalogue_commands_map_one_to_one() {
        assert!(matches!(catalogue_cmd(&Command::GetHome), Some(Ok(CatCmd::Home))));
        assert!(matches!(catalogue_cmd(&Command::GetLibrary), Some(Ok(CatCmd::Library))));
        assert!(matches!(catalogue_cmd(&Command::Search { query: "x".into() }), Some(Ok(CatCmd::Search(q))) if q == "x"));
        assert!(matches!(catalogue_cmd(&Command::PlayNext { uri: "spotify:track:x".into() }), Some(Ok(CatCmd::Queue(connect::QueueEdit::PlayNext(u)))) if u == "spotify:track:x"));
        assert!(matches!(catalogue_cmd(&Command::SetPlaylistCover { playlist: "p".into(), jpeg: Some("not base64!".into()) }), Some(Err(_))), "an unreadable cover is refused, not turned into a Home fetch");
        assert!(matches!(catalogue_cmd(&Command::SetPlaylistCover { playlist: "p".into(), jpeg: None }), Some(Ok(CatCmd::SetCover { jpeg: None, .. }))));
        // the player's own commands don't go to the Catalogue at all
        assert!(catalogue_cmd(&Command::Pause).is_none());
        assert!(catalogue_cmd(&Command::Seek { position_ms: 1 }).is_none());
    }

    /// B6/C12: a playback event that the snapshot already contains is not forwarded to the new Client.
    #[test]
    fn playback_older_than_the_snapshot_is_superseded() {
        let at = |ms| PlaybackState { timestamp_ms: ms, ..Default::default() };
        assert!(superseded(&at(500), 1_000), "queued before the snapshot: it's in it already");
        assert!(!superseded(&at(1_000), 1_000), "the same moment: a duplicate is harmless, and it may be the last word");
        assert!(!superseded(&at(1_500), 1_000), "newer than the snapshot: the Client has to see it");
        assert!(!superseded(&at(0), 1_000), "a state without an anchor carries no time to compare");
    }

    /// A1: a background task that panics is named in the health view and started again.
    #[tokio::test]
    async fn a_background_task_that_panics_is_restarted_and_named() {
        let hub = test_hub();
        let mut events = hub.events.subscribe();
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = runs.clone();
        let task = supervise(hub.clone(), "the doomed task", move || {
            let n = counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if n == 0 {
                    panic!("boom");
                }
                // the second run stays up, like the real tasks
                std::future::pending::<()>().await
            }
        });
        // the first run panics; the restart comes after the first backoff
        tokio::time::timeout(Duration::from_secs(10), async {
            while runs.load(std::sync::atomic::Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the task was never started again");
        // an outage is one health row, and one error to the Clients
        let mut told = false;
        while let Ok(ev) = events.try_recv() {
            told |= matches!(ev, Event::Error { .. });
        }
        assert!(told, "no Client was told the task had stopped");
        assert!(hub.stopped_tasks.lock().await.contains("the doomed task"), "the health view doesn't name it");
        // once it has run for a while the row goes (the timer `supervise` sets)
        hub.task_running("the doomed task").await;
        assert!(hub.stopped_tasks.lock().await.is_empty());
        task.abort();
    }
}
