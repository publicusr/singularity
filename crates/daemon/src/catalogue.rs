//! PrivateCatalogue (ADR-0002): Spotify's private APIs through a librespot session.
//!
//! Login is the OAuth device flow with Spotify's desktop client id (as librespot/spotatui do): the user
//! approves a code at spotify.com/pair; this program never sees a password. Reusable credentials are
//! cached under `$XDG_DATA_HOME/singularity/catalogue` (owner-only).
//! Rules: metadata only, no crawling, back off on errors, never retry in a loop.

use crate::queries::QueryRegistry;
use futures_util::{FutureExt, StreamExt};
use librespot_core::{authentication::Credentials, cache::Cache, config::SessionConfig, error::ErrorKind, session::Session};
use librespot_oauth::DeviceAuthClientBuilder;
use serde_json::{Value, json};
use crate::connect::{ConnectRemote, QueueEdit};
use crate::engine::Identity;
use crate::library;
use singularity_proto::{Card, CatalogueStatus, Command, EngineStatus, HomeFeed, Library, Shelf, TrackPage};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[cfg(feature = "fake")]
mod fake;
#[cfg(feature = "fake")]
pub use fake::spawn_fake;

const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const SCOPES: [&str; 8] = [
    "streaming",
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "user-library-read",
    "user-library-modify",
    "user-read-private",
    "playlist-read-private",
];

#[derive(Debug)]
pub enum CatCmd {
    Login,
    Logout,
    Home,
    Library,
    Tracks(String),
    /// a show's next episodes (#59)
    MoreTracks { uri: String, offset: u32 },
    /// an episode's show (#59)
    FindShow(String),
    PlayFrom { context_uri: String, index: u32, track_uri: Option<String> },
    Queue(QueueEdit),
    /// look up names, artists and durations for these track URIs (Soloist sometimes sends bare URIs)
    Describe(Vec<String>),
    /// lyrics for a track; `meta` (name, artists, album, duration) helps the LRCLIB fallback
    Lyrics { uri: String, meta: Option<singularity_proto::Item> },
    Artist(String),
    Search(String),
    Devices,
    Transfer(String),
    /// Jam (D64): start one hosted on the Engine's device, end the one this account runs
    StartJam,
    EndJam,
    /// a transport command for the device that's playing (not our Engine)
    Remote(Command),
    SetSaved { uri: String, saved: bool },
    CheckSaved(Vec<String>),
    /// a new playlist of your own (#51), with these tracks in it (#53)
    CreatePlaylist { name: String, uris: Vec<String> },
    /// tracks at the end of one of your playlists, unless it has them already (or `anyway`) (#53)
    AddToPlaylist { playlist: String, name: String, uris: Vec<String>, anyway: bool },
    /// rows out of one of your playlists, by uid (#54)
    RemoveFromPlaylist { playlist: String, uids: Vec<String> },
    /// a new cover (a JPEG) for one of your playlists, or None for Spotify's mosaic (#56)
    SetCover { playlist: String, jpeg: Option<Vec<u8>> },
    /// rows of one of your playlists to a new place (#55)
    MoveInPlaylist { playlist: String, uids: Vec<String>, before: Option<String>, after: Option<String> },
    /// one of your playlists out of the library (#51)
    DeletePlaylist(String),
    /// a new name and description for one of your playlists (#52)
    EditPlaylist { uri: String, name: String, description: String },
    /// the Engine restarted (a Soloist update): start the context again where it was, on our device
    Resume(crate::engine::Resume),
    /// a command for the Engine when it's the Spotify app (D51): sent to its Connect device
    Engine(Command),
    /// one raw private-API query, answered on `reply` (the library backup, one request at a time)
    Query { op: String, vars: Value, reply: tokio::sync::oneshot::Sender<anyhow::Result<Value>> },
}

#[derive(Debug, Clone)]
pub enum CatEvent {
    Status(CatalogueStatus),
    Home(HomeFeed),
    Library(Library),
    Tracks(TrackPage),
    /// the Catalogue couldn't do it; the Engine should (e.g. play the track alone)
    ToEngine(Command),
    Described(Vec<singularity_proto::Item>),
    Lyrics(singularity_proto::Lyrics),
    /// send the Clients the last playback state again (an edit they showed early failed)
    Resync,
    Artist(singularity_proto::ArtistPage),
    Search(singularity_proto::SearchResults),
    Devices(Vec<singularity_proto::Device>),
    /// the account's Jam, or none (D64)
    Jam(Option<singularity_proto::Jam>),
    /// another device's playback (None: our Engine is the active device again)
    Remote(Option<singularity_proto::PlaybackState>),
    Saved(Vec<singularity_proto::SavedFlag>),
    /// a playlist was created (#51)
    Created { uri: String, name: String },
    /// a playlist's details were saved, or refused (`ok: false`)
    Edited { uri: String, name: String, description: String, ok: bool },
    /// how adding tracks to a playlist went
    Added { playlist: String, name: String, uris: Vec<String>, outcome: singularity_proto::AddOutcome },
    /// rows taken out of a playlist, or refused
    Removed { playlist: String, uids: Vec<String>, ok: bool },
    /// a cover set (or put back), or refused
    CoverSet { playlist: String, ok: bool },
    /// rows moved, or refused
    Moved { playlist: String, uids: Vec<String>, ok: bool },
    /// the show an episode belongs to (#59)
    EpisodeShow { episode: String, show: singularity_proto::Link },
    /// what the Engine does, when it's the Spotify app (D51): read from the Connect cluster
    Engine(crate::engine::EngineEvent),
    /// these URIs couldn't be looked up: the Daemon clears its "being asked" marks so they're asked for again later
    /// (A8: a failed lookup used to wedge them for the Daemon's life)
    DescribeFailed(Vec<String>),
    /// a background task inside the Catalogue stopped, or has been running again for a while (the health view
    /// names it in between, A1)
    TaskStopped(&'static str, String),
    TaskStarted(&'static str),
    Error(String),
}

/// Track lists stay fresh this long: reopening a playlist doesn't refetch it.
const TRACKS_TTL: Duration = Duration::from_secs(10 * 60);
const SEARCH_TTL: Duration = Duration::from_secs(300);
/// How many entries each cache keeps (#71: they grew for as long as the Daemon ran). Lists and artist pages are the
/// big ones (a long playlist is a few MB); names and lyrics are small.
const MAX_LISTS: usize = 40;
const MAX_PAGES: usize = 60;
pub const MAX_DESCRIBED: usize = 2000;
const MAX_LYRICS: usize = 300;

/// Make room for one more entry in a timed cache: what has expired goes, then the oldest, down to `max - 1`.
pub fn trim_timed<V>(map: &mut HashMap<String, (Instant, V)>, ttl: Duration, max: usize) {
    map.retain(|_, (at, _)| at.elapsed() < ttl);
    while map.len() >= max {
        let Some(oldest) = map.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone()) else { break };
        map.remove(&oldest);
    }
}

/// What a `Describe` batch does, decided before any request (A8): names already known are answered at once, URIs
/// that failed a moment ago are reported failed again (so the Daemon clears its "being asked" mark and can ask later),
/// and every other URI is looked up — all of them, however big the batch (it used to stop at 20, silently).
fn describe_plan(uris: Vec<String>, described: &HashMap<String, singularity_proto::Item>, failed: &HashMap<String, Instant>) -> (Vec<singularity_proto::Item>, Vec<String>, Vec<String>) {
    let (mut known, mut ask, mut bad) = (vec![], vec![], vec![]);
    for uri in uris {
        if let Some(item) = described.get(&uri) {
            known.push(item.clone());
        } else if failed.get(&uri).is_some_and(|at| at.elapsed() < DESCRIBE_RETRY) {
            bad.push(uri);
        } else {
            ask.push(uri);
        }
    }
    (known, ask, bad)
}

/// Whether the library walk asks for another page (B10/C15). `totalCount` comes from Spotify, so it isn't trusted:
/// a broken one near `u32::MAX` with non-empty pages used to mean millions of sequential requests.
fn library_page_continues(page_len: usize, offset: u32, total: u32, have: usize) -> bool {
    page_len > 0 && offset < total && have < MAX_TRACKS as usize
}

/// Make room for one more entry in a cache of timestamps (a negative cache): what's older than `ttl` goes, then the
/// oldest.
pub fn trim_ages(map: &mut HashMap<String, Instant>, ttl: Duration, max: usize) {
    map.retain(|_, at| at.elapsed() < ttl);
    while map.len() >= max {
        let Some(oldest) = map.iter().min_by_key(|(_, at)| **at).map(|(k, _)| k.clone()) else { break };
        map.remove(&oldest);
    }
}

/// Make room for one more entry in a cache without ages: any entries go (they can be asked for again).
pub fn trim_any<V>(map: &mut HashMap<String, V>, max: usize) {
    if map.len() >= max {
        let drop: Vec<String> = map.keys().take(map.len() + 1 - max).cloned().collect();
        for k in drop {
            map.remove(&k);
        }
    }
}
/// Longest track list fetched (Liked Songs can be huge; pages are sequential and never retried in a loop).
const MAX_TRACKS: u32 = 10_000;

/// A track Spotify couldn't describe isn't asked about again for this long: a queue full of them must not mean a
/// lookup on every playback update, and the track gets another chance later (A8's retry limit).
const DESCRIBE_RETRY: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub struct Dirs {
    pub credentials: PathBuf,
    pub queries: PathBuf,
}

impl Dirs {
    pub fn from_env() -> Dirs {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
        let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".cache"));
        Dirs { credentials: data.join("singularity/catalogue"), queries: cache.join("singularity/pathfinder-queries.json") }
    }
}

