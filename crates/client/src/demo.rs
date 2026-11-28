//! Demo mode (D90): a Client started with `--demo` (or `SINGULARITY_DEMO=1`) for recording videos on the owner's own
//! account. Daemon events pass [`Demo::scrub`] where they enter (`App::receive`, D82): the library shows only the Demo
//! set, Home is built from the Demo set alone (Q155, Q157), the player stays empty until a song from the Demo set or
//! one this Client played is on (Q156), devices and Jam guests get neutral names, the Jam's invite is swapped for one
//! that leads nowhere, and every free text goes through the [`Redactor`], which replaces what it has learned is
//! personal (the account's username and display name, device and computer names, `$HOME`, `$USER`). Only this Client
//! changes: the Daemon and other Clients don't.
//!
//! Everything here is a function on data: no terminal, no Daemon.

use singularity_proto::{Card, Command, Event, HomeFeed, Library, LibraryItem, PlaybackState, SearchResults, Shelf, TrackPage, TrackRow};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The Spotify folder whose playlists the library shows.
pub const FOLDER: &str = "Demo";
/// The library's only row when there's no such folder.
pub const HINT: &str = "Make a folder called Demo in Spotify";
/// This computer's Engine, wherever its name would show.
pub const ENGINE: &str = "Singularity";
/// What the Jam's invite QR encodes instead (padded to the real link's length, so the QR is as big).
pub const FAKE_INVITE: &str = "https://example.invalid/demo";
/// What shows instead of the Jam's invite link.
pub const LINK_HIDDEN: &str = "Invite link hidden in demo mode";
/// The owner's own name, and other people's.
pub const YOU: &str = "you";
pub const FRIEND: &str = "a friend";
/// A Jam's host that isn't one of the account's devices.
pub const FRIENDS_DEVICE: &str = "A friend's device";
/// What a demo Client says when its Daemon didn't confirm demo mode (an older one): no stats, no export or delete.
pub const TOO_OLD: &str = "This background player is too old for demo mode; restart it";
/// A context that isn't in the Demo set ("Playing from a playlist").
pub const SOME_PLAYLIST: &str = "a playlist";
/// Home's Shelves in demo mode (Q157), in this order.
pub const PLAYLISTS: &str = "Playlists";
pub const ALBUMS: &str = "Albums";
pub const ARTISTS: &str = "Artists";
pub const LIKED: &str = "Liked Songs";
/// How many Demo playlists' songs Home reads (a fetch each), and how many albums and artists it shows (an artist's
/// picture is a fetch each).
const MAX_LISTS: usize = 12;
const MAX_ALBUMS: usize = 20;
const MAX_ARTISTS: usize = 12;

static ON: AtomicBool = AtomicBool::new(false);

/// This Client runs in demo mode (set once at start, before anything is read or drawn).
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn turn_on() {
    ON.store(true, Ordering::Relaxed);
}

/// The switch: `--demo` among the arguments, or `SINGULARITY_DEMO=1`.
pub fn asked(args: &[String], env: Option<&str>) -> bool {
    args.iter().any(|a| a == "--demo") || env.is_some_and(|v| v == "1")
}

/// `demo.toml` in the config folder: the Demo set's albums, artists, podcasts and audiobooks, and Liked Songs. (An
/// older file's `[home]` rules are ignored: Home is built from the Demo set alone.)
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct DemoFile {
    pub library: Vec<String>,
    pub liked: bool,
}

pub fn file_path() -> PathBuf {
    singularity_config::config_path().with_file_name("demo.toml")
}

/// What `demo.toml` says, and what's wrong with it (a missing file is an empty Demo set, not a problem).
pub fn load(path: &Path) -> (DemoFile, Option<String>) {
    match std::fs::read_to_string(path) {
        Err(_) => (DemoFile::default(), None),
        Ok(s) => match singularity_config::toml_edit::de::from_str::<DemoFile>(&s) {
            Ok(f) => (f, None),
            Err(e) => (DemoFile::default(), Some(format!("demo.toml: {}", e.message()))),
        },
    }
}

