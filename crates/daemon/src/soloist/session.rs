//! One run of Soloist as a state machine: what the Daemon makes of what Soloist says (its WebSocket events and its
//! log), of our commands, of a staged build and of the clock. No I/O: [`Session::step`] takes one input and the time,
//! and returns what to do (send to Soloist, tell the Daemon, stop Soloist). `run_once` only pumps the process and the
//! socket into it, so the order in which login, a track change, the queue answer, a quiet moment and a dry spell
//! interact is all here, and testable with a made-up clock.

use super::LogNews;
use crate::engine::{EngineEvent, Resume};
use serde_json::{Value, json};
use singularity_proto::{Command, Delivery, EngineStatus, Item, PlayStatus, PlaybackState, QueueEntry, RepeatMode};
use std::time::Duration;
use tokio::time::Instant;

/// The time a step happens at: a monotonic instant for the windows and deadlines, and Unix ms for positions.
#[derive(Debug, Clone, Copy)]
pub struct Now {
    pub at: Instant,
    pub ms: u64,
}

impl Now {
    pub fn real() -> Now {
        Now { at: Instant::now(), ms: singularity_proto::now_ms() }
    }
}

impl std::ops::Add<Duration> for Now {
    type Output = Now;
    fn add(self, d: Duration) -> Now {
        Now { at: self.at + d, ms: self.ms + d.as_millis() as u64 }
    }
}

/// What happened, as far as the session is concerned.
#[derive(Debug)]
pub enum Input {
    /// an event from Soloist's WebSocket
    Ws(Value),
    /// a line of Soloist's log that means something
    Log(LogNews),
    /// a command for the Engine
    Command(Command),
    /// a new build is staged and still waiting
    Staged,
    /// the time [`Session::deadline`] named has come
    Tick,
}

/// What to do about it, in this order.
// (an EngineEvent is big, like in `EngineEvent` itself: an action is carried out once)
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Action {
    /// a message to Soloist's WebSocket
    Send(Value),
    /// news for the Daemon
    Event(EngineEvent),
    /// the volume a restarted Soloist should start at
    SaveVolume(u8),
    /// a new API key from the setup wizard, to store
    StoreKey(String),
    /// stop Soloist: this run is over
    Stop(Stop),
}

/// Why the run ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// straight back up (asked for, a new key, a stuck stream)
    Restart,
    /// into the new build
    Update,
    /// Spotify rejected the API key: no use starting again until there's a new one
    BadKey,
    /// another Soloist already runs on this data directory
    Locked,
}

/// What a run leaves for the next one.
#[derive(Debug, Default)]
pub struct Handover {
    /// where playback was when we restarted it (for an update, or a stuck stream)
    pub resume: Option<Resume>,
    /// when Soloist was last restarted because its stream was stuck (one try per `DRY_RETRY`)
    pub unstuck_at: Option<Instant>,
}

/// How long playback may sit dry, with no data arriving, before Soloist's connections count as stuck.
const DRY_STUCK: Duration = Duration::from_secs(20);
/// At most one restart per this long for a stuck stream: a connection that is really down isn't fixed by restarting.
const DRY_RETRY: Duration = Duration::from_secs(180);
/// How long after one of our commands the track change it asked for can arrive (a Play names its track, a
/// Next/Previous/TogglePlay doesn't, so its window is shorter).
const OUR_CHANGE: Duration = Duration::from_secs(30);
const OUR_CHANGE_UNTIL: Duration = Duration::from_secs(10);
/// How long an answer to our `get_queue` may take (after that a playback state is news, not the answer).
const QUEUE_ANSWER: Duration = Duration::from_secs(5);
/// After a restart, how long the new Soloist has to say what it is playing before the remembered state is put back.
const RESUME_GRACE: Duration = Duration::from_millis(1500);
/// How long our own play/pause outranks Soloist's last report for the next toggle.
const TOGGLE_FRESH: Duration = Duration::from_secs(2);