/// The Catalogue, for an Engine that is `engine` on Connect.
pub fn spawn(dirs: Dirs, engine: Identity) -> (mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let (ev_tx, ev_rx) = mpsc::channel(64);
    tokio::spawn(supervise(dirs, engine, cmd_rx, ev_tx, cmd_tx.clone()));
    (cmd_tx, ev_rx)
}

/// Run the Catalogue, and keep it running: a panic would otherwise leave a Daemon that browses nothing, says
/// "logged in" and never answers again (A1/C2). The command channel survives a restart, so Clients keep working.
async fn supervise(dirs: Dirs, engine: Identity, mut cmds: mpsc::Receiver<CatCmd>, ev: mpsc::Sender<CatEvent>, self_tx: mpsc::Sender<CatCmd>) {
    let mut backoff = Duration::from_secs(2);
    loop {
        let began = Instant::now();
        // the health row goes once the Catalogue has run for a while (one that keeps dying keeps its row)
        let back = tokio::spawn({
            let ev = ev.clone();
            async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let _ = ev.send(CatEvent::TaskStarted("the Catalogue")).await;
            }
        });
        let done = std::panic::AssertUnwindSafe(run(dirs.clone(), engine.clone(), &mut cmds, ev.clone(), self_tx.clone())).catch_unwind().await;
        back.abort();
        match done {
            // the Daemon dropped its end: we're shutting down
            Ok(()) => return,
            Err(_) => {
                if began.elapsed() >= Duration::from_secs(60) {
                    backoff = Duration::from_secs(2);
                }
                let _ = ev.send(CatEvent::Status(CatalogueStatus::Unavailable { message: "the Catalogue stopped; it's starting again".into() })).await;
                let _ = ev.send(CatEvent::TaskStopped("the Catalogue", "it panicked".into())).await;
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(300));
            }
        }
    }
}

/// The commands that arrived behind a search: the newest search replaces this one, everything else waits its turn.
/// (A2/C4: they used to be sent back into the channel this task alone reads, which deadlocked it when it was full
/// and ran them after the search in any case.)
fn drain_for_search(mut q: String, cmds: &mut mpsc::Receiver<CatCmd>) -> (String, VecDeque<CatCmd>) {
    let mut later = VecDeque::new();
    while let Ok(c) = cmds.try_recv() {
        match c {
            CatCmd::Search(n) => q = n,
            other => later.push_back(other),
        }
    }
    (q, later)
}

struct State {
    dirs: Dirs,
    /// this computer's Engine on Connect
    engine: Identity,
    session: Option<Session>,
    registry: QueryRegistry,
    ev: mpsc::Sender<CatEvent>,
    remote: Option<ConnectRemote>,
    tracks: HashMap<String, (Instant, Vec<TrackPage>)>,
    /// shows found to be audiobooks (#62): their pages come from `queryBookChapters`
    books: std::collections::HashSet<String>,
    described: HashMap<String, singularity_proto::Item>,
    /// URIs whose lookup failed, and when: not asked about again for `DESCRIBE_RETRY` (A8)
    describe_failed: HashMap<String, Instant>,
    lyrics: HashMap<String, singularity_proto::Lyrics>,
    artists: HashMap<String, (Instant, singularity_proto::ArtistPage)>,
    searches: HashMap<String, (Instant, singularity_proto::SearchResults)>,
    /// the latest Connect cluster, kept fresh by the dealer
    cluster: std::sync::Arc<std::sync::Mutex<Option<librespot_protocol::connect::Cluster>>>,
    /// told whenever a new cluster arrives (the Spotify app as Engine waits on it for its device)
    cluster_changed: std::sync::Arc<tokio::sync::Notify>,
}

async fn run(dirs: Dirs, engine: Identity, cmds: &mut mpsc::Receiver<CatCmd>, ev: mpsc::Sender<CatEvent>, self_tx: mpsc::Sender<CatCmd>) {
    let registry = QueryRegistry::new(dirs.queries.clone());
    let mut st = State { dirs, engine, session: None, registry, ev, remote: None, tracks: HashMap::new(), books: Default::default(), described: HashMap::new(), describe_failed: HashMap::new(), lyrics: HashMap::new(), artists: HashMap::new(), searches: HashMap::new(), cluster: Default::default(), cluster_changed: Default::default() };
    // resume a previous login
    if saved_credentials(&st.dirs).is_some() {
        if st.connect(None).await {
            let _ = self_tx.send(CatCmd::Home).await;
            let _ = self_tx.send(CatCmd::Library).await;
            let _ = self_tx.send(CatCmd::Devices).await;
        }
    } else {
        st.status(CatalogueStatus::LoggedOut).await;
    }
    // commands drained from behind a search, in their original order
    let mut waiting: VecDeque<CatCmd> = VecDeque::new();
    loop {
        let cmd = match waiting.pop_front() {
            Some(c) => c,
            None => match cmds.recv().await {
                Some(c) => c,
                None => return,
            },
        };
        if let CatCmd::Search(q) = cmd {
            // while typing, only the newest search matters; whatever else waited ran before the search was asked for
            let (q, later) = drain_for_search(q, cmds);
            st.search(q).await;
            waiting.extend(later);
            continue;
        }
        handle(&mut st, cmd, &self_tx).await;
    }
}

async fn handle(st: &mut State, cmd: CatCmd, self_tx: &mpsc::Sender<CatCmd>) {
    match cmd {
        CatCmd::Login => {
            if st.login().await {
                let _ = self_tx.send(CatCmd::Home).await;
                let _ = self_tx.send(CatCmd::Library).await;
                let _ = self_tx.send(CatCmd::Devices).await;
            }
        }
        CatCmd::Logout => st.logout().await,
        CatCmd::Home => st.home().await,
        CatCmd::Library => st.library().await,
        CatCmd::Tracks(uri) => st.track_list(uri).await,
        CatCmd::MoreTracks { uri, offset } => st.more_tracks(uri, offset).await,
        CatCmd::FindShow(uri) => st.find_show(uri).await,
        CatCmd::PlayFrom { context_uri, index, track_uri } => st.play_from(context_uri, index, track_uri).await,
        CatCmd::Queue(edit) => st.queue(edit).await,
        CatCmd::Describe(uris) => st.describe(uris).await,
        CatCmd::Lyrics { uri, meta } => st.lyrics(uri, meta).await,
        CatCmd::Artist(uri) => st.artist(uri).await,
        CatCmd::Devices => st.devices().await,
        CatCmd::Transfer(id) => st.transfer(id).await,
        CatCmd::StartJam => st.start_jam().await,
        CatCmd::EndJam => st.end_jam().await,
        CatCmd::Resume(r) => st.resume(r).await,
        CatCmd::Remote(cmd) => st.remote_command(cmd).await,
        CatCmd::Engine(cmd) => st.engine_command(cmd).await,
        CatCmd::SetSaved { uri, saved } => {
            // the library changed: the sidebar and Liked Songs follow (Liked Songs from its next open)
            if st.set_saved(uri.clone(), saved).await && !uri.starts_with("spotify:track:") {
                let _ = self_tx.send(CatCmd::Library).await;
            }
        }
        CatCmd::CheckSaved(uris) => st.check_saved(uris).await,
        // either way the sidebar shows the truth afterwards
        CatCmd::CreatePlaylist { name, uris } => {
            st.create_playlist(name, uris).await;
            let _ = self_tx.send(CatCmd::Library).await;
        }
        CatCmd::AddToPlaylist { playlist, name, uris, anyway } => st.add_to_playlist(playlist, name, uris, anyway).await,
        CatCmd::RemoveFromPlaylist { playlist, uids } => st.remove_from_playlist(playlist, uids).await,
        CatCmd::MoveInPlaylist { playlist, uids, before, after } => st.move_in_playlist(playlist, uids, before, after).await,
        CatCmd::SetCover { playlist, jpeg } => {
            if st.set_cover(playlist, jpeg).await {
                let _ = self_tx.send(CatCmd::Library).await;
            }
        }
        CatCmd::DeletePlaylist(uri) => {
            st.delete_playlist(uri).await;
            let _ = self_tx.send(CatCmd::Library).await;
        }
        CatCmd::EditPlaylist { uri, name, description } => {
            if st.edit_playlist(uri, name, description).await {
                let _ = self_tx.send(CatCmd::Library).await;
            }
        }
        CatCmd::Query { op, vars, reply } => {
            let _ = reply.send(st.query(&op, vars).await);
        }
        // handled by `run` itself (it drains what came behind a search)
        CatCmd::Search(_) => {}
    }
}

