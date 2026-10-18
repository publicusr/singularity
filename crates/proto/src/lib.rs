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
    /// Queue a track to play right after the current one.
    PlayNext { uri: String },
    /// Take a queued track out of the queue (`uid` picks the entry when the same track is queued twice).
    RemoveFromQueue { uri: String, uid: Option<String> },
    /// Remove everything the user queued (the context and autoplay continue).
    ClearQueue,
    /// Move a track the user queued, or one coming up from the context, to place `to` among the tracks of its own
    /// kind (0 = first of them). Autoplay tracks don't move.
    /// `into_queue`: a context song goes up into the user's queue, at place `to` among the queued songs (#50).
    MoveInQueue {
        uri: String,
        uid: Option<String>,
        to: usize,
        #[serde(default)]
        into_queue: bool,
    },
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
    /// Store the Soloist API key (keyring, else an owner-only file) and (re)start the Engine with it.
    SetSoloistKey { key: String },
    /// Like or unlike a track; save/unsave an album; follow/unfollow an artist or playlist. Answered by `Event::Saved`.
    SetSaved { uri: String, saved: bool },
    /// Ask whether these are in the library (liked, saved, followed); answered by `Event::Saved`.
    CheckSaved { uris: Vec<String> },
    /// List Spotify Connect devices; arrives as `Event::Devices` (and again whenever they change).
    GetDevices,
    /// Move playback to another Connect device.
    Transfer { device_id: String },
    /// Search everything; arrives as `Event::Search`.
    Search { query: String },
    /// Fetch an artist page; arrives as `Event::Artist`.
    GetArtist { uri: String },
    /// Fetch lyrics for a track (Spotify's, else LRCLIB's); arrives as `Event::Lyrics`.
    GetLyrics { uri: String },
    /// Ask for a health report; arrives as `Event::Health`.
    GetHealth,
    /// Exit so a newer `singularityd` can take over (sent by a Client that's newer than this Daemon).
    Shutdown,
    /// Start or stop receiving `Event::Spectrum` frames (the tap runs only while someone listens).
    Spectrum { on: bool },
    /// The listening log, summed up; arrives as `Event::Listening` (also sent after every logged play).
    GetListening,
    /// Stats for a calendar period from the listening log; arrives as `Event::Stats`. (Ask again after an
    /// `Event::Listening`: that's when a new Play was logged.)
    GetStats { period: StatsPeriod },
    /// Write the listening log and the latest library backup to one JSON file in the Downloads folder (`Notice`).
    ExportLocalData,
    /// Delete the listening log and every library backup (the Client confirms first).
    DeleteLocalData,
    /// Create a playlist of your own at the top of the library (#51); answered by `Event::PlaylistCreated` and a
    /// fresh `Event::Library`.
    CreatePlaylist { name: String },
    /// Delete one of your playlists (Spotify's "delete": it leaves your library); a fresh `Event::Library` follows,
    /// with it still there if Spotify refused (the Client confirms first).
    DeletePlaylist { uri: String },
    /// Rename and re-describe one of your playlists (#52); an empty description clears it. Answered by
    /// `Event::PlaylistEdited`.
    EditPlaylist { uri: String, name: String, description: String },
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
    /// What the Engine is streaming right now (from Soloist's verbose log).
    Stream(StreamInfo),
    /// How well the stream keeps up: the Engine's download speed and its recent stalls (#45).
    Delivery(Delivery),
    /// Whether things are in the library, as Spotify says (after `SetSaved` this is the truth to settle on: on a
    /// failure it carries the old value back).
    Saved { entries: Vec<SavedFlag> },
    /// The account's Connect devices. (A struct variant: the tagged encoding can't hold a bare list.)
    Devices { devices: Vec<Device> },
    /// Search results (answer to `Search`).
    Search(SearchResults),
    /// An artist page (answer to `GetArtist`).
    Artist(ArtistPage),
    /// Lyrics for a track (answer to `GetLyrics`); `lines` is empty when there are none.
    Lyrics(Lyrics),
    /// Answer to `GetHealth`.
    Health(Health),
    /// Something worth telling the user that isn't an error.
    Notice { message: String },
    /// One frame of audio analysis (~30 Hz while audio plays), only to Clients that asked for it.
    Spectrum(SpectrumFrame),
    /// A command was refused or failed.
    Error { message: String },
    /// The listening log, summed up (answer to `GetListening`, and after every logged play).
    Listening(ListeningSummary),
    /// Stats for one period (answer to `GetStats`).
    Stats(Stats),
    /// A playlist was created (answer to `CreatePlaylist`; the Client that asked opens it).
    PlaylistCreated { uri: String, name: String },
    /// A playlist's name and description were saved (`ok`), or Spotify refused them (the Client that asked rolls
    /// back; the others ignore it).
    PlaylistEdited { uri: String, name: String, description: String, ok: bool },
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
    /// the artists' URIs, parallel to `artists` (for links)
    #[serde(default)]
    pub artist_uris: Vec<String>,
    #[serde(default)]
    pub album_uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub item: Item,
    /// `context`, `queue` or `autoplay`.
    pub source: String,
    /// the player's id for this entry (tells apart the same track queued twice)
    #[serde(default)]
    pub uid: String,
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
    /// a playlist you own (you can edit or delete it)
    #[serde(default)]
    pub own: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrackPage {
    /// the list this page belongs to (as requested)
    pub uri: String,
    pub offset: u32,
    pub total: u32,
    pub tracks: Vec<TrackRow>,
    /// only on the first page
    #[serde(default)]
    pub header: Option<ListHeader>,
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
    /// the artists' URIs, parallel to `artists` (for links)
    #[serde(default)]
    pub artist_uris: Vec<String>,
    #[serde(default)]
    pub album_uri: Option<String>,
}