/// One run of Soloist, from its start to its stop.
pub struct Session {
    st: SoloistState,
    device_name: String,
    version: String,
    /// the volume a restarted Soloist starts at (re-sent on login: one asked for while it was starting is lost)
    volume: Option<u8>,
    logged_in: bool,
    active: bool,
    expires_in_days: Option<u32>,
    /// the stream's download speed and recent stalls (#45)
    delivery: Delivery,
    /// a build staged while this one runs waits for a quiet moment
    staged: bool,
    /// where playback was before the restart, and when to put it back
    pending_resume: Option<Resume>,
    resume_at: Option<Instant>,
    dry: DryWatch,
    handover: Handover,
}

impl Session {
    /// A run of Soloist named `device_name` on Connect, `version`, started at `volume`, after a run that left
    /// `handover`.
    pub fn new(device_name: String, version: String, volume: Option<u8>, handover: Handover) -> Session {
        Session {
            st: SoloistState::default(),
            device_name,
            version,
            volume,
            logged_in: false,
            active: false,
            expires_in_days: None,
            delivery: Delivery::default(),
            staged: false,
            pending_resume: None,
            resume_at: None,
            dry: DryWatch::default(),
            handover,
        }
    }

    /// Is Soloist logged in (paired with an account)?
    pub fn logged_in(&self) -> bool {
        self.logged_in
    }

