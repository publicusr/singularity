//! `singularityd`: owns playback (Engine), later the Catalogue session, the audio tap and local data.
//! Clients talk to it over the protocol in `singularity-proto` (ADR-0003).

pub mod app_engine;
pub mod assembly;
pub mod librespot;
pub mod backup;
pub mod catalogue;
pub mod connect;
pub mod engine;
pub mod health;
pub mod jam;
pub mod last_seen;
pub mod library;
pub mod lyrics;
pub mod os_media;
pub mod pcm;
#[cfg(target_os = "macos")]
pub mod process_tap;
pub mod playlists;
pub mod programs;
pub mod pathfinder;
pub mod playing;
pub mod queries;
pub mod secrets;
pub mod soloist;
pub mod soloist_vm;
/// The Spotify app as this OS installs and starts it (a Platform seam; the AppEngine asks it)
#[cfg_attr(target_os = "macos", path = "app_engine/macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "app_engine/linux.rs")]
pub mod spotify_app;
pub mod store;
pub mod tap;
pub mod update;
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

/// The systemd user unit's name (packaging/systemd).
pub const SYSTEMD_UNIT: &str = "singularityd.service";

/// The LaunchAgent's label (packaging/launchd/singularityd.plist).
pub const LAUNCHD_LABEL: &str = "io.github.publicusr.singularityd";

/// Does this Daemon run under the OS's service manager, as our own unit (#145)? It says so in its welcome, so that when
/// it goes away Clients leave the next one to the service manager. Only our unit counts: a Daemon a Client started
/// from a terminal that itself runs under some unit (a terminal's D-Bus service, `systemd-run`) is not systemd's.
/// Linux: this process's cgroup is the unit's. macOS: launchd sets `XPC_SERVICE_NAME` to the job's label (a shell in a
/// terminal app has its own, `application.…`, or "0"; the Client doesn't pass it on, `conn.rs`).
pub fn run_by_service() -> bool {
    if cfg!(target_os = "macos") {
        launchd_job(std::env::var("XPC_SERVICE_NAME").ok().as_deref())
    } else {
        std::fs::read_to_string("/proc/self/cgroup").is_ok_and(|c| in_systemd_unit(&c))
    }
}

/// Does `/proc/self/cgroup` put this process in [`SYSTEMD_UNIT`] (`0::/user.slice/…/app.slice/singularityd.service`)?
fn in_systemd_unit(cgroup: &str) -> bool {
    cgroup.lines().any(|l| l.rsplit_once(':').is_some_and(|(_, path)| path.rsplit('/').next() == Some(SYSTEMD_UNIT)))
}

/// Is `XPC_SERVICE_NAME` our LaunchAgent's label?
fn launchd_job(xpc_service_name: Option<&str>) -> bool {
    xpc_service_name == Some(LAUNCHD_LABEL)
}

/// Shared Daemon state that every connection reads.
pub struct Hub {
    pub started_unix_ms: u64,
    /// which Engine this Daemon runs
    pub engine_kind: proto::EngineKind,
    /// Soloist runs in the Soloist VM (Q94)
    pub soloist_in_vm: bool,
    /// what is playing and on which device (D80); never held across an await
    pub now: std::sync::Mutex<playing::NowPlaying>,
    pub engine_status: Mutex<Option<proto::EngineStatus>>,
    pub catalogue_status: Mutex<Option<proto::CatalogueStatus>>,
    pub home: Mutex<Option<proto::HomeFeed>>,
    pub library: Mutex<Option<proto::Library>>,
    pub stream: Mutex<Option<proto::StreamInfo>>,
    /// the Engine's download speed and stalls (#45)
    pub delivery: Mutex<Option<proto::Delivery>>,
    pub devices: Mutex<Option<Vec<proto::Device>>>,
    /// the account's Jam (D64): `None` until the Catalogue has looked
    pub jam: Mutex<Option<Option<proto::Jam>>>,
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
    /// the demo Listening log beside it (D90): plays go there too while a demo Client is connected, and demo
    /// Clients' stats come from it
    pub demo_store: Option<Arc<std::sync::Mutex<store::Store>>>,
    /// how many demo Clients are connected
    pub demo_clients: std::sync::atomic::AtomicUsize,
    /// tracks a demo Client showed as playing (#128): only their plays go into the demo log; forgotten when the last
    /// demo Client leaves
    pub demo_shown: std::sync::Mutex<std::collections::HashSet<String>>,
    /// background tasks that stopped and haven't run for a while: the health view names them (A1)
    pub stopped_tasks: Mutex<BTreeSet<String>>,
    /// the last Home and library on disk (#81); None: off (tests, fake Daemons)
    pub last_seen: Option<last_seen::LastSeen>,
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
    /// What Clients see playing now.
    pub fn playback(&self) -> PlaybackState {
        self.now().shown().clone()
    }

