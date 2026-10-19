//! LibrespotEngine (D52): librespot inside the Daemon as this computer's Engine, the same on Linux and macOS.
//!
//! - The lightest Engine: no second app, tens of MB. The trade-off: Ogg Vorbis at 320 kbps at most, never lossless
//!   (Spotify keeps lossless to its own players), and it plays through the private API, like the Catalogue.
//! - A Spotify Connect receiver (librespot's Spirc) under its own name, logged in with the Catalogue's saved login.
//!   Commands go to the Catalogue (`CatCmd::Engine`) and reach the receiver over Connect, and what it plays comes
//!   back from the Connect cluster, exactly as with the Spotify app (D51): one code path for both.
//! - Audio goes to the system output through cpal (CoreAudio, ALSA), and the same samples go to the AudioTap
//!   in-process ([`Feed`]) while a Client shows audio Widgets: no capture, no permission prompt, no `/proc` lookups.
//! - If the session drops (a network change) the receiver is started again where it was.

use crate::catalogue::{CatCmd, Dirs};
use crate::engine::{Engine, EngineEvent, Identity};
use crate::tap::Feed;
use librespot_connect::{ConnectConfig, Spirc};
use librespot_core::{authentication::Credentials, config::DeviceType, config::SessionConfig, session::Session};
use librespot_playback::audio_backend::{Sink, SinkResult};
use librespot_playback::config::{AudioFormat, Bitrate, PlayerConfig};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::{Mixer, MixerConfig, softmixer::SoftMixer};
use librespot_playback::player::{Player, PlayerEvent};
use singularity_proto::{Command, EngineStatus, StreamInfo};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct LibrespotEngine {
    pub identity: Identity,
    /// where commands go; None: this Daemon runs without the Catalogue (no login to play with)
    pub catalogue: Option<mpsc::Sender<CatCmd>>,
    /// where the saved Spotify login is
    pub dirs: Dirs,
    /// the AudioTap's side of the player's output
    pub feed: Arc<Feed>,
}

impl LibrespotEngine {
    /// librespot as Engine, under `device` on Connect (empty: "Singularity on <this computer>").
    pub fn new(device: &str, catalogue: Option<mpsc::Sender<CatCmd>>, dirs: Dirs) -> LibrespotEngine {
        let device_name = if device.trim().is_empty() { format!("Singularity on {}", crate::app_engine::this_computer()) } else { device.trim().to_string() };
        LibrespotEngine { identity: Identity { device_name, via_connect: true, version: String::new(), player: "librespot" }, catalogue, dirs, feed: Feed::new() }
    }
}

/// Where the last volume is kept: Connect doesn't remember it for a receiver, and starting at 50% every time
/// would be a surprise either way (the catalogue dir: owner-only, next to the login it plays with).
fn volume_file(dirs: &Dirs) -> std::path::PathBuf {
    dirs.credentials.join("librespot-volume")
}

/// The volume to start at (librespot's 0–65535), as last set.
pub fn saved_volume(dirs: &Dirs) -> Option<u16> {
    std::fs::read_to_string(volume_file(dirs)).ok()?.trim().parse().ok()
}

/// What librespot streams: always this (it never gets anything better). As the Connect cluster says it, so the two
/// never flicker.
pub fn stream_info() -> StreamInfo {
    StreamInfo { codec: "vorbis".into(), bitrate: 320_000, average_bitrate: None, sample_rate: 44_100, sample_format: String::new() }
}

/// A stable Connect device id for `name`, so a restart is the same device, not a new one next to a stale one.
pub fn device_id(name: &str) -> String {
    // FNV-1a, twice with different offsets: 32 hex digits, the same in every build
    let fnv = |seed: u64| name.bytes().fold(seed, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{:016x}{:016x}", fnv(0xcbf29ce484222325), fnv(0x84222325cbf29ce4 ^ 0x5349_4e47))
}

impl Engine for LibrespotEngine {
    fn kind(&self) -> singularity_proto::EngineKind {
        singularity_proto::EngineKind::Librespot
    }

    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(16);
        let (ev_tx, ev_rx) = mpsc::channel(16);
        tokio::spawn(run(*self, cmd_rx, ev_tx));
        (cmd_tx, ev_rx)
    }
}

async fn status(ev: &mpsc::Sender<EngineEvent>, s: EngineStatus) {
    let _ = ev.send(EngineEvent::Status(s)).await;
}

