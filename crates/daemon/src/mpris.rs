//! MPRIS (#21): media keys, KDE's media widget and `playerctl` drive the Daemon over D-Bus, and optional desktop
//! notifications say what's playing on a track change.
//!
//! The player follows `Hub::playback` (so it shows another device's playback while that one plays) and sends the same
//! transport commands a Client would (`crate::transport`).

use crate::Hub;
use singularity_proto::{Command, Event, PlayStatus, PlaybackState, RepeatMode};
use std::collections::HashMap;
use std::sync::Arc;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

pub const BUS_NAME: &str = "org.mpris.MediaPlayer2.singularity";
const PATH: &str = "/org/mpris/MediaPlayer2";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// Where the Daemon meets the desktop.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// a D-Bus address (tests use a private bus); `None` is the session bus
    pub address: Option<String>,
    /// a desktop notification on each new track (`notifications.track_change`, changed live)
    pub notify: Arc<std::sync::atomic::AtomicBool>,
}

/// Serve MPRIS until the Daemon stops. Without a bus (a headless box, a container) the Daemon carries on without it.
pub async fn run(hub: Arc<Hub>, opts: Options) {
    if let Err(e) = serve(hub, opts).await {
        tracing::warn!("MPRIS is off: {e}");
    }
}

async fn serve(hub: Arc<Hub>, opts: Options) -> zbus::Result<()> {
    // subscribe first, so nothing between the snapshot and the loop is missed
    let mut events = hub.events.subscribe();
    let state = hub.playback.lock().await.clone();
    let builder = match &opts.address {
        Some(a) => zbus::connection::Builder::address(a.as_str())?,
        None => zbus::connection::Builder::session()?,
    };
    let conn = builder.name(BUS_NAME)?.serve_at(PATH, Root)?.serve_at(PATH, Player { hub: hub.clone(), state })?.build().await?;
    tracing::info!("MPRIS: {BUS_NAME}");
    let player = conn.object_server().interface::<_, Player>(PATH).await?;
    let mut notifier = Notifier { conn: conn.clone(), news: Default::default(), id: 0, on: opts.notify.clone() };
    loop {
        match events.recv().await {
            Ok(Event::Playback(new)) => {
                let old = std::mem::replace(&mut player.get_mut().await.state, new.clone());
                announce(&player, &old, &new).await;
                notifier.track(&new).await;
            }
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

/// Tell D-Bus what changed (PropertiesChanged, and Seeked for jumps).
async fn announce(player: &zbus::object_server::InterfaceRef<Player>, old: &PlaybackState, new: &PlaybackState) {
    let em = player.signal_emitter();
    let p = player.get().await;
    let same_track = old.item.as_ref().map(|i| &i.uri) == new.item.as_ref().map(|i| &i.uri);
    let r = async {
        if status(old) != status(new) {
            p.playback_status_changed(em).await?;
        }
        if !same_track || old.item != new.item {
            p.metadata_changed(em).await?;
            p.can_go_next_changed(em).await?;
            p.can_go_previous_changed(em).await?;
            p.can_play_changed(em).await?;
            p.can_pause_changed(em).await?;
            p.can_seek_changed(em).await?;
        }
        if old.volume != new.volume {
            p.volume_changed(em).await?;
        }
        if old.shuffle != new.shuffle {
            p.shuffle_changed(em).await?;
        }
        if old.repeat != new.repeat {
            p.loop_status_changed(em).await?;
        }
        if same_track && let Some(us) = seeked(old, new, singularity_proto::now_ms()) {
            Player::seeked(em, us).await?;
        }
        zbus::Result::Ok(())
    };
    if let Err(e) = r.await {
        tracing::debug!("MPRIS signal: {e}");
    }
}

/// Where the track jumped to (µs), if the new state isn't where the old one would have been by now.
pub fn seeked(old: &PlaybackState, new: &PlaybackState, now_ms: u64) -> Option<i64> {
    let (was, is) = (old.position_at(now_ms), new.position_at(now_ms));
    (was.abs_diff(is) > 1500).then_some(is as i64 * 1000)
}

pub fn status(s: &PlaybackState) -> &'static str {
    match (s.item.is_some(), s.status) {
        (false, _) | (_, PlayStatus::Idle) => "Stopped",
        (true, PlayStatus::Playing | PlayStatus::Buffering) => "Playing",
        (true, PlayStatus::Paused) => "Paused",
    }
}

pub fn loop_status(r: RepeatMode) -> &'static str {
    match r {
        RepeatMode::Off => "None",
        RepeatMode::Context => "Playlist",
        RepeatMode::Track => "Track",
    }
}

/// `spotify:track:abc` → an object path MPRIS accepts (letters, digits, `_` only).
pub fn track_id(uri: &str) -> OwnedObjectPath {
    let id: String = uri.rsplit(':').next().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    let path = if id.is_empty() { NO_TRACK.to_string() } else { format!("/org/singularity/track/{id}") };
    ObjectPath::try_from(path).map(Into::into).unwrap_or_else(|_| ObjectPath::from_static_str_unchecked(NO_TRACK).into())
}

fn owned<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
    v.into().try_into_owned().expect("plain values have no file descriptors")
}

