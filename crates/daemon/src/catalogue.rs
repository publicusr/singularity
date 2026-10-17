//! PrivateCatalogue (ADR-0002): Spotify's private APIs through a librespot session.
//!
//! Login is the OAuth device flow with Spotify's desktop client id (as librespot/spotatui do): the user
//! approves a code at spotify.com/pair; this program never sees a password. Reusable credentials are
//! cached under `$XDG_DATA_HOME/singularity/catalogue` (owner-only).
//! Rules: metadata only, no crawling, back off on errors, never retry in a loop.

use crate::queries::QueryRegistry;
use librespot_core::{authentication::Credentials, cache::Cache, config::SessionConfig, error::ErrorKind, session::Session};
use librespot_oauth::DeviceAuthClientBuilder;
use serde_json::{Value, json};
use crate::connect::ConnectRemote;
use crate::library;
use singularity_proto::{Card, CatalogueStatus, Command, HomeFeed, Library, Shelf, TrackPage};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::path::PathBuf;
use tokio::sync::mpsc;

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
    PlayFrom { context_uri: String, index: u32, track_uri: Option<String> },
}

#[derive(Debug, Clone)]
pub enum CatEvent {
    Status(CatalogueStatus),
    Home(HomeFeed),
    Library(Library),
    Tracks(TrackPage),
    /// the Catalogue couldn't do it; the Engine should (e.g. play the track alone)
    ToEngine(Command),
    Error(String),
}

/// Track lists stay fresh this long: reopening a playlist doesn't refetch it.
const TRACKS_TTL: Duration = Duration::from_secs(10 * 60);
/// Longest track list fetched (Liked Songs can be huge; pages are sequential and never retried in a loop).
const MAX_TRACKS: u32 = 10_000;

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

pub fn spawn(dirs: Dirs) -> (mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let (ev_tx, ev_rx) = mpsc::channel(64);
    tokio::spawn(run(dirs, cmd_rx, ev_tx, cmd_tx.clone()));
    (cmd_tx, ev_rx)
}

struct State {
    dirs: Dirs,
    session: Option<Session>,
    registry: QueryRegistry,
    ev: mpsc::Sender<CatEvent>,
    remote: Option<ConnectRemote>,
    tracks: HashMap<String, (Instant, Vec<TrackPage>)>,
}

async fn run(dirs: Dirs, mut cmds: mpsc::Receiver<CatCmd>, ev: mpsc::Sender<CatEvent>, self_tx: mpsc::Sender<CatCmd>) {
    let registry = QueryRegistry::new(dirs.queries.clone());
    let mut st = State { dirs, session: None, registry, ev, remote: None, tracks: HashMap::new() };
    // resume a previous login
    if saved_credentials(&st.dirs).is_some() {
        if st.connect(None).await {
            let _ = self_tx.send(CatCmd::Home).await;
            let _ = self_tx.send(CatCmd::Library).await;
        }
    } else {
        st.status(CatalogueStatus::LoggedOut).await;
    }
    while let Some(cmd) = cmds.recv().await {
        match cmd {
            CatCmd::Login => {
                if st.login().await {
                    let _ = self_tx.send(CatCmd::Home).await;
                    let _ = self_tx.send(CatCmd::Library).await;
                }
            }
            CatCmd::Logout => st.logout().await,
            CatCmd::Home => st.home().await,
            CatCmd::Library => st.library().await,
            CatCmd::Tracks(uri) => st.track_list(uri).await,
            CatCmd::PlayFrom { context_uri, index, track_uri } => st.play_from(context_uri, index, track_uri).await,
        }
    }
}

/// A pretend Catalogue serving a recorded Home feed (`--fake-catalogue <home.json>`): UI work without a Spotify login.
pub fn spawn_fake(home: HomeFeed) -> (mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>) {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
    let (ev_tx, ev_rx) = mpsc::channel(64);
    tokio::spawn(async move {
        let ready = CatalogueStatus::Ready { username: "fake".into(), country: "SE".into(), premium: true };
        let _ = ev_tx.send(CatEvent::Status(ready.clone())).await;
        let _ = ev_tx.send(CatEvent::Home(home.clone())).await;
        let _ = ev_tx.send(CatEvent::Library(fake_library(&home))).await;
        while let Some(cmd) = cmd_rx.recv().await {
            let evs = match cmd {
                CatCmd::Home => vec![CatEvent::Home(home.clone())],
                CatCmd::Login => vec![CatEvent::Status(ready.clone())],
                CatCmd::Logout => vec![CatEvent::Status(CatalogueStatus::LoggedOut)],
                CatCmd::Library => vec![CatEvent::Library(fake_library(&home))],
                CatCmd::Tracks(uri) => fake_tracks(&uri),
                CatCmd::PlayFrom { context_uri, index, track_uri } => vec![CatEvent::ToEngine(Command::PlayFrom { context_uri, index, track_uri })],
            };
            for ev in evs {
                let _ = ev_tx.send(ev).await;
            }
        }
    });
    (cmd_tx, ev_rx)
}

/// The fake library: Liked Songs plus the Home feed's shortcuts and first Shelf.
fn fake_library(home: &HomeFeed) -> Library {
    let mut items = vec![singularity_proto::LibraryItem {
        uri: library::LIKED.into(),
        name: "Liked Songs".into(),
        subtitle: "250 songs".into(),
        kind: "liked".into(),
        image_url: None,
        depth: 0,
    }];
    for c in home.shelves.iter().take(2).flat_map(|s| &s.cards).filter(|c| c.uri != "spotify:playlist:liked") {
        items.push(singularity_proto::LibraryItem { uri: c.uri.clone(), name: c.name.clone(), subtitle: c.subtitle.clone(), kind: c.kind.clone(), image_url: c.image_url.clone(), depth: 0 });
    }
    Library { items }
}

