//! The Engine interface (ADR-0001): something that plays audio and reports what it's doing.
//! Implementations run as their own task and talk through channels, so the Daemon never blocks on them.

use singularity_proto::{Command, EngineStatus, PlaybackState};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum EngineEvent {
    Playback(PlaybackState),
    Status(EngineStatus),
    /// what is streaming now (codec, bitrate)
    Stream(singularity_proto::StreamInfo),
    /// download speed and stalls (#45)
    Delivery(singularity_proto::Delivery),
    Error(String),
    Notice(String),
    /// the Engine restarted (a Soloist update): pick up where playback was
    Resume(Resume),
}

/// Where playback was when the Engine restarted.
#[derive(Debug, Clone, PartialEq)]
pub struct Resume {
    pub context_uri: Option<String>,
    pub track_uri: String,
    pub position_ms: u64,
    pub paused: bool,
}

/// Which Connect device is this computer's Engine, and where its playback comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// the device's name on Connect
    pub device_name: String,
    /// AppEngine (D51): driven over Connect, its playback read from the Connect cluster; Soloist reports its own
    pub via_connect: bool,
    /// the app's version, for the health view (AppEngine only)
    pub version: String,
}

impl Default for Identity {
    /// Soloist, as before there was a choice
    fn default() -> Identity {
        Identity { device_name: crate::soloist::DEVICE_NAME.into(), via_connect: false, version: String::new() }
    }
}

pub trait Engine: Send + 'static {
    /// Start the Engine task. Returns the command sender and the event stream.
    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>);
}

/// Used until an Engine is configured: reports why, refuses commands politely.
pub struct NoEngine {
    pub reason: String,
}

impl Engine for NoEngine {
    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(16);
        let (ev_tx, ev_rx) = mpsc::channel(16);
        tokio::spawn(async move {
            let _ = ev_tx.send(EngineEvent::Status(EngineStatus::NotConfigured { reason: self.reason.clone() })).await;
            while cmd_rx.recv().await.is_some() {
                let _ = ev_tx.send(EngineEvent::Error(format!("No playback engine yet: {}", self.reason))).await;
            }
        });
        (cmd_tx, ev_rx)
    }
}

/// A pretend Engine for tests and offline UI work: plays an imaginary three-track album instantly.
pub struct FakeEngine;