    /// When to come back with [`Input::Tick`].
    pub fn deadline(&self) -> Option<Instant> {
        match (self.resume_at, self.dry.deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// What this run leaves for the next one.
    pub fn into_handover(self) -> Handover {
        self.handover
    }

    pub fn step(&mut self, input: Input, now: Now) -> Vec<Action> {
        let mut out = vec![];
        match input {
            Input::Ws(ev) => self.ws(&ev, now, &mut out),
            Input::Log(n) => self.log(n, now, &mut out),
            Input::Command(c) => self.command(c, now, &mut out),
            Input::Staged => {
                self.staged = true;
                if quiet_moment(&self.st.playback, false, false) {
                    self.restart_for_update(false, now, &mut out);
                }
            }
            Input::Tick => self.tick(now, &mut out),
        }
        out
    }

    fn ready(&self) -> EngineStatus {
        EngineStatus::Ready { device_name: self.device_name.clone(), active: self.active, version: self.version.clone(), expires_in_days: self.expires_in_days }
    }

    fn ws(&mut self, ev: &Value, now: Now, out: &mut Vec<Action>) {
        match ev["type"].as_str() {
            Some("auth_state") | Some("device_changed") => {
                let was_logged_in = self.logged_in;
                if let Some(l) = ev["logged_in"].as_bool() {
                    self.logged_in = l;
                }
                // a volume asked for while Soloist was starting is lost (it starts at its own default once logged
                // in): set it again
                if self.logged_in
                    && !was_logged_in
                    && let Some(v) = self.volume
                {
                    out.push(Action::Send(json!({"type":"command","command":"set_volume","volume":v})));
                }
                if let Some(a) = ev["is_active"].as_bool() {
                    self.active = a;
                }
                // back after a restart: pick up where playback was, once the new Soloist has had a moment to say
                // whether it came back playing something already (see below)
                if self.logged_in
                    && let Some(r) = self.handover.resume.take()
                {
                    self.pending_resume = Some(r);
                    self.resume_at = Some(now.at + RESUME_GRACE);
                }
                // (the lifetime may still arrive; status is re-sent then)
                let s = if self.logged_in { self.ready() } else { EngineStatus::NeedsPairing { device_name: self.device_name.clone() } };
                out.push(Action::Event(EngineEvent::Status(s)));
            }
            Some("error") => {
                out.push(Action::Event(EngineEvent::Error(ev["message"].as_str().unwrap_or("Soloist error").to_string())));
            }
            _ => {
                let track_changed = ev["type"] == "track_changed";
                if self.st.apply(ev, now) {
                    self.dry.status(self.st.playback.status);
                    out.push(Action::Event(EngineEvent::Playback(self.st.playback.clone())));
                }
                // (every change: one to 0 looks like no change from the state's default)
                if ev["type"] == "volume_changed" {
                    self.volume = Some(self.st.playback.volume);
                    out.push(Action::SaveVolume(self.st.playback.volume));
                }
                // the new Soloist came back playing something: someone (a phone on Connect) put it there, or Spotify
                // restored it, so putting our remembered state back would undo it
                if self.pending_resume.is_some() && self.st.playback.item.is_some() {
                    self.pending_resume = None;
                    self.resume_at = None;
                    tracing::info!("not picking up where playback was after the restart: the player came back with a track of its own");
                }
                let ours = track_changed && self.st.change_is_ours(now.at);
                if track_changed {
                    self.st.asking_for_queue(now.at);
                    out.push(Action::Send(json!({"type":"command","command":"get_queue","limit":20})));
                }
                if self.staged && quiet_moment(&self.st.playback, track_changed, ours) {
                    self.restart_for_update(track_changed, now, out);
                }
            }
        }
    }

    fn log(&mut self, n: LogNews, now: Now, out: &mut Vec<Action>) {
        self.dry.news(&n, now.at);
        match n {
            LogNews::Stream(info) => out.push(Action::Event(EngineEvent::Stream(info))),
            LogNews::Bandwidth(k) => {
                self.delivery.bandwidth_kbps = Some(k);
                self.delivery.measured_ms = now.ms;
                out.push(Action::Event(EngineEvent::Delivery(self.delivery.clone())));
            }
            LogNews::Unplayable => {
                out.push(Action::Event(EngineEvent::Notice("Spotify won't play that here (DJ isn't supported yet)".into())));
            }
            LogNews::Stall => {
                self.delivery.stalls_ms.retain(|t| now.ms.saturating_sub(*t) <= 120_000);
                self.delivery.stalls_ms.push(now.ms);
                out.push(Action::Event(EngineEvent::Delivery(self.delivery.clone())));
            }
            LogNews::Lifetime(d) => {
                self.expires_in_days = Some(d);
                if self.logged_in {
                    out.push(Action::Event(EngineEvent::Status(self.ready())));
                }
            }
            // whenever it comes (also once the WebSocket is up), retrying won't help (C4: it was ignored then, and
            // became a restart loop without a Failed status)
            LogNews::BadKey => out.push(Action::Stop(Stop::BadKey)),
            LogNews::Locked => out.push(Action::Stop(Stop::Locked)),
            LogNews::ZeroConf(_) | LogNews::Flowing => {}
        }
    }

    fn command(&mut self, cmd: Command, now: Now, out: &mut Vec<Action>) {
        match cmd {
            Command::RestartEngine => {
                out.push(Action::Event(EngineEvent::Notice("Restarting Soloist…".into())));
                out.push(Action::Stop(Stop::Restart));
            }
            // a new key: restart Soloist with it
            Command::SetSoloistKey { key } => {
                out.push(Action::StoreKey(key));
                out.push(Action::Stop(Stop::Restart));
            }
            cmd => {
                // the volume asked for is the one a restarted Soloist starts at, even if this one never applies it
                if let Command::SetVolume { percent } = cmd {
                    self.volume = Some(percent.min(100));
                    out.push(Action::SaveVolume(percent.min(100)));
                }
                out.extend(self.st.soloist_commands(cmd, now.at).into_iter().map(Action::Send));
            }
        }
    }

    fn tick(&mut self, now: Now, out: &mut Vec<Action>) {
        // the moment has come to pick up where playback was after the restart
        if self.resume_at.is_some_and(|t| t <= now.at) {
            self.resume_at = None;
            if let Some(r) = self.pending_resume.take() {
                out.push(Action::Event(EngineEvent::Resume(r)));
            }
        }
        // playback has been dry too long: Soloist's connections to Spotify are stuck
        if self.dry.deadline().is_some_and(|t| t <= now.at) {
            self.dry.clear();
            if self.handover.unstuck_at.is_some_and(|t| now.at.saturating_duration_since(t) < DRY_RETRY) {
                tracing::warn!("playback is still stuck without audio data after restarting Soloist");
                out.push(Action::Event(EngineEvent::Notice("Spotify still isn't sending audio: is the connection (or a VPN) down?".into())));
            } else {
                tracing::warn!("no audio data for {} s while playing: restarting Soloist to reconnect", DRY_STUCK.as_secs());
                out.push(Action::Event(EngineEvent::Notice("No audio from Spotify for 20 s: reconnecting the player…".into())));
                self.handover.unstuck_at = Some(now.at);
                self.handover.resume = resume_point(&self.st.playback, false, now.ms);
                out.push(Action::Stop(Stop::Restart));
            }
        }
    }

    /// Stop Soloist so the new build takes over, remembering where playback was.
    fn restart_for_update(&mut self, track_changed: bool, now: Now, out: &mut Vec<Action>) {
        self.handover.resume = resume_point(&self.st.playback, track_changed, now.ms);
        tracing::info!("restarting Soloist into the new build ({})", if track_changed { "between tracks" } else { "while quiet" });
        out.push(Action::Stop(Stop::Update));
    }
}

/// Is now a good moment to restart into a new build? Paused or idle at once. In the middle of a playing track, only
/// between tracks, and only when a command of ours caused the change: playback someone else started (a phone on
/// Connect) is followed, not interrupted (restarting Soloist would undo it, and the restart picks up our old state).
fn quiet_moment(p: &PlaybackState, track_changed: bool, ours: bool) -> bool {
    if matches!(p.status, PlayStatus::Playing | PlayStatus::Buffering) { track_changed && ours } else { true }
}

/// Where to pick up after the restart (a track that just started resumes from its start).
fn resume_point(p: &PlaybackState, track_changed: bool, now_ms: u64) -> Option<Resume> {
    let item = p.item.as_ref()?;
    let position_ms = if track_changed { 0 } else { p.position_at(now_ms) };
    Some(Resume { context_uri: p.context_uri.clone(), track_uri: item.uri.clone(), position_ms, paused: p.status == PlayStatus::Paused })
}

/// Is playback dry and staying dry? Verified 2026-11-02: after the VPN's tunnel was re-established, Soloist's
/// requests for audio failed for 16 minutes (`CURL error: 42`, no bandwidth samples) while it kept reporting
/// "playing" at the same second; new connections worked, so a fresh Soloist gets the music back. Data arriving
/// (a bandwidth sample) or audio flowing again ends a dry spell; so does anything but playing.
#[derive(Default)]
struct DryWatch {
    since: Option<Instant>,
}

impl DryWatch {
    fn news(&mut self, n: &LogNews, now: Instant) {
        match n {
            LogNews::Stall => {
                self.since.get_or_insert(now);
            }
            LogNews::Bandwidth(_) | LogNews::Flowing => self.since = None,
            _ => {}
        }
    }

    /// Only playing (or buffering) can be dry: a pause or a stop ends it.
    fn status(&mut self, s: PlayStatus) {
        if !matches!(s, PlayStatus::Playing | PlayStatus::Buffering) {
            self.since = None;
        }
    }

    /// When the current dry spell becomes stuck.
    fn deadline(&self) -> Option<Instant> {
        self.since.map(|t| t + DRY_STUCK)
    }

    fn clear(&mut self) {
        self.since = None;
    }
}

/// Translates Soloist's WebSocket events into our `PlaybackState`, and our commands into Soloist's.
#[derive(Default)]
struct SoloistState {
    playback: PlaybackState,
    /// the track one of our commands asked for, and when: the change that follows it is ours
    expected: Option<(String, Instant)>,
    /// when we sent a next/previous/play/pause: the change that follows it is ours (its target isn't known)
    sent_at: Option<Instant>,
    /// when we asked for the queue, oldest first, until answered: each answer is a snapshot from before the change we
    /// had just heard about
    queue_asked: Vec<Instant>,
    /// what our last play/pause asked for, and when: a toggle right after it goes from there, not from a state
    /// Soloist hasn't reported yet (two quick presses sent "pause" twice)
    toggled: Option<(PlayStatus, Instant)>,
}

fn entity(v: &Value) -> Option<Item> {
    let uri = v["uri"].as_str().filter(|u| !u.is_empty())?.to_string();
    let d = &v["decorations"];
    let artists: Vec<String> = d["creators"].as_array().map(|a| a.iter().filter_map(|c| c["entity"]["decorations"]["identity"]["name"].as_str().map(String::from)).collect()).unwrap_or_default();
    let covers = d["visual_identity"]["cover"].as_array().cloned().unwrap_or_default();
    let cover_url = ["large", "xlarge", "default", "small"]
        .iter()
        .find_map(|size| covers.iter().find(|c| c["size"] == *size))
        .or(covers.first())
        .and_then(|c| c["url"].as_str().map(String::from));
    // an episode's creator may be missing: its show (the parent) stands in
    let parent = d["parent"]["entity"]["decorations"]["identity"]["name"].as_str().map(String::from);
    let artists = if artists.is_empty() && uri.starts_with("spotify:episode:") { parent.clone().into_iter().collect() } else { artists };
    Some(Item {
        uri,
        name: d["identity"]["name"].as_str().unwrap_or_default().to_string(),
        artists,
        album: d["parent"]["entity"]["decorations"]["identity"]["name"].as_str().map(String::from),
        cover_url,
        duration_ms: d["playback"]["duration_ms"].as_u64().unwrap_or(0),
        explicit: d["playback"]["content_ratings"].as_array().is_some_and(|r| r.iter().any(|x| x == "explicit")),
        artist_uris: d["creators"].as_array().map(|a| a.iter().filter_map(|c| c["entity"]["uri"].as_str().map(String::from)).collect()).unwrap_or_default(),
        album_uri: d["parent"]["entity"]["uri"].as_str().map(String::from),
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
    /// Was the track change that just arrived (`playback` already applied) caused by a command of ours? A change from
    /// another app is news to follow, not something to restart Soloist over. (Consumes what we expected.)
    fn change_is_ours(&mut self, now: Instant) -> bool {
        let by_track = match (&self.expected, self.playback.item.as_ref().map(|i| i.uri.as_str())) {
            (Some((want, at)), Some(uri)) => want == uri && now.saturating_duration_since(*at) <= OUR_CHANGE,
            _ => false,
        };
        let by_time = self.sent_at.is_some_and(|at| now.saturating_duration_since(at) <= OUR_CHANGE_UNTIL);
        self.expected = None;
        self.sent_at = None;
        by_track || by_time
    }

    /// We asked Soloist for the queue: its answer shows the player as it was, which may not have the track that just
    /// changed yet, so only the queue is taken from it.
    fn asking_for_queue(&mut self, now: Instant) {
        self.queue_asked.push(now);
    }

    /// Apply the queue in an event's `upcoming`. True if it differs from what we had. (An event without a list
    /// doesn't wipe the queue: it says nothing about it.)
    fn queue(&mut self, upcoming: &Value) -> bool {
        let Some(list) = upcoming.as_array() else { return false };
        let q: Vec<QueueEntry> = list
            .iter()
            .map(|q| {
                let item = entity(&q["item"]).unwrap_or_else(|| Item { uri: q["uid"].as_str().unwrap_or_default().into(), ..Default::default() });
                QueueEntry { item, source: q["source"].as_str().unwrap_or("context").into(), uid: q["uid"].as_str().unwrap_or_default().into() }
            })
            .collect();
        if q == self.playback.queue {
            return false;
        }
        self.playback.queue = q;
        true
    }

    /// Apply one Soloist event. Returns true when our playback state changed.
    fn apply(&mut self, ev: &Value, now: Now) -> bool {
        // the answer to our own `get_queue`: the track it still shows may be the one that just stopped. Only a state
        // that actually carries the queue we asked for is that answer: any other state in the window (a phone's
        // pause reaching us first) is real news and is applied as usual (audit C9).
        // Each ask in flight has its answer (two quick skips ask twice, C4); one not answered in time is forgotten.
        if ev["type"] == "playback_state" && ev["upcoming"].is_array() {
            self.queue_asked.retain(|at| now.at.saturating_duration_since(*at) <= QUEUE_ANSWER);
            if !self.queue_asked.is_empty() {
                self.queue_asked.remove(0);
                return self.queue(&ev["upcoming"]);
            }
        }
        // what plays next: nothing about the current track
        if ev["type"] == "queue_changed" {
            return self.queue(&ev["upcoming"]);
        }
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
            "track_changed" => {
                let item = entity(&ev["item"]);
                // a different track starts at its beginning: the old track's anchor would show its position until
                // the next `position_sync` (the bar and synced lyrics jumped, MPRIS sent a spurious Seeked)
                if item.as_ref().map(|i| &i.uri) != p.item.as_ref().map(|i| &i.uri) {
                    (p.position_ms, p.timestamp_ms, p.speed) = (0, now.ms, 0.0);
                }
                p.item = item;
            }
            "playback_changed" => {
                p.status = status_of(ev["status"].as_str().unwrap_or(""));
                if p.status != PlayStatus::Playing && p.speed != 0.0 {
                    // stop where it is now, not back at the last anchor
                    (p.position_ms, p.timestamp_ms, p.speed) = (p.position_at(now.ms), now.ms, 0.0);
                }
            }
            "volume_changed" => p.volume = ev["volume"].as_u64().unwrap_or(0).min(100) as u8,
            "context_changed" => {
                p.context_uri = ev["context"]["uri"].as_str().filter(|u| !u.is_empty()).map(String::from);
                p.context_name = ev["context"]["decorations"]["identity"]["name"].as_str().map(String::from);
            }
            "options_changed" => self.options(&ev["options"]),
            "position_sync" => self.position(&ev["position"]),
            // "queue_changed" is handled above (the queue only)
            _ => return false,
        }
        // Soloist now says what our last play/pause asked for: from here on its word is the state a toggle goes
        // from (a remembered press that outlived its answer turned the next toggle the wrong way)
        if let Some((s, _)) = self.toggled
            && (self.playback.status == s || (self.playback.status, s) == (PlayStatus::Buffering, PlayStatus::Playing))
        {
            self.toggled = None;
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

    /// Our command → Soloist WebSocket command(s). Notes what the command is expected to cause (see
    /// [`SoloistState::change_is_ours`]).
    fn soloist_commands(&mut self, cmd: Command, now: Instant) -> Vec<Value> {
        match &cmd {
            Command::Play { uri: Some(u) } => self.expected = Some((u.clone(), now)),
            Command::PlayFrom { track_uri: Some(u), .. } => self.expected = Some((u.clone(), now)),
            Command::Play { uri: None } | Command::Next | Command::Previous | Command::TogglePlay => self.sent_at = Some(now),
            _ => {}
        }
        let c = |name: &str| json!({"type":"command","command":name});
        match cmd {
            Command::Play { uri: Some(u) } => vec![json!({"type":"command","command":"play","uri":u})],
            Command::Play { uri: None } => {
                self.toggled = Some((PlayStatus::Playing, now));
                vec![c("play")]
            }
            Command::Pause => {
                self.toggled = Some((PlayStatus::Paused, now));
                vec![c("pause")]
            }
            Command::TogglePlay => {
                let now_status = match self.toggled {
                    Some((s, at)) if now.saturating_duration_since(at) < TOGGLE_FRESH => s,
                    _ => self.playback.status,
                };
                // buffering is playing too: "play" then was a no-op
                let pause = matches!(now_status, PlayStatus::Playing | PlayStatus::Buffering);
                self.toggled = Some((if pause { PlayStatus::Paused } else { PlayStatus::Playing }, now));
                vec![c(if pause { "pause" } else { "play" })]
            }
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
            Command::PlayFrom { .. } | Command::GetLibrary | Command::GetTracks { .. } | Command::MoreTracks { .. } | Command::FindShow { .. } | Command::Spectrum { .. } | Command::Shutdown | Command::SetSoloistKey { .. } | Command::GetHealth | Command::GetLyrics { .. } | Command::GetArtist { .. } | Command::Search { .. } | Command::GetDevices | Command::Transfer { .. } | Command::StartJam | Command::EndJam | Command::SetSaved { .. } | Command::CheckSaved { .. } | Command::GetListening | Command::GetStats { .. } | Command::ExportLocalData | Command::DeleteLocalData | Command::CreatePlaylist { .. } | Command::DeletePlaylist { .. } | Command::EditPlaylist { .. } | Command::AddToPlaylist { .. } | Command::RemoveFromPlaylist { .. } | Command::MoveInPlaylist { .. } | Command::SetPlaylistCover { .. } => vec![],
            // Soloist only appends; the rest goes through ConnectRemote
            Command::PlayNext { uri } => vec![json!({"type":"command","command":"add_to_queue","uri":uri})],
            Command::RemoveFromQueue { .. } | Command::ClearQueue | Command::MoveInQueue { .. } => vec![],
            Command::Login | Command::Logout | Command::GetHome | Command::RestartEngine | Command::Unknown => vec![],
        }
    }
}