/// The `Metadata` map: id, length, title, artists, album, cover, web link.
pub fn metadata(s: &PlaybackState) -> HashMap<String, OwnedValue> {
    let mut m = HashMap::new();
    let Some(i) = &s.item else {
        m.insert("mpris:trackid".into(), owned(track_id("")));
        return m;
    };
    m.insert("mpris:trackid".into(), owned(track_id(&i.uri)));
    m.insert("mpris:length".into(), owned(i.duration_ms as i64 * 1000));
    m.insert("xesam:title".into(), owned(i.name.clone()));
    m.insert("xesam:artist".into(), owned(i.artists.clone()));
    if let Some(a) = &i.album {
        m.insert("xesam:album".into(), owned(a.clone()));
    }
    if let Some(c) = &i.cover_url {
        m.insert("mpris:artUrl".into(), owned(c.clone()));
    }
    if let Some(l) = web_link(&i.uri) {
        m.insert("xesam:url".into(), owned(l));
    }
    m
}

/// `spotify:track:abc` → `https://open.spotify.com/track/abc`
fn web_link(uri: &str) -> Option<String> {
    let mut p = uri.split(':');
    match (p.next(), p.next(), p.next()) {
        (Some("spotify"), Some(kind), Some(id)) => Some(format!("https://open.spotify.com/{kind}/{id}")),
        _ => None,
    }
}

/// `https://open.spotify.com/track/abc?si=…` or `spotify:track:abc` → `spotify:track:abc`
pub fn spotify_uri(s: &str) -> Option<String> {
    if s.starts_with("spotify:") {
        return Some(s.to_string());
    }
    let rest = s.strip_prefix("https://open.spotify.com/")?;
    let rest = rest.split(['?', '#']).next()?;
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty() && !p.starts_with("intl-")).collect();
    match parts.as_slice() {
        [kind, id] => Some(format!("spotify:{kind}:{id}")),
        _ => None,
    }
}

/// `org.mpris.MediaPlayer2`: who we are.
struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}
    fn quit(&self) {}
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &str {
        "Singularity"
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "singularity"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["spotify".into(), "https".into()]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![]
    }
}

/// `org.mpris.MediaPlayer2.Player`: what plays, and the transport.
struct Player {
    hub: Arc<Hub>,
    state: PlaybackState,
}

impl Player {
    async fn send(&self, cmd: Command) {
        crate::transport(&self.hub, cmd).await;
    }