impl Engine for FakeEngine {
    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        use singularity_proto::{Item, PlayStatus};
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(16);
        let (ev_tx, ev_rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut tracks: Vec<Item> = (1..=3)
                .map(|i| Item { uri: format!("spotify:track:fake{i}"), name: format!("Track {i}"), artists: vec!["Fake Artist".into()], duration_ms: 200_000, cover_url: Some(fixture_cover(i)), ..Default::default() })
                .collect();
            let mut idx = 0usize;
            let mut st = PlaybackState { volume: 50, ..Default::default() };
            let _ = ev_tx.send(EngineEvent::Status(EngineStatus::Ready { device_name: "Fake".into(), active: true, version: "fake".into(), expires_in_days: Some(90) })).await;
            // SINGULARITY_FAKE_STALLS=1: a lossless stream on a starving connection, for the stream Pane (#45)
            if std::env::var("SINGULARITY_FAKE_STALLS").is_ok_and(|v| v == "1") {
                let ev = ev_tx.clone();
                tokio::spawn(async move {
                    let info = singularity_proto::StreamInfo { codec: "flac".into(), bitrate: 1_400_000, average_bitrate: Some(1_643_334), sample_rate: 44_100, sample_format: "32-bit-float-pcm".into() };
                    let _ = ev.send(EngineEvent::Stream(info)).await;
                    let mut d = singularity_proto::Delivery::default();
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                        let now = singularity_proto::now_ms();
                        d.bandwidth_kbps = Some(44);
                        d.measured_ms = now;
                        d.stalls_ms.retain(|t| now.saturating_sub(*t) <= 120_000);
                        d.stalls_ms.push(now);
                        if ev.send(EngineEvent::Delivery(d.clone())).await.is_err() {
                            return;
                        }
                    }
                });
            }
            while let Some(cmd) = cmd_rx.recv().await {
                let now = singularity_proto::now_ms();
                let pos = st.position_at(now);
                match cmd {
                    Command::Play { uri } => {
                        if let Some(u) = uri {
                            idx = pick(&mut tracks, &u, None);
                            st.position_ms = 0;
                            st.context_uri = Some(u);
                        } else {
                            st.position_ms = pos;
                        }
                        st.status = PlayStatus::Playing;
                    }
                    Command::Pause => {
                        st.position_ms = pos;
                        st.status = PlayStatus::Paused;
                    }
                    Command::TogglePlay => {
                        st.position_ms = pos;
                        st.status = if st.status == PlayStatus::Playing { PlayStatus::Paused } else { PlayStatus::Playing };
                    }
                    Command::Next => {
                        idx = (idx + 1) % tracks.len();
                        st.position_ms = 0;
                    }
                    Command::Previous => {
                        idx = (idx + tracks.len() - 1) % tracks.len();
                        st.position_ms = 0;
                    }
                    Command::PlayFrom { context_uri, index, track_uri } => {
                        let uri = track_uri.unwrap_or_else(|| format!("{context_uri}#{index}"));
                        idx = pick(&mut tracks, &uri, Some(index));
                        st.position_ms = 0;
                        st.context_uri = Some(context_uri);
                        st.status = PlayStatus::Playing;
                        // the next few songs of the list, as "Next from …" entries (bare: the Catalogue names them), so the
                        // queue Widget has a context to drag from (#50)
                        st.queue.retain(|q| q.source == "queue");
                        st.queue.extend((1..=4).map(|k| singularity_proto::QueueEntry { item: Item { uri: format!("spotify:track:ctx{}", index + k), ..Default::default() }, source: "context".into(), uid: format!("c{now}{k}") }));
                    }
                    // a queue, so the queue Widget has something to show; entries come bare, like Soloist's sometimes do
                    Command::AddToQueue { ref uri } | Command::PlayNext { ref uri } => {
                        let uri = uri.clone();
                        let n = st.queue.len();
                        let entry = singularity_proto::QueueEntry { item: Item { uri, ..Default::default() }, source: "queue".into(), uid: format!("q{now}{n}") };
                        let first_context = st.queue.iter().position(|q| q.source != "queue").unwrap_or(n);
                        let at = if matches!(cmd, Command::PlayNext { .. }) { 0 } else { first_context };
                        st.queue.insert(at, entry);
                    }
                    Command::RemoveFromQueue { uri, uid } => {
                        if let Some(i) = st.queue.iter().position(|q| q.source == "queue" && (uid.as_deref() == Some(q.uid.as_str()) || q.item.uri == uri)) {
                            st.queue.remove(i);
                        }
                    }
                    Command::ClearQueue => st.queue.retain(|q| q.source != "queue"),
                    Command::MoveInQueue { uri, uid, to, into_queue: true } => {
                        let Some(i) = st.queue.iter().position(|q| q.source == "context" && (uid.as_deref() == Some(q.uid.as_str()) || q.item.uri == uri)) else { continue };
                        let mut e = st.queue.remove(i);
                        e.source = "queue".into();
                        let queued = st.queue.iter().take_while(|q| q.source == "queue").count();
                        st.queue.insert(to.min(queued), e);
                    }
                    Command::MoveInQueue { uri, uid, to, into_queue: false } => {
                        let Some(i) = st.queue.iter().position(|q| q.source != "autoplay" && (uid.as_deref() == Some(q.uid.as_str()) || q.item.uri == uri)) else { continue };
                        let e = st.queue.remove(i);
                        // its own kind's run: the queued ones come first, then the context's
                        let start = st.queue.iter().position(|q| q.source == e.source).unwrap_or(i);
                        let len = st.queue[start..].iter().take_while(|q| q.source == e.source).count();
                        st.queue.insert(start + to.min(len), e);
                    }
                    Command::Seek { position_ms } => st.position_ms = position_ms,
                    Command::SetVolume { percent } => st.volume = percent.min(100),
                    Command::SetShuffle { enabled } => st.shuffle = enabled,
                    Command::SetRepeat { mode } => st.repeat = mode,
                    _ => continue,
                }
                st.item = Some(tracks[idx].clone());
                st.timestamp_ms = now;
                st.speed = if st.status == PlayStatus::Playing { 1.0 } else { 0.0 };
                let _ = ev_tx.send(EngineEvent::Playback(st.clone())).await;
            }
        });
        (cmd_tx, ev_rx)
    }
}

/// The fake track for `uri`, made up on first sight.
fn pick(tracks: &mut Vec<singularity_proto::Item>, uri: &str, index: Option<u32>) -> usize {
    if let Some(i) = tracks.iter().position(|t| t.uri == uri) {
        return i;
    }
    let name = index.map(|i| format!("Fake Track {:03}", i + 1)).unwrap_or_else(|| uri.rsplit(':').next().unwrap_or(uri).to_string());
    let cover_url = Some(fixture_cover(index.unwrap_or(tracks.len() as u32) as usize));
    tracks.push(singularity_proto::Item { uri: uri.into(), name, artists: vec!["Fake Artist".into()], duration_ms: 200_000, cover_url, ..Default::default() });
    tracks.len() - 1
}

/// A cover for a fake track: the recorded Home feed's covers (the pty harness keeps them cached), so screenshots
/// show real pictures in Now playing and the demos.
fn fixture_cover(i: usize) -> String {
    format!("https://i.scdn.co/image/fixture{:03}", i % 30)
}
