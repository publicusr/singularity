//! PrivateCatalogue (ADR-0002): Spotify's private APIs through a librespot session.
//!
//! Login is the OAuth device flow with Spotify's desktop client id (as librespot/spotatui do): the user
//! approves a code at spotify.com/pair; this program never sees a password. Reusable credentials are
//! cached under `$XDG_DATA_HOME/singularity/catalogue` (owner-only).
//! Rules: metadata only, no crawling, back off on errors, never retry in a loop.

use crate::pathfinder::{Fetch, Pathfinder, SessionRaw, SessionSlot, session_slot};
use crate::queries::QueryRegistry;
use futures_util::{FutureExt, StreamExt};
use librespot_core::{authentication::Credentials, cache::Cache, config::SessionConfig, session::Session};
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

mod connect;
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
    /// Home and the library at once, each sent as soon as it's in (#81): what a login sets off
    HomeAndLibrary,
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
    /// `words`: also ask LRCLIB for word timing when Spotify's lyrics only time lines (a Karaoke Widget is on screen)
    Lyrics { uri: String, meta: Option<singularity_proto::Item>, words: bool },
    Artist(String),
    /// demo mode's quiet look (#128): a playlist's first page or an artist's page, no toast if it fails
    Peek(String),
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
    /// Connect found the session ended (D79): browsing, which owns the login, logs in again
    Reconnect,
}

#[derive(Debug, Clone)]
pub enum CatEvent {
    Status(CatalogueStatus),
    /// the account the saved login is for, named before Spotify answers (#81: its last Home and library show then)
    Account(String),
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
    /// the answer to a `Peek`: a first page or an artist page, neither when it failed
    Peeked { uri: String, tracks: Option<TrackPage>, artist: Option<singularity_proto::ArtistPage> },
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
        let f = singularity_config::app_folders::AppFolders::this();
        Dirs { credentials: f.data.join("catalogue"), queries: f.cache.join("pathfinder-queries.json") }
    }
}

/// The Catalogue, for an Engine that is `engine` on Connect.
/// The Catalogue's name in the health view.
pub const TASK: &str = "the Catalogue";

pub fn spawn(dirs: Dirs, engine: Identity) -> (mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>) {
    let queries = dirs.queries.clone();
    spawn_with(dirs, engine, move |slot| Pathfinder::new(QueryRegistry::new(queries.clone()), SessionRaw(slot)))
}

/// The Catalogue over the query port `make` builds for each run (D78): Spotify's adapter, or fixtures in tests.
fn spawn_with<Q: Fetch + Sync + Clone + 'static>(dirs: Dirs, engine: Identity, make: impl Fn(SessionSlot) -> Q + Send + 'static) -> (mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>) {
    STARTED.get_or_init(Instant::now);
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let (ev_tx, ev_rx) = mpsc::channel(64);
    tokio::spawn(supervise(dirs, engine, make, cmd_rx, ev_tx));
    (cmd_tx, ev_rx)
}

/// Run the Catalogue, and keep it running: a panic would otherwise leave a Daemon that browses nothing, says
/// "logged in" and never answers again (A1/C2). The command channel survives a restart, so Clients keep working.
async fn supervise<Q: Fetch + Sync + Clone>(dirs: Dirs, engine: Identity, make: impl Fn(SessionSlot) -> Q, mut cmds: mpsc::Receiver<CatCmd>, ev: mpsc::Sender<CatEvent>) {
    let mut backoff = Duration::from_secs(2);
    loop {
        let began = Instant::now();
        // the health row goes once the Catalogue has run for a while (one that keeps dying keeps its row)
        let back = tokio::spawn({
            let ev = ev.clone();
            async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let _ = ev.send(CatEvent::TaskStarted(TASK)).await;
            }
        });
        let slot = session_slot();
        let done = std::panic::AssertUnwindSafe(run(dirs.clone(), engine.clone(), make(slot.clone()), slot, &mut cmds, ev.clone())).catch_unwind().await;
        back.abort();
        match done {
            // the Daemon dropped its end: we're shutting down
            Ok(()) => return,
            Err(_) => {
                if began.elapsed() >= Duration::from_secs(60) {
                    backoff = Duration::from_secs(2);
                }
                let _ = ev.send(CatEvent::Status(CatalogueStatus::Unavailable { message: "the Catalogue stopped; it's starting again".into() })).await;
                let _ = ev.send(CatEvent::TaskStopped(TASK, "it panicked".into())).await;
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(300));
            }
        }
    }
}

/// The newest search not made yet, and when it came (#126). Searches don't queue with the rest of browsing: while
/// typing only the newest matters, it goes ahead of the browsing work waiting, and long work (a track list's pages,
/// naming a queue) makes it between two of its requests.
type PendingSearch = std::sync::Arc<tokio::sync::watch::Sender<Option<(String, Instant)>>>;

/// A login or logout changes what every query after it gets: a search doesn't go ahead of one.
fn changes_session(cmd: &CatCmd) -> bool {
    matches!(cmd, CatCmd::Login | CatCmd::Logout | CatCmd::Reconnect)
}

/// A command's name for the log (its variant, none of what it carries).
fn cmd_name(cmd: &CatCmd) -> String {
    format!("{cmd:?}").split(|c: char| !c.is_alphanumeric()).next().unwrap_or_default().to_string()
}

/// Browsing work this long is logged, with its name: it is what a search (or anything else) waited behind.
const SLOW_BROWSING: Duration = Duration::from_secs(1);

/// How a search was answered, for its log line.
enum Searched {
    Kept,
    Found { spotify: Duration, read: Duration, found: usize },
    Failed { spotify: Duration },
}

/// A search's line in the log (#126): where its time went. It is given no words, so it can't write them (what you
/// search for is personal; a failure's reason isn't written either, the Client shows it).
fn search_line(waited: Duration, behind: &str, how: &Searched, all: Duration) -> String {
    let ms = |d: Duration| d.as_millis();
    let how = match how {
        Searched::Kept => "answered from what was kept".to_string(),
        Searched::Found { spotify, read, found } => format!("Spotify {} ms, read {} ms, {found} results", ms(*spotify), ms(*read)),
        Searched::Failed { spotify } => format!("Spotify failed after {} ms", ms(*spotify)),
    };
    format!("search: waited {} ms{behind}, {how}; {} ms in all", ms(waited), ms(all))
}

