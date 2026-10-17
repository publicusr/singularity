//! The Engine interface (ADR-0001): something that plays audio and reports what it's doing.
//! Implementations run as their own task and talk through channels, so the Daemon never blocks on them.

use singularity_proto::{Command, EngineStatus, PlaybackState};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum EngineEvent {
    Playback(PlaybackState),
    Status(EngineStatus),
    Error(String),
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
