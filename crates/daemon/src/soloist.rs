//! SoloistEngine (ADR-0001): supervises Spotify Soloist and drives it over its local WebSocket.
//!
//! - binary: `$XDG_DATA_HOME/singularity/soloist/soloist`, downloaded from Spotify's CDN when missing
//!   (never redistributed; auto-update arrives in #6)
//! - API key: from the system keyring (`secret-tool … service singularity account soloist`) or
//!   `SOLOIST_API_KEY`; the setup wizard (#5) will manage it
//! - Soloist's own state (pairing, cache) lives under `$XDG_DATA_HOME/singularity/soloist-state`

use crate::engine::{Engine, EngineEvent};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use singularity_proto::{Command, EngineStatus, Item, PlayStatus, PlaybackState, QueueEntry, RepeatMode};
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::{Child, Command as Proc};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

pub const DEVICE_NAME: &str = "Singularity";
const CDN: &str = "https://soloist-builds.spotifycdn.com/soloist_release_x86_64.tar.gz";

pub struct SoloistEngine {
    pub data_dir: PathBuf,
}

impl SoloistEngine {
    pub fn from_env() -> SoloistEngine {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir);
        SoloistEngine { data_dir: base.join("singularity") }
    }
    fn binary(&self) -> PathBuf {
        self.data_dir.join("soloist").join("soloist")
    }
    fn state_dir(&self) -> PathBuf {
        self.data_dir.join("soloist-state")
    }
}

/// Soloist's API key: environment first, then the keyring.
pub async fn api_key() -> Option<String> {
    if let Ok(k) = std::env::var("SOLOIST_API_KEY") {
        if !k.trim().is_empty() {
            return Some(k.trim().into());
        }
    }
    let out = Proc::new("secret-tool").args(["lookup", "service", "singularity", "account", "soloist"]).output().await.ok()?;
    let k = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!k.is_empty()).then_some(k)
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

/// Keep Soloist running: start, connect, pump; on exit wait (with backoff) and start again.
async fn supervise(engine: SoloistEngine, mut cmd_rx: mpsc::Receiver<Command>, ev_tx: mpsc::Sender<EngineEvent>) {
    let mut backoff = Duration::from_secs(2);
    loop {
        let Some(key) = api_key().await else {
            status(&ev_tx, EngineStatus::NotConfigured { reason: "no Soloist API key in the keyring yet".into() }).await;
            if !wait_or_refuse(&mut cmd_rx, &ev_tx, Duration::from_secs(10), "No Soloist API key yet").await {
                return;
            }
            continue;
        };
        status(&ev_tx, EngineStatus::Starting).await;
        if let Err(e) = ensure_binary(&engine).await {
            status(&ev_tx, EngineStatus::Failed { message: format!("couldn't download Soloist: {e}") }).await;
            if !wait_or_refuse(&mut cmd_rx, &ev_tx, backoff, "Soloist isn't available").await {
                return;
            }
            backoff = (backoff * 2).min(Duration::from_secs(300));
            continue;
        }
        match run_once(&engine, &key, &mut cmd_rx, &ev_tx).await {
            Ok(Exit::Quit) => return,
            Ok(Exit::Expired) => {
                status(&ev_tx, EngineStatus::Failed { message: "this Soloist build has expired; updating (#6)".into() }).await;
                let _ = std::fs::remove_file(engine.binary());
                backoff = Duration::from_secs(2);
            }
            Ok(Exit::Died) => {}
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
                Some(_) => { let _ = ev_tx.send(EngineEvent::Error(why.into())).await; }
                None => return false,
            },
        }
    }
}

async fn ensure_binary(engine: &SoloistEngine) -> anyhow::Result<()> {
    let exe = engine.binary();
    if exe.exists() {
        return Ok(());
    }
    let dir = exe.parent().unwrap().to_path_buf();
    tokio::fs::create_dir_all(&dir).await?;
    let tgz = dir.join("soloist.tar.gz");
    tracing::info!("downloading Soloist from Spotify's CDN");
    let ok = Proc::new("curl").args(["--fail", "--location", "--silent", "--show-error", "-o"]).arg(&tgz).arg(CDN).status().await?.success()
        && Proc::new("tar").arg("-xzf").arg(&tgz).arg("-C").arg(&dir).status().await?.success();
    let _ = tokio::fs::remove_file(&tgz).await;
    anyhow::ensure!(ok && exe.exists(), "download or extraction failed");
    Ok(())
}

enum Exit {
    Quit,
    Died,
    Expired,
}