/// "Add to demo": `uri` at the end of `library` in `demo.toml` (made if missing; comments kept). False: already there.
pub fn add_to_file(path: &Path, uri: &str) -> anyhow::Result<bool> {
    use singularity_config::toml_edit::{Array, DocumentMut, Item, Value};
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: DocumentMut = text.parse()?;
    let list = doc.entry("library").or_insert(Item::Value(Value::Array(Array::new())));
    let Some(arr) = list.as_array_mut() else { anyhow::bail!("library in demo.toml isn't a list") };
    if arr.iter().any(|v| v.as_str() == Some(uri)) {
        return Ok(false);
    }
    arr.push(uri);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, doc.to_string())?;
    Ok(true)
}

/// Replaces what it has learned is personal, each kind where it can be told from ordinary words: the account's
/// username and display name (and Jam guests) as whole words anywhere, ignoring ASCII case; device and computer names
/// only as a whole text or in quotes (a device called "Speaker" mustn't eat "Speaker & the Bunnymen"); `$USER` only as
/// a path's folder, `$HOME` where it starts a path.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    /// (secret, stand-in, kind), longest first
    secrets: Vec<(String, String, Kind)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// a whole word anywhere
    Word,
    /// the whole text, or in quotes
    Whole,
    /// a path's folder: after a `/`
    Folder,
    /// the start of a path
    Prefix,
}

impl Redactor {
    /// One from this process's surroundings: `$HOME` → `~`, `$USER` → `user`, the computer's name → `computer`.
    pub fn from_env(home: Option<String>, user: Option<String>, computer: Option<String>) -> Redactor {
        let mut r = Redactor::default();
        if let Some(h) = home.filter(|h| h.len() > 1) {
            r.add(h.trim_end_matches('/'), "~", Kind::Prefix);
        }
        if let Some(u) = user {
            r.add(u.trim(), "user", Kind::Folder);
        }
        if let Some(c) = computer {
            r.learn_device(&c, "computer");
        }
        r
    }

    /// `secret` (the username, the display name, a guest's name) shows as `with` from now on, wherever it is a whole
    /// word. Names shorter than two characters are left alone (they'd eat ordinary words), as is a name that is its
    /// own stand-in.
    pub fn learn(&mut self, secret: &str, with: &str) {
        self.add(secret.trim(), with, Kind::Word);
    }

    /// A device's or the computer's name: replaced where it is the whole text, or quoted.
    pub fn learn_device(&mut self, secret: &str, with: &str) {
        self.add(secret.trim(), with, Kind::Whole);
    }

    fn add(&mut self, secret: &str, with: &str, kind: Kind) {
        if secret.chars().count() < 2 || secret.eq_ignore_ascii_case(with) || self.secrets.iter().any(|(s, ..)| s.eq_ignore_ascii_case(secret)) {
            return;
        }
        self.secrets.push((secret.to_string(), with.to_string(), kind));
        // longer first: "Singularity on box" before "box"
        self.secrets.sort_by_key(|(s, ..)| std::cmp::Reverse(s.len()));
    }

    pub fn redact(&self, s: &str) -> String {
        let mut out = s.to_string();
        for (secret, with, kind) in &self.secrets {
            out = match kind {
                Kind::Whole if out.trim() == secret => with.clone(),
                Kind::Whole => [('"', '"'), ('“', '”'), ('\'', '\''), ('‘', '’')].iter().fold(out, |o, (a, b)| o.replace(&format!("{a}{secret}{b}"), &format!("{a}{with}{b}"))),
                k => replace(&out, secret, with, *k),
            };
        }
        out
    }
}