    fn now(&self) -> std::sync::MutexGuard<'_, playing::NowPlaying> {
        self.now.lock().expect("now playing lock")
    }

    /// Give Now playing one input and send on what it changed: the playback to Clients (sent under the lock, so
    /// Clients hear states in the order they were made), the tracks to name to the Catalogue.
    async fn feed(&self, input: impl FnOnce(&mut playing::NowPlaying) -> playing::Changed) {
        let ask = {
            let mut now = self.now();
            let changed = input(&mut now);
            if let Some(s) = changed.show {
                let _ = self.events.send(Event::Playback(s));
            }
            changed.ask
        };
        if !ask.is_empty()
            && let Some(c) = &self.catalogue
            && c.send(CatCmd::Describe(ask)).await.is_err()
        {
            self.report("the Catalogue has stopped; those tracks won't be named");
        }
    }

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
/// socket (and its directory) where someone else chose. Root's own links are the system's layout (macOS: `/var` and
/// `/tmp` lead to `/private/…`, so its temp directory is behind one), and only root can make one.
fn no_symlinks(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let mut cur = PathBuf::new();
    for part in dir.components() {
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() && m.uid() == 0 && std::fs::metadata(&cur).is_ok_and(|t| t.is_dir()) => {}
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
    /// OS media integration: the desktop's media controls, media keys and track notifications
    pub os_media: Option<os_media::Options>,
    /// the LocalStore's directory: the listening log and nightly library backups
    pub store: Option<PathBuf>,
    /// the desktop-notification flag a background task keeps following `config.toml` (`notifications.track_change`)
    pub notify: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// where the last Home and library are kept for the next start (#81)
    pub last_seen: Option<PathBuf>,
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
    let demo_store = match extras.store.as_deref().filter(|_| store.is_some()).map(store::Store::open_demo) {
        Some(Ok(s)) => Some(Arc::new(std::sync::Mutex::new(s))),
        Some(Err(e)) => {
            tracing::warn!("the demo listening log is unavailable: {e}");
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
    // without the Catalogue nothing would replace them
    let last_seen = extras.last_seen.filter(|_| cat_rx.is_some()).map(last_seen::LastSeen::at);
    // shown once the Catalogue names the account (`claim_last_seen`)
    if let Some(l) = &last_seen {
        l.read();
    }
    let hub = Arc::new(Hub {
        started_unix_ms: proto::now_ms(),
        engine_kind: engine.kind(),
        soloist_in_vm: engine.soloist_in_vm(),
        now: Default::default(),
        engine_status: Mutex::new(None),
        catalogue_status: Mutex::new(None),
        home: Mutex::new(None),
        library: Mutex::new(None),
        stream: Mutex::new(None),
        delivery: Mutex::new(None),
        devices: Mutex::new(None),
        jam: Mutex::new(None),
        events,
        commands,
        catalogue: cat_tx,
        spectrum: broadcast::channel(8).0,
        spectrum_listeners: Default::default(),
        spectrum_wake: Default::default(),
        shutdown: Default::default(),
        store,
        demo_store,
        demo_clients: Default::default(),
        demo_shown: Default::default(),
        stopped_tasks: Default::default(),
        last_seen,
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
    os_media::start(&hub, extras.os_media);
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
pub(crate) fn supervise<F, Fut>(hub: Arc<Hub>, name: &'static str, mut start: F) -> tokio::task::JoinHandle<()>
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
            let r = singularity_config::load_config(&path);
            // the start's problems the Daemon logged with its Engine assembly; a save's are new
            for p in r.problems.iter().filter(|_| !first) {
                tracing::warn!("{p}");
            }
            first = false;
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
        // (while another device plays, Clients follow that one)
        EngineEvent::Playback(state) => hub.feed(|now| now.engine(state)).await,
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
            // while a demo Client is connected, its own log gets the play too (D90; the real one keeps it as well), if
            // that Client showed the track: a hidden one (the song left from before, one queued elsewhere) stays out
            // of demo stats (#128)
            if demo_logs(&hub, &play)
                && let Some(d) = &hub.demo_store
                && let Err(e) = d.lock().expect("demo store lock").record(&play)
            {
                tracing::warn!("couldn't log a play in the demo log: {e}");
            }
            if let Ok(summary) = summary {
                let _ = hub.events.send(Event::Listening(summary));
            }
        }
    }
}

/// Does this play go into the demo Listening log: a demo Client is connected and showed its track (#128)?
fn demo_logs(hub: &Hub, play: &store::Play) -> bool {
    hub.demo_clients.load(std::sync::atomic::Ordering::SeqCst) > 0 && hub.demo_shown.lock().expect("demo shown lock").contains(&play.item.uri)
}

/// Where playback is happening, for the listening log.
async fn device_name(hub: &Hub) -> String {
    let elsewhere = hub.now().elsewhere();
    if elsewhere
        && let Some(d) = hub.devices.lock().await.iter().flatten().find(|d| d.active && !d.is_engine)
    {
        return d.name.clone();
    }
    match hub.engine_status.lock().await.clone() {
        Some(proto::EngineStatus::Ready { device_name, .. }) => device_name,
        _ => "this computer".into(),
    }
}

/// The demo Listening log summed up, for a demo Client (D90), in place of the real one's summary.
fn demo_summary(hub: &Hub) -> Option<Event> {
    let s = hub.demo_store.as_ref()?.lock().expect("demo store lock");
    s.summary(proto::now_ms()).ok().map(Event::Listening)
}

/// Answer a LocalStore command (`GetListening`, `GetStats`, `ExportLocalData`, `DeleteLocalData`). A demo Client's
/// questions and its "delete" go to the demo Listening log (D90).
async fn local_data(hub: &Hub, cmd: Command, out: &mpsc::Sender<DaemonMsg>, demo: bool) {
    let store = if demo { hub.demo_store.clone() } else { hub.store.clone() };
    let Some(store) = store else {
        // questions go unanswered (Widgets keep waiting quietly); asking to export or delete is told
        if matches!(cmd, Command::ExportLocalData | Command::DeleteLocalData) {
            let _ = out.send(DaemonMsg::Event(Event::Error { message: "the listening log is off in this Daemon".into() })).await;
        }
        return;
    };
    let now = proto::now_ms();
    let (reply, everyone) = {
        let mut s = store.lock().expect("store lock");
        match cmd {
            Command::GetListening => (s.summary(now).map(Event::Listening), None),
            Command::GetStats { period } => {
                // plays logged before links were kept get them from the Catalogue, a few at a time (see Described)
                if !demo && let Some(c) = &hub.catalogue
                    && let Ok(uris) = s.unlinked_tracks(20)
                    && !uris.is_empty()
                {
                    let _ = c.try_send(CatCmd::Describe(uris));
                }
                (s.stats(period, now).map(Event::Stats), None)
            }
            Command::ExportLocalData => (store::downloads_dir().and_then(|d| s.export(&d, now)).map(|p| Event::Notice { message: format!("Listening data exported to {}", p.display()) }), None),
            // (the real log's summary goes to everyone: demo Clients swap it for theirs)
            _ if demo => match s.delete_all() {
                Ok(()) => (Ok(Event::Notice { message: "Deleted the demo listening log".into() }), hub.store.as_ref().and_then(|r| r.lock().expect("store lock").summary(now).ok()).map(Event::Listening)),
                Err(e) => (Err(e), None),
            },
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
                // shown at once
                hub.feed(|now| now.described(items)).await;
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
            CatEvent::Jam(jam) => {
                let mut known = hub.jam.lock().await;
                if known.as_ref() != Some(&jam) {
                    *known = Some(jam.clone());
                    let _ = hub.events.send(Event::Jam { jam });
                }
            }
            CatEvent::Remote(r) => hub.feed(|now| now.remote(r)).await,
            CatEvent::Status(s) => {
                // librespot plays as the Catalogue's login: waiting for one, it starts as soon as there is one
                let waiting = matches!(*hub.engine_status.lock().await, Some(proto::EngineStatus::NotConfigured { .. }));
                if hub.engine_kind == proto::EngineKind::Librespot && waiting && matches!(s, proto::CatalogueStatus::Ready { .. }) {
                    to_player(&hub, Command::RestartEngine).await;
                }
                // logged out, or the session gone, no other device is ours to follow or control any more (C5)
                if matches!(s, proto::CatalogueStatus::LoggedOut | proto::CatalogueStatus::Unavailable { .. }) {
                    hub.feed(|now| now.gone()).await;
                }
                // Home and the library were the account's that left (#81), on disk or not: a Client that connects
                // now isn't shown them, and they aren't written back
                if matches!(s, proto::CatalogueStatus::LoggedOut) {
                    *hub.home.lock().await = None;
                    *hub.library.lock().await = None;
                    if let Some(l) = &hub.last_seen {
                        l.forget();
                    }
                }
                if let proto::CatalogueStatus::Ready { username, .. } = &s {
                    claim_last_seen(&hub, username).await;
                }
                // the listening summary speaks of the logged-in account's backups only, and of none while logged out
                let account = match &s {
                    proto::CatalogueStatus::Ready { username, .. } => Some(Some(username.clone())),
                    proto::CatalogueStatus::LoggedOut | proto::CatalogueStatus::LoginPending { .. } => Some(None),
                    _ => None,
                };
                if let (Some(account), Some(store)) = (account, &hub.store) {
                    let summary = {
                        let mut st = store.lock().expect("store lock");
                        st.set_account(account).then(|| st.summary(proto::now_ms()).ok()).flatten()
                    };
                    if let Some(summary) = summary {
                        let _ = hub.events.send(Event::Listening(summary));
                    }
                }
                *hub.catalogue_status.lock().await = Some(s.clone());
                let _ = hub.events.send(Event::Catalogue(s));
            }
            CatEvent::Home(h) => {
                *hub.home.lock().await = Some(h.clone());
                let _ = hub.events.send(Event::Home(h));
                keep_last_seen(&hub).await;
            }
            CatEvent::Library(l) => {
                *hub.library.lock().await = Some(l.clone());
                let _ = hub.events.send(Event::Library(l));
                keep_last_seen(&hub).await;
            }
            CatEvent::Account(name) => claim_last_seen(&hub, &name).await,
            CatEvent::Search(r) => {
                let _ = hub.events.send(Event::Search(r));
            }
            CatEvent::Artist(p) => {
                let _ = hub.events.send(Event::Artist(p));
            }
            CatEvent::Peeked { uri, tracks, artist } => {
                let _ = hub.events.send(Event::Peeked { uri, tracks, artist });
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
            CatEvent::TaskStopped(name, why) => {
                // with the Catalogue gone, so is what Connect told us (C5); it says again once it's back
                if name == catalogue::TASK {
                    hub.feed(|now| now.gone()).await;
                }
                hub.task_stopped(name, &why).await
            }
            CatEvent::TaskStarted(name) => hub.task_running(name).await,
            // these couldn't be looked up: clear the marks so a later playback state asks again (A8; the Catalogue
            // keeps them from being asked about too often)
            CatEvent::DescribeFailed(uris) => hub.feed(|now| now.describe_failed(uris)).await,
            // a change the Clients showed early didn't happen: the last real state puts it back
            CatEvent::Resync => hub.feed(|now| now.resync()).await,
        }
    }
}

/// Keep the Hub's Home and library for the next start, as the named account's (#81). A logout empties both, so what
/// the Hub holds is always the current account's.
async fn keep_last_seen(hub: &Hub) {
    let Some(l) = &hub.last_seen else { return };
    l.write(hub.home.lock().await.clone(), hub.library.lock().await.clone());
}

/// The Catalogue named its account (#81): the last copy on disk is shown when it is that account's (and nothing fresher
/// is in yet); another account's Home and library, on disk or held, go.
async fn claim_last_seen(hub: &Hub, account: &str) {
    let Some(l) = &hub.last_seen else { return };
    let claimed = l.claim(account);
    if claimed.switched {
        *hub.home.lock().await = None;
        *hub.library.lock().await = None;
    }
    let Some(seen) = claimed.seen else { return };
    if let Some(h) = seen.home {
        let mut home = hub.home.lock().await;
        if home.is_none() {
            *home = Some(h.clone());
            let _ = hub.events.send(Event::Home(h));
        }
    }
    if let Some(lib) = seen.library {
        let mut library = hub.library.lock().await;
        if library.is_none() {
            *library = Some(lib.clone());
            let _ = hub.events.send(Event::Library(lib));
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
        Command::Peek { uri } => CatCmd::Peek(uri.clone()),
        Command::Search { query } => CatCmd::Search(query.clone()),
        Command::GetDevices => CatCmd::Devices,
        Command::Transfer { device_id } => CatCmd::Transfer(device_id.clone()),
        Command::StartJam => CatCmd::StartJam,
        Command::EndJam => CatCmd::EndJam,
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
    // a demo Client (D90): its stats come from the demo Listening log, which logs plays while it's connected
    let demo = matches!(hello, Some(ClientMsg::Hello { demo: true, .. }));
    match hello {
        Some(ClientMsg::Hello { protocol, client, demo: d }) if protocol == PROTOCOL_VERSION => {
            tracing::info!("client connected: {client}{}", if d { " (demo mode)" } else { "" });
            let account = if d { demo_account(&hub).await } else { String::new() };
            let welcome = DaemonMsg::Welcome { protocol: PROTOCOL_VERSION, daemon_version: VERSION.into(), started_unix_ms: hub.started_unix_ms, engine: hub.engine_kind, soloist_in_vm: hub.soloist_in_vm, demo: d, account, service: run_by_service() };
            proto::write_msg(&mut wr, &welcome).await?;
        }
        Some(ClientMsg::Hello { protocol, .. }) => {
            let message = format!("this singularityd speaks protocol {PROTOCOL_VERSION}, the Client speaks {protocol}. Restart the Daemon ({RESTART}) after updating.");
            proto::write_msg(&mut wr, &DaemonMsg::Incompatible { protocol: PROTOCOL_VERSION, message }).await?;
            return Ok(());
        }
        _ => return Ok(()),
    }
    let _demo_client = demo.then(|| DemoClient::new(hub.clone()));
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
    let playback = hub.playback();
    let mut newest = playback.timestamp_ms;
    snapshot.push(Event::Playback(playback));
    snapshot.extend(hub.catalogue_status.lock().await.clone().map(Event::Catalogue));
    snapshot.extend(hub.home.lock().await.clone().map(Event::Home));
    snapshot.extend(hub.library.lock().await.clone().map(Event::Library));
    snapshot.extend(hub.devices.lock().await.clone().map(|devices| Event::Devices { devices }));
    snapshot.extend(hub.jam.lock().await.clone().map(|jam| Event::Jam { jam }));
    snapshot.extend(hub.stream.lock().await.clone().map(Event::Stream));
    snapshot.extend(hub.delivery.lock().await.clone().map(Event::Delivery));
    // what was queued while the snapshot was read: only these can be older than it. Everything after is news in the
    // order it happened, whatever its anchor says (a remote device's states are stamped when they arrive, the
    // computer's own player keeps its last anchor, so going back to it must not be taken for going back in time)
    let mut early = events.len();
    for ev in snapshot {
        let _ = out_tx.send(DaemonMsg::Event(ev)).await;
    }

    let fwd_tx = out_tx.clone();
    let fwd_hub = hub.clone();
    let forwarder = tokio::spawn(async move {
        loop {
            let ev = match events.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    early = early.min(events.len());
                    continue;
                }
                Err(_) => break,
            };
            let queued_early = early > 0;
            early = early.saturating_sub(1);
            if let Event::Playback(p) = &ev
                && queued_early
            {
                // older than what the snapshot already showed: it would put the Client back in time (B6)
                if superseded(p, newest) {
                    continue;
                }
                newest = newest.max(p.timestamp_ms);
            }
            // a demo Client never sees the real log's summary: its own instead (D90)
            let ev = match ev {
                Event::Listening(_) if demo => match demo_summary(&fwd_hub) {
                    Some(ev) => ev,
                    None => continue,
                },
                ev => ev,
            };
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
                ClientMsg::Command(Command::GetLyrics { uri, words }) => match &hub.catalogue {
                    Some(c) => {
                        // what we know about the track helps the fallback search
                        let meta = hub.now().lyrics_hint(&uri);
                        if c.send(CatCmd::Lyrics { uri, meta, words }).await.is_err() {
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
                ClientMsg::Command(cmd @ (Command::GetListening | Command::GetStats { .. } | Command::ExportLocalData | Command::DeleteLocalData)) => local_data(&hub, cmd, &out_tx, demo).await,
                // (only a demo Client's word counts)
                ClientMsg::Command(Command::ShownInDemo { uri }) => {
                    if demo {
                        hub.demo_shown.lock().expect("demo shown lock").insert(uri);
                    }
                }
                ClientMsg::Command(Command::GetState) => {
                    let s = hub.playback();
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

/// The account's username for a demo Client to redact (D90): the logged-in one, else the saved login's.
async fn demo_account(hub: &Hub) -> String {
    if let Some(proto::CatalogueStatus::Ready { username, .. }) = hub.catalogue_status.lock().await.clone() {
        return username;
    }
    if hub.catalogue.is_none() {
        return String::new();
    }
    tokio::task::spawn_blocking(|| catalogue::saved_credentials(&catalogue::Dirs::from_env()).and_then(|c| c.username)).await.ok().flatten().unwrap_or_default()
}

/// A demo Client's session, counted while it lasts (D90: plays go to the demo Listening log meanwhile).
struct DemoClient(Arc<Hub>);

impl DemoClient {
    fn new(hub: Arc<Hub>) -> DemoClient {
        hub.demo_clients.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        DemoClient(hub)
    }
}

impl Drop for DemoClient {
    fn drop(&mut self) {
        if self.0.demo_clients.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            self.0.demo_shown.lock().expect("demo shown lock").clear();
        }
    }
}

/// How long one request to Spotify may take. librespot's HTTP client has no timeout of its own: a request on a
/// connection that died quietly (the dealer's "Websocket peer does not respond") waits minutes for the OS, and holds
/// whatever awaits it — for the Catalogue, every search, page and Connect command queued behind it.
pub(crate) const SPOTIFY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(12);

/// A call to Spotify, given up after [`SPOTIFY_DEADLINE`] (as librespot's "deadline exceeded").
pub(crate) async fn spotify<T>(call: impl std::future::Future<Output = Result<T, librespot_core::Error>>) -> Result<T, librespot_core::Error> {
    match tokio::time::timeout(SPOTIFY_DEADLINE, call).await {
        Ok(r) => r,
        Err(_) => Err(librespot_core::Error::deadline_exceeded(format!("Spotify didn't answer within {} s", SPOTIFY_DEADLINE.as_secs()))),
    }
}

/// How long a Client may take to accept one message before its session is closed.
const WRITE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// A command for the player: to the Engine, or over Connect while another device plays.
pub(crate) async fn transport(hub: &Hub, cmd: Command) {
    // (without the Catalogue no other device is followed: the command goes to the Engine as it is)
    let Some(c) = &hub.catalogue else { return to_player(hub, cmd).await };
    let route = hub.now().route(cmd);
    match route {
        playing::Route::Connect(cmd) => {
            if c.send(CatCmd::Remote(cmd)).await.is_err() {
                hub.report("the Catalogue has stopped; that command wasn't delivered");
            }
        }
        playing::Route::Engine(cmd) => to_player(hub, cmd).await,
    }
}

/// A command for the Engine; one the Engine can't take is reported, never swallowed (A1).
async fn to_player(hub: &Hub, cmd: Command) {
    if hub.commands.send(cmd).await.is_err() {
        hub.report("the player has stopped; that command wasn't delivered");
    }
}


/// Default socket location (see `singularity_proto::socket_path`).
pub fn default_socket() -> PathBuf {
    proto::socket_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #128: while a demo Client is connected, the demo Listening log gets only the plays of tracks it showed; a
    /// hidden one (the song left from before) goes only into the real log.
    #[tokio::test]
    async fn the_demo_log_gets_only_the_tracks_a_demo_client_showed() {
        let dir = tempfile::tempdir().unwrap();
        let mut hub = Arc::into_inner(test_hub()).expect("one owner");
        hub.store = Some(Arc::new(std::sync::Mutex::new(store::Store::open(dir.path()).unwrap())));
        hub.demo_store = Some(Arc::new(std::sync::Mutex::new(store::Store::open_demo(dir.path()).unwrap())));
        let hub = Arc::new(hub);
        hub.demo_clients.store(1, std::sync::atomic::Ordering::SeqCst);
        hub.demo_shown.lock().unwrap().insert("spotify:track:shown".into());
        tokio::spawn(log_plays(hub.clone()));
        tokio::task::yield_now().await;
        let playing = |uri: &str| PlaybackState { status: proto::PlayStatus::Playing, item: Some(proto::Item { uri: uri.into(), name: "T".into(), artists: vec!["A".into()], duration_ms: 400, ..Default::default() }), speed: 1.0, ..Default::default() };
        for uri in ["spotify:track:hidden", "spotify:track:shown", "spotify:track:last"] {
            let _ = hub.events.send(Event::Playback(playing(uri)));
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        }
        let now = proto::now_ms();
        assert_eq!(hub.store.as_ref().unwrap().lock().unwrap().summary(now).unwrap().plays, 2, "the real log: both");
        assert_eq!(hub.demo_store.as_ref().unwrap().lock().unwrap().summary(now).unwrap().plays, 1, "the demo log: only the shown one");
    }

    /// #145: the Daemon knows it runs under the service manager from what systemd or launchd put in its environment.
    /// #145: only our own unit counts, never a unit the terminal (and so the Client) runs under.
    #[test]
    fn a_daemon_under_its_systemd_unit_knows_it() {
        assert!(in_systemd_unit("0::/user.slice/user-1000.slice/user@1000.service/app.slice/singularityd.service\n"));
        assert!(in_systemd_unit("1:name=systemd:/user.slice/user-1000.slice/user@1000.service/app.slice/singularityd.service\n0::/\n"), "cgroup v1");
        // started by a Client in a terminal: the terminal's scope or service, a systemd-run unit, a login session
        for other in [
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-ghostty-surface-transient-6202.scope\n",
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-gnome\\x2dterminal\\x2dserver.service\n",
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/run-u42.service\n",
            "0::/user.slice/user-1000.slice/session-2.scope\n",
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/not-singularityd.service\n",
            "",
        ] {
            assert!(!in_systemd_unit(other), "{other}");
        }
    }

    #[test]
    fn a_daemon_under_its_launchagent_knows_it() {
        assert!(launchd_job(Some("io.github.publicusr.singularityd")));
        // a shell in Terminal, iTerm2 or Ghostty passes on its own value (the Client drops it, but an older one won't)
        for other in [None, Some("0"), Some(""), Some("application.com.googlecode.iterm2.1234.5678"), Some("application.com.mitchellh.ghostty.1.2")] {
            assert!(!launchd_job(other), "{other:?}");
        }
    }

    fn test_hub() -> Arc<Hub> {
        let (events, _) = broadcast::channel(8);
        let (commands, _) = mpsc::channel(4);
        Arc::new(Hub {
            started_unix_ms: 0,
            engine_kind: proto::EngineKind::Fake,
            soloist_in_vm: false,
            now: Default::default(),
            engine_status: Mutex::new(None),
            catalogue_status: Mutex::new(None),
            home: Mutex::new(None),
            library: Mutex::new(None),
            stream: Mutex::new(None),
            delivery: Mutex::new(None),
            devices: Mutex::new(None),
            jam: Mutex::new(None),
            events,
            commands,
            catalogue: None,
            spectrum: broadcast::channel(1).0,
            spectrum_listeners: Default::default(),
            spectrum_wake: Default::default(),
            shutdown: Default::default(),
            store: None,
            demo_store: None,
            demo_clients: Default::default(),
            demo_shown: Default::default(),
            stopped_tasks: Default::default(),
            last_seen: None,
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
    /// macOS keeps its temp directory behind root's `/var` link: refusing every link kept the Daemon from making its
    /// socket there at all. Root's own links are the system's; anyone else's is still refused (protocol tests).
    #[test]
    fn root_s_own_symlinks_are_the_system_s_layout() {
        use std::os::unix::fs::MetadataExt;
        // a root-owned link to a directory: `/bin` on merged-/usr Linux, `/var` and `/tmp` on macOS
        let Some(link) = ["/var", "/tmp", "/bin"].into_iter().map(Path::new).find(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink() && m.uid() == 0)) else { return };
        assert!(no_symlinks(&link.join("sgl-none/singularity")).is_ok(), "{} is root's", link.display());
    }

    #[test]
    fn playback_older_than_the_snapshot_is_superseded() {
        let at = |ms| PlaybackState { timestamp_ms: ms, ..Default::default() };
        assert!(superseded(&at(500), 1_000), "queued before the snapshot: it's in it already");
        assert!(!superseded(&at(1_000), 1_000), "the same moment: a duplicate is harmless, and it may be the last word");
        assert!(!superseded(&at(1_500), 1_000), "newer than the snapshot: the Client has to see it");
        assert!(!superseded(&at(0), 1_000), "a state without an anchor carries no time to compare");
    }

    /// C5 (2026-11-20 bug hunt): logged out, "another device is playing" stayed, so the bar kept showing that device's
    /// song and our transport buttons kept going to it.
    #[tokio::test]
    async fn logging_out_lets_go_of_the_other_device() {
        let hub = test_hub();
        let phone = PlaybackState { context_uri: Some("spotify:playlist:on-the-phone".into()), ..Default::default() };
        let ours = PlaybackState { context_uri: Some("spotify:playlist:here".into()), ..Default::default() };
        hub.now().engine(ours.clone());
        hub.now().remote(Some(phone));
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(run_catalogue(rx, hub.clone()));
        tx.send(CatEvent::Status(proto::CatalogueStatus::LoggedOut)).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while hub.now().elsewhere() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the other device is still remembered after logout");
        assert_eq!(hub.playback(), ours, "the bar is back on this computer");
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
