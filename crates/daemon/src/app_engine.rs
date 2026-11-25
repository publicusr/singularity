//! AppEngine (D25, D51): the official Spotify app as this computer's Engine, the default on macOS.
//!
//! - The Daemon starts the app hidden (macOS: `open -gj`, so it never takes the foreground) and leaves it running when
//!   it stops: it's the user's app too.
//! - Every command goes to the Catalogue (`CatCmd::Engine`), which sends it to the app's Connect device through
//!   ConnectRemote; what the app plays, its queue and stream quality come back from the Connect cluster as Engine
//!   events (`catalogue::app_engine_events`). So the Hub, the Clients and the device list treat it like Soloist.
//! - Its Connect device is found by name: this computer's name (what the app registers under), or `[engine] device`.
//! - Only finding and starting the app is per OS (`spotify_app`, `app_engine/linux.rs` and `macos.rs`); whether it
//!   runs, and quitting it, the Host answers. The rest is shared.

use crate::catalogue::CatCmd;
use crate::engine::{Engine, EngineEvent, Identity};
use crate::spotify_app::{self, SEARCHED};
use singularity_config::host::Host;
use singularity_proto::{Command, EngineStatus};
use std::path::PathBuf;
use tokio::sync::mpsc;

pub struct AppEngine {
    pub identity: Identity,
    /// where commands go; None: this Daemon runs without the Catalogue, and the app can't be driven
    pub catalogue: Option<mpsc::Sender<CatCmd>>,
    /// find the app and start it if it isn't running (off in tests: they never touch the real one)
    pub start_app: bool,
}

/// The installed app.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub path: PathBuf,
    pub version: String,
}

impl Engine for AppEngine {
    fn kind(&self) -> singularity_proto::EngineKind {
        singularity_proto::EngineKind::SpotifyApp
    }

    fn respawn(&self) -> Option<Box<dyn Engine>> {
        Some(Box::new(AppEngine { identity: self.identity.clone(), catalogue: self.catalogue.clone(), start_app: self.start_app }))
    }

    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(16);
        let (ev_tx, ev_rx) = mpsc::channel(16);
        tokio::spawn(async move {
            let why = if self.catalogue.is_none() {
                Some("the Spotify app is driven over Connect, and this Daemon runs without the Catalogue".to_string())
            } else if self.start_app && installed().is_none() {
                Some(format!("the Spotify app isn't installed: get it from https://www.spotify.com/download ({SEARCHED})"))
            } else {
                None
            };
            if let Some(reason) = why {
                let _ = ev_tx.send(EngineEvent::Status(EngineStatus::NotConfigured { reason: reason.clone() })).await;
                while cmd_rx.recv().await.is_some() {
                    let _ = ev_tx.send(EngineEvent::Error(format!("No playback engine: {reason}"))).await;
                }
                return;
            }
            // the Catalogue says Ready (or why not) once Connect shows the app's device
            let _ = ev_tx.send(EngineEvent::Status(EngineStatus::Starting)).await;
            if self.start_app && !running() {
                match spotify_app::start() {
                    Ok(()) => tracing::info!("started the Spotify app (hidden) as the Engine, {:?} on Connect", self.identity.device_name),
                    Err(e) => {
                        let _ = ev_tx.send(EngineEvent::Status(EngineStatus::Failed { message: format!("couldn't start the Spotify app: {e}") })).await;
                    }
                }
            }
            let cat = self.catalogue.expect("checked above");
            // when the app was last seen running: a volume drag sends a command per percent, each needn't look it up
            let mut seen: Option<std::time::Instant> = None;
            while let Some(cmd) = cmd_rx.recv().await {
                if self.start_app {
                    // asked to: quit it and start it again (the fix when it's gone from Connect after a network change)
                    if cmd == Command::RestartEngine {
                        let _ = ev_tx.send(EngineEvent::Notice("Restarting the Spotify app…".into())).await;
                        let _ = ev_tx.send(EngineEvent::Status(EngineStatus::Starting)).await;
                        seen = None;
                        if let Err(e) = restart().await {
                            let _ = ev_tx.send(EngineEvent::Status(EngineStatus::Failed { message: format!("couldn't restart the Spotify app: {e}") })).await;
                        }
                        continue;
                    }
                    // the app was quit: start it again, hidden; the Catalogue waits for it on Connect
                    if !seen.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2)) {
                        if running() {
                            seen = Some(std::time::Instant::now());
                        } else {
                            let _ = ev_tx.send(EngineEvent::Notice("Starting the Spotify app…".into())).await;
                            if let Err(e) = spotify_app::start() {
                                let _ = ev_tx.send(EngineEvent::Status(EngineStatus::Failed { message: format!("couldn't start the Spotify app: {e}") })).await;
                                continue;
                            }
                        }
                    }
                }
                if cmd == Command::RestartEngine {
                    continue;
                }
                if cat.send(CatCmd::Engine(cmd)).await.is_err() {
                    return;
                }
            }
        });
        (cmd_tx, ev_rx)
    }
}

/// The app, if it's installed.
pub fn installed() -> Option<App> {
    spotify_app::installed()
}

/// The app's processes, as the Host lists them.
fn app_pids(host: &Host) -> Vec<u32> {
    host.processes().into_iter().filter(|p| p.name == spotify_app::PROCESS).map(|p| p.pid).collect()
}

/// Is the app running already?
fn running() -> bool {
    !app_pids(&Host::this()).is_empty()
}

/// Quit the app (SIGTERM: it shuts down as from its menu), wait up to 5 s for it to go, and start it again hidden.
async fn restart() -> std::io::Result<()> {
    let host = Host::this();
    for pid in app_pids(&host) {
        host.stop(pid, false);
    }
    for _ in 0..50 {
        if !running() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    spotify_app::start()
}
