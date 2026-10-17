//! The Daemon↔Client protocol (ADR-0003, D27).
//!
//! Transport: a Unix socket in the user's runtime dir. Framing: a 4-byte big-endian length followed by a
//! JSON message. JSON keeps the wire debuggable; spectrum frames are small enough that it doesn't matter.
//!
//! Versioning: the first exchange is `Hello` → `Welcome` (or `Incompatible`). Bump [`PROTOCOL_VERSION`] on
//! any breaking change; additive changes (new optional fields, new variants the other side may ignore)
//! don't need a bump because unknown fields are ignored and unknown variants decode as `Unknown`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const PROTOCOL_VERSION: u32 = 1;
/// Upper bound for one frame; anything larger is a bug or an attack.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Messages a Client sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello { protocol: u32, client: String },
    Ping { id: u64 },
    Command(Command),
    #[serde(other)]
    Unknown,
}

/// Messages the Daemon sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum DaemonMsg {
    Welcome { protocol: u32, daemon_version: String, started_unix_ms: u64 },
    Incompatible { protocol: u32, message: String },
    Pong { id: u64 },
    Event(Event),
    #[serde(other)]
    Unknown,
}

/// Things a Client asks the Daemon to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Resume, or start playing a Spotify URI (track, album, playlist, …).
    Play { uri: Option<String> },
    Pause,
    TogglePlay,
    Next,
    Previous,
    Seek { position_ms: u64 },
    SetVolume { percent: u8 },
    SetShuffle { enabled: bool },
    SetRepeat { mode: RepeatMode },
    AddToQueue { uri: String },
    /// Ask for a full snapshot (sent automatically after `Welcome`).
    GetState,
    /// Start the Spotify login (device-code flow); progress arrives as `Event::Catalogue`.
    Login,
    Logout,
    /// Fetch (or refresh) the Home feed; arrives as `Event::Home`.
    GetHome,
    /// Fetch the library sidebar; arrives as `Event::Library`.
    GetLibrary,
    /// Fetch a track list (playlist, album, or `spotify:collection` for Liked Songs); arrives page by page as `Event::Tracks`.
    GetTracks { uri: String },
    /// Play a context starting at its `index`-th track (`track_uri` guards against the list having changed).
    PlayFrom { context_uri: String, index: u32, track_uri: Option<String> },
    /// Exit so a newer `singularityd` can take over (sent by a Client that's newer than this Daemon).
    Shutdown,
    /// Start or stop receiving `Event::Spectrum` frames (the tap runs only while someone listens).
    Spectrum { on: bool },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    #[default]
    Off,
    Context,
    Track,
}

