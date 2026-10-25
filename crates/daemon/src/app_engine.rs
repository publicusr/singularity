//! AppEngine (D25, D51): the official Spotify app as this computer's Engine, the default on macOS.
//!
//! - The Daemon starts the app hidden (macOS: `open -gj`, so it never takes the foreground) and leaves it running when
//!   it stops: it's the user's app too.
//! - Every command goes to the Catalogue (`CatCmd::Engine`), which sends it to the app's Connect device through
//!   ConnectRemote; what the app plays, its queue and stream quality come back from the Connect cluster as Engine
//!   events (`catalogue::app_engine_events`). So the Hub, the Clients and the device list treat it like Soloist.
//! - Its Connect device is found by name: this computer's name (what the app registers under), or `[engine] device`.
//! - Only finding and starting the app is per OS; the rest is shared.

use crate::catalogue::CatCmd;
use crate::engine::{Engine, EngineEvent, Identity};
use crate::programs::{Tool, command};
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

impl AppEngine {
    /// The app as Engine, under `device` on Connect (empty: this computer's name).
    pub fn new(device: &str, catalogue: Option<mpsc::Sender<CatCmd>>) -> AppEngine {
        let device_name = if device.trim().is_empty() { this_computer() } else { device.trim().to_string() };
        let version = installed().map(|a| a.version).unwrap_or_default();
        AppEngine { identity: Identity { device_name, via_connect: true, version, player: "the Spotify app" }, catalogue, start_app: true }
    }
}

impl Engine for AppEngine {
    fn kind(&self) -> singularity_proto::EngineKind {
        singularity_proto::EngineKind::SpotifyApp
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
                match start() {
                    Ok(()) => tracing::info!("started the Spotify app (hidden) as the Engine, {:?} on Connect", self.identity.device_name),
                    Err(e) => {
                        let _ = ev_tx.send(EngineEvent::Status(EngineStatus::Failed { message: format!("couldn't start the Spotify app: {e}") })).await;
                    }
                }
            }
            let cat = self.catalogue.expect("checked above");
            // when the app was last seen running: a volume drag sends a command per percent, each needn't ask `pgrep`
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
                            if let Err(e) = start() {
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

#[cfg(target_os = "macos")]
const SEARCHED: &str = "looked in /Applications and ~/Applications";
#[cfg(not(target_os = "macos"))]
const SEARCHED: &str = "looked in /usr/bin, /bin, /usr/local/bin and /snap/bin";

/// The app, if it's installed.
#[cfg(target_os = "macos")]
pub fn installed() -> Option<App> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let path = [PathBuf::from("/Applications/Spotify.app"), home.join("Applications/Spotify.app")].into_iter().find(|p| p.join("Contents/Info.plist").exists())?;
    let plist = path.join("Contents/Info.plist");
    let version = Tool::Defaults.find().and_then(|d| output(&d, &["read", &plist.to_string_lossy(), "CFBundleShortVersionString"])).unwrap_or_default();
    Some(App { path, version })
}

/// The app, if it's installed.
#[cfg(not(target_os = "macos"))]
pub fn installed() -> Option<App> {
    // a fixed location, never `PATH` (S3): a planted `spotify` would run with the Daemon's privileges
    let path = Tool::Spotify.find()?;
    Some(App { path, version: String::new() })
}

/// Is the app running already?
fn running() -> bool {
    let Some(pgrep) = Tool::Pgrep.find() else { return false };
    command(pgrep).args(["-x", if cfg!(target_os = "macos") { "Spotify" } else { "spotify" }]).stdout(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Start the app without showing it.
#[cfg(target_os = "macos")]
fn start() -> std::io::Result<()> {
    // -g: don't bring it to the foreground; -j: launch it hidden
    let open = Tool::Open.find().ok_or_else(|| std::io::Error::other("`open` is missing (looked in /usr/bin)"))?;
    let s = command(open).args(["-gj", "-a", "Spotify"]).status()?;
    if s.success() { Ok(()) } else { Err(std::io::Error::other(format!("`open -gj -a Spotify` failed ({s})"))) }
}

/// Start the app without showing it.
#[cfg(not(target_os = "macos"))]
fn start() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let null = || std::process::Stdio::null();
    let spotify = Tool::Spotify.find().ok_or_else(|| std::io::Error::other(format!("the Spotify app is missing ({SEARCHED})")))?;
    // its own process group, so it outlives the Daemon like the desktop app it is
    command(spotify).arg("--minimized").stdin(null()).stdout(null()).stderr(null()).process_group(0).spawn().map(|_| ())
}

/// Quit the app (SIGTERM: it shuts down as from its menu), wait up to 5 s for it to go, and start it again hidden.
async fn restart() -> std::io::Result<()> {
    let name = if cfg!(target_os = "macos") { "Spotify" } else { "spotify" };
    if let Some(pkill) = Tool::Pkill.find() {
        let _ = command(pkill).args(["-TERM", "-x", name]).status();
    }
    for _ in 0..50 {
        if !running() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    start()
}

/// This computer's name, as the Spotify app registers on Connect.
pub fn this_computer() -> String {
    let name = if cfg!(target_os = "macos") {
        Tool::Scutil.find().and_then(|s| output(&s, &["--get", "ComputerName"]))
    } else {
        std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).or_else(|| Tool::Hostname.find().and_then(|h| output(&h, &[])))
    };
    name.unwrap_or_else(|| "this computer".into())
}

/// A command's trimmed output, if it worked.
fn output(exe: &std::path::Path, args: &[&str]) -> Option<String> {
    let o = command(exe).args(args).stderr(std::process::Stdio::null()).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !s.is_empty()).then_some(s)
}