/// What a track list is, shown above it (sent with the first page).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ListHeader {
    /// "Album · 2023 · Fixture Records", "Playlist · Nova Drift · 1 234 likes"
    pub subtitle: String,
    /// the album's artists (for links)
    pub artists: Vec<Link>,
    pub cover_url: Option<String>,
    /// a playlist's description, as plain text (#52)
    #[serde(default)]
    pub description: String,
}

/// A name that opens something.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Link {
    pub name: String,
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SavedFlag {
    pub uri: String,
    pub saved: bool,
}

/// A Spotify Connect device.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// `computer`, `smartphone`, `speaker`, `tv`, `tablet`, …
    pub kind: String,
    /// playing (or paused) on this one right now
    pub active: bool,
    /// 0–100, when it has a volume
    pub volume: Option<u8>,
    /// this is Singularity's own Engine
    pub is_engine: bool,
}

/// What a search found, by type (each list in Spotify's relevance order).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SearchResults {
    pub query: String,
    /// the best matches across types
    pub top: Vec<Card>,
    pub tracks: Vec<TrackRow>,
    pub artists: Vec<Card>,
    pub albums: Vec<Card>,
    pub playlists: Vec<Card>,
    pub podcasts: Vec<Card>,
}

/// An artist page: header, popular tracks, and Shelves of releases and related artists.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ArtistPage {
    pub uri: String,
    pub name: String,
    pub monthly_listeners: Option<u64>,
    pub image_url: Option<String>,
    pub top_tracks: Vec<TrackRow>,
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StreamInfo {
    /// `flac`, `vorbis`, …
    pub codec: String,
    /// nominal bitrate, bits/s
    pub bitrate: u32,
    /// the file's measured average, bits/s (when Soloist reports it)
    pub average_bitrate: Option<u32>,
    pub sample_rate: u32,
    /// what Soloist decodes to, e.g. `32-bit-float-pcm`
    pub sample_format: String,
}

impl StreamInfo {
    pub fn lossless(&self) -> bool {
        self.codec.eq_ignore_ascii_case("flac")
    }
}

/// How well the Engine's stream keeps up (#45), from its own measurements.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Delivery {
    /// the latest download speed, kB/s, and when it was measured (Unix ms)
    pub bandwidth_kbps: Option<u32>,
    pub measured_ms: u64,
    /// when playback ran dry recently (Unix ms, the last two minutes)
    pub stalls_ms: Vec<u64>,
}

