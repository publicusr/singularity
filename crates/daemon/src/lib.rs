//! `singularityd`: owns playback (Engine), later the Catalogue session, the audio tap and local data.
//! Clients talk to it over the protocol in `singularity-proto` (ADR-0003).

pub mod catalogue;
pub mod connect;
pub mod engine;
pub mod library;
pub mod queries;
pub mod soloist;
pub mod tap;

use catalogue::{CatCmd, CatEvent};
use engine::{Engine, EngineEvent};
use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg, Event, PROTOCOL_VERSION, PlaybackState};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, broadcast, mpsc};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Shared Daemon state that every connection reads.
pub struct Hub {
    pub started_unix_ms: u64,
    pub playback: Mutex<PlaybackState>,
    pub engine_status: Mutex<Option<proto::EngineStatus>>,
    pub catalogue_status: Mutex<Option<proto::CatalogueStatus>>,
    pub home: Mutex<Option<proto::HomeFeed>>,
    pub library: Mutex<Option<proto::Library>>,
    pub events: broadcast::Sender<Event>,
    pub commands: mpsc::Sender<Command>,
    pub catalogue: Option<mpsc::Sender<CatCmd>>,
    /// AudioTap frames, and how many Clients want them (the tap runs only while > 0)
    pub spectrum: broadcast::Sender<proto::SpectrumFrame>,
    pub spectrum_listeners: std::sync::atomic::AtomicUsize,
    pub spectrum_wake: tokio::sync::Notify,
    /// a Client asked this Daemon to make way for a newer one
    pub shutdown: tokio::sync::Notify,
}

/// What `serve` ended with.
#[derive(Debug, PartialEq, Eq)]
pub enum Served {
    /// asked to make way for a newer Daemon (exit non-zero, so systemd's `Restart=on-failure` starts the new one)
    Replaced,
}

/// A running Catalogue task (see `catalogue::spawn`); `None` disables it (tests, offline use).
pub type CatalogueHandle = (mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>);

/// Bind the socket (replacing a stale one) with owner-only permissions on its directory.
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
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

/// Run the Daemon on `listener` with `engine` until the process ends.
pub async fn serve(listener: UnixListener, engine: Box<dyn Engine>, catalogue: Option<CatalogueHandle>, tap: Option<tap::TapSource>) -> anyhow::Result<Served> {
    let (events, _) = broadcast::channel(256);
    let (commands, command_rx) = mpsc::channel(64);
    let (cat_tx, cat_rx) = match catalogue {
        Some((tx, rx)) => (Some(tx), Some(rx)),
        None => (None, None),
    };
    let hub = Arc::new(Hub {
        started_unix_ms: proto::now_ms(),
        playback: Mutex::new(PlaybackState::default()),
        engine_status: Mutex::new(None),
        catalogue_status: Mutex::new(None),
        home: Mutex::new(None),
        library: Mutex::new(None),
        events,
        commands,
        catalogue: cat_tx,
        spectrum: broadcast::channel(8).0,
        spectrum_listeners: Default::default(),
        spectrum_wake: Default::default(),
        shutdown: Default::default(),
    });
    if let Some(source) = tap {
        tokio::spawn(tap::run(hub.clone(), source));
    }
    tokio::spawn(run_engine(engine, hub.clone(), command_rx));
    if let Some(rx) = cat_rx {
        tokio::spawn(run_catalogue(rx, hub.clone()));
    }
    loop {
        let stream = tokio::select! {
            r = listener.accept() => r?.0,
            _ = hub.shutdown.notified() => {
                tracing::info!("a newer Client asked this Daemon to make way");
                return Ok(Served::Replaced);
            }
        };
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
                Some(EngineEvent::Playback(state)) => {
                    *hub.playback.lock().await = state.clone();
                    let _ = hub.events.send(Event::Playback(state));
                }
                Some(EngineEvent::Status(status)) => {
                    *hub.engine_status.lock().await = Some(status.clone());
                    let _ = hub.events.send(Event::Engine(status));
                }
                Some(EngineEvent::Error(message)) => {
                    let _ = hub.events.send(Event::Error { message });
                }
                None => break,
            },
            Some(cmd) = commands.recv() => {
                let _ = engine_tx.send(cmd).await;
            }
        }
    }
}