async fn run_once(engine: &SoloistEngine, key: &str, cmd_rx: &mut mpsc::Receiver<Command>, ev_tx: &mpsc::Sender<EngineEvent>) -> anyhow::Result<Exit> {
    let state = engine.state_dir();
    tokio::fs::create_dir_all(state.join("data")).await?;
    tokio::fs::create_dir_all(state.join("cache")).await?;
    let _ = tokio::fs::remove_file(state.join("data/ws.port")).await;
    let log = std::fs::File::create(state.join("soloist.log"))?;
    let mut cmd = Proc::new(engine.binary());
    cmd.args(["--device-name", DEVICE_NAME, "--api-key", key, "--ws", "127.0.0.1:0", "--verbose"])
        .arg("--data-dir")
        .arg(state.join("data"))
        .arg("--cache-dir")
        .arg(state.join("cache"))
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true);
    // Soloist must never outlive the Daemon, even if the Daemon is SIGKILLed: ask the kernel to
    // SIGTERM it when its parent dies.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child: Child = cmd.spawn()?;
    let version = Proc::new(engine.binary()).arg("--version").output().await.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();

    // wait for the WebSocket endpoint files
    let mut addr = None;
    for _ in 0..100 {
        if let Some(st) = child.try_wait()? {
            return Ok(if st.code() == Some(10) { Exit::Expired } else { Exit::Died });
        }
        if let (Ok(p), Ok(a)) = (std::fs::read_to_string(state.join("data/ws.port")), std::fs::read_to_string(state.join("data/ws.addr"))) {
            addr = Some(format!("ws://{}:{}", a.trim(), p.trim()));
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let Some(addr) = addr else { anyhow::bail!("Soloist never opened its WebSocket") };
    let (ws, _) = tokio_tungstenite::connect_async(addr).await?;
    let (mut ws_tx, mut ws_rx) = ws.split();
    let expires_in_days = lifetime_days(&state.join("soloist.log"));
    let mut st = SoloistState::default();
    let mut logged_in = false;
    let mut active = false;

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
                        let s = if logged_in {
                            EngineStatus::Ready { device_name: DEVICE_NAME.into(), active, version: version.clone(), expires_in_days }
                        } else {
                            EngineStatus::NeedsPairing { device_name: DEVICE_NAME.into() }
                        };
                        status(ev_tx, s).await;
                    }
                    Some("error") => {
                        let _ = ev_tx.send(EngineEvent::Error(ev["message"].as_str().unwrap_or("Soloist error").to_string())).await;
                    }
                    _ => {
                        let track_changed = ev["type"] == "track_changed";
                        if st.apply(&ev) {
                            let _ = ev_tx.send(EngineEvent::Playback(st.playback.clone())).await;
                        }
                        if track_changed {
                            let _ = ws_tx.send(Message::text(json!({"type":"command","command":"get_queue","limit":20}).to_string())).await;
                        }
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { let _ = child.kill().await; return Ok(Exit::Quit) };
                for m in st.to_soloist(cmd) {
                    ws_tx.send(Message::text(m.to_string())).await?;
                }
            }
        }
    }
}

/// "client expires in 88 days" from Soloist's startup log.
fn lifetime_days(log: &std::path::Path) -> Option<u32> {
    let text = std::fs::read_to_string(log).ok()?;
    let line = text.lines().find(|l| l.contains("expires in"))?;
    line.split("expires in").nth(1)?.split_whitespace().next()?.parse().ok()
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
    let artists = d["creators"].as_array().map(|a| a.iter().filter_map(|c| c["entity"]["decorations"]["identity"]["name"].as_str().map(String::from)).collect()).unwrap_or_default();
    let covers = d["visual_identity"]["cover"].as_array().cloned().unwrap_or_default();
    let cover_url = ["large", "xlarge", "default", "small"]
        .iter()
        .find_map(|size| covers.iter().find(|c| c["size"] == *size))
        .or(covers.first())
        .and_then(|c| c["url"].as_str().map(String::from));
    Some(Item {
        uri,
        name: d["identity"]["name"].as_str().unwrap_or_default().to_string(),
        artists,
        album: d["parent"]["entity"]["decorations"]["identity"]["name"].as_str().map(String::from),
        cover_url,
        duration_ms: d["playback"]["duration_ms"].as_u64().unwrap_or(0),
        explicit: d["playback"]["content_ratings"].as_array().is_some_and(|r| r.iter().any(|x| x == "explicit")),
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
                                Some(QueueEntry { item, source: q["source"].as_str().unwrap_or("context").into() })
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
            Command::PlayFrom { .. } | Command::GetLibrary | Command::GetTracks { .. } => vec![],
            Command::Login | Command::Logout | Command::GetHome | Command::Unknown => vec![],
        }
    }
}