/// Searches made in a row ahead of queued work before one queued command runs: typing can't hold the rest back.
const SEARCHES_IN_A_ROW: u32 = 3;

struct State<Q> {
    dirs: Dirs,
    session: Option<Session>,
    /// the same session, for the query port's adapter
    slot: SessionSlot,
    /// the query port (D78)
    queries: Q,
    ev: mpsc::Sender<CatEvent>,
    /// Connect's own loop (D79): its commands go there and never wait behind browsing. Dropping it (when browsing
    /// ends) ends Connect.
    to_connect: mpsc::UnboundedSender<CatCmd>,
    /// a login under way: Connect waits for it rather than failing (D79)
    logging_in: std::sync::Arc<tokio::sync::watch::Sender<bool>>,
    tracks: HashMap<String, (Instant, Vec<TrackPage>)>,
    /// shows found to be audiobooks (#62): their pages come from `queryBookChapters`
    books: std::collections::HashSet<String>,
    described: HashMap<String, singularity_proto::Item>,
    /// URIs whose lookup failed, and when: not asked about again for `DESCRIBE_RETRY` (A8)
    describe_failed: HashMap<String, Instant>,
    lyrics: HashMap<String, singularity_proto::Lyrics>,
    /// tracks LRCLIB was already asked about for word timing (D67): asked once per track
    word_tried: HashMap<String, ()>,
    artists: HashMap<String, (Instant, singularity_proto::ArtistPage)>,
    searches: HashMap<String, (Instant, singularity_proto::SearchResults)>,
    /// the newest search not made yet (#126)
    pending: PendingSearch,
    /// browsing's commands as they come (searches aside), and those taken from it waiting their turn, with the
    /// follow-ups of commands already run (A2: never sent back into the channel). Kept here so a search made in the
    /// middle of long work sees what is queued (a login or logout it mustn't go ahead of).
    inbox: mpsc::UnboundedReceiver<CatCmd>,
    waiting: VecDeque<CatCmd>,
    /// the work running (a command, or a search), by name and since when, and the last that ended: what a search
    /// waited behind, for the log
    busy: Option<(String, Instant)>,
    last: Option<(String, Instant)>,
    /// when to log in again after Spotify ended the session (Q171)
    redial: Redial,
}

/// How often idle browsing looks whether Spotify ended the session (Q171): librespot only marks it ended, so it's
/// looked at, not waited on. (A request in between finds it ended too, and logs in again first.)
const LOOK_EVERY: Duration = Duration::from_secs(5);
/// The first wait before logging in again after a failed try, or after a session that ended soon after its login;
/// it doubles each time, up to `REDIAL_MAX`.
const REDIAL_FIRST: Duration = Duration::from_secs(5);
const REDIAL_MAX: Duration = Duration::from_secs(300);
/// A session that lasted this long ended in the ordinary way (Spotify drops an idle one): logged in again at once.
const STEADY: Duration = Duration::from_secs(60);

/// When to log in again after Spotify ended the session (Q171, #126). Spotify drops a quiet connection after a while;
/// logging in again when that's seen, not at the next request, keeps the next search from waiting for the login.
/// A session that lasted is replaced at once; one that ends soon after its login, or a login that fails (no network),
/// is tried again later and later, so Spotify isn't asked over and over. Logging out stops it.
#[derive(Debug)]
struct Redial {
    /// how often idle browsing looks (`LOOK_EVERY`; shorter in tests)
    every: Duration,
    /// the next wait
    backoff: Duration,
    /// a login to try then (none: nothing is waiting)
    at: Option<Instant>,
    /// when the current session logged in
    since: Option<Instant>,
}

impl Redial {
    fn new() -> Redial {
        Redial { every: LOOK_EVERY, backoff: REDIAL_FIRST, at: None, since: None }
    }

    /// A session is in: nothing waits.
    fn logged_in(&mut self, now: Instant) {
        self.since = Some(now);
        self.at = None;
    }

    /// The session Spotify ended was seen at `now`: when to log in again (`now` for one that lasted).
    fn ended(&mut self, now: Instant) -> Instant {
        let lasted = self.since.is_some_and(|s| now.duration_since(s) >= STEADY);
        self.since = None;
        if lasted {
            self.backoff = REDIAL_FIRST;
            self.at = Some(now);
            now
        } else {
            self.later(now)
        }
    }

    /// Logging in again failed at `now`: when to try next.
    fn failed(&mut self, now: Instant) -> Instant {
        self.later(now)
    }

    fn later(&mut self, now: Instant) -> Instant {
        let at = now + self.backoff;
        self.backoff = (self.backoff * 2).min(REDIAL_MAX);
        self.at = Some(at);
        at
    }

    /// Logged out, or no saved login: nothing to log in again with.
    fn stop(&mut self) {
        *self = Redial { every: self.every, ..Redial::new() };
    }

    fn due(&self, now: Instant) -> bool {
        self.at.is_some_and(|at| at <= now)
    }
}

/// How a login went.
#[derive(Debug, PartialEq)]
enum LoggedIn {
    Yes,
    /// no saved login to use (logged out)
    NoLogin,
    /// Spotify couldn't be reached, or refused it
    Failed,
}

/// Which loop runs a command (D79): Connect's, or browsing's.
fn is_connect(cmd: &CatCmd) -> bool {
    // every command named, so a new one is routed on purpose
    match cmd {
        CatCmd::Devices
        | CatCmd::Transfer(_)
        | CatCmd::StartJam
        | CatCmd::EndJam
        | CatCmd::Remote(_)
        | CatCmd::Engine(_)
        | CatCmd::Queue(_)
        | CatCmd::PlayFrom { .. }
        | CatCmd::Resume(_) => true,
        CatCmd::Login
        | CatCmd::Logout
        | CatCmd::Reconnect
        | CatCmd::Home
        | CatCmd::Library
        | CatCmd::HomeAndLibrary
        | CatCmd::Tracks(_)
        | CatCmd::MoreTracks { .. }
        | CatCmd::FindShow(_)
        | CatCmd::Describe(_)
        | CatCmd::Lyrics { .. }
        | CatCmd::Artist(_)
        | CatCmd::Peek(_)
        | CatCmd::Search(_)
        | CatCmd::SetSaved { .. }
        | CatCmd::CheckSaved(_)
        | CatCmd::CreatePlaylist { .. }
        | CatCmd::AddToPlaylist { .. }
        | CatCmd::RemoveFromPlaylist { .. }
        | CatCmd::SetCover { .. }
        | CatCmd::MoveInPlaylist { .. }
        | CatCmd::DeletePlaylist(_)
        | CatCmd::EditPlaylist { .. }
        | CatCmd::Query { .. } => false,
    }
}