fn remote_cluster(c: &std::sync::Mutex<Option<librespot_protocol::connect::Cluster>>) -> anyhow::Result<librespot_protocol::connect::Cluster> {
    c.lock().expect("cluster lock").clone().ok_or_else(|| anyhow::anyhow!("no Connect state yet"))
}

/// The latest cluster's device names by id, for naming a Jam's host, and the Engine's own id.
fn jam_devices(latest: &std::sync::Mutex<Option<librespot_protocol::connect::Cluster>>, engine: &Identity) -> (HashMap<String, String>, Option<String>) {
    let latest = latest.lock().expect("cluster lock");
    let names: HashMap<String, String> = latest.iter().flat_map(|c| c.device.iter()).map(|(id, d)| (id.clone(), d.name.clone())).collect();
    let engine_id = names.iter().find(|(_, n)| **n == engine.device_name).map(|(id, _)| id.clone());
    (names, engine_id)
}

/// Tell the Daemon what a cluster says: the devices, and whether another device is the one playing. For the Spotify
/// app as Engine (D51), also the Engine's own state: whether its device is there and active, what it plays, how well.
async fn emit_cluster(ev: &mpsc::Sender<CatEvent>, c: &librespot_protocol::connect::Cluster, engine: &Identity) {
    let now = singularity_proto::now_ms();
    let _ = ev.send(CatEvent::Devices(crate::connect::devices(c, &engine.device_name))).await;
    let _ = ev.send(CatEvent::Remote(crate::connect::remote_playback(c, &engine.device_name, now))).await;
    if !engine.via_connect {
        return;
    }
    for e in app_engine_events(c, engine, now) {
        let _ = ev.send(CatEvent::Engine(e)).await;
    }
}

/// The Spotify app's state as Engine events: its status (found on Connect? the active device?), and while it's the
/// active device its playback and stream quality; idle while no device plays.
pub fn app_engine_events(c: &librespot_protocol::connect::Cluster, engine: &Identity, now: u64) -> Vec<crate::engine::EngineEvent> {
    use crate::engine::EngineEvent;
    let name = &engine.device_name;
    let present = c.device.values().any(|d| d.name == *name);
    let active = c.device.get(&c.active_device_id).map(|d| d.name == *name);
    let status = if present {
        EngineStatus::Ready { device_name: name.clone(), active: active == Some(true), version: engine.version.clone(), expires_in_days: None }
    } else {
        let check = if engine.player == "the Spotify app" { ", and check the Spotify app is logged in to this account" } else { "" };
        EngineStatus::Failed { message: format!("\"{name}\" isn't on Spotify Connect: Ctrl+K → \"Restart the player\"{check}") }
    };
    let mut out = vec![EngineEvent::Status(status)];
    match active {
        Some(true) => {
            out.extend(crate::connect::engine_playback(c, name, now).map(EngineEvent::Playback));
            out.extend(crate::connect::stream_quality(c).map(EngineEvent::Stream));
        }
        // nothing plays anywhere: the app is idle too (another device playing is `Remote`)
        None => out.push(EngineEvent::Playback(singularity_proto::PlaybackState { timestamp_ms: now, ..Default::default() })),
        Some(false) => {}
    }
    out
}

/// The reusable Spotify login: keyring first, then librespot's cache file (older installs, or no keyring).
pub fn saved_credentials(dirs: &Dirs) -> Option<Credentials> {
    if let Some(json) = crate::secrets::get("spotify")
        && let Ok(c) = serde_json::from_str(&json)
    {
        return Some(c);
    }
    cache(dirs).ok().and_then(|c| c.credentials())
}

/// librespot writes the reusable login to its cache file after connecting; move it into the keyring (spec D24).
/// Without a keyring it stays where it is, owner-only.
fn keep_credentials_in_keyring(dirs: &Dirs) {
    let file = dirs.credentials.join("credentials.json");
    let Ok(json) = std::fs::read_to_string(&file) else { return };
    if let Ok(crate::secrets::Stored::Keyring) = crate::secrets::set("spotify", json.trim()) {
        let _ = std::fs::remove_file(&file);
    } else {
        crate::secrets::clear("spotify"); // the fallback copy would only duplicate the cache file
    }
}

fn cache(dirs: &Dirs) -> anyhow::Result<Cache> {
    std::fs::create_dir_all(&dirs.credentials)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dirs.credentials, std::fs::Permissions::from_mode(0o700))?;
    Ok(Cache::new(Some(&dirs.credentials), None, None, None)?)
}

impl State {
    /// An event for the Daemon; if its channel is gone the event is lost, and at least the log says so (Q11).
    async fn emit(&self, ev: CatEvent) {
        if self.ev.send(ev).await.is_err() {
            tracing::debug!("the Daemon is gone; an event was dropped");
        }
    }

    /// An error for the Clients; when even that can't be delivered, the log keeps the message (Q11).
    async fn fail(&self, message: String) {
        if self.ev.send(CatEvent::Error(message.clone())).await.is_err() {
            tracing::warn!("no one heard: {message}");
        }
    }

    async fn status(&self, s: CatalogueStatus) {
        let _ = self.ev.send(CatEvent::Status(s)).await;
    }

    async fn login(&mut self) -> bool {
        let client = match DeviceAuthClientBuilder::new(CLIENT_ID, SCOPES.to_vec()).build() {
            Ok(c) => c,
            Err(e) => {
                self.status(CatalogueStatus::Unavailable { message: format!("login unavailable: {e}") }).await;
                return false;
            }
        };
        let auth = match client.request_device_code_async().await {
            Ok(a) => a,
            Err(e) => {
                self.status(CatalogueStatus::Unavailable { message: format!("login unavailable: {e}") }).await;
                return false;
            }
        };
        self.status(CatalogueStatus::LoginPending { url: auth.url().to_string(), code: auth.user_code().to_string() }).await;
        match client.poll_for_token_async(&auth).await {
            Ok(token) => self.connect(Some(Credentials::with_access_token(token.access_token))).await,
            Err(e) => {
                self.status(CatalogueStatus::LoggedOut).await;
                self.fail(format!("Spotify login failed: {e}")).await;
                false
            }
        }
    }