/// Things the Daemon tells Clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
pub enum Event {
    /// Full playback snapshot; Clients replace their copy.
    Playback(PlaybackState),
    /// Engine health/status changes (starting, needs pairing, ready, error…).
    Engine(EngineStatus),
    /// Catalogue (private API) session status.
    Catalogue(CatalogueStatus),
    /// The Home feed, Shelf by Shelf.
    Home(HomeFeed),
    /// The library sidebar: Liked Songs, playlists (folders flattened with a depth), albums, artists.
    Library(Library),
    /// One page of a track list; pages arrive in order until `offset + tracks.len() == total`.
    Tracks(TrackPage),
    /// One frame of audio analysis (~30 Hz while audio plays), only to Clients that asked for it.
    Spectrum(SpectrumFrame),
    /// A command was refused or failed.
    Error { message: String },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlaybackState {
    pub status: PlayStatus,
    pub item: Option<Item>,
    pub context_uri: Option<String>,
    pub context_name: Option<String>,
    /// Position anchor: `position_ms` at `timestamp_ms` (Unix ms), advancing at `speed` (0 when paused).
    pub position_ms: u64,
    pub timestamp_ms: u64,
    pub speed: f32,
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub queue: Vec<QueueEntry>,
}

impl PlaybackState {
    /// Interpolated position at `now_ms` (Unix ms), clamped to the item's duration.
    pub fn position_at(&self, now_ms: u64) -> u64 {
        let elapsed = now_ms.saturating_sub(self.timestamp_ms) as f64 * self.speed as f64;
        let pos = self.position_ms as f64 + elapsed;
        let max = self.item.as_ref().map(|i| i.duration_ms).filter(|d| *d > 0).unwrap_or(u64::MAX);
        (pos.max(0.0) as u64).min(max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayStatus {
    #[default]
    Idle,
    Playing,
    Paused,
    Buffering,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Item {
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub duration_ms: u64,
    pub explicit: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub item: Item,
    /// `context`, `queue` or `autoplay`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EngineStatus {
    NotConfigured { reason: String },
    Starting,
    /// Running, but no Spotify Connect session yet: pick the device on a phone once.
    NeedsPairing { device_name: String },
    Ready { device_name: String, active: bool, version: String, expires_in_days: Option<u32> },
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CatalogueStatus {
    LoggedOut,
    /// Open `url` (or go to spotify.com/pair) and enter `code`.
    LoginPending { url: String, code: String },
    Connecting,
    Ready { username: String, country: String, premium: bool },
    /// The private API is unreachable or changed; playback keeps working.
    Unavailable { message: String },
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HomeFeed {
    pub fetched_unix_ms: u64,
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shelf {
    pub title: String,
    /// `shortcuts` (the grid at the top), `shelf` (a regular row) or `feed` (the endless tail)
    pub kind: String,
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub uri: String,
    pub name: String,
    pub subtitle: String,
    pub image_url: Option<String>,
    /// `album`, `playlist`, `artist`, `episode`, `show` or `other`
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Library {
    pub items: Vec<LibraryItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub uri: String,
    pub name: String,
    pub subtitle: String,
    /// `liked`, `playlist`, `folder`, `album`, `artist` or `other`
    pub kind: String,
    pub image_url: Option<String>,
    /// nesting inside folders (0 = top level)
    pub depth: u8,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrackPage {
    /// the list this page belongs to (as requested)
    pub uri: String,
    pub offset: u32,
    pub total: u32,
    pub tracks: Vec<TrackRow>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrackRow {
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album: String,
    /// when it was added to the list (ISO 8601), for playlists and Liked Songs
    pub added_at: Option<String>,
    pub duration_ms: u64,
    pub playable: bool,
}

/// What the AudioTap heard in the last ~33 ms. Levels are relative to the track's own recent loudness
/// (automatic gain), so they look the same at any volume. All values 0–255.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SpectrumFrame {
    /// log-spaced bands, 35 Hz → 16 kHz, smoothed (fast rise, slow fall)
    pub bands: Vec<u8>,
    /// overall loudness
    pub level: u8,
    /// left/right peak meters
    pub vu: [u8; 2],
    /// an onset (beat) starts in this frame
    pub beat: bool,
}

/// Where the Daemon listens: `$XDG_RUNTIME_DIR/singularity/daemon.sock` (falls back to /tmp/singularity-$UID).
pub fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("SINGULARITY_SOCKET") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("singularity-{}", uid())));
    base.join("singularity").join("daemon.sock")
}

fn uid() -> String {
    std::env::var("UID").unwrap_or_else(|_| {
        std::fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m).to_string()).unwrap_or_else(|_| "user".into())
    })
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Write one frame.
pub async fn write_msg<W: AsyncWriteExt + Unpin, T: Serialize>(w: &mut W, msg: &T) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::other("frame too large"));
    }
    w.write_all(&(body.len() as u32).to_be_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await
}

/// Read one frame. `Ok(None)` means the peer closed the connection cleanly.
pub async fn read_msg<R: AsyncReadExt + Unpin, T: for<'de> Deserialize<'de>>(r: &mut R) -> std::io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    serde_json::from_slice(&body).map(Some).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let msg = ClientMsg::Command(Command::Seek { position_ms: 42_000 });
        write_msg(&mut a, &msg).await.unwrap();
        let got: ClientMsg = read_msg(&mut b).await.unwrap().unwrap();
        assert_eq!(got, msg);
    }

    #[test]
    fn unknown_variants_do_not_break_old_peers() {
        let m: DaemonMsg = serde_json::from_str(r#"{"t":"something_new","x":1}"#).unwrap();
        assert_eq!(m, DaemonMsg::Unknown);
        let c: Command = serde_json::from_str(r#"{"cmd":"teleport"}"#).unwrap();
        assert_eq!(c, Command::Unknown);
    }

    #[test]
    fn position_interpolates_and_clamps() {
        let s = PlaybackState {
            position_ms: 10_000,
            timestamp_ms: 1_000,
            speed: 1.0,
            item: Some(Item { duration_ms: 12_000, ..Default::default() }),
            ..Default::default()
        };
        assert_eq!(s.position_at(2_000), 11_000);
        assert_eq!(s.position_at(60_000), 12_000);
        let paused = PlaybackState { speed: 0.0, ..s };
        assert_eq!(paused.position_at(60_000), 10_000);
    }
}