/// Browsing and Connect side by side in this one task, so a panic in either is caught by `supervise` and a wait in
/// one never holds up the other. Commands are passed on as they arrive; a search is put where browsing takes it next.
async fn run<Q: Fetch + Sync + Clone>(dirs: Dirs, engine: Identity, q: Q, slot: SessionSlot, cmds: &mut mpsc::Receiver<CatCmd>, ev: mpsc::Sender<CatEvent>) {
    let (browse_tx, browse_rx) = mpsc::unbounded_channel();
    let (connect_tx, connect_rx) = mpsc::unbounded_channel();
    let logging_in = std::sync::Arc::new(tokio::sync::watch::Sender::new(false));
    let connect = connect::Connect::new(engine.clone(), slot.clone(), logging_in.clone(), ev.clone(), browse_tx.downgrade());
    let st = State::new(dirs, q, slot, ev, connect_tx.clone(), logging_in, browse_rx);
    let pending = st.pending.clone();
    let pass_on = async move {
        while let Some(c) = cmds.recv().await {
            if let CatCmd::Search(q) = c {
                // replaces one not made yet: while typing only the newest matters
                pending.send_replace(Some((q, Instant::now())));
                continue;
            }
            let to = if is_connect(&c) { &connect_tx } else { &browse_tx };
            let _ = to.send(c);
        }
        // the Daemon hung up: browsing ends with its channel, and Connect with browsing
    };
    futures_util::join!(pass_on, browse(st), connect.run(connect_rx));
}

async fn browse<Q: Fetch + Sync + Clone>(mut st: State<Q>) {
    let mut searched = st.pending.subscribe();
    // resume a previous login (the keyring is read once: it can be slow)
    let began = Instant::now();
    let saved = saved_credentials(&st.dirs);
    timing("keyring read", began);
    if let Some(creds) = saved {
        if let Some(name) = creds.username.clone() {
            st.emit(CatEvent::Account(name)).await;
        }
        let was = st.begin("Login".into());
        let ok = st.connect(Some(creds)).await;
        st.end(was);
        if ok {
            st.waiting.extend([CatCmd::Devices, CatCmd::HomeAndLibrary]);
        }
    } else {
        st.status(CatalogueStatus::LoggedOut).await;
    }
    // searches made ahead of queued work since a queued command last ran
    let mut in_a_row = 0;
    loop {
        // a session Spotify ended is replaced now, not at the next request (Q171)
        st.look().await;
        // a search goes ahead of what waits (#126), unless that logs in or out, or typing has held it back long enough
        st.take_arrived();
        if (st.waiting.is_empty() || in_a_row < SEARCHES_IN_A_ROW) && st.search_between().await {
            in_a_row += 1;
            continue;
        }
        let cmd = match st.waiting.pop_front() {
            Some(c) => c,
            None => tokio::select! {
                c = st.inbox.recv() => match c {
                    Some(c) => c,
                    None => return,
                },
                _ = searched.changed() => continue,
                () = tokio::time::sleep(st.redial.every) => continue,
            },
        };
        in_a_row = 0;
        let (name, began) = (cmd_name(&cmd), Instant::now());
        let was = st.begin(name.clone());
        handle(&mut st, cmd).await;
        st.end(was);
        if began.elapsed() >= SLOW_BROWSING {
            tracing::info!("browsing: {name} took {} ms", began.elapsed().as_millis());
        }
    }
}

