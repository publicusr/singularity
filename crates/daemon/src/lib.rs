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
use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg, Event, PROTOCOL_VERSION, PlaybackState};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
    });
    if hub.store.is_some() {
        tokio::spawn(log_plays(hub.clone()));
        tokio::spawn(backup::run(hub.clone()));
    }
    if let Some(source) = tap {
        tokio::spawn(tap::run(hub.clone(), source));
    }
    tokio::spawn(run_engine(engine, hub.clone(), command_rx));
    if let Some(rx) = cat_rx {
        tokio::spawn(run_catalogue(rx, hub.clone()));
    }
    if let Some(opts) = extras.mpris {
        tokio::spawn(mpris::run(hub.clone(), opts));
    }
    #[cfg(target_os = "macos")]
    match extras.now_playing {
        Some(opts) => {
            tokio::spawn(nowplaying::run(hub.clone(), opts));
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

/// Engine events update the shared state and fan out to Clients; Client commands go to the Engine.
async fn run_engine(engine: Box<dyn Engine>, hub: Arc<Hub>, mut commands: mpsc::Receiver<Command>) {
    let (engine_tx, mut events) = engine.spawn();
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Some(ev) => on_engine_event(&hub, ev, &engine_tx).await,
                None => break,
            },
            Some(cmd) = commands.recv() => {
                let _ = engine_tx.send(cmd).await;
            }
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
            {
                let _ = c.send(CatCmd::Describe(missing)).await;
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
        EngineEvent::Error(message) => {
            let _ = hub.events.send(Event::Error { message });
        }
        EngineEvent::Notice(message) => {
            let _ = hub.events.send(Event::Notice { message });
        }
        // the Engine restarted for an update: the context goes back on through Connect (position, paused),
        // else the track alone through the Engine
        EngineEvent::Resume(r) => match &hub.catalogue {
            Some(c) => {
                let _ = c.send(CatCmd::Resume(r)).await;
            }
            None => {
                let _ = engine_tx.send(Command::Play { uri: Some(r.track_uri) }).await;
                let _ = engine_tx.send(Command::Seek { position_ms: r.position_ms }).await;
                if r.paused {
                    let _ = engine_tx.send(Command::Pause).await;
                }
            }
        },
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
            None if asking.insert(item.uri.clone()) => missing.push(item.uri.clone()),
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
                    for i in items {
                        asking.remove(&i.uri);
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
                        {
                            let _ = c.send(CatCmd::Describe(missing)).await;
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
                    let _ = hub.commands.send(Command::RestartEngine).await;
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
            CatEvent::ToEngine(cmd) => {
                let _ = hub.commands.send(cmd).await;
            }
            CatEvent::Error(message) => {
                let _ = hub.events.send(Event::Error { message });
            }
            // a change the Clients showed early didn't happen: the last real state puts it back
            CatEvent::Resync => {
                let state = hub.playback.lock().await.clone();
                let _ = hub.events.send(Event::Playback(state));
            }
        }
    }
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
    // initial snapshot
    let mut snapshot = vec![];
    if let Some(st) = hub.engine_status.lock().await.clone() {
        snapshot.push(Event::Engine(st));
    }
    snapshot.push(Event::Playback(hub.playback.lock().await.clone()));
    snapshot.extend(hub.catalogue_status.lock().await.clone().map(Event::Catalogue));
    snapshot.extend(hub.home.lock().await.clone().map(Event::Home));
    snapshot.extend(hub.library.lock().await.clone().map(Event::Library));
    snapshot.extend(hub.devices.lock().await.clone().map(|devices| Event::Devices { devices }));
    snapshot.extend(hub.stream.lock().await.clone().map(Event::Stream));
    snapshot.extend(hub.delivery.lock().await.clone().map(Event::Delivery));
    for ev in snapshot {
        let _ = out_tx.send(DaemonMsg::Event(ev)).await;
    }

    let mut events = hub.events.subscribe();
    let fwd_tx = out_tx.clone();
    let forwarder = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) => {
                    if fwd_tx.send(DaemonMsg::Event(ev)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
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
                        let _ = c.send(CatCmd::Lyrics { uri, meta }).await;
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
                    let _ = out_tx.send(DaemonMsg::Event(Event::Health(h))).await;
                }
                ClientMsg::Command(cmd @ (Command::GetListening | Command::GetStats { .. } | Command::ExportLocalData | Command::DeleteLocalData)) => local_data(&hub, cmd, &out_tx).await,
                ClientMsg::Command(Command::GetState) => {
                    let s = hub.playback.lock().await.clone();
                    let _ = out_tx.send(DaemonMsg::Event(Event::Playback(s))).await;
                }
                ClientMsg::Command(cmd @ (Command::Login | Command::Logout | Command::GetHome | Command::GetLibrary | Command::GetTracks { .. } | Command::MoreTracks { .. } | Command::FindShow { .. } | Command::PlayFrom { .. } | Command::PlayNext { .. } | Command::RemoveFromQueue { .. } | Command::ClearQueue | Command::MoveInQueue { .. } | Command::GetArtist { .. } | Command::Search { .. } | Command::GetDevices | Command::Transfer { .. } | Command::SetSaved { .. } | Command::CheckSaved { .. } | Command::CreatePlaylist { .. } | Command::DeletePlaylist { .. } | Command::EditPlaylist { .. } | Command::AddToPlaylist { .. } | Command::RemoveFromPlaylist { .. } | Command::MoveInPlaylist { .. } | Command::SetPlaylistCover { .. })) => match &hub.catalogue {
                    Some(tx) => {
                        let c = match cmd {
                            Command::Login => CatCmd::Login,
                            Command::Logout => CatCmd::Logout,
                            Command::GetLibrary => CatCmd::Library,
                            Command::GetTracks { uri } => CatCmd::Tracks(uri),
                            Command::MoreTracks { uri, offset } => CatCmd::MoreTracks { uri, offset },
                            Command::FindShow { uri } => CatCmd::FindShow(uri),
                            Command::PlayFrom { context_uri, index, track_uri } => CatCmd::PlayFrom { context_uri, index, track_uri },
                            Command::PlayNext { uri } => CatCmd::Queue(connect::QueueEdit::PlayNext(uri)),
                            Command::RemoveFromQueue { uri, uid } => CatCmd::Queue(connect::QueueEdit::Remove { uri, uid }),
                            Command::ClearQueue => CatCmd::Queue(connect::QueueEdit::Clear),
                            Command::MoveInQueue { uri, uid, to, into_queue } => CatCmd::Queue(connect::QueueEdit::Move { uri, uid, to, into_queue }),
                            Command::GetArtist { uri } => CatCmd::Artist(uri),
                            Command::Search { query } => CatCmd::Search(query),
                            Command::GetDevices => CatCmd::Devices,
                            Command::Transfer { device_id } => CatCmd::Transfer(device_id),
                            Command::SetSaved { uri, saved } => CatCmd::SetSaved { uri, saved },
                            Command::CheckSaved { uris } => CatCmd::CheckSaved(uris),
                            Command::CreatePlaylist { name, uris } => CatCmd::CreatePlaylist { name, uris },
                            Command::AddToPlaylist { playlist, name, uris, anyway } => CatCmd::AddToPlaylist { playlist, name, uris, anyway },
                            Command::RemoveFromPlaylist { playlist, uids } => CatCmd::RemoveFromPlaylist { playlist, uids },
                            Command::MoveInPlaylist { playlist, uids, before, after } => CatCmd::MoveInPlaylist { playlist, uids, before, after },
                            Command::SetPlaylistCover { playlist, jpeg } => match jpeg.map(|j| crate::playlists::base64_decode(&j)) {
                                Some(None) => {
                                    let _ = out_tx.send(DaemonMsg::Event(Event::Error { message: "That cover wasn't readable".into() })).await;
                                    continue;
                                }
                                jpeg => CatCmd::SetCover { playlist, jpeg: jpeg.flatten() },
                            },
                            Command::DeletePlaylist { uri } => CatCmd::DeletePlaylist(uri),
                            Command::EditPlaylist { uri, name, description } => CatCmd::EditPlaylist { uri, name, description },
                            _ => CatCmd::Home,
                        };
                        let _ = tx.send(c).await;
                    }
                    // without a Catalogue the Engine still plays the track itself (and appends instead of "next")
                    None if matches!(cmd, Command::PlayFrom { .. } | Command::PlayNext { .. }) => {
                        let _ = hub.commands.send(cmd).await;
                    }
                    None => {
                        let _ = out_tx.send(DaemonMsg::Event(Event::Error { message: "the Catalogue is disabled in this Daemon".into() })).await;
                    }
                },
                ClientMsg::Command(cmd) => transport(&hub, cmd).await,
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
            let _ = c.send(CatCmd::Remote(cmd)).await;
        }
        _ => {
            let _ = hub.commands.send(cmd).await;
        }
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
