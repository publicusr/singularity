//! `singularityd`: owns playback (Engine), later the Catalogue session, the audio tap and local data.
//! Clients talk to it over the protocol in `singularity-proto` (ADR-0003).

pub mod engine;
pub mod soloist;

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
    pub events: broadcast::Sender<Event>,
    pub commands: mpsc::Sender<Command>,
}

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
pub async fn serve(listener: UnixListener, engine: Box<dyn Engine>) -> anyhow::Result<()> {
    let (events, _) = broadcast::channel(256);
    let (commands, command_rx) = mpsc::channel(64);
    let hub = Arc::new(Hub {
        started_unix_ms: proto::now_ms(),
        playback: Mutex::new(PlaybackState::default()),
        engine_status: Mutex::new(None),
        events,
        commands,
    });
    tokio::spawn(run_engine(engine, hub.clone(), command_rx));
    loop {
        let (stream, _) = listener.accept().await?;
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
    while let Some(msg) = proto::read_msg::<_, ClientMsg>(&mut rd).await? {
        match msg {
            ClientMsg::Ping { id } => {
                let _ = out_tx.send(DaemonMsg::Pong { id }).await;
            }
            ClientMsg::Command(Command::GetState) => {
                let s = hub.playback.lock().await.clone();
                let _ = out_tx.send(DaemonMsg::Event(Event::Playback(s))).await;
            }
            ClientMsg::Command(cmd) => {
                let _ = hub.commands.send(cmd).await;
            }
            ClientMsg::Hello { .. } | ClientMsg::Unknown => {}
        }
    }
    forwarder.abort();
    writer.abort();
    Ok(())
}

/// Default socket location (see `singularity_proto::socket_path`).
pub fn default_socket() -> PathBuf {
    proto::socket_path()
}
