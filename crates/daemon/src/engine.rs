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
                .map(|i| Item { uri: format!("spotify:track:fake{i}"), name: format!("Track {i}"), artists: vec!["Fake Artist".into()], duration_ms: 200_000, ..Default::default() })
                .collect();
            let mut idx = 0usize;
            let mut st = PlaybackState { volume: 50, ..Default::default() };
            let _ = ev_tx.send(EngineEvent::Status(EngineStatus::Ready { device_name: "Fake".into(), active: true, version: "fake".into(), expires_in_days: Some(90) })).await;
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
                    Command::MoveInQueue { uri, uid, to } => {
                        let Some(i) = st.queue.iter().position(|q| q.source == "queue" && (uid.as_deref() == Some(q.uid.as_str()) || q.item.uri == uri)) else { continue };
                        let e = st.queue.remove(i);
                        let queued = st.queue.iter().take_while(|q| q.source == "queue").count();
                        st.queue.insert(to.min(queued), e);
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
    tracks.push(singularity_proto::Item { uri: uri.into(), name, artists: vec!["Fake Artist".into()], duration_ms: 200_000, ..Default::default() });
    tracks.len() - 1
}