/// Invented tracks in pages of 100: 250 for Liked Songs, 40 elsewhere.
fn fake_tracks(uri: &str) -> Vec<CatEvent> {
    let total = if uri == library::LIKED { 250 } else { 40 };
    let artists = ["Nova Drift", "Mira Vale", "Lumen Fields", "Glass Harbour", "Polar Static"];
    (0..total)
        .step_by(100)
        .map(|offset| {
            let tracks = (offset..(offset + 100).min(total))
                .map(|i| singularity_proto::TrackRow {
                    uri: format!("spotify:track:fake{i}"),
                    name: format!("Fake Track {:03}", i + 1),
                    artists: vec![artists[i as usize % artists.len()].into()],
                    album: format!("Album {}", (i * 7) % 13 + 1),
                    added_at: Some(format!("2025-{:02}-{:02}T12:00:00Z", 1 + (i % 12), 1 + (i * 3) % 28)),
                    duration_ms: 120_000 + (i as u64 * 7919) % 240_000,
                    playable: i % 17 != 16,
                })
                .collect();
            CatEvent::Tracks(TrackPage { uri: uri.into(), offset, total, tracks })
        })
        .collect()
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
                let _ = self.ev.send(CatEvent::Error(format!("Spotify login failed: {e}"))).await;
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
            self.remote = None;
            self.connect(None).await;
        }
        self.session.clone()
    }

    /// Run a persisted query; on a retired hash (412), refresh the registry and retry once.
    async fn query(&mut self, op: &str, vars: Value) -> anyhow::Result<Value> {
        let session = self.session().await.ok_or_else(|| anyhow::anyhow!("not logged in to Spotify"))?;
        let hash = self.registry.hash(op).await?;
        match session.spclient().pathfinder_query(op, &hash, vars.clone()).await {
            Err(e) if e.kind == ErrorKind::FailedPrecondition => {
                tracing::warn!("`{op}` hash retired; refreshing the query registry");
                self.registry.refresh().await?;
                let hash = self.registry.hash(op).await?;
                Ok(session.spclient().pathfinder_query(op, &hash, vars).await?)
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
                let _ = self.ev.send(CatEvent::Error(format!("Library unavailable: {e}"))).await;
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
            let page = library::parse_library(&self.query("libraryV3", library::library_vars(offset, folder)).await?, depth);
            let n = page.items.len();
            items.extend(page.items);
            offset += library::LIBRARY_PAGE;
            if n == 0 || offset >= page.total {
                return Ok(items);
            }
        }
    }

    /// Fetch a track list page by page, sending each as it arrives (so long lists show at once).
    async fn track_list(&mut self, uri: String) {
        if let Some((at, pages)) = self.tracks.get(&uri)
            && at.elapsed() < TRACKS_TTL
        {
            for p in pages.clone() {
                let _ = self.ev.send(CatEvent::Tracks(p)).await;
            }
            return;
        }
        let mut pages = vec![];
        let mut offset = 0u32;
        loop {
            let Some((op, vars)) = library::tracks_query(&uri, offset) else {
                let _ = self.ev.send(CatEvent::Error(format!("can't list the tracks of {uri}"))).await;
                return;
            };
            let v = match self.query(op, vars).await {
                Ok(v) => v,
                Err(e) => {
                    let _ = self.ev.send(CatEvent::Error(format!("Track list unavailable: {e}"))).await;
                    return;
                }
            };
            let (tracks, total) = library::parse_tracks(&uri, &v);
            let page_len = match op {
                "getAlbum" => library::ALBUM_PAGE,
                _ => library::TRACK_PAGE,
            };
            let page = TrackPage { uri: uri.clone(), offset, total, tracks };
            let _ = self.ev.send(CatEvent::Tracks(page.clone())).await;
            pages.push(page);
            offset += page_len;
            if offset >= total.min(MAX_TRACKS) {
                break;
            }
        }
        self.tracks.insert(uri, (Instant::now(), pages));
    }

    /// Start a context at track N through ConnectRemote; if that fails, the Engine plays the track alone.
    async fn play_from(&mut self, context_uri: String, index: u32, track_uri: Option<String>) {
        let result = async {
            let session = self.session().await.ok_or_else(|| anyhow::anyhow!("not logged in to Spotify"))?;
            if self.remote.is_none() {
                self.remote = Some(ConnectRemote::start(session.clone()).await?);
            }
            // a failure below drops the remote, so a stale device id is looked up again next time
            let remote = self.remote.as_mut().expect("just started");
            let to = remote.device_id(crate::soloist::DEVICE_NAME).await?;
            let context = if context_uri == library::LIKED { format!("spotify:user:{}:collection", session.username()) } else { context_uri.clone() };
            remote.play_from(&to, &context, index, track_uri.as_deref()).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("play from track {index} of {context_uri} via Connect failed: {e}");
            self.remote = None;
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
                    let _ = self.ev.send(CatEvent::Error("Spotify answered, but the Home feed had an unexpected shape".into())).await;
                }
                let _ = self.ev.send(CatEvent::Home(feed)).await;
            }
            Err(e) => {
                let _ = self.ev.send(CatEvent::Error(format!("Home feed unavailable: {e}"))).await;
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

fn plain(html: &str) -> String {
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
    out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#x27;", "'").replace("&#39;", "'").trim().to_string()
}

fn card(d: &Value) -> Option<Card> {
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