    /// Connect with fresh credentials, or the cached reusable ones.
    async fn connect(&mut self, fresh: Option<Credentials>) -> bool {
        self.status(CatalogueStatus::Connecting).await;
        let cache = match cache(&self.dirs) {
            Ok(c) => c,
            Err(e) => {
                self.status(CatalogueStatus::Unavailable { message: e.to_string() }).await;
                return false;
            }
        };
        let Some(creds) = fresh.or_else(|| saved_credentials(&self.dirs)) else {
            self.status(CatalogueStatus::LoggedOut).await;
            return false;
        };
        let session = Session::new(SessionConfig::default(), Some(cache));
        if let Err(e) = session.connect(creds, true).await {
            self.status(CatalogueStatus::Unavailable { message: format!("couldn't reach Spotify: {e}") }).await;
            return false;
        }
        // the account type and country arrive shortly after login
        let mut product = None;
        for _ in 0..30 {
            product = session.get_user_attribute("type");
            if product.is_some() && !session.country().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let premium = product.as_deref() == Some("premium");
        keep_credentials_in_keyring(&self.dirs);
        self.status(CatalogueStatus::Ready { username: session.username(), country: session.country(), premium }).await;
        self.session = Some(session);
        true
    }

    async fn logout(&mut self) {
        self.remote = None;
        self.tracks.clear();
        if let Some(s) = self.session.take() {
            s.shutdown();
        }
        let _ = std::fs::remove_file(self.dirs.credentials.join("credentials.json"));
        crate::secrets::clear("spotify");
        self.status(CatalogueStatus::LoggedOut).await;
    }

    async fn session(&mut self) -> Option<Session> {
        if self.session.as_ref().is_some_and(|s| s.is_invalid()) {
            tracing::info!("catalogue session ended; reconnecting");
            self.session = None;
            // a new session needs a new dealer, so a new remote
            self.remote = None;
            self.connect(None).await;
        }
        self.session.clone()
    }

    /// Run a persisted query; on a retired hash (412), refresh the registry and retry once.
    async fn query(&mut self, op: &str, vars: Value) -> anyhow::Result<Value> {
        let session = self.session().await.ok_or_else(|| anyhow::anyhow!("not logged in to Spotify"))?;
        let hash = self.registry.hash(op).await?;
        // a request that hangs is asked once more: the stuck connection went with the abandoned request
        let first = match crate::spotify(session.spclient().pathfinder_query(op, &hash, vars.clone())).await {
            Err(e) if e.kind == ErrorKind::DeadlineExceeded => {
                tracing::warn!("`{op}`: {e}; asking again");
                crate::spotify(session.spclient().pathfinder_query(op, &hash, vars.clone())).await
            }
            r => r,
        };
        match first {
            Err(e) if e.kind == ErrorKind::FailedPrecondition => {
                tracing::warn!("`{op}` hash retired; refreshing the query registry");
                self.registry.refresh().await?;
                let hash = self.registry.hash(op).await?;
                Ok(crate::spotify(session.spclient().pathfinder_query(op, &hash, vars)).await?)
            }
            r => Ok(r?),
        }
    }

    async fn library(&mut self) {
        match self.library_items().await {
            Ok(items) => {
                let _ = self.ev.send(CatEvent::Library(Library { items })).await;
            }
            Err(e) => {
                self.fail(format!("Library unavailable: {e}")).await;
            }
        }
    }

    /// The top level, with each folder's contents spliced in right after it (one level deeper).
    async fn library_items(&mut self) -> anyhow::Result<Vec<singularity_proto::LibraryItem>> {
        let mut items = self.library_level(None, 0).await?;
        let mut i = 0;
        while i < items.len() {
            // folders nest a few levels at most; guard anyway
            if items[i].kind == "folder" && items[i].depth < 4 {
                let children = self.library_level(Some(&items[i].uri.clone()), items[i].depth + 1).await?;
                items.splice(i + 1..i + 1, children);
            }
            i += 1;
        }
        Ok(items)
    }

    async fn library_level(&mut self, folder: Option<&str>, depth: u8) -> anyhow::Result<Vec<singularity_proto::LibraryItem>> {
        let mut items = vec![];
        let mut offset = 0;
        loop {
            let me = self.session.as_ref().map(|s| s.username()).unwrap_or_default();
            let page = library::parse_library(&self.query("libraryV3", library::library_vars(offset, folder)).await?, depth, &me);
            let n = page.items.len();
            items.extend(page.items);
            offset = offset.saturating_add(library::LIBRARY_PAGE);
            if !library_page_continues(n, offset, page.total, items.len()) {
                return Ok(items);
            }
        }
    }

    /// Fetch a track list page by page, sending each as it arrives (so long lists show at once). A show sends its
    /// newest `SHOW_BATCH` episodes; `more_tracks` fetches the next ones.
    async fn track_list(&mut self, uri: String) {
        if let Some((at, pages)) = self.tracks.get(&uri)
            && at.elapsed() < TRACKS_TTL
        {
            for p in pages.clone() {
                let _ = self.ev.send(CatEvent::Tracks(p)).await;
            }
            return;
        }
        let until = if uri.starts_with("spotify:show:") { library::SHOW_BATCH } else { MAX_TRACKS };
        if let Some(pages) = self.fetch_pages(&uri, 0, until).await {
            trim_timed(&mut self.tracks, TRACKS_TTL, MAX_LISTS);
            self.tracks.insert(uri, (Instant::now(), pages));
        }
    }

    /// A show's next episodes from `offset` (#59), added to what's cached.
    async fn more_tracks(&mut self, uri: String, offset: u32) {
        let cached = self.tracks.get(&uri).map(|(_, p)| p.iter().map(|p| p.offset + p.tracks.len() as u32).max().unwrap_or(0));
        // pages already here are sent again (cheap); the rest are fetched
        if let Some((_, pages)) = self.tracks.get(&uri) {
            for p in pages.iter().filter(|p| p.offset >= offset).cloned().collect::<Vec<_>>() {
                let _ = self.ev.send(CatEvent::Tracks(p)).await;
            }
        }
        let from = cached.unwrap_or(0).max(offset);
        if !uri.starts_with("spotify:show:") || from >= offset + library::SHOW_BATCH {
            return;
        }
        if let Some(pages) = self.fetch_pages(&uri, from, offset + library::SHOW_BATCH).await
            && let Some((_, kept)) = self.tracks.get_mut(&uri)
        {
            kept.extend(pages);
        }
    }

    /// Pages from `from` until `until` (or the list's end), each sent as it arrives; None if one failed.
    async fn fetch_pages(&mut self, uri: &str, from: u32, until: u32) -> Option<Vec<TrackPage>> {
        let mut pages = vec![];
        let mut offset = from;
        loop {
            let asked = if self.books.contains(uri) { Some(library::chapters_query(uri, offset)) } else { library::tracks_query(uri, offset) };
            let Some((mut op, vars)) = asked else {
                self.fail(format!("can't list the tracks of {uri}")).await;
                return None;
            };
            let mut answer = self.query(op, vars).await;
            // a show that turns out to be a book: its chapters instead (#62)
            if op == "queryPodcastEpisodes"
                && answer.as_ref().is_ok_and(library::is_book)
            {
                self.books.insert(uri.to_string());
                let (o, vars) = library::chapters_query(uri, offset);
                op = o;
                answer = self.query(op, vars).await;
            }
            let v = match answer {
                Ok(v) => v,
                Err(e) => {
                    self.fail(format!("Track list unavailable: {e}")).await;
                    return None;
                }
            };
            let (tracks, total) = library::parse_tracks(uri, &v);
            let page_len = match op {
                "getAlbum" => library::ALBUM_PAGE,
                "queryPodcastEpisodes" | "queryBookChapters" => library::SHOW_PAGE,
                _ => library::TRACK_PAGE,
            };
            let header = match offset {
                0 if uri.starts_with("spotify:show:") => {
                    let vars = json!({ "uri": uri, "includeContentCapabilityTrait": false, "includeEpisodeContentRatingsV2": false });
                    self.query("queryShowMetadataV2", vars).await.ok().and_then(|m| library::parse_show_header(&m))
                }
                0 => library::parse_header(uri, &v),
                _ => None,
            };
            let page = TrackPage { uri: uri.to_string(), offset, total, tracks, header };
            let _ = self.ev.send(CatEvent::Tracks(page.clone())).await;
            pages.push(page);
            offset += page_len;
            if offset >= total.min(until) {
                return Some(pages);
            }
        }
    }

    /// ConnectRemote (started on first use) and Soloist's Connect id.
    async fn remote(&mut self) -> anyhow::Result<(Session, &mut ConnectRemote, String)> {
        let session = self.session().await.ok_or_else(|| anyhow::anyhow!("not logged in to Spotify"))?;
        if self.remote.is_none() {
            let (remote, cluster, mut updates, mut ids) = ConnectRemote::start(session.clone()).await?;
            self.remote = Some(remote);
            *self.cluster.lock().expect("cluster lock") = Some(cluster.clone());
            emit_cluster(&self.ev, &cluster, &self.engine).await;
            // the dealer pushes every change; keep the latest and tell the Daemon. After a reconnect (a network
            // change) it has a new connection id, which must be registered again, or the pushes stop.
            let (latest, ev, engine, changed) = (self.cluster.clone(), self.ev.clone(), self.engine.clone(), self.cluster_changed.clone());
            let session = session.clone();
            // every change to the account's Jam (D64): read it again, the update itself is loose JSON
            let mut jam_updates = session.dealer().listen_for(crate::jam::UPDATE_TOPIC, |_| -> Result<(), librespot_core::Error> { Ok(()) })?;
            tokio::spawn(async move {
                loop {
                    let c = tokio::select! {
                        u = jam_updates.next() => match u {
                            Some(_) => {
                                let (names, engine_id) = jam_devices(&latest, &engine);
                                match crate::jam::current(&session, &names, engine_id.as_deref()).await {
                                    Ok(jam) => {
                                        let _ = ev.send(CatEvent::Jam(jam)).await;
                                    }
                                    Err(e) => tracing::debug!("Jam: couldn't read it after an update: {e}"),
                                }
                                continue;
                            }
                            None => return,
                        },
                        u = updates.next() => match u {
                            Some(Ok(u)) => match u.cluster.into_option() {
                                Some(c) => c,
                                None => continue,
                            },
                            Some(Err(_)) => continue,
                            None => return,
                        },
                        id = ids.next() => match id {
                            Some(Ok(id)) => match crate::connect::reconnected(&session, &id).await {
                                Ok(c) => {
                                    tracing::info!("Connect: registered again after a reconnect");
                                    c
                                }
                                Err(e) => {
                                    tracing::warn!("Connect: couldn't register again after a reconnect: {e}");
                                    continue;
                                }
                            },
                            Some(Err(_)) => continue,
                            None => return,
                        },
                    };
                    *latest.lock().expect("cluster lock") = Some(c.clone());
                    emit_cluster(&ev, &c, &engine).await;
                    changed.notify_waiters();
                }
            });
        }
        let remote = self.remote.as_mut().expect("just started");
        // the latest cluster shows the device under another id, or not at all: it re-registered (the app restarted,
        // or came back after a network change), so look it up again
        let name = &self.engine.device_name;
        if let Some(cached) = remote.cached_id(name) {
            let latest = self.cluster.lock().expect("cluster lock").as_ref().map(|c| c.device.iter().find(|(_, d)| d.name == *name).map(|(id, _)| id.clone()));
            if latest.is_some_and(|now| now.as_deref() != Some(cached)) {
                remote.forget_ids();
            }
        }
        let to = remote.device_id(name).await?;
        Ok((session, remote, to))
    }

    /// Add to or remove from the library. Returns whether it worked; either way the Daemon hears the truth.
    async fn set_saved(&mut self, uri: String, saved: bool) -> bool {
        let op = if saved { "addToLibrary" } else { "removeFromLibrary" };
        let result = self.query(op, json!({ "libraryItemUris": [uri], "interactionId": null })).await;
        let ok = match &result {
            Ok(v) => v.get("errors").is_none(),
            Err(_) => false,
        };
        if ok {
            // Liked Songs changed: its cached pages are stale now
            if uri.starts_with("spotify:track:") {
                self.tracks.remove(library::LIKED);
            }
        } else {
            let why = result.err().map(|e| e.to_string()).unwrap_or_else(|| "Spotify refused it".into());
            self.fail(format!("Couldn't {} it: {why}", if saved { "save" } else { "remove" })).await;
        }
        // on failure this carries the old value back, so the Client rolls back
        let _ = self.ev.send(CatEvent::Saved(vec![singularity_proto::SavedFlag { uri, saved: if ok { saved } else { !saved } }])).await;
        ok
    }

    /// A new playlist, put at the top of the library; the Client that asked opens it.
    async fn create_playlist(&mut self, name: String, uris: Vec<String>) {
        let Some(session) = self.session().await else { return };
        let made = async {
            let v = crate::playlists::post(&session, "/playlist", &crate::playlists::create(&name)).await?;
            let uri = v["uri"].as_str().ok_or_else(|| anyhow::anyhow!("no URI in the answer"))?.to_string();
            crate::playlists::post(&session, &crate::playlists::rootlist_path(&session.username()), &crate::playlists::rootlist_add(&uri, singularity_proto::now_ms())).await?;
            anyhow::Ok(uri)
        };
        match made.await {
            Ok(uri) => {
                tracing::info!("created playlist {uri}");
                let _ = self.ev.send(CatEvent::Created { uri: uri.clone(), name: name.clone() }).await;
                if !uris.is_empty() {
                    // new: nothing to be a duplicate of
                    self.add_to_playlist(uri, name, uris, true).await;
                }
            }
            Err(e) => {
                self.fail(format!("Couldn't create the playlist: {e}")).await;
            }
        }
    }

    async fn delete_playlist(&mut self, uri: String) {
        let Some(session) = self.session().await else { return };
        match crate::playlists::post(&session, &crate::playlists::rootlist_path(&session.username()), &crate::playlists::rootlist_remove(&uri)).await {
            Ok(_) => tracing::info!("deleted playlist {uri}"),
            Err(e) => {
                self.fail(format!("Couldn't delete the playlist: {e}")).await;
            }
        }
    }

    /// Add `uris` at the end of `playlist`. Unless `anyway`, first ask (one small `editablePlaylists`, by name) whether
    /// it has them already; if that can't tell, add.
    async fn add_to_playlist(&mut self, playlist: String, name: String, uris: Vec<String>, anyway: bool) {
        use singularity_proto::AddOutcome;
        if !anyway {
            match self.query("editablePlaylists", crate::playlists::editable_vars(&name, &uris)).await {
                Ok(v) if crate::playlists::curates(&v, &playlist) == Some(true) => {
                    let _ = self.ev.send(CatEvent::Added { playlist, name, uris, outcome: AddOutcome::AlreadyThere }).await;
                    return;
                }
                Ok(_) => {}
                Err(e) => tracing::debug!("editablePlaylists: {e}"),
            }
        }
        let outcome = match self.query("addToPlaylist", crate::playlists::add_vars(&playlist, &uris)).await {
            Ok(v) => match crate::playlists::graphql_error(&v) {
                None => {
                    // its cached pages are stale now
                    self.tracks.remove(&playlist);
                    AddOutcome::Added
                }
                Some(e) => {
                    self.fail(format!("Couldn't add to {name}: {e}")).await;
                    AddOutcome::Failed
                }
            },
            Err(e) => {
                self.fail(format!("Couldn't add to {name}: {e}")).await;
                AddOutcome::Failed
            }
        };
        let _ = self.ev.send(CatEvent::Added { playlist, name, uris, outcome }).await;
    }

    /// Upload, register and set a cover (or put the mosaic back). True if Spotify took it.
    async fn set_cover(&mut self, playlist: String, jpeg: Option<Vec<u8>>) -> bool {
        let Some(session) = self.session().await else { return false };
        let set = async {
            let changes = crate::playlists::changes_path(&playlist).ok_or_else(|| anyhow::anyhow!("not a playlist"))?;
            let picture = match jpeg {
                Some(jpeg) => {
                    let token = crate::playlists::upload(&session, jpeg).await?;
                    let register = crate::playlists::playlist_path(&playlist, "register-image").unwrap();
                    let v = crate::playlists::post(&session, &register, &json!({ "uploadToken": token })).await?;
                    Some(v["picture"].as_str().ok_or_else(|| anyhow::anyhow!("no picture from the registration"))?.to_string())
                }
                None => None,
            };
            crate::playlists::post(&session, &changes, &crate::playlists::picture(picture.as_deref())).await
        };
        let ok = match set.await {
            Ok(_) => {
                self.tracks.remove(&playlist);
                true
            }
            Err(e) => {
                self.fail(format!("Couldn't change the cover: {e}")).await;
                false
            }
        };
        let _ = self.ev.send(CatEvent::CoverSet { playlist, ok }).await;
        ok
    }

    async fn move_in_playlist(&mut self, playlist: String, uids: Vec<String>, before: Option<String>, after: Option<String>) {
        let vars = crate::playlists::move_vars(&playlist, &uids, before.as_deref(), after.as_deref());
        let ok = match self.query("moveItemsInPlaylist", vars).await {
            Ok(v) => match crate::playlists::graphql_error(&v) {
                None => true,
                Some(e) => {
                    self.fail(format!("Couldn't move it: {e}")).await;
                    false
                }
            },
            Err(e) => {
                self.fail(format!("Couldn't move it: {e}")).await;
                false
            }
        };
        if ok {
            self.tracks.remove(&playlist);
        }
        let _ = self.ev.send(CatEvent::Moved { playlist, uids, ok }).await;
    }

    async fn remove_from_playlist(&mut self, playlist: String, uids: Vec<String>) {
        let ok = match self.query("removeFromPlaylist", crate::playlists::remove_vars(&playlist, &uids)).await {
            Ok(v) => match crate::playlists::graphql_error(&v) {
                None => true,
                Some(e) => {
                    self.fail(format!("Couldn't remove it from the playlist: {e}")).await;
                    false
                }
            },
            Err(e) => {
                self.fail(format!("Couldn't remove it from the playlist: {e}")).await;
                false
            }
        };
        if ok {
            self.tracks.remove(&playlist);
        }
        let _ = self.ev.send(CatEvent::Removed { playlist, uids, ok }).await;
    }

    /// True if Spotify took the new details.
    async fn edit_playlist(&mut self, uri: String, name: String, description: String) -> bool {
        let Some(session) = self.session().await else { return false };
        let saved = async {
            let path = crate::playlists::changes_path(&uri).ok_or_else(|| anyhow::anyhow!("not a playlist"))?;
            crate::playlists::post(&session, &path, &crate::playlists::details(&name, &description)).await
        };
        let ok = match saved.await {
            Ok(_) => {
                // the cached first page carries the old description
                self.tracks.remove(&uri);
                true
            }
            Err(e) => {
                self.fail(format!("Couldn't save the playlist's details: {e}")).await;
                false
            }
        };
        let _ = self.ev.send(CatEvent::Edited { uri, name, description, ok }).await;
        ok
    }

    async fn check_saved(&mut self, uris: Vec<String>) {
        for chunk in uris.chunks(50) {
            match self.query("areEntitiesInLibrary", json!({ "uris": chunk })).await {
                Ok(v) => {
                    let flags = library::parse_saved(chunk, &v);
                    let _ = self.ev.send(CatEvent::Saved(flags)).await;
                }
                Err(e) => {
                    tracing::debug!("areEntitiesInLibrary: {e}");
                    return;
                }
            }
        }
    }

    async fn devices(&mut self) {
        if let Err(e) = self.remote().await {
            tracing::debug!("Connect devices: {e}");
            return;
        }
        let c = self.cluster.lock().expect("cluster lock").clone();
        if let Some(c) = c {
            emit_cluster(&self.ev, &c, &self.engine).await;
        }
        self.refresh_jam().await;
    }

    async fn transfer(&mut self, to: String) {
        let cluster = self.cluster.clone();
        let result = async {
            let (_, remote, _) = self.remote().await?;
            remote.transfer(&remote_cluster(&cluster)?, &to).await
        }
        .await;
        if let Err(e) = result {
            if let Some(r) = self.remote.as_mut() {
                r.forget_ids();
            }
            self.fail(format!("Couldn't move playback: {e}")).await;
        }
    }

    /// Start a Jam hosted on the Engine's own device (D64; verified to work for Soloist's, 2026-10-30).
    async fn start_jam(&mut self) {
        let result = async {
            let (session, _, to) = self.remote().await?;
            let (names, _) = jam_devices(&self.cluster, &self.engine);
            crate::jam::start(&session, &to, &names).await
        }
        .await;
        match result {
            Ok(Some(jam)) => {
                tracing::info!("Jam started on {}", jam.host.as_deref().unwrap_or("an unlisted device"));
                let _ = self.ev.send(CatEvent::Jam(Some(jam))).await;
            }
            Ok(None) => self.fail("Spotify didn't start a Jam (Premium is needed to host one)".into()).await,
            Err(e) => self.fail(format!("Couldn't start a Jam: {e}")).await,
        }
    }

    /// End the Jam this account runs, for everyone; the music keeps playing.
    async fn end_jam(&mut self) {
        let result = async {
            let (session, _, _) = self.remote().await?;
            let (names, engine_id) = jam_devices(&self.cluster, &self.engine);
            match crate::jam::current(&session, &names, engine_id.as_deref()).await? {
                None => Ok(None),
                Some(jam) if !jam.owner => anyhow::bail!("only the one who started it can end it"),
                Some(jam) => crate::jam::end(&session, &jam.session_id).await.map(|()| Some(jam)),
            }
        }
        .await;
        match result {
            Ok(ended) => {
                if ended.is_some() {
                    tracing::info!("Jam ended");
                }
                let _ = self.ev.send(CatEvent::Jam(None)).await;
            }
            Err(e) => self.fail(format!("Couldn't end the Jam: {e}")).await,
        }
    }

    /// Read the account's Jam (on login and when the Devices list is asked for).
    async fn refresh_jam(&mut self) {
        let Some(session) = self.session.clone() else { return };
        let (names, engine_id) = jam_devices(&self.cluster, &self.engine);
        match crate::jam::current(&session, &names, engine_id.as_deref()).await {
            Ok(jam) => {
                let _ = self.ev.send(CatEvent::Jam(jam)).await;
            }
            Err(e) => tracing::debug!("Jam: couldn't read it: {e}"),
        }
    }

    /// After a Soloist update: the restarted device takes a moment to show up on Connect, so this tries a few times,
    /// then falls back to the Engine playing the track alone.
    async fn resume(&mut self, r: crate::engine::Resume) {
        for attempt in 0..4 {
            tokio::time::sleep(Duration::from_millis(if attempt == 0 { 500 } else { 2000 })).await;
            let result = async {
                let (_, remote, to) = self.remote().await?;
                remote.resume(&to, &r, "").await
            }
            .await;
            match result {
                Ok(()) => return,
                Err(e) => {
                    tracing::debug!("resume after the update (attempt {}): {e}", attempt + 1);
                    if let Some(rm) = self.remote.as_mut() {
                        rm.forget_ids();
                    }
                }
            }
        }
        let _ = self.ev.send(CatEvent::ToEngine(Command::Play { uri: Some(r.track_uri) })).await;
        let _ = self.ev.send(CatEvent::ToEngine(Command::Seek { position_ms: r.position_ms })).await;
        if r.paused {
            let _ = self.ev.send(CatEvent::ToEngine(Command::Pause)).await;
        }
    }

    /// Our transport buttons, for the device that's playing.
    async fn remote_command(&mut self, cmd: Command) {
        let active = self.cluster.lock().expect("cluster lock").as_ref().map(|c| c.active_device_id.clone()).filter(|a| !a.is_empty());
        let result = async {
            let to = active.ok_or_else(|| anyhow::anyhow!("no device is playing"))?;
            let (_, remote, _) = self.remote().await?;
            remote.control(&to, &cmd).await
        }
        .await;
        if let Err(e) = result {
            self.fail(format!("Couldn't control the other device: {e}")).await;
        }
    }

    /// An Engine played over Connect (the Spotify app, D51; librespot, D52): its commands go to its Connect device. Failures are reported, never handed
    /// back to the Engine (that's where they came from).
    async fn engine_command(&mut self, cmd: Command) {
        let cluster = self.cluster.clone();
        let result = async {
            let (session, remote, to) = self.engine_device().await?;
            let state = remote_cluster(&cluster).ok();
            let ours = state.as_ref().is_some_and(|c| c.active_device_id == to);
            let ps = state.as_ref().and_then(|c| c.player_state.as_ref());
            let playing = ours && ps.is_some_and(|p| p.is_playing && !p.is_paused);
            match cmd {
                Command::SetSoloistKey { .. } => anyhow::bail!("the Spotify app plays without a Soloist key"),
                Command::TogglePlay if playing => remote.control(&to, &Command::Pause).await,
                // resume what the account played last, on this computer
                Command::TogglePlay | Command::Play { uri: None } if !ours => match state.as_ref().filter(|_| ps.is_some_and(|p| p.track.is_some())) {
                    Some(c) => remote.transfer(c, &to).await,
                    None => anyhow::bail!("nothing to resume yet: play something from a list"),
                },
                Command::TogglePlay => remote.control(&to, &Command::Play { uri: None }).await,
                Command::AddToQueue { uri } => remote.add_to_queue(&to, &uri).await,
                Command::PlayFrom { context_uri, index, track_uri } => {
                    let context = if context_uri == library::LIKED { format!("spotify:user:{}:collection", session.username()) } else { context_uri };
                    remote.play_from(&to, &context, index, track_uri.as_deref()).await
                }
                other => remote.control(&to, &other).await,
            }
        }
        .await;
        if let Err(e) = result {
            if let Some(r) = self.remote.as_mut() {
                r.forget_ids();
            }
            let player = self.engine.player;
            let mut player = player.strip_prefix("the ").unwrap_or(player).to_string();
            player[..1].make_ascii_uppercase();
            self.fail(format!("{player}: {e}")).await;
        }
    }

    /// ConnectRemote and the Spotify app's device. A device that isn't on Connect (the app just started, or came back
    /// after a network change) is waited for: each pushed cluster is looked at, for up to 20 s. No polling.
    async fn engine_device(&mut self) -> anyhow::Result<(Session, &mut ConnectRemote, String)> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let name = self.engine.device_name.clone();
        loop {
            let changed = self.cluster_changed.clone();
            let next = changed.notified();
            match self.remote().await {
                Ok(_) => break,
                Err(e) if Instant::now() >= deadline || !e.to_string().contains("not found") => return Err(e),
                Err(_) => {
                    tracing::debug!("waiting for {name:?} on Connect");
                    if let Some(r) = self.remote.as_mut() {
                        r.forget_ids();
                    }
                    let _ = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), next).await;
                }
            }
        }
        self.remote().await
    }

    /// Queue edits Soloist can't make (play next, remove, clear, move) go through Connect.
    async fn queue(&mut self, edit: QueueEdit) {
        let result = async {
            let (_, remote, to) = self.remote().await?;
            remote.edit_queue(&to, &edit).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("queue edit via Connect failed: {e}");
            if let Some(r) = self.remote.as_mut() {
                r.forget_ids();
            }
            match edit {
                // Soloist can at least append it
                QueueEdit::PlayNext(uri) => {
                    let _ = self.ev.send(CatEvent::ToEngine(Command::AddToQueue { uri })).await;
                    self.fail("Couldn't put it first; added it to the end of the queue".into()).await;
                }
                _ => {
                    self.fail(format!("Couldn't change the queue: {e}")).await;
                    let _ = self.ev.send(CatEvent::Resync).await;
                }
            }
        }
    }

    /// Names for bare track URIs, from `getTrack` (cached; one request at a time). Every URI in the batch is
    /// answered one way or the other (A8: the Hub asks about a whole queue at once, and used to stop at 20 without
    /// telling anyone, which left those rows blank for the Daemon's life).
    async fn describe(&mut self, uris: Vec<String>) {
        let (mut out, ask, mut failed) = describe_plan(uris, &self.described, &self.describe_failed);
        for uri in ask {
            let episode = uri.starts_with("spotify:episode:");
            let answer = if episode {
                self.query("getEpisodeOrChapter", json!({ "uri": uri, "includeEpisodeContentRatingsV2": false })).await
            } else {
                self.query("getTrack", json!({ "uri": uri })).await
            };
            let item = match answer {
                Ok(v) => if episode { library::parse_episode(&v).map(|(i, _)| i) } else { library::parse_track(&v) },
                Err(e) => {
                    tracing::debug!("getTrack {uri}: {e}");
                    None
                }
            };
            match item {
                Some(item) => {
                    self.describe_failed.remove(&uri);
                    trim_any(&mut self.described, MAX_DESCRIBED);
                    self.described.insert(uri, item.clone());
                    out.push(item);
                }
                None => {
                    // not asked about again for a while; the rest of the batch still is (a track Spotify can't
                    // describe must not block the queue behind it)
                    self.describe_failed.insert(uri.clone(), Instant::now());
                    failed.push(uri);
                }
            }
        }
        trim_ages(&mut self.describe_failed, DESCRIBE_RETRY, MAX_DESCRIBED);
        if !out.is_empty() {
            self.emit(CatEvent::Described(out)).await;
        }
        if !failed.is_empty() {
            self.emit(CatEvent::DescribeFailed(failed)).await;
        }
    }

    /// Which show an episode is from (an episode card opens its show).
    async fn find_show(&mut self, uri: String) {
        match self.query("getEpisodeOrChapter", json!({ "uri": uri, "includeEpisodeContentRatingsV2": false })).await.map(|v| library::parse_episode(&v)) {
            Ok(Some((item, show))) => {
                trim_any(&mut self.described, MAX_DESCRIBED);
                self.described.insert(uri.clone(), item);
                let _ = self.ev.send(CatEvent::EpisodeShow { episode: uri, show }).await;
            }
            Ok(None) => {
                self.fail("That episode isn't available".into()).await;
            }
            Err(e) => {
                self.fail(format!("Episode unavailable: {e}")).await;
            }
        }
    }

    async fn search(&mut self, query: String) {
        let key = query.trim().to_lowercase();
        if let Some((at, r)) = self.searches.get(&key)
            && at.elapsed() < SEARCH_TTL
        {
            let mut r = r.clone();
            r.query = query;
            let _ = self.ev.send(CatEvent::Search(r)).await;
            return;
        }
        match self.query("searchDesktop", library::search_vars(query.trim())).await {
            Ok(v) => {
                let r = library::parse_search(&query, &v);
                trim_timed(&mut self.searches, SEARCH_TTL, MAX_PAGES);
                self.searches.insert(key, (Instant::now(), r.clone()));
                let _ = self.ev.send(CatEvent::Search(r)).await;
            }
            Err(e) => {
                // with the search, so the Client stops waiting on "searching…"
                let r = singularity_proto::SearchResults { query, error: Some(e.to_string()), ..Default::default() };
                let _ = self.ev.send(CatEvent::Search(r)).await;
            }
        }
    }

    async fn artist(&mut self, uri: String) {
        if let Some((at, p)) = self.artists.get(&uri)
            && at.elapsed() < TRACKS_TTL
        {
            let _ = self.ev.send(CatEvent::Artist(p.clone())).await;
            return;
        }
        match self.query("queryArtistOverview", json!({ "uri": uri, "locale": "", "includePrerelease": true })).await {
            Ok(v) => match library::parse_artist(&v) {
                Some(p) => {
                    trim_timed(&mut self.artists, TRACKS_TTL, MAX_PAGES);
                    self.artists.insert(uri, (Instant::now(), p.clone()));
                    let _ = self.ev.send(CatEvent::Artist(p)).await;
                }
                None => {
                    self.fail("Spotify answered, but the artist page had an unexpected shape".into()).await;
                }
            },
            Err(e) => {
                self.fail(format!("Artist page unavailable: {e}")).await;
            }
        }
    }

    /// Spotify's lyrics, else LRCLIB's. Remembered when found or when both said "none", so a track is asked about once;
    /// a lookup that couldn't reach a source is asked again next time.
    async fn lyrics(&mut self, uri: String, meta: Option<singularity_proto::Item>) {
        use crate::lyrics::Outcome;
        if let Some(l) = self.lyrics.get(&uri) {
            let _ = self.ev.send(CatEvent::Lyrics(l.clone())).await;
            return;
        }
        let mut found = None;
        let spotify = match (self.session().await, librespot_core::SpotifyId::from_base62(uri.rsplit(':').next().unwrap_or(""))) {
            (Some(session), Ok(id)) => match crate::spotify(session.spclient().get_lyrics(&id)).await {
                Ok(bytes) => {
                    found = serde_json::from_slice(&bytes).ok().and_then(|v| crate::lyrics::parse_spotify(&uri, &v));
                    if found.is_some() { Outcome::Found } else { Outcome::Nothing }
                }
                Err(e) if e.kind == librespot_core::error::ErrorKind::NotFound => Outcome::Nothing,
                Err(e) => {
                    tracing::info!("lyrics for {uri}: Spotify failed: {e}");
                    Outcome::Failed
                }
            },
            (None, _) => Outcome::Failed,
            (_, Err(_)) => Outcome::Nothing,
        };
        let mut lrclib = Outcome::Nothing;
        if found.is_none() {
            let meta = match meta.filter(|m| !m.name.is_empty()) {
                Some(m) => Some(m),
                None => {
                    self.describe(vec![uri.clone()]).await;
                    self.described.get(&uri).cloned()
                }
            };
            lrclib = match meta {
                Some(m) => match crate::lyrics::lrclib(&uri, &m.artists.first().cloned().unwrap_or_default(), &m.name, m.album.as_deref(), m.duration_ms).await {
                    Ok(Some(l)) => {
                        found = Some(l);
                        Outcome::Found
                    }
                    Ok(None) => Outcome::Nothing,
                    Err(e) => {
                        tracing::info!("lyrics for {uri}: LRCLIB failed: {e}");
                        Outcome::Failed
                    }
                },
                // without a name and artist there's nothing to look up
                None => Outcome::Failed,
            };
        }
        tracing::info!("lyrics for {uri}: Spotify {spotify:?}, LRCLIB {}", if spotify == Outcome::Found { "not asked".to_string() } else { format!("{lrclib:?}") });
        let (l, keep) = match found {
            Some(l) => (l, true),
            None => {
                let (missing, keep) = crate::lyrics::missing(spotify, lrclib);
                (singularity_proto::Lyrics { uri: uri.clone(), missing, ..Default::default() }, keep)
            }
        };
        if keep {
            trim_any(&mut self.lyrics, MAX_LYRICS);
            self.lyrics.insert(uri, l.clone());
        }
        let _ = self.ev.send(CatEvent::Lyrics(l)).await;
    }

    /// Start a context at track N through ConnectRemote; if that fails, the Engine plays the track alone.
    async fn play_from(&mut self, context_uri: String, index: u32, track_uri: Option<String>) {
        let result = async {
            // a failure below drops the remote, so a stale device id is looked up again next time
            let (session, remote, to) = self.remote().await?;
            let context = if context_uri == library::LIKED { format!("spotify:user:{}:collection", session.username()) } else { context_uri.clone() };
            remote.play_from(&to, &context, index, track_uri.as_deref()).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("play from track {index} of {context_uri} via Connect failed: {e}");
            if let Some(r) = self.remote.as_mut() {
                r.forget_ids();
            }
            let _ = self.ev.send(CatEvent::ToEngine(Command::PlayFrom { context_uri, index, track_uri })).await;
        }
    }

    async fn home(&mut self) {
        let vars = json!({
            "includeHomeChapterVideoCards": false,
            "homeEndUserIntegration": "INTEGRATION_WEB_PLAYER",
            "timeZone": local_timezone(),
            "sp_t": "",
            "facet": "",
            "sectionItemsLimit": 12,
            "includeEpisodeContentRatingsV2": false,
        });
        match self.query("home", vars).await {
            Ok(v) => {
                let feed = parse_home(&v);
                if feed.shelves.is_empty() {
                    self.fail("Spotify answered, but the Home feed had an unexpected shape".into()).await;
                }
                let _ = self.ev.send(CatEvent::Home(feed)).await;
            }
            Err(e) => {
                self.fail(format!("Home feed unavailable: {e}")).await;
            }
        }
    }
}