/// `s` with every `secret` (ASCII case ignored) replaced by `with`, where no letter or number follows it, and before
/// it: none either (a word), a `/` (a path's folder), anything (a path's start).
fn replace(s: &str, secret: &str, with: &str, kind: Kind) -> String {
    let n = secret.len();
    let inside = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let hit = i + n <= s.len()
            && s.is_char_boundary(i + n)
            && s[i..i + n].eq_ignore_ascii_case(secret)
            && match kind {
                Kind::Word => !inside(s[..i].chars().next_back()),
                Kind::Folder => s[..i].ends_with('/'),
                _ => true,
            }
            && !inside(s[i + n..].chars().next());
        if hit {
            out.push_str(with);
            i += n;
        } else {
            let c = s[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// The neutral name for a device of `kind`, the `n`-th of that kind (1-based): "Phone", "Phone 2", "Speaker", "TV".
pub fn neutral_name(kind: &str, n: usize) -> String {
    let base = match kind {
        "smartphone" => "Phone",
        "tablet" => "Tablet",
        "speaker" | "avr" | "castaudio" | "cast_audio" | "audiodongle" => "Speaker",
        "tv" | "castvideo" | "cast_video" | "stb" => "TV",
        "computer" => "Computer",
        "gameconsole" | "game_console" => "Console",
        "automobile" => "Car",
        _ => "Device",
    };
    if n <= 1 { base.to_string() } else { format!("{base} {n}") }
}

/// One demo Client's filter: what it has learned so far, and the Demo set.
#[derive(Debug, Clone, Default)]
pub struct Demo {
    pub file: DemoFile,
    pub redactor: Redactor,
    /// the Demo set as the last library had it: its playlists, the allow-listed items, Liked Songs if on
    set: HashSet<String>,
    /// every playlist of the owner's (to tell "you" from "a friend"), and those outside the Demo set (dropped from
    /// Search and artist pages)
    own: HashSet<String>,
    own_outside: HashSet<String>,
    /// playlists created in this session: they stay in the library
    created: HashSet<String>,
    /// the Daemon said it runs this Client in demo mode (its Welcome): until then no stats, export or delete
    pub confirmed: bool,
    /// device id → its neutral name, stable for the session
    devices: HashMap<String, String>,
    /// Jam member id → "Guest N"
    guests: HashMap<String, String>,
    /// the last library and playback as the Daemon sent them, scrubbed again when what decides them changes
    raw_library: Option<Library>,
    raw_playback: Option<PlaybackState>,
    /// the player as last passed on (it goes again only when it would look different)
    shown_playback: Option<PlaybackState>,
    /// the library as shown (the Demo set), what Home is built from
    shown: Vec<LibraryItem>,
    /// the Demo playlists' songs, as far as they've come (their albums and artists make Home's Shelves)
    songs: HashMap<String, Vec<TrackRow>>,
    /// an artist's picture, once its page came
    artist_images: HashMap<String, Option<String>>,
    /// track lists and artist pages asked for, so each is asked once
    asked: HashSet<String>,
    /// what this Client asked to play or queue in this session: shown when it plays
    played: HashSet<String>,
    /// when the Daemon's last Home feed was fetched (demo Home takes its time, not its Shelves)
    home_fetched: u64,
    /// Home and the player changed with what came (a library, a Demo playlist's songs, an artist's picture)
    changed: bool,
    /// this Client played or queued something new: the player may show now
    replay: bool,
}

/// What a demo Client does after an event: Home and the player as they are now, and what to fetch for Home.
#[derive(Debug, Default)]
pub struct FollowUp {
    pub home: Option<HomeFeed>,
    pub playback: Option<PlaybackState>,
    pub fetch: Vec<Command>,
}

impl Demo {
    pub fn new(file: DemoFile, redactor: Redactor) -> Demo {
        Demo { file, redactor, ..Default::default() }
    }

    pub fn redact(&self, s: &str) -> String {
        self.redactor.redact(s)
    }

    /// A Daemon event as this Client may show it.
    pub fn scrub(&mut self, ev: Event) -> Event {
        use singularity_proto::{CatalogueStatus as C, EngineStatus as E};
        match ev {
            Event::Engine(E::NeedsPairing { device_name }) => {
                self.redactor.learn_device(&device_name, ENGINE);
                Event::Engine(E::NeedsPairing { device_name: ENGINE.into() })
            }
            Event::Engine(E::Ready { device_name, active, version, expires_in_days }) => {
                self.redactor.learn_device(&device_name, ENGINE);
                Event::Engine(E::Ready { device_name: ENGINE.into(), active, version, expires_in_days })
            }
            Event::Engine(E::NotConfigured { reason }) => Event::Engine(E::NotConfigured { reason: self.redact(&reason) }),
            Event::Engine(E::Failed { message }) => Event::Engine(E::Failed { message: self.redact(&message) }),
            Event::Catalogue(C::Ready { username, country, premium }) => {
                self.redactor.learn(&username, "user");
                Event::Catalogue(C::Ready { username: String::new(), country, premium })
            }
            Event::Catalogue(C::Unavailable { message }) => Event::Catalogue(C::Unavailable { message: self.redact(&message) }),
            Event::Home(h) => {
                self.home_fetched = h.fetched_unix_ms;
                Event::Home(self.home())
            }
            Event::Library(l) => {
                self.raw_library = Some(l.clone());
                self.changed = true;
                Event::Library(self.library(l))
            }
            Event::Tracks(p) => {
                self.keep_songs(&p);
                Event::Tracks(self.tracks(p))
            }
            Event::Search(r) => Event::Search(self.search(r)),
            Event::Artist(p) => {
                if self.asked.contains(&p.uri) && self.artist_images.insert(p.uri.clone(), p.image_url.clone()) != Some(p.image_url.clone()) {
                    self.changed = true;
                }
                Event::Artist(self.artist(p))
            }
            Event::Devices { devices } => Event::Devices { devices: devices.into_iter().map(|d| self.device(d)).collect() },
            Event::Jam { jam } => Event::Jam { jam: jam.map(|j| self.jam(j)) },
            Event::Playback(p) => {
                self.raw_playback = Some(p.clone());
                let shown = self.playback(p);
                self.shown_playback = Some(shown.clone());
                Event::Playback(shown)
            }
            Event::Health(mut h) => {
                h.soloist_key = self.redact(&h.soloist_key);
                h.spotify_login = self.redact(&h.spotify_login);
                h.audio_output = h.audio_output.map(|_| "playing".into());
                Event::Health(h)
            }
            Event::Notice { message } => Event::Notice { message: self.redact(&message) },
            Event::Error { message } => Event::Error { message: self.redact(&message) },
            ev => ev,
        }
    }

    /// A playlist this Client made in this session: it stays in the library (one made elsewhere doesn't).
    pub fn created(&mut self, uri: &str) {
        self.created.insert(uri.to_string());
    }

    /// The library has come at least once (so the owner's playlists and display name are known).
    pub fn knows_library(&self) -> bool {
        self.raw_library.is_some()
    }

    /// The library again, after something that decides it changed (a playlist made here): None before the first.
    pub fn library_again(&mut self) -> Option<Library> {
        let l = self.raw_library.clone()?;
        Some(self.library(l))
    }

    /// After an event: Home and the player again if what decides them changed (nothing before the library), and the
    /// Demo playlists' songs and artists' pages Home still needs.
    pub fn follow_up(&mut self) -> FollowUp {
        if !self.knows_library() {
            return FollowUp::default();
        }
        let mut up = FollowUp::default();
        let replay = std::mem::take(&mut self.replay);
        if std::mem::take(&mut self.changed) {
            up.home = Some(self.home());
        }
        if (replay || up.home.is_some())
            && let Some(p) = self.raw_playback.clone().map(|p| self.playback(p))
            && self.shown_playback.as_ref() != Some(&p)
        {
            self.shown_playback = Some(p.clone());
            up.playback = Some(p);
        }
        let lists: Vec<String> = self.demo_playlists().take(MAX_LISTS).map(|i| i.uri.clone()).collect();
        for uri in lists {
            if !self.songs.contains_key(&uri) && self.asked.insert(uri.clone()) {
                up.fetch.push(Command::GetTracks { uri });
            }
        }
        for (uri, ..) in self.artists() {
            if !self.artist_images.contains_key(&uri) && self.asked.insert(uri.clone()) {
                up.fetch.push(Command::GetArtist { uri });
            }
        }
        up
    }

    /// A command this Client sends: what it plays or queues shows in the player when it's on.
    pub fn sent(&mut self, c: &Command) {
        let uris: Vec<&String> = match c {
            Command::Play { uri: Some(u) } | Command::AddToQueue { uri: u } | Command::PlayNext { uri: u } => vec![u],
            Command::PlayFrom { context_uri, track_uri, .. } => std::iter::once(context_uri).chain(track_uri).collect(),
            _ => vec![],
        };
        for u in uris {
            if self.played.insert(u.clone()) {
                self.replay = true;
            }
        }
    }

    /// The Demo set's playlists, in the library's order.
    fn demo_playlists(&self) -> impl Iterator<Item = &LibraryItem> {
        self.shown.iter().filter(|i| i.kind == "playlist")
    }

    /// A page of a Demo playlist's songs, kept for Home.
    fn keep_songs(&mut self, p: &TrackPage) {
        if !self.demo_playlists().any(|i| i.uri == p.uri) {
            return;
        }
        let rows = self.songs.entry(p.uri.clone()).or_default();
        rows.truncate(p.offset as usize);
        rows.extend(p.tracks.iter().cloned());
        self.changed = true;
    }

    /// The Demo playlists' songs (those Home reads), in order.
    fn demo_songs(&self) -> impl Iterator<Item = &TrackRow> {
        self.demo_playlists().take(MAX_LISTS).filter_map(|i| self.songs.get(&i.uri)).flatten().filter(|t| t.playable)
    }

    /// Things the Demo playlists' songs are on (an album, an artist), the most songs first (then the first seen), as
    /// (uri, its first song), leaving out those the library already holds, at most `max`.
    fn ranked(&self, of: impl Fn(&TrackRow) -> Vec<String>, max: usize) -> Vec<(String, &TrackRow)> {
        let mut seen: Vec<(String, &TrackRow, usize)> = Vec::new();
        let mut at: HashMap<String, usize> = HashMap::new();
        for t in self.demo_songs() {
            for uri in of(t) {
                if uri.is_empty() || self.shown.iter().any(|i| i.uri == uri) {
                    continue;
                }
                match at.get(&uri) {
                    Some(&n) => seen[n].2 += 1,
                    None => {
                        at.insert(uri.clone(), seen.len());
                        seen.push((uri, t, 1));
                    }
                }
            }
        }
        // (a stable sort: ties stay in the order first seen)
        seen.sort_by_key(|(.., n)| std::cmp::Reverse(*n));
        seen.into_iter().take(max).map(|(u, r, _)| (u, r)).collect()
    }

    /// The Demo playlists' artists Home shows: (uri, name), after those `demo.toml` lists.
    fn artists(&self) -> Vec<(String, String)> {
        let listed = self.shown.iter().filter(|i| i.kind == "artist").count();
        self.ranked(|t| t.artist_uris.clone(), MAX_ARTISTS.saturating_sub(listed))
            .into_iter()
            .filter_map(|(uri, t)| {
                let n = t.artist_uris.iter().position(|u| *u == uri)?;
                Some((uri, t.artists.get(n)?.clone()))
            })
            .collect()
    }

    /// Only the Demo set: the "Demo" folder's playlists (flattened to the top level), what `demo.toml` lists, Liked
    /// Songs if it says so, and playlists made in this session. Owners are "you" or "a friend".
    pub fn library(&mut self, l: Library) -> Library {
        for it in l.items.iter().filter(|i| i.own) {
            for a in &it.authors {
                self.redactor.learn(a, YOU);
            }
        }
        self.own = l.items.iter().filter(|i| i.own).map(|i| i.uri.clone()).collect();
        let folder = l.items.iter().position(|i| i.kind == "folder" && i.name.trim().eq_ignore_ascii_case(FOLDER));
        let in_folder: HashSet<usize> = match folder {
            Some(f) => {
                let depth = l.items[f].depth;
                l.items.iter().enumerate().skip(f + 1).take_while(|(_, i)| i.depth > depth).filter(|(_, i)| i.kind == "playlist").map(|(n, _)| n).collect()
            }
            None => HashSet::new(),
        };
        let mut items: Vec<LibraryItem> = l
            .items
            .into_iter()
            .enumerate()
            .filter(|(n, i)| in_folder.contains(n) || self.file.library.contains(&i.uri) || (i.kind == "liked" && self.file.liked) || self.created.contains(&i.uri))
            .map(|(_, mut i)| {
                i.depth = 0;
                if i.kind == "playlist" {
                    let who = if i.own { YOU } else if i.authors == ["Spotify"] { "Spotify" } else { FRIEND };
                    i.subtitle = format!("Playlist · {who}");
                    i.authors = vec![who.into()];
                }
                i.subtitle = self.redact(&i.subtitle);
                i
            })
            .collect();
        self.set = items.iter().map(|i| i.uri.clone()).collect();
        self.own_outside = self.own.difference(&self.set).cloned().collect();
        self.shown = items.clone();
        if folder.is_none() {
            items.insert(0, LibraryItem { uri: String::new(), name: HINT.into(), subtitle: String::new(), kind: "hint".into(), image_url: None, depth: 0, own: false, authors: vec![] });
        }
        Library { items }
    }

    /// Home from the Demo set alone (Q155, Q157): its playlists, then albums and then artists (those `demo.toml` lists
    /// first, then the ones the Demo playlists' songs are on, the most songs first), then Liked Songs if it's on.
    /// Nothing from the account's own Home: no recommendations, stations or history.
    pub fn home(&self) -> HomeFeed {
        let card = |i: &LibraryItem, kind: &str| Card { uri: i.uri.clone(), name: i.name.clone(), subtitle: i.subtitle.clone(), image_url: i.image_url.clone(), kind: kind.into() };
        let listed = |kind: &str| self.shown.iter().filter(|i| i.kind == kind).map(|i| card(i, kind)).collect::<Vec<_>>();
        let playlists = listed("playlist");
        let mut albums = listed("album");
        albums.extend(self.ranked(|t| t.album_uri.iter().cloned().collect(), MAX_ALBUMS.saturating_sub(albums.len())).into_iter().map(|(uri, t)| Card { uri, name: t.album.clone(), subtitle: self.redact(&t.artists.join(", ")), image_url: t.cover_url.clone(), kind: "album".into() }));
        let mut artists = listed("artist");
        artists.extend(self.artists().into_iter().map(|(uri, name)| Card { image_url: self.artist_images.get(&uri).cloned().flatten(), uri, name, subtitle: "Artist".into(), kind: "artist".into() }));
        // (Liked Songs opens as a list, like Home's own shortcut to it)
        let liked = self.shown.iter().filter(|i| i.kind == "liked").map(|i| card(i, "playlist")).collect();
        let shelves = [(PLAYLISTS, playlists), (ALBUMS, albums), (ARTISTS, artists), (LIKED, liked)]
            .into_iter()
            .filter(|(_, cards)| !cards.is_empty())
            .map(|(title, cards)| Shelf { id: String::new(), title: title.into(), kind: "shelf".into(), cards })
            .collect();
        HomeFeed { fetched_unix_ms: self.home_fetched, shelves }
    }

    /// The player as this Client may show it (Q156): a song shows once it's from the Demo set (its context is, or it's
    /// one of the Demo playlists' songs) or this Client played or queued it (or its list) in this session; any other
    /// is as if nothing played, even while it plays on. Play, pause, skip and volume still work. Shown, the queue keeps
    /// the context's songs when the context shows, and the songs queued that show on their own; a context's name
    /// outside the Demo set is "a playlist".
    pub fn playback(&self, mut p: PlaybackState) -> PlaybackState {
        let ctx = p.context_uri.as_deref().unwrap_or("");
        let ctx_shown = !ctx.is_empty() && (self.set.contains(ctx) || self.played.contains(ctx) || (self.file.liked && (ctx.ends_with(":collection") || ctx == crate::tracks::LIKED)));
        let song_shown = |uri: &str| self.played.contains(uri) || self.demo_songs().any(|t| t.uri == uri);
        if !p.item.as_ref().is_some_and(|i| ctx_shown || song_shown(&i.uri)) {
            return PlaybackState { status: p.status, timestamp_ms: p.timestamp_ms, volume: p.volume, shuffle: p.shuffle, repeat: p.repeat, ..Default::default() };
        }
        p.queue.retain(|e| song_shown(&e.item.uri) || (ctx_shown && e.source != "queue"));
        if p.context_name.is_some() && !self.context_shown(ctx) {
            p.context_name = Some(SOME_PLAYLIST.into());
        }
        p
    }

    /// A list without when its songs were added, its owner as "you" or "a friend", and an episode's progress only
    /// for shows in the Demo set.
    pub fn tracks(&self, mut p: TrackPage) -> TrackPage {
        let show = p.uri.starts_with("spotify:show:");
        for t in &mut p.tracks {
            if show {
                // (an episode's date is when it came out: public)
                if !self.set.contains(&p.uri) {
                    t.progress = None;
                }
            } else {
                t.added_at = None;
            }
        }
        if let Some(h) = p.header.as_mut() {
            if p.uri.starts_with("spotify:playlist:") {
                let mut parts: Vec<String> = h.subtitle.split(" · ").map(String::from).collect();
                // (no owner: the second part is the likes)
                if parts.len() >= 2 && parts[0] == "Playlist" && !parts[1].ends_with(" likes") {
                    parts[1] = if self.own.contains(&p.uri) { YOU.into() } else if parts[1] == "Spotify" { parts[1].clone() } else { FRIEND.into() };
                }
                h.subtitle = parts.join(" · ");
            }
            h.subtitle = self.redact(&h.subtitle);
            h.description = self.redact(&h.description);
        }
        p
    }

    /// Results without the owner's playlists outside the Demo set, and without episode progress.
    pub fn search(&self, mut r: SearchResults) -> SearchResults {
        let keep = |c: &singularity_proto::Card| !self.own_outside.contains(&c.uri);
        for list in [&mut r.top, &mut r.playlists] {
            list.retain(keep);
        }
        for list in [&mut r.top, &mut r.artists, &mut r.albums, &mut r.playlists, &mut r.podcasts, &mut r.audiobooks] {
            for c in list.iter_mut() {
                c.subtitle = self.redact(&c.subtitle);
            }
        }
        for e in &mut r.episodes {
            e.progress = None;
        }
        r.error = r.error.map(|e| self.redact(&e));
        r
    }

    /// An artist page without the owner's playlists outside the Demo set ("Discovered on" can hold them).
    pub fn artist(&self, mut p: singularity_proto::ArtistPage) -> singularity_proto::ArtistPage {
        for s in &mut p.shelves {
            s.cards.retain(|c| !self.own_outside.contains(&c.uri));
            for c in &mut s.cards {
                c.subtitle = self.redact(&c.subtitle);
            }
        }
        p.shelves.retain(|s| !s.cards.is_empty());
        p
    }

    /// A Connect device by a neutral name: the Engine "Singularity", the others by kind and number, kept for the
    /// session (the real name is learned as a secret).
    fn device(&mut self, mut d: singularity_proto::Device) -> singularity_proto::Device {
        let name = if d.is_engine {
            ENGINE.to_string()
        } else if let Some(n) = self.devices.get(&d.id) {
            n.clone()
        } else {
            let base = neutral_name(&d.kind, 1);
            let n = self.devices.values().filter(|v| **v == base || v.rsplit_once(' ').is_some_and(|(b, num)| b == base && num.parse::<usize>().is_ok())).count();
            let name = neutral_name(&d.kind, n + 1);
            self.devices.insert(d.id.clone(), name.clone());
            name
        };
        self.redactor.learn_device(&d.name, &name);
        d.name = name;
        d
    }

    /// The Jam with its host's neutral name, an invite that leads nowhere (as long as the real one, so its QR is as
    /// big) and its guests as "Guest 1", "Guest 2"…
    fn jam(&mut self, mut j: singularity_proto::Jam) -> singularity_proto::Jam {
        // (a host not among the account's devices is someone else's: its name stays unknown)
        j.host = j.host.map(|h| if j.host_is_engine { ENGINE.to_string() } else { Some(self.redact(&h)).filter(|r| *r != h).unwrap_or_else(|| FRIENDS_DEVICE.into()) });
        j.invite = j.invite.map(|real| {
            let mut fake = FAKE_INVITE.to_string();
            if real.len() > fake.len() + 1 {
                fake.push('#');
                fake.extend(std::iter::repeat_n('0', real.len() - fake.len()));
            }
            fake
        });
        for m in &mut j.members {
            let name = if m.owner {
                YOU.to_string()
            } else {
                let n = self.guests.len() + 1;
                self.guests.entry(m.id.clone()).or_insert_with(|| format!("Guest {n}")).clone()
            };
            self.redactor.learn(&m.name, &name);
            m.name = name;
        }
        j
    }

    /// A context's name shows when it's in the Demo set, this Client played it, Liked Songs, or an album or artist
    /// (public); else it's "a playlist".
    fn context_shown(&self, uri: &str) -> bool {
        self.set.contains(uri) || self.played.contains(uri) || uri.starts_with("spotify:album:") || uri.starts_with("spotify:artist:") || uri.ends_with(":collection") || uri == crate::tracks::LIKED
    }
}

#[cfg(test)]
mod tests;