    fn duration_ms(&self) -> u64 {
        self.state.item.as_ref().map(|i| i.duration_ms).unwrap_or(0)
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    async fn next(&self) {
        self.send(Command::Next).await;
    }
    async fn previous(&self) {
        self.send(Command::Previous).await;
    }
    async fn pause(&self) {
        self.send(Command::Pause).await;
    }
    async fn play_pause(&self) {
        self.send(Command::TogglePlay).await;
    }
    /// Spotify has no "stop": it pauses.
    async fn stop(&self) {
        self.send(Command::Pause).await;
    }
    async fn play(&self) {
        self.send(Command::Play { uri: None }).await;
    }
    /// Relative, in µs; past the end means the next track (MPRIS spec).
    async fn seek(&self, offset: i64) {
        let dur = self.duration_ms();
        if dur == 0 {
            return;
        }
        let to = self.state.position_at(singularity_proto::now_ms()) as i64 + offset / 1000;
        if to >= dur as i64 {
            return self.send(Command::Next).await;
        }
        self.send(Command::Seek { position_ms: to.max(0) as u64 }).await;
    }
    /// Absolute, in µs; ignored if `track_id` isn't the current track any more (MPRIS spec).
    async fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        let current = self.state.item.as_ref().map(|i| self::track_id(&i.uri));
        if current.as_ref().map(|p| p.as_str()) != Some(track_id.as_str()) || position < 0 || position / 1000 > self.duration_ms() as i64 {
            return;
        }
        self.send(Command::Seek { position_ms: (position / 1000) as u64 }).await;
    }
    async fn open_uri(&self, uri: String) -> zbus::fdo::Result<()> {
        let uri = spotify_uri(&uri).ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("not a Spotify link: {uri}")))?;
        self.send(Command::Play { uri: Some(uri) }).await;
        Ok(())
    }

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> &str {
        status(&self.state)
    }
    #[zbus(property)]
    fn loop_status(&self) -> &str {
        loop_status(self.state.repeat)
    }
    #[zbus(property)]
    async fn set_loop_status(&mut self, v: String) {
        let mode = match v.as_str() {
            "Track" => RepeatMode::Track,
            "Playlist" => RepeatMode::Context,
            _ => RepeatMode::Off,
        };
        self.send(Command::SetRepeat { mode }).await;
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn set_rate(&mut self, _v: f64) {}
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.state.shuffle
    }
    #[zbus(property)]
    async fn set_shuffle(&mut self, enabled: bool) {
        self.send(Command::SetShuffle { enabled }).await;
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        metadata(&self.state)
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.state.volume as f64 / 100.0
    }
    #[zbus(property)]
    async fn set_volume(&mut self, v: f64) {
        let percent = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
        self.send(Command::SetVolume { percent }).await;
    }
    /// Asked for, never announced (MPRIS spec): computed from the anchor at read time.
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        self.state.position_at(singularity_proto::now_ms()) as i64 * 1000
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.state.item.is_some()
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.state.item.is_some()
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.state.item.is_some()
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.state.item.is_some()
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.duration_ms() > 0
    }
    #[zbus(property(emits_changed_signal = "const"))]
    fn can_control(&self) -> bool {
        true
    }
}

/// Desktop notifications on track change (`org.freedesktop.Notifications`), each replacing the last.
struct Notifier {
    conn: zbus::Connection,
    news: crate::nowplaying::TrackNews,
    /// the notification to replace (0: none yet)
    id: u32,
    on: Arc<std::sync::atomic::AtomicBool>,
}

#[zbus::proxy(interface = "org.freedesktop.Notifications", default_service = "org.freedesktop.Notifications", default_path = "/org/freedesktop/Notifications")]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(&self, app_name: &str, replaces_id: u32, app_icon: &str, summary: &str, body: &str, actions: &[&str], hints: HashMap<&str, Value<'_>>, expire_timeout: i32) -> zbus::Result<u32>;
}

impl Notifier {
    /// Notify once per new track, when it starts playing (which changes are news: shared with macOS).
    async fn track(&mut self, s: &PlaybackState) {
        let Some(n) = self.news.news(s, self.on.load(std::sync::atomic::Ordering::Relaxed)) else { return };
        let mut hints = HashMap::new();
        hints.insert("desktop-entry", Value::from("singularity"));
        hints.insert("transient", Value::from(true));
        if let Some(c) = &n.cover_url
            && let Some(path) = cached_cover(c).await
        {
            hints.insert("image-path", Value::from(format!("file://{}", path.display())));
        }
        let r = async {
            let proxy = NotificationsProxy::new(&self.conn).await?;
            proxy.notify("Singularity", self.id, "media-playback-start", &n.title, &n.body, &[], hints, -1).await
        };
        match r.await {
            Ok(id) => self.id = id,
            Err(e) => tracing::debug!("notification: {e}"),
        }
    }
}