async fn run(engine: LibrespotEngine, mut cmds: mpsc::Receiver<Command>, ev: mpsc::Sender<EngineEvent>) {
    let Some(cat) = engine.catalogue.clone() else {
        let reason = "librespot plays as your Spotify login, and this Daemon runs without the Catalogue".to_string();
        status(&ev, EngineStatus::NotConfigured { reason: reason.clone() }).await;
        while cmds.recv().await.is_some() {
            let _ = ev.send(EngineEvent::Error(format!("No playback engine: {reason}"))).await;
        }
        return;
    };
    let mut saved = None;
    loop {
        // the Catalogue's login; until there is one, look again on every command (the Hub sends one when the login
        // is done) and every 10 s
        let Some(creds) = crate::catalogue::saved_credentials(&engine.dirs) else {
            status(&ev, EngineStatus::NotConfigured { reason: "log in to Spotify first: librespot plays as your account".into() }).await;
            match tokio::time::timeout(Duration::from_secs(10), cmds.recv()).await {
                Ok(None) => return,
                _ => continue,
            }
        };
        status(&ev, EngineStatus::Starting).await;
        let receiver = match start(&engine, creds, saved.take()).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("librespot: couldn't start: {e}");
                status(&ev, EngineStatus::Failed { message: format!("librespot couldn't start: {e}") }).await;
                // try again in a while, or at the next command
                match tokio::time::timeout(Duration::from_secs(10), cmds.recv()).await {
                    Ok(None) => return,
                    _ => continue,
                }
            }
        };
        tracing::info!("librespot: {:?} is on Connect", engine.identity.device_name);
        // the Catalogue says Ready once Connect shows the device
        let Receiver { spirc, mut task, player, mut events } = receiver;
        let mut told_stream = false;
        let restart = loop {
            tokio::select! {
                cmd = cmds.recv() => match cmd {
                    None => {
                        let _ = spirc.shutdown();
                        return;
                    }
                    Some(Command::RestartEngine) => {
                        let _ = ev.send(EngineEvent::Notice("Restarting librespot…".into())).await;
                        let _ = spirc.shutdown();
                        let _ = (&mut task).await;
                        break false;
                    }
                    Some(cmd) => {
                        if cat.send(CatCmd::Engine(cmd)).await.is_err() {
                            return;
                        }
                    }
                },
                e = events.recv() => match e {
                    Some(PlayerEvent::Playing { .. }) if !told_stream => {
                        told_stream = true;
                        let _ = ev.send(EngineEvent::Stream(stream_info())).await;
                    }
                    Some(PlayerEvent::VolumeChanged { volume }) => {
                        let _ = std::fs::write(volume_file(&engine.dirs), volume.to_string());
                    }
                    Some(PlayerEvent::Unavailable { track_id, .. }) => {
                        let _ = ev.send(EngineEvent::Error(format!("librespot can't play {track_id}: not available to it"))).await;
                    }
                    Some(_) => {}
                    None => break true,
                },
                // the session ended (a network change, a logout elsewhere): start again where it was
                r = &mut task => {
                    saved = r.ok().flatten();
                    break true;
                }
            }
        };
        drop(player);
        if restart {
            tracing::info!("librespot: the session ended; starting again");
            status(&ev, EngineStatus::Starting).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

struct Receiver {
    spirc: Spirc,
    task: tokio::task::JoinHandle<Option<librespot_connect::SavedPlaybackState>>,
    player: Arc<Player>,
    events: mpsc::UnboundedReceiver<PlayerEvent>,
}

/// A new session, player and Connect receiver (picking up `saved`, after a reconnect).
async fn start(engine: &LibrespotEngine, creds: Credentials, saved: Option<librespot_connect::SavedPlaybackState>) -> anyhow::Result<Receiver> {
    let name = engine.identity.device_name.clone();
    let session = Session::new(SessionConfig { device_id: device_id(&name), ..SessionConfig::default() }, None);
    let mixer: Arc<dyn Mixer> = Arc::new(SoftMixer::open(MixerConfig::default())?);
    let config = PlayerConfig { bitrate: Bitrate::Bitrate320, ..PlayerConfig::default() };
    let feed = engine.feed.clone();
    let sink = librespot_playback::audio_backend::find(None).ok_or_else(|| anyhow::anyhow!("no audio backend in this build"))?;
    let player = Player::new(config, session.clone(), mixer.get_soft_volume(), move || Box::new(Tee { inner: sink(None, AudioFormat::F32), feed }) as Box<dyn Sink>);
    let events = player.get_player_event_channel();
    let defaults = ConnectConfig::default();
    let initial_volume = saved_volume(&engine.dirs).unwrap_or(defaults.initial_volume);
    let connect = ConnectConfig { name, device_type: DeviceType::Computer, initial_volume, ..defaults };
    let (spirc, task) = Spirc::with_saved_state(connect, session, creds, player.clone(), mixer, saved).await?;
    Ok(Receiver { spirc, task: tokio::spawn(task), player, events })
}

/// The system output, and the same samples to the AudioTap while it wants them.
struct Tee {
    inner: Box<dyn Sink>,
    feed: Arc<Feed>,
}

impl Sink for Tee {
    fn start(&mut self) -> SinkResult<()> {
        self.inner.start()
    }

    fn stop(&mut self) -> SinkResult<()> {
        self.inner.stop()
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        if self.feed.wanted()
            && let AudioPacket::Samples(s) = &packet
        {
            self.feed.push(converter.f64_to_f32(s).to_vec());
        }
        self.inner.write(packet, converter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_id_is_stable_and_differs_per_name() {
        let a = device_id("Singularity on Mac");
        assert_eq!(a, device_id("Singularity on Mac"));
        assert_ne!(a, device_id("Singularity on desk"));
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn it_never_claims_lossless() {
        assert!(!stream_info().lossless());
    }
}