impl Delivery {
    /// Stalls in the minute before `now_ms`.
    pub fn stalls_last_minute(&self, now_ms: u64) -> usize {
        self.stalls_ms.iter().filter(|t| now_ms.saturating_sub(**t) <= 60_000).count()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Lyrics {
    pub uri: String,
    /// lines carry start times
    pub synced: bool,
    pub lines: Vec<LyricLine>,
    /// where they came from, for the credit line ("Musixmatch", "LRCLIB")
    pub source: String,
    /// why there are none, when `lines` is empty ("no lyrics on Spotify or LRCLIB")
    #[serde(default)]
    pub missing: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LyricLine {
    pub start_ms: u64,
    pub text: String,
}

/// The local listening log, summed up for the listening Pane. Days are local calendar days.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ListeningSummary {
    /// minutes listened per day, oldest first, the last one today (`LISTENING_DAYS` of them)
    pub minutes: Vec<u32>,
    /// weekday of the last day (0 = Monday), so a heatmap can line its columns up by week
    pub today_weekday: u8,
    pub hours_this_year: f32,
    /// the artist with the most listening time this year
    pub top_artist: Option<String>,
    /// days in a row with any listening, up to today (or yesterday, if nothing yet today)
    pub streak_days: u32,
    /// plays in the whole log
    pub plays: u64,
    /// plays today
    #[serde(default)]
    pub plays_today: u32,
}

/// A calendar period in local time (weeks start on Monday), for stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsPeriod {
    #[default]
    Week,
    Month,
    Year,
    All,
}

impl StatsPeriod {
    pub const ALL: [StatsPeriod; 4] = [StatsPeriod::Week, StatsPeriod::Month, StatsPeriod::Year, StatsPeriod::All];

    pub fn label(self) -> &'static str {
        match self {
            StatsPeriod::Week => "This week",
            StatsPeriod::Month => "This month",
            StatsPeriod::Year => "This year",
            StatsPeriod::All => "All time",
        }
    }

    /// The label when space is short.
    pub fn short(self) -> &'static str {
        match self {
            StatsPeriod::Week => "Week",
            StatsPeriod::Month => "Month",
            StatsPeriod::Year => "Year",
            StatsPeriod::All => "All",
        }
    }
}

/// The listening log for one period: a live "Wrapped". Rankings go by minutes, then plays.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Stats {
    pub period: StatsPeriod,
    /// the local days covered, as days since 1970-01-01 (the first is the period's start, the last today)
    pub from_day: i64,
    pub to_day: i64,
    pub minutes: u32,
    pub plays: u32,
    /// days in a row with a Play, up to today (or yesterday), and the longest run in the period
    pub streak_days: u32,
    pub longest_streak_days: u32,
    /// minutes listened per local hour of the day (24)
    pub by_hour: Vec<u32>,
    pub top_artists: Vec<StatsRow>,
    pub top_tracks: Vec<StatsRow>,
    pub top_albums: Vec<StatsRow>,
}

/// One ranked artist, track or album.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StatsRow {
    pub name: String,
    /// its own link, when the log has it (plays before 2026-10-18 lack album and artist links)
    pub uri: Option<String>,
    /// a track of it, to find the link through when `uri` is missing (and what a track row plays)
    pub track_uri: String,
    /// the artist, for tracks and albums
    pub subtitle: String,
    pub minutes: u32,
    pub plays: u32,
    /// tracks: plays in the whole log ("played N times")
    pub all_time_plays: u32,
}

/// How many days `ListeningSummary::minutes` covers: 26 weeks.
pub const LISTENING_DAYS: usize = 182;

/// What the Daemon knows about its own health (the rest comes from the status events).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Health {
    /// where the Soloist key lives: `keyring`, a file path, or `missing`
    pub soloist_key: String,
    /// where the Spotify login lives: `keyring`, a file path, or `missing`
    pub spotify_login: String,
    /// the PipeWire output the Engine plays to, if it's playing
    pub audio_output: Option<String>,
    /// days until this Soloist build expires (the Daemon updates it well before)
    pub engine_expires_in_days: Option<u32>,
    /// when the installed Soloist build was published (`2026-10-16`)
    #[serde(default)]
    pub engine_build: Option<String>,
}

/// What the AudioTap heard in the last ~33 ms. `bands`, `level` and `vu` are relative to the track's own recent
/// loudness (automatic gain), so Visuals look the same at any volume; the `out_*` meters follow the volume.
/// All values 0–255.
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
    /// the real output level, without the automatic gain (so it follows the volume): loudness, left/right peaks,
    /// and bass / mid / treble, each on a fixed 60 dB scale ending at full scale
    #[serde(default)]
    pub out_level: u8,
    #[serde(default)]
    pub out_vu: [u8; 2],
    #[serde(default)]
    pub out_bands: [u8; 3],
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

    #[test]
    fn list_carrying_events_encode() {
        // internally tagged enums can't hold a bare sequence; this caught Devices once
        let e = Event::Devices { devices: vec![Device::default()] };
        let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }
}