/// The cover as a local file (notification servers don't fetch URLs): `~/.cache/singularity/notify-cover.jpg`.
async fn cached_cover(url: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let dir = std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("singularity");
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)).build().ok()?;
    let resp = client.get(url).send().await.ok()?.error_for_status().ok()?;
    // (#71) a cover is tens of KB: nothing past 8 MB is read
    if resp.content_length().is_some_and(|n| n > 8 << 20) {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > 8 << 20 {
        return None;
    }
    tokio::fs::create_dir_all(&dir).await.ok()?;
    // written aside, then moved, so the notification server never reads half a file
    let (tmp, path) = (dir.join("notify-cover.jpg.part"), dir.join("notify-cover.jpg"));
    tokio::fs::write(&tmp, &bytes).await.ok()?;
    tokio::fs::rename(&tmp, &path).await.ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::Item;

    fn playing(uri: &str, pos: u64, at: u64) -> PlaybackState {
        PlaybackState { status: PlayStatus::Playing, item: Some(Item { uri: uri.into(), name: "Song".into(), duration_ms: 200_000, ..Default::default() }), position_ms: pos, timestamp_ms: at, speed: 1.0, ..Default::default() }
    }

    #[test]
    fn track_ids_are_valid_object_paths() {
        assert_eq!(track_id("spotify:track:4uLU6hMCjMI75M1A2tKUQC").as_str(), "/org/singularity/track/4uLU6hMCjMI75M1A2tKUQC");
        assert_eq!(track_id("").as_str(), NO_TRACK);
        assert_eq!(track_id("spotify:episode:a-b.c").as_str(), "/org/singularity/track/abc");
    }

    #[test]
    fn status_and_loop_follow_the_spec_words() {
        assert_eq!(status(&PlaybackState::default()), "Stopped");
        assert_eq!(status(&playing("spotify:track:a", 0, 0)), "Playing");
        assert_eq!(loop_status(RepeatMode::Context), "Playlist");
    }

    #[test]
    fn seeks_are_told_apart_from_time_passing() {
        let old = playing("spotify:track:a", 10_000, 1_000);
        // ten seconds later, the new anchor says what time passing would: no Seeked
        assert_eq!(seeked(&old, &playing("spotify:track:a", 20_000, 11_000), 11_000), None);
        // a jump to 1:00
        assert_eq!(seeked(&old, &playing("spotify:track:a", 60_000, 11_000), 11_000), Some(60_000_000));
    }

    #[test]
    fn metadata_carries_the_cover_for_the_media_widget() {
        let mut s = playing("spotify:track:a", 0, 0);
        s.item.as_mut().unwrap().cover_url = Some("https://i.scdn.co/image/x".into());
        s.item.as_mut().unwrap().artists = vec!["A".into(), "B".into()];
        let m = metadata(&s);
        assert_eq!(String::try_from(m["mpris:artUrl"].clone()).unwrap(), "https://i.scdn.co/image/x");
        assert_eq!(<Vec<String>>::try_from(m["xesam:artist"].clone()).unwrap(), vec!["A", "B"]);
        assert_eq!(i64::try_from(m["mpris:length"].clone()).unwrap(), 200_000_000);
        assert_eq!(metadata(&PlaybackState::default()).len(), 1, "only the NoTrack id when nothing plays");
    }

    #[test]
    fn links_become_uris() {
        assert_eq!(spotify_uri("https://open.spotify.com/intl-de/track/abc?si=x").as_deref(), Some("spotify:track:abc"));
        assert_eq!(spotify_uri("spotify:album:x").as_deref(), Some("spotify:album:x"));
        assert_eq!(spotify_uri("https://example.com/a/b"), None);
    }
}