async fn run_catalogue(mut events: mpsc::Receiver<CatEvent>, hub: Arc<Hub>) {
    while let Some(ev) = events.recv().await {
        match ev {
            CatEvent::Status(s) => {
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
            CatEvent::Tracks(t) => {
                let _ = hub.events.send(Event::Tracks(t));
            }
            CatEvent::ToEngine(cmd) => {
                let _ = hub.commands.send(cmd).await;
            }
            CatEvent::Error(message) => {
                let _ = hub.events.send(Event::Error { message });
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
            proto::write_msg(&mut wr, &DaemonMsg::Welcome { protocol: PROTOCOL_VERSION, daemon_version: VERSION.into(), started_unix_ms: hub.started_unix_ms })
                .await?;
        }
        Some(ClientMsg::Hello { protocol, .. }) => {
            let message = format!(
                "this singularityd speaks protocol {PROTOCOL_VERSION}, the Client speaks {protocol}. Restart the Daemon (`systemctl --user restart singularityd`) after updating."
            );
            proto::write_msg(&mut wr, &DaemonMsg::Incompatible { protocol: PROTOCOL_VERSION, message }).await?;
            return Ok(());
        }
        _ => return Ok(()),
    }
    // initial snapshot
    if let Some(st) = hub.engine_status.lock().await.clone() {
        proto::write_msg(&mut wr, &DaemonMsg::Event(Event::Engine(st))).await?;
    }
    let snapshot = hub.playback.lock().await.clone();
    proto::write_msg(&mut wr, &DaemonMsg::Event(Event::Playback(snapshot))).await?;
    if let Some(c) = hub.catalogue_status.lock().await.clone() {
        proto::write_msg(&mut wr, &DaemonMsg::Event(Event::Catalogue(c))).await?;
    }
    if let Some(h) = hub.home.lock().await.clone() {
        proto::write_msg(&mut wr, &DaemonMsg::Event(Event::Home(h))).await?;
    }
    if let Some(l) = hub.library.lock().await.clone() {
        proto::write_msg(&mut wr, &DaemonMsg::Event(Event::Library(l))).await?;
    }

    let mut events = hub.events.subscribe();
    let (out_tx, mut out_rx) = mpsc::channel::<DaemonMsg>(64);
    let writer = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            if proto::write_msg(&mut wr, &m).await.is_err() {
                break;
            }
        }
    });
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
    let result = async {
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
                ClientMsg::Command(Command::Shutdown) => hub.shutdown.notify_one(),
                ClientMsg::Command(Command::GetState) => {
                    let s = hub.playback.lock().await.clone();
                    let _ = out_tx.send(DaemonMsg::Event(Event::Playback(s))).await;
                }
                ClientMsg::Command(cmd @ (Command::Login | Command::Logout | Command::GetHome | Command::GetLibrary | Command::GetTracks { .. } | Command::PlayFrom { .. })) => match &hub.catalogue {
                    Some(tx) => {
                        let c = match cmd {
                            Command::Login => CatCmd::Login,
                            Command::Logout => CatCmd::Logout,
                            Command::GetLibrary => CatCmd::Library,
                            Command::GetTracks { uri } => CatCmd::Tracks(uri),
                            Command::PlayFrom { context_uri, index, track_uri } => CatCmd::PlayFrom { context_uri, index, track_uri },
                            _ => CatCmd::Home,
                        };
                        let _ = tx.send(c).await;
                    }
                    // without a Catalogue the Engine still plays the track itself
                    None if matches!(cmd, Command::PlayFrom { .. }) => {
                        let _ = hub.commands.send(cmd).await;
                    }
                    None => {
                        let _ = out_tx.send(DaemonMsg::Event(Event::Error { message: "the Catalogue is disabled in this Daemon".into() })).await;
                    }
                },
                ClientMsg::Command(cmd) => {
                    let _ = hub.commands.send(cmd).await;
                }
                ClientMsg::Hello { .. } | ClientMsg::Unknown => {}
            }
        }
        Ok::<(), std::io::Error>(())
    }
    .await;
    if let Some(t) = spectrum.take() {
        t.abort();
        hub.spectrum_listeners.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
    forwarder.abort();
    writer.abort();
    result
}

/// Default socket location (see `singularity_proto::socket_path`).
pub fn default_socket() -> PathBuf {
    proto::socket_path()
}