fn local_timezone() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| p.to_str().and_then(|s| s.split("zoneinfo/").nth(1)).map(String::from))
        .unwrap_or_else(|| "UTC".into())
}

fn first_url(sources: &Value) -> Option<String> {
    // prefer a mid-sized image: ~300 px is plenty for cards
    let list = sources.as_array()?;
    list.iter()
        .min_by_key(|s| (s["width"].as_i64().unwrap_or(300) - 300).abs())
        .or(list.first())
        .and_then(|s| s["url"].as_str().map(String::from))
}

pub(crate) fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    // `&amp;` last, so an escaped entity stays as typed
    let out = out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#x27;", "'").replace("&#39;", "'").replace("&#x2F;", "/");
    out.replace("&amp;", "&").trim().to_string()
}

/// A Pathfinder entity (Album, Playlist, Artist, Episode, Podcast) as a Card.
pub fn card(d: &Value) -> Option<Card> {
    let uri = d["uri"].as_str()?.to_string();
    let (kind, name, subtitle, image) = match d["__typename"].as_str()? {
        "Album" => (
            "album",
            d["name"].as_str()?.to_string(),
            d["artists"]["items"].as_array().map(|a| a.iter().filter_map(|x| x["profile"]["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            first_url(&d["coverArt"]["sources"]),
        ),
        "Playlist" => (
            "playlist",
            d["name"].as_str()?.to_string(),
            plain(d["description"].as_str().unwrap_or_default()),
            first_url(&d["images"]["items"][0]["sources"]),
        ),
        "Artist" => ("artist", d["profile"]["name"].as_str()?.to_string(), "Artist".into(), first_url(&d["visuals"]["avatarImage"]["sources"])),
        "Episode" | "PodcastEpisode" => ("episode", d["name"].as_str()?.to_string(), d["podcastV2"]["data"]["name"].as_str().unwrap_or_default().to_string(), first_url(&d["coverArt"]["sources"])),
        "Podcast" => ("show", d["name"].as_str()?.to_string(), d["publisher"]["name"].as_str().unwrap_or_default().to_string(), first_url(&d["coverArt"]["sources"])),
        // a book is a show (#62)
        "Audiobook" => ("show", d["name"].as_str()?.to_string(), library::book_subtitle(d), first_url(&d["coverArt"]["sources"])),
        "Track" => (
            "track",
            d["name"].as_str()?.to_string(),
            d["artists"]["items"].as_array().map(|a| a.iter().filter_map(|x| x["profile"]["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            first_url(&d["albumOfTrack"]["coverArt"]["sources"]),
        ),
        _ => return None,
    };
    Some(Card { uri, name, subtitle, image_url: image, kind: kind.into() })
}

/// Pathfinder `home` answer → Shelves. Tolerant: unknown sections and items are skipped.
pub fn parse_home(v: &Value) -> HomeFeed {
    let sections = v["data"]["home"]["sectionContainer"]["sections"]["items"].as_array().cloned().unwrap_or_default();
    let mut shelves = vec![];
    for s in &sections {
        let kind = match s["data"]["__typename"].as_str().unwrap_or("") {
            "HomeShortsSectionData" => "shortcuts",
            "HomeFeedBaselineSectionData" => "feed",
            _ => "shelf",
        };
        let title = s["data"]["title"]["transformedLabel"].as_str().or_else(|| s["data"]["title"]["text"].as_str()).unwrap_or("").to_string();
        let cards: Vec<Card> = s["sectionItems"]["items"].as_array().into_iter().flatten().filter_map(|it| card(&it["content"]["data"])).collect();
        if !cards.is_empty() {
            shelves.push(Shelf { title, kind: kind.into(), cards });
        }
    }
    HomeFeed { fetched_unix_ms: singularity_proto::now_ms(), shelves }
}

#[cfg(test)]
mod tests {
    #[test]
    fn caches_stay_bounded() {
        use std::collections::HashMap;
        use std::time::{Duration, Instant};
        // (#71) expired entries go first, then the oldest; never more than `max`
        let mut timed: HashMap<String, (Instant, u32)> = HashMap::new();
        let old = Instant::now() - Duration::from_secs(700);
        timed.insert("stale".into(), (old, 0));
        for i in 0..10u32 {
            super::trim_timed(&mut timed, Duration::from_secs(600), 5);
            timed.insert(format!("k{i}"), (Instant::now() + Duration::from_millis(i as u64), i));
            assert!(timed.len() <= 5);
        }
        assert!(!timed.contains_key("stale") && timed.contains_key("k9") && !timed.contains_key("k0"));
        let mut any: HashMap<String, u32> = (0..10).map(|i| (format!("k{i}"), i)).collect();
        super::trim_any(&mut any, 4);
        assert_eq!(any.len(), 3);
    }

    use super::*;

    /// Anonymised miniature of a real `home` answer (structure from spike 2, invented names).
    const FIXTURE: &str = r#"{"data":{"home":{"sectionContainer":{"sections":{"items":[
      {"data":{"__typename":"HomeShortsSectionData","title":null},"sectionItems":{"items":[
        {"content":{"__typename":"AlbumResponseWrapper","data":{"__typename":"Album","uri":"spotify:album:a","name":"Event Horizon","artists":{"items":[{"profile":{"name":"Nova Drift"}}]},"coverArt":{"sources":[{"url":"https://i.scdn.co/image/64","width":64},{"url":"https://i.scdn.co/image/300","width":300},{"url":"https://i.scdn.co/image/640","width":640}]}}}},
        {"content":{"__typename":"UnknownType","uri":"spotify:x"}}]}},
      {"data":{"__typename":"HomeGenericSectionData","title":{"transformedLabel":"Made For You"}},"sectionItems":{"items":[
        {"content":{"__typename":"PlaylistResponseWrapper","data":{"__typename":"Playlist","uri":"spotify:playlist:p","name":"Daily Mix 1","description":"Nova Drift, <a href=\"x\">Mira Vale</a> &amp; more","images":{"items":[{"sources":[{"url":"https://i.scdn.co/image/mix","width":null}]}]}}}},
        {"content":{"__typename":"ArtistResponseWrapper","data":{"__typename":"Artist","uri":"spotify:artist:r","profile":{"name":"Lumen Fields"},"visuals":{"avatarImage":{"sources":[{"url":"https://i.scdn.co/image/av","width":320}]}}}}}]}},
      {"data":{"__typename":"HomeFeedBaselineSectionData","title":{"text":"Based on your recent listening"}},"sectionItems":{"items":[
        {"content":{"__typename":"PlaylistResponseWrapper","data":{"__typename":"Playlist","uri":"spotify:playlist:q","name":"Night Drive","description":"","images":{"items":[]}}}}]}},
      {"data":{"__typename":"HomeGenericSectionData","title":{"text":"Empty"}},"sectionItems":{"items":[]}}
    ]}}}}}"#;

    /// A8: a 51-URI queue is all asked about (no `take(20)`), cached names come for free, and a URI that just failed
    /// is left out (and reported failed) until its retry window is over.
    #[test]
    fn a_describe_batch_covers_every_uri() {
        use std::collections::HashMap;
        use std::time::{Duration, Instant};
        let mut described: HashMap<String, singularity_proto::Item> = HashMap::new();
        let mut failed: HashMap<String, Instant> = HashMap::new();
        described.insert("spotify:track:known".into(), singularity_proto::Item { uri: "spotify:track:known".into(), name: "Known".into(), ..Default::default() });
        failed.insert("spotify:track:broken".into(), Instant::now());
        let mut uris: Vec<String> = (0..51).map(|i| format!("spotify:track:q{i}")).collect();
        uris.push("spotify:track:known".into());
        uris.push("spotify:track:broken".into());
        let (known, ask, bad) = describe_plan(uris, &described, &failed);
        assert_eq!(ask.len(), 51, "every URI of a queue-sized batch is looked up: {ask:?}");
        assert_eq!(known.len(), 1);
        assert_eq!(bad, vec!["spotify:track:broken".to_string()], "a failed lookup is reported failed, so the Hub asks again later");
        // ... and after the retry window it gets another chance
        failed.insert("spotify:track:broken".into(), Instant::now() - Duration::from_secs(super::DESCRIBE_RETRY.as_secs() + 1));
        let (_, ask, bad) = describe_plan(vec!["spotify:track:broken".into()], &described, &failed);
        assert_eq!((ask.len(), bad.len()), (1, 0));
    }

    /// B10/C15: a broken `totalCount` from Spotify can't walk the library for millions of pages.
    #[test]
    fn the_library_walk_has_a_page_cap() {
        assert!(library_page_continues(50, 50, u32::MAX, 50), "an ordinary full page continues");
        assert!(!library_page_continues(0, 50, u32::MAX, 50), "an empty page is the end");
        assert!(!library_page_continues(50, 100, 100, 50), "the reported total is reached");
        assert!(!library_page_continues(50, 10_000, u32::MAX, super::MAX_TRACKS as usize), "the cap stops a lying totalCount");
    }

    /// A2/C4: commands that arrived behind a search are kept, in order, and run after it — not sent back into this
    /// task's own full channel (a deadlock) and not lost.
    #[tokio::test]
    async fn a_search_drains_the_commands_behind_it_in_order() {
        let (tx, mut rx) = mpsc::channel(16);
        for c in [CatCmd::Search("a".into()), CatCmd::Library, CatCmd::Devices, CatCmd::Search("b".into())] {
            tx.send(c).await.unwrap();
        }
        let first = match rx.recv().await.unwrap() {
            CatCmd::Search(q) => q,
            other => panic!("unexpected {other:?}"),
        };
        let (q, later) = drain_for_search(first, &mut rx);
        assert_eq!(q, "b", "only the newest of a burst of searches is made");
        assert!(matches!(later.iter().collect::<Vec<_>>().as_slice(), [CatCmd::Library, CatCmd::Devices]), "the others wait their turn, in order: {later:?}");
    }

    #[test]
    fn parses_shelves_cards_and_kinds() {
        let feed = parse_home(&serde_json::from_str(FIXTURE).unwrap());
        assert_eq!(feed.shelves.len(), 3, "empty shelves are dropped");
        assert_eq!(feed.shelves[0].kind, "shortcuts");
        assert_eq!(feed.shelves[0].cards.len(), 1, "unknown items are skipped");
        let album = &feed.shelves[0].cards[0];
        assert_eq!((album.kind.as_str(), album.subtitle.as_str()), ("album", "Nova Drift"));
        assert_eq!(album.image_url.as_deref(), Some("https://i.scdn.co/image/300"));
        let mix = &feed.shelves[1].cards[0];
        assert_eq!(feed.shelves[1].title, "Made For You");
        assert_eq!(mix.subtitle, "Nova Drift, Mira Vale & more");
        assert_eq!(feed.shelves[1].cards[1].kind, "artist");
        assert_eq!(feed.shelves[2].kind, "feed");
        assert_eq!(feed.shelves[2].cards[0].image_url, None);
    }
}