/// Run one command. What it makes necessary next (the sidebar after a playlist change) goes in `st.waiting`: sending
/// it back into the channel only this task reads deadlocked the Catalogue once 16 commands were waiting (2026-11-20
/// bug hunt, A2; the pattern Search had already lost in A2/C4), and with it every Client session and the Engine task
/// waiting to send it something.
async fn handle<Q: Fetch + Sync + Clone>(st: &mut State<Q>, cmd: CatCmd) {
    let library = |next: &mut VecDeque<CatCmd>| {
        if !next.iter().any(|c| matches!(c, CatCmd::Library)) {
            next.push_back(CatCmd::Library);
        }
    };
    match cmd {
        // Connect saw the session end: replace it
        CatCmd::Reconnect => {
            st.session().await;
        }
        CatCmd::Login => {
            if st.login().await {
                st.waiting.extend([CatCmd::Devices, CatCmd::HomeAndLibrary]);
            }
        }
        CatCmd::Logout => st.logout().await,
        CatCmd::Home => st.home().await,
        CatCmd::Library => st.library().await,
        CatCmd::HomeAndLibrary => st.home_and_library().await,
        CatCmd::Tracks(uri) => st.track_list(uri).await,
        CatCmd::MoreTracks { uri, offset } => st.more_tracks(uri, offset).await,
        CatCmd::FindShow(uri) => st.find_show(uri).await,
        CatCmd::Describe(uris) => st.describe(uris).await,
        CatCmd::Lyrics { uri, meta, words } => st.lyrics(uri, meta, words).await,
        CatCmd::Artist(uri) => st.artist(uri).await,
        CatCmd::Peek(uri) => st.peek(uri).await,
        CatCmd::SetSaved { uri, saved } => {
            // the library changed: the sidebar and Liked Songs follow (Liked Songs from its next open)
            if st.set_saved(uri.clone(), saved).await && !uri.starts_with("spotify:track:") {
                library(&mut st.waiting);
            }
        }
        CatCmd::CheckSaved(uris) => st.check_saved(uris).await,
        // either way the sidebar shows the truth afterwards
        CatCmd::CreatePlaylist { name, uris } => {
            st.create_playlist(name, uris).await;
            library(&mut st.waiting);
        }
        CatCmd::AddToPlaylist { playlist, name, uris, anyway } => st.add_to_playlist(playlist, name, uris, anyway).await,
        CatCmd::RemoveFromPlaylist { playlist, uids } => st.remove_from_playlist(playlist, uids).await,
        CatCmd::MoveInPlaylist { playlist, uids, before, after } => st.move_in_playlist(playlist, uids, before, after).await,
        CatCmd::SetCover { playlist, jpeg } => {
            if st.set_cover(playlist, jpeg).await {
                library(&mut st.waiting);
            }
        }
        CatCmd::DeletePlaylist(uri) => {
            st.delete_playlist(uri).await;
            library(&mut st.waiting);
        }
        CatCmd::EditPlaylist { uri, name, description } => {
            if st.edit_playlist(uri, name, description).await {
                library(&mut st.waiting);
            }
        }
        CatCmd::Query { op, vars, reply } => {
            let _ = reply.send(st.query(&op, vars).await);
        }
        // searches are kept aside as they come (`run`) and never queue here; one that did would join them
        CatCmd::Search(q) => {
            st.pending.send_replace(Some((q, Instant::now())));
        }
        // the rest is Connect's (`is_connect`): a follow-up such as the device list after a login
        other => {
            let _ = st.to_connect.send(other);
        }
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

impl<Q: Fetch + Sync + Clone> State<Q> {
    fn new(dirs: Dirs, queries: Q, slot: SessionSlot, ev: mpsc::Sender<CatEvent>, to_connect: mpsc::UnboundedSender<CatCmd>, logging_in: std::sync::Arc<tokio::sync::watch::Sender<bool>>, inbox: mpsc::UnboundedReceiver<CatCmd>) -> State<Q> {
        State { dirs, session: None, slot, queries, ev, to_connect, logging_in, tracks: HashMap::new(), books: Default::default(), described: HashMap::new(), describe_failed: HashMap::new(), lyrics: HashMap::new(), word_tried: HashMap::new(), artists: HashMap::new(), searches: HashMap::new(), pending: Default::default(), inbox, waiting: VecDeque::new(), busy: None, last: None, redial: Redial::new() }
    }

    /// The session changes in one place, so the query port's adapter always asks through the current one.
    fn set_session(&mut self, s: Option<Session>) {
        // answers kept for one account (or none) aren't the next one's
        self.searches.clear();
        self.slot.send_replace(s.clone());
        self.session = s;
    }

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

    /// Connect with fresh credentials, or the cached reusable ones. Connect waits meanwhile.
    async fn connect(&mut self, fresh: Option<Credentials>) -> bool {
        self.connect_how(fresh).await == LoggedIn::Yes
    }

    async fn connect_how(&mut self, fresh: Option<Credentials>) -> LoggedIn {
        self.logging_in.send_replace(true);
        let how = self.log_in(fresh).await;
        self.logging_in.send_replace(false);
        how
    }

    async fn log_in(&mut self, fresh: Option<Credentials>) -> LoggedIn {
        self.status(CatalogueStatus::Connecting).await;
        let cache = match cache(&self.dirs) {
            Ok(c) => c,
            Err(e) => {
                self.status(CatalogueStatus::Unavailable { message: e.to_string() }).await;
                return LoggedIn::Failed;
            }
        };
        let Some(creds) = fresh.or_else(|| saved_credentials(&self.dirs)) else {
            self.status(CatalogueStatus::LoggedOut).await;
            return LoggedIn::NoLogin;
        };
        let session = Session::new(SessionConfig::default(), Some(cache));
        let began = Instant::now();
        if let Err(e) = session.connect(creds, true).await {
            self.status(CatalogueStatus::Unavailable { message: format!("couldn't reach Spotify: {e}") }).await;
            return LoggedIn::Failed;
        }
        timing("login", began);
        // the account type and country arrive shortly after login
        let began = Instant::now();
        let mut product = None;
        for _ in 0..30 {
            product = session.get_user_attribute("type");
            if product.is_some() && !session.country().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        timing("account type", began);
        let premium = product.as_deref() == Some("premium");
        let began = Instant::now();
        keep_credentials_in_keyring(&self.dirs);
        timing("keyring save", began);
        let ready = CatalogueStatus::Ready { username: session.username(), country: session.country(), premium };
        // in the slot first: what Ready sets off (librespot starting, an Engine's commands) finds the session there
        self.set_session(Some(session));
        self.redial.logged_in(Instant::now());
        self.status(ready).await;
        LoggedIn::Yes
    }

    /// Logged out, Connect drops its remote too: it finds the session gone from the slot.
    async fn logout(&mut self) {
        self.tracks.clear();
        if let Some(s) = &self.session {
            s.shutdown();
        }
        self.set_session(None);
        self.redial.stop();
        let _ = std::fs::remove_file(self.dirs.credentials.join("credentials.json"));
        crate::secrets::clear("spotify");
        self.status(CatalogueStatus::LoggedOut).await;
    }

    /// The session, replaced first if Spotify ended it: a request can't wait for `look`.
    async fn session(&mut self) -> Option<Session> {
        if self.ended() {
            self.redial.ended(Instant::now());
            self.reconnect().await;
        }
        self.session.clone()
    }

    /// Did Spotify end the session? (librespot marks it when the connection closes or its keep-alive goes unanswered)
    fn ended(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.is_invalid())
    }

    /// Between two pieces of work (and every `LOOK_EVERY` while idle): a session Spotify ended is replaced at once,
    /// so the next search doesn't wait for the login (Q171); one that ended soon after its login, or a login that
    /// failed, is tried again when its wait is over.
    async fn look(&mut self) {
        let now = Instant::now();
        if self.ended() {
            let at = self.redial.ended(now);
            if at > now {
                tracing::info!("catalogue session ended soon after its login; logging in again in {} s", at.duration_since(now).as_secs());
                // gone: the next request doesn't log in again before the wait is over
                self.set_session(None);
                return;
            }
        } else if self.session.is_some() || !self.redial.due(now) {
            return;
        }
        self.reconnect().await;
    }

    /// Log in again with the saved login, on a line of its own with its time (Q171: a search's "Spotify" time
    /// doesn't include it).
    async fn reconnect(&mut self) {
        tracing::info!("catalogue session ended; reconnecting");
        // Connect sees the new session in the slot and starts a new remote for it (a dealer belongs to one)
        self.set_session(None);
        let began = Instant::now();
        let how = self.connect_how(None).await;
        let ms = began.elapsed().as_millis();
        match how {
            LoggedIn::Yes => tracing::info!("reconnect: logged in again in {ms} ms"),
            LoggedIn::NoLogin => {
                self.redial.stop();
                tracing::info!("reconnect: no saved login ({ms} ms)");
            }
            LoggedIn::Failed => {
                let now = Instant::now();
                let next = self.redial.failed(now).duration_since(now);
                tracing::info!("reconnect: failed after {ms} ms; trying again in {} s", next.as_secs());
            }
        }
    }

    /// One Pathfinder query through the port (D78); a session Spotify ended is replaced first.
    async fn query(&mut self, op: &str, vars: Value) -> anyhow::Result<Value> {
        self.session().await;
        self.queries.query(op, vars).await
    }

    async fn library(&mut self) {
        self.session().await;
        let walk = Walk::new(Paced::new(self.queries.clone()), self.username(), false);
        send_library(&self.ev, walk.items().await).await;
    }

    /// Home and the library side by side (#81), each sent as it's in, every step's time in the log. They share one
    /// gate: never more than `PARALLEL` requests out between them.
    async fn home_and_library(&mut self) {
        self.session().await;
        let paced = Paced::new(self.queries.clone());
        let walk = Walk::new(paced.clone(), self.username(), true);
        let ev = self.ev.clone();
        // owns all it uses, so a search can be made while it runs (#126)
        let fetch = async move {
            let home = async {
                let began = Instant::now();
                let feed = home_feed(&paced).await;
                timing("Home", began);
                send_home(&ev, feed).await;
            };
            let library = async {
                let began = Instant::now();
                let items = walk.items().await;
                let asked = walk.requests.load(std::sync::atomic::Ordering::Relaxed);
                timing(&format!("library ({asked} requests)"), began);
                send_library(&ev, items).await;
            };
            futures_util::join!(home, library);
        };
        let mut fetch = std::pin::pin!(fetch);
        let mut searched = self.pending.subscribe();
        // a search typed meanwhile is made at once, beside the fetch, not after all of it (#126)
        loop {
            tokio::select! {
                () = &mut fetch => return,
                _ = searched.changed() => {
                    let search = self.search_between();
                    let mut search = std::pin::pin!(search);
                    let mut fetched = false;
                    loop {
                        tokio::select! {
                            _ = &mut search => break,
                            () = &mut fetch, if !fetched => fetched = true,
                        }
                    }
                    if fetched {
                        return;
                    }
                }
            }
        }
    }

    /// The logged-in username (playlists it owns are marked `own`).
    fn username(&self) -> String {
        self.session.as_ref().map(|s| s.username()).unwrap_or_default()
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
            let (page, page_len) = match self.fetch_page(uri, offset).await {
                Ok(p) => p,
                Err(message) => {
                    self.fail(message).await;
                    return None;
                }
            };
            let total = page.total;
            let _ = self.ev.send(CatEvent::Tracks(page.clone())).await;
            pages.push(page);
            self.search_between().await;
            offset += page_len;
            if offset >= total.min(until) {
                return Some(pages);
            }
        }
    }

    /// One page of a list from `offset`, and how many rows a page of it holds; what went wrong if it failed.
    async fn fetch_page(&mut self, uri: &str, offset: u32) -> Result<(TrackPage, u32), String> {
        let asked = if self.books.contains(uri) { Some(library::chapters_query(uri, offset)) } else { library::tracks_query(uri, offset) };
        let Some((mut op, vars)) = asked else {
            return Err(format!("can't list the tracks of {uri}"));
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
            Err(e) => return Err(format!("Track list unavailable: {e}")),
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
        Ok((TrackPage { uri: uri.to_string(), offset, total, tracks, header }, page_len))
    }

    /// Demo mode's quiet look (#128): an artist's page, or a list's first page (from what's kept, else fetched alone),
    /// answered as `Peeked`; a failure is logged, not toasted, and answered with neither.
    async fn peek(&mut self, uri: String) {
        let (mut tracks, mut artist) = (None, None);
        if uri.starts_with("spotify:artist:") {
            match self.artists.get(&uri) {
                Some((at, p)) if at.elapsed() < TRACKS_TTL => artist = Some(p.clone()),
                _ => match self.query("queryArtistOverview", json!({ "uri": uri, "locale": "", "includePrerelease": true })).await {
                    Ok(v) => match library::parse_artist(&v) {
                        Some(p) => {
                            trim_timed(&mut self.artists, TRACKS_TTL, MAX_PAGES);
                            self.artists.insert(uri.clone(), (Instant::now(), p.clone()));
                            artist = Some(p);
                        }
                        None => tracing::warn!("demo Home: the artist page of {uri} had an unexpected shape"),
                    },
                    Err(e) => tracing::warn!("demo Home: artist page of {uri} unavailable: {e}"),
                },
            }
        } else {
            match self.tracks.get(&uri).and_then(|(at, pages)| (at.elapsed() < TRACKS_TTL).then(|| pages.first().cloned()).flatten()) {
                Some(p) => tracks = Some(p),
                // (one page alone isn't kept: a later open fetches the whole list)
                None => match self.fetch_page(&uri, 0).await {
                    Ok((p, _)) => tracks = Some(p),
                    Err(e) => tracing::warn!("demo Home: {e}"),
                },
            }
        }
        let _ = self.ev.send(CatEvent::Peeked { uri, tracks, artist }).await;
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
                    self.search_between().await;
                }
                Err(e) => {
                    tracing::debug!("areEntitiesInLibrary: {e}");
                    return;
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
            self.search_between().await;
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

    /// Commands that came meanwhile join those waiting, in order.
    fn take_arrived(&mut self) {
        while let Ok(c) = self.inbox.try_recv() {
            self.waiting.push_back(c);
        }
    }

    /// Work `name` starts; returns what was running (a search made in the middle of a command).
    fn begin(&mut self, name: String) -> Option<(String, Instant)> {
        self.busy.replace((name, Instant::now()))
    }

    /// The work running ends; what it interrupted (`was`) carries on.
    fn end(&mut self, was: Option<(String, Instant)>) {
        if let Some((name, _)) = std::mem::replace(&mut self.busy, was) {
            self.last = Some((name, Instant::now()));
        }
    }

    /// The search that came while browsing was busy, if one did: made now, unless a login or logout is queued (it
    /// waits for that). True if one was made. Long work calls this between two of its requests.
    async fn search_between(&mut self) -> bool {
        self.take_arrived();
        if self.waiting.iter().any(changes_session) {
            return false;
        }
        // taken quietly: an empty slot wakes no one
        let mut taken = None;
        self.pending.send_if_modified(|p| {
            taken = p.take();
            false
        });
        let Some((query, came)) = taken else { return false };
        self.search(query, came).await;
        true
    }

    /// `came`: when the Client's search reached the Catalogue. The log says where its time went (#126), without the
    /// words (what you search for is personal).
    async fn search(&mut self, query: String, came: Instant) {
        let waited = came.elapsed();
        // the work running now, else the last that ended after the search came
        let behind = match (&self.busy, &self.last) {
            (Some((b, _)), _) => format!(" behind {b}"),
            (None, Some((l, ended))) if *ended > came => format!(" behind {l}"),
            _ => String::new(),
        };
        let was = self.begin("Search".into());
        let how = self.search_now(query).await;
        self.end(was);
        tracing::info!("{}", search_line(waited, &behind, &how, came.elapsed()));
    }

    /// The answer, from what was kept or from Spotify, sent on; how it went, for the log.
    async fn search_now(&mut self, query: String) -> Searched {
        let key = query.trim().to_lowercase();
        if let Some((at, r)) = self.searches.get(&key)
            && at.elapsed() < SEARCH_TTL
        {
            let mut r = r.clone();
            r.query = query;
            let _ = self.ev.send(CatEvent::Search(r)).await;
            return Searched::Kept;
        }
        // a login again first, logged on its own: not Spotify's time for the search (Q171)
        self.session().await;
        let asked = Instant::now();
        let answer = self.query("searchDesktop", library::search_vars(query.trim())).await;
        let spotify = asked.elapsed();
        match answer {
            Ok(v) => {
                let read = Instant::now();
                let r = library::parse_search(&query, &v);
                let read = read.elapsed();
                let found = r.tracks.len() + r.episodes.len() + r.top.len() + r.artists.len() + r.albums.len() + r.playlists.len() + r.podcasts.len() + r.audiobooks.len();
                trim_timed(&mut self.searches, SEARCH_TTL, MAX_PAGES);
                self.searches.insert(key, (Instant::now(), r.clone()));
                let _ = self.ev.send(CatEvent::Search(r)).await;
                Searched::Found { spotify, read, found }
            }
            Err(e) => {
                // with the search, so the Client stops waiting on "searching…"
                let r = singularity_proto::SearchResults { query, error: Some(e.to_string()), ..Default::default() };
                let _ = self.ev.send(CatEvent::Search(r)).await;
                Searched::Failed { spotify }
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
    /// What the fallback search needs: the track's name and artists, from the Client's hint or a lookup.
    async fn lyrics_meta(&mut self, uri: &str, meta: Option<singularity_proto::Item>) -> Option<singularity_proto::Item> {
        match meta.filter(|m| !m.name.is_empty()) {
            Some(m) => Some(m),
            None => {
                self.describe(vec![uri.to_string()]).await;
                self.described.get(uri).cloned()
            }
        }
    }

    async fn lrclib_for(uri: &str, m: &singularity_proto::Item) -> Result<Option<singularity_proto::Lyrics>, String> {
        crate::lyrics::lrclib(uri, &m.artists.first().cloned().unwrap_or_default(), &m.name, m.album.as_deref(), m.duration_ms).await
    }

    /// Line-synced lyrics without word times, for a Karaoke Widget: LRCLIB's word-synced version instead when it has
    /// one (D67, the owner: yes). Asked once per track, whatever the answer.
    async fn with_words(&mut self, l: singularity_proto::Lyrics, meta: Option<singularity_proto::Item>) -> singularity_proto::Lyrics {
        if !l.synced || l.lines.iter().any(|x| !x.words.is_empty()) || self.word_tried.contains_key(&l.uri) {
            return l;
        }
        trim_any(&mut self.word_tried, MAX_LYRICS);
        self.word_tried.insert(l.uri.clone(), ());
        let Some(m) = self.lyrics_meta(&l.uri, meta).await else { return l };
        match Self::lrclib_for(&l.uri, &m).await {
            Ok(Some(w)) if w.synced && w.lines.iter().any(|x| !x.words.is_empty()) => {
                tracing::info!("lyrics for {}: word timing from LRCLIB", l.uri);
                trim_any(&mut self.lyrics, MAX_LYRICS);
                self.lyrics.insert(l.uri.clone(), w.clone());
                w
            }
            Ok(_) => l,
            Err(e) => {
                tracing::info!("lyrics for {}: LRCLIB (word timing) failed: {e}", l.uri);
                l
            }
        }
    }

    async fn lyrics(&mut self, uri: String, meta: Option<singularity_proto::Item>, words: bool) {
        use crate::lyrics::Outcome;
        if let Some(l) = self.lyrics.get(&uri).cloned() {
            let l = if words { self.with_words(l, meta).await } else { l };
            let _ = self.ev.send(CatEvent::Lyrics(l)).await;
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
            // LRCLIB can take seconds
            self.search_between().await;
            // LRCLIB answered here: its word timing, if any, is already in it
            self.word_tried.insert(uri.clone(), ());
            lrclib = match self.lyrics_meta(&uri, meta.clone()).await {
                Some(m) => match Self::lrclib_for(&uri, &m).await {
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
        let l = if words && keep { self.with_words(l, meta).await } else { l };
        let _ = self.ev.send(CatEvent::Lyrics(l)).await;
    }

    async fn home(&mut self) {
        self.session().await;
        let feed = home_feed(&Paced::new(self.queries.clone())).await;
        send_home(&self.ev, feed).await;
    }
}

/// The Home feed, asked through `q`.
async fn home_feed<Q: Fetch + Sync + Clone>(q: &Paced<Q>) -> anyhow::Result<HomeFeed> {
    let vars = json!({
        "includeHomeChapterVideoCards": false,
        "homeEndUserIntegration": "INTEGRATION_WEB_PLAYER",
        "timeZone": local_timezone(),
        "sp_t": "",
        "facet": "",
        "sectionItemsLimit": 12,
        "includeEpisodeContentRatingsV2": false,
    });
    Ok(parse_home(&q.ask("home", vars, None).await?))
}

/// A Home feed for the Daemon, or what went wrong.
async fn send_home(ev: &mpsc::Sender<CatEvent>, feed: anyhow::Result<HomeFeed>) {
    match feed {
        Ok(feed) => {
            if feed.shelves.is_empty() {
                let _ = ev.send(CatEvent::Error("Spotify answered, but the Home feed had an unexpected shape".into())).await;
            }
            let _ = ev.send(CatEvent::Home(feed)).await;
        }
        Err(e) => {
            let _ = ev.send(CatEvent::Error(format!("Home feed unavailable: {e}"))).await;
        }
    }
}

/// The library for the Daemon, or what went wrong.
async fn send_library(ev: &mpsc::Sender<CatEvent>, items: anyhow::Result<Vec<singularity_proto::LibraryItem>>) {
    let _ = match items {
        Ok(items) => ev.send(CatEvent::Library(Library { items })).await,
        Err(e) => ev.send(CatEvent::Error(format!("Library unavailable: {e}"))).await,
    };
}

/// How many of Home's and the library's requests are out at once (#81), between them. Spotify documents no limit for
/// these private queries (`docs/research/2026-10-private-api-risk.md`: human-paced, back off on 429/5xx, never retry
/// in a loop); librespot's own guess is 300 calls per 30 s, and it waits out a 429 that says how long. A library of a
/// few hundred items is a handful of requests.
const PARALLEL: usize = 4;
/// A request refused for the rate (a 429 that doesn't say how long, or librespot's own limiter) or failed on
/// Spotify's side (5xx) is asked once more after this and a jitter of up to as long again; then it fails (never
/// retried in a loop).
const BACKOFF: Duration = Duration::from_secs(2);

/// Is this a failure worth one more try after a pause: refused for the rate (429), or Spotify's side failing (5xx: 500
/// and 503 come back as Unavailable, 502 as an unknown status; a 504 is already asked again by the adapter)?
fn worth_another_try(e: &anyhow::Error) -> bool {
    use librespot_core::error::ErrorKind;
    e.downcast_ref::<librespot_core::Error>().is_some_and(|e| match e.kind {
        ErrorKind::ResourceExhausted | ErrorKind::Unavailable => true,
        ErrorKind::Unknown => e.to_string().contains("status code: 5"),
        _ => false,
    })
}

/// Up to `max` more: a cheap jitter, without a random number generator.
fn jitter(max: Duration) -> Duration {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    max.mul_f64(nanos as f64 / 1e9)
}

/// When the Catalogue started: the timings in the log count from here.
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// One step's time in the log (#81): `grep timing: daemon.log` shows where a slow start went.
fn timing(step: &str, began: Instant) {
    let at = STARTED.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    tracing::info!("timing: {step} took {} ms ({at:.1} s after the Catalogue started)", began.elapsed().as_millis());
}

/// Queries through the port behind one gate (#81): at most `PARALLEL` out among all clones, and one more try after
/// a pause for a 429 or a 5xx.
#[derive(Clone)]
struct Paced<Q> {
    q: Q,
    gate: std::sync::Arc<tokio::sync::Semaphore>,
}

impl<Q: Fetch + Sync + Clone> Paced<Q> {
    fn new(q: Q) -> Paced<Q> {
        Paced { q, gate: std::sync::Arc::new(tokio::sync::Semaphore::new(PARALLEL)) }
    }

    /// One query; `what` names it in a `timing:` line.
    async fn ask(&self, op: &str, vars: Value, what: Option<&str>) -> anyhow::Result<Value> {
        match self.once(op, vars.clone(), what).await {
            Err(e) if worth_another_try(&e) => {
                let wait = BACKOFF + jitter(BACKOFF);
                tracing::warn!("`{op}`: {e}; asking once more in {} ms", wait.as_millis());
                // the pause holds no place at the gate
                tokio::time::sleep(wait).await;
                self.once(op, vars, what).await
            }
            r => r,
        }
    }

    async fn once(&self, op: &str, vars: Value, what: Option<&str>) -> anyhow::Result<Value> {
        let _out = self.gate.acquire().await.expect("the gate is never closed");
        let began = Instant::now();
        let r = self.q.clone().query(op, vars).await;
        if let Some(what) = what {
            timing(what, began);
        }
        r
    }
}

/// The library walk (#81): the top level, then every folder's contents spliced in right after it (one level deeper).
/// Every page of a level after the first, and every folder of one depth, are asked at once (as many as the gate lets
/// out); the items come out in the order a walk one request at a time gives.
struct Walk<Q> {
    q: Paced<Q>,
    me: String,
    /// each request's time in the log (the walk after a login)
    log: bool,
    requests: std::sync::atomic::AtomicUsize,
}

impl<Q: Fetch + Sync + Clone> Walk<Q> {
    fn new(q: Paced<Q>, me: String, log: bool) -> Walk<Q> {
        Walk { q, me, log, requests: Default::default() }
    }

    async fn items(&self) -> anyhow::Result<Vec<singularity_proto::LibraryItem>> {
        let mut items = self.level(None, 0, "the top level".into()).await?;
        // folders nest a few levels at most; guard anyway
        for depth in 0..4 {
            let folders: Vec<(usize, String)> = items.iter().enumerate().filter(|(_, it)| it.kind == "folder" && it.depth == depth).map(|(i, it)| (i, it.uri.clone())).collect();
            if folders.is_empty() {
                break;
            }
            let n = folders.len();
            let children = futures_util::future::try_join_all(folders.iter().enumerate().map(|(k, (_, uri))| self.level(Some(uri), depth + 1, format!("folder {} of {n} at depth {depth}", k + 1)))).await?;
            // from the last, so the earlier indices still hold
            for ((i, _), c) in folders.iter().zip(children).rev() {
                items.splice(i + 1..i + 1, c);
            }
        }
        Ok(items)
    }

    /// One level: its first page says how many there are, the rest are asked at once.
    async fn level(&self, folder: Option<&str>, depth: u8, name: String) -> anyhow::Result<Vec<singularity_proto::LibraryItem>> {
        let first = self.page(folder, 0, depth, &name).await?;
        let total = first.total;
        let mut items = first.items;
        if !library_page_continues(items.len(), library::LIBRARY_PAGE, total, items.len()) {
            return Ok(items);
        }
        let offsets = (1..).map(|k| k * library::LIBRARY_PAGE).take_while(|o| *o < total && *o < MAX_TRACKS);
        let name = &name;
        let mut pages = futures_util::stream::iter(offsets).map(|o| self.page(folder, o, depth, name).map(move |p| (o, p))).buffered(PARALLEL);
        while let Some((offset, page)) = pages.next().await {
            let page = page?;
            let n = page.items.len();
            items.extend(page.items);
            if !library_page_continues(n, offset.saturating_add(library::LIBRARY_PAGE), total, items.len()) {
                break;
            }
        }
        Ok(items)
    }

    async fn page(&self, folder: Option<&str>, offset: u32, depth: u8, name: &str) -> anyhow::Result<library::LibraryPage> {
        self.requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let what = self.log.then(|| format!("library page at {offset} of {name}"));
        let v = self.q.ask("libraryV3", library::library_vars(offset, folder), what.as_deref()).await?;
        Ok(library::parse_library(&v, depth, &self.me))
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
        // the section's own id (`spotify:section:…`), when it has one
        let id = s["uri"].as_str().unwrap_or_default().to_string();
        let title = s["data"]["title"]["transformedLabel"].as_str().or_else(|| s["data"]["title"]["text"].as_str()).unwrap_or("").to_string();
        let cards: Vec<Card> = s["sectionItems"]["items"].as_array().into_iter().flatten().filter_map(|it| card(&it["content"]["data"])).collect();
        if !cards.is_empty() {
            shelves.push(Shelf { id, title, kind: kind.into(), cards });
        }
    }
    HomeFeed { fetched_unix_ms: singularity_proto::now_ms(), shelves }
}

#[cfg(test)]
mod port_tests;

#[cfg(test)]
mod tests {
    /// A2 (2026-11-20 bug hunt): a playlist command sent its sidebar refresh back into the Catalogue's own channel of
    /// 16. With the channel full that send never returned: the Catalogue stopped for good, and every Client session
    /// and the Engine task waiting to send it something stopped with it.
    #[tokio::test]
    async fn a_full_queue_of_playlist_edits_does_not_lock_the_catalogue() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs { credentials: dir.path().join("creds"), queries: dir.path().join("q.json") };
        let (tx, mut ev) = spawn(dirs, crate::engine::Identity::default());
        // whatever it reports is read, as the Hub does
        tokio::spawn(async move { while ev.recv().await.is_some() {} });
        let flood = async {
            for i in 0..100 {
                tx.send(CatCmd::DeletePlaylist(format!("spotify:playlist:{i}"))).await.unwrap();
            }
            // and it still answers
            let (reply, answer) = tokio::sync::oneshot::channel();
            tx.send(CatCmd::Query { op: "x".into(), vars: Value::Null, reply }).await.unwrap();
            answer.await.unwrap()
        };
        let answer = tokio::time::timeout(Duration::from_secs(10), flood).await.expect("the Catalogue locked up");
        assert!(answer.is_err(), "logged out: no query runs");
    }

    /// Q171: a session that lasted is replaced at once; quick ends and failed logins wait longer each time, up to a
    /// cap; a login that holds starts over; logging out stops it.
    #[test]
    fn logging_in_again_backs_off_and_starts_over() {
        use super::*;
        let t0 = Instant::now();
        let mut r = Redial::new();
        r.logged_in(t0);
        let t = t0 + STEADY;
        assert_eq!(r.ended(t), t, "a session that lasted: at once");
        assert!(r.due(t));
        // no network: later and later
        let waits: Vec<u64> = (0..8).map(|_| r.failed(t).duration_since(t).as_secs()).collect();
        assert_eq!(waits, [5, 10, 20, 40, 80, 160, 300, 300]);
        assert!(!r.due(t) && r.due(t + REDIAL_MAX));
        // it comes back and holds: the next ordinary end is at once again
        r.logged_in(t);
        assert!(!r.due(t + REDIAL_MAX), "nothing waits once logged in");
        let t = t + STEADY;
        assert_eq!(r.ended(t), t);
        // a session that ends soon after its login waits, longer each time
        r.logged_in(t);
        assert_eq!(r.ended(t + Duration::from_secs(1)).duration_since(t).as_secs(), 6);
        r.logged_in(t);
        assert_eq!(r.ended(t + Duration::from_secs(1)).duration_since(t).as_secs(), 11);
        r.stop();
        assert!(!r.due(t + REDIAL_MAX), "logged out: nothing waits");
        r.logged_in(t);
        assert_eq!(r.ended(t + Duration::from_secs(1)).duration_since(t).as_secs(), 6, "and it starts over");
    }

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
      {"uri":"spotify:section:fakemadeforyou","data":{"__typename":"HomeGenericSectionData","title":{"transformedLabel":"Made For You"}},"sectionItems":{"items":[
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

    /// #126: a search's log line says where its time went, and is never given the words to write.
    #[test]
    fn a_search_log_line_says_where_the_time_went() {
        let ms = Duration::from_millis;
        let found = Searched::Found { spotify: ms(312), read: ms(1), found: 47 };
        assert_eq!(search_line(ms(2140), " behind Describe", &found, ms(2453)), "search: waited 2140 ms behind Describe, Spotify 312 ms, read 1 ms, 47 results; 2453 ms in all");
        assert_eq!(search_line(ms(0), "", &Searched::Kept, ms(1)), "search: waited 0 ms, answered from what was kept; 1 ms in all");
        assert_eq!(search_line(ms(5), "", &Searched::Failed { spotify: ms(12_000) }, ms(12_005)), "search: waited 5 ms, Spotify failed after 12000 ms; 12005 ms in all");
    }

    /// #126: the log names what a search waited behind by the command alone, none of what it carries.
    #[test]
    fn a_command_is_logged_by_its_name_alone() {
        assert_eq!(cmd_name(&CatCmd::Tracks("spotify:playlist:p".into())), "Tracks");
        assert_eq!(cmd_name(&CatCmd::Lyrics { uri: "spotify:track:t".into(), meta: None, words: true }), "Lyrics");
        assert_eq!(cmd_name(&CatCmd::Library), "Library");
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
        // (a section without an id has none)
        assert_eq!((feed.shelves[1].id.as_str(), feed.shelves[0].id.as_str()), ("spotify:section:fakemadeforyou", ""));
        assert_eq!(mix.subtitle, "Nova Drift, Mira Vale & more");
        assert_eq!(feed.shelves[1].cards[1].kind, "artist");
        assert_eq!(feed.shelves[2].kind, "feed");
        assert_eq!(feed.shelves[2].cards[0].image_url, None);
    }
}
