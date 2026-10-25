//! T0025 audit (correctness): tests that show real defects, kept `#[ignore]` so `main` stays green.
//! Removing the `#[ignore = "..."]` line makes the named test fail against today's code.

use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg, EngineKind, Event, Item, PlayStatus, PlaybackState, QueueEntry, PROTOCOL_VERSION};
use singularityd::catalogue::{CatCmd, CatEvent};
use singularityd::engine::{Engine, EngineEvent};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

// ── C2: a dead Engine task is silent; the Daemon never reports it ─────────────────────────────────────

/// An Engine whose task has already ended (a panic or an error path in the real ones): its event channel's
/// sender is dropped before `serve` even starts the command pump.
struct DeadEngine;

impl Engine for DeadEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Fake
    }

    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (commands, _dropped) = mpsc::channel(4);
        let (events_tx, events) = mpsc::channel(4);
        drop(events_tx);
        (commands, events)
    }
}

async fn connect(path: &std::path::Path) -> UnixStream {
    let mut s = UnixStream::connect(path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: "audit".into() }).await.unwrap();
    let _welcome: DaemonMsg = proto::read_msg(&mut s).await.unwrap().unwrap();
    s
}

/// `cargo test -p singularityd --test audit_correctness an_engine_that_dies -- --ignored`
///
/// Today: the Engine's task ending makes `run_engine` return, which drops the command receiver; `transport()`
/// then throws every player command away (`let _ = hub.commands.send(..)`) and nobody tells the Client, restarts
/// the Engine or shows a status. The Daemon looks alive and ignores play/pause forever.
#[tokio::test]
#[ignore = "audit C2: a dead Engine task is silent (commands dropped, no status, no restart)"]
async fn an_engine_that_dies_is_reported_and_playback_commands_do_not_vanish() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(DeadEngine), None, None));
    let mut s = connect(&path).await;
    // let the engine pump notice its channel closed
    tokio::time::sleep(Duration::from_millis(100)).await;
    proto::write_msg(&mut s, &ClientMsg::Command(Command::Play { uri: Some("spotify:track:x".into()) })).await.unwrap();
    // the Daemon should say something: the Engine failed, or the command couldn't be delivered
    let told = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match proto::read_msg::<_, DaemonMsg>(&mut s).await {
                Ok(Some(DaemonMsg::Event(Event::Error { .. }))) => return true,
                Ok(Some(DaemonMsg::Event(Event::Engine(singularity_proto::EngineStatus::Failed { .. })))) => return true,
                Ok(Some(_)) => continue,
                _ => return false,
            }
        }
    })
    .await;
    assert_eq!(told, Ok(true), "a dead Engine was never reported: the command went nowhere, silently");
}

// ── C1: a description that never comes back wedges those URIs for the Daemon's life ───────────────────

/// Plays a fixed playback state (25 bare queue entries, like the app Engine's queue) and repeats it on any
/// command, so the Hub's `fill_queue` runs a second time.
struct BareQueueEngine;

impl Engine for BareQueueEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Fake
    }

    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (commands, mut cmd_rx) = mpsc::channel(16);
        let (events, ev_rx) = mpsc::channel(16);
        tokio::spawn(async move {
            let state = PlaybackState {
                status: PlayStatus::Playing,
                item: Some(Item { uri: "spotify:track:playing".into(), name: "Playing".into(), artists: vec!["A".into()], ..Default::default() }),
                queue: (0..25)
                    .map(|i| QueueEntry { item: Item { uri: format!("spotify:track:q{i}"), ..Default::default() }, source: "context".into(), uid: format!("u{i}") })
                    .collect(),
                ..Default::default()
            };
            let _ = events.send(EngineEvent::Playback(state.clone())).await;
            while cmd_rx.recv().await.is_some() {
                let _ = events.send(EngineEvent::Playback(state.clone())).await;
            }
        });
        (commands, ev_rx)
    }
}

/// `cargo test -p singularityd --test audit_correctness a_description_that -- --ignored`
///
/// Today: `fill_queue` marks all 25 URIs as being asked; the Catalogue answers for the first one only (as the
/// real one does for the 21st+ of a batch, and after the first `getTrack` failure), and nothing ever clears the
/// other 24 from `Hub::describing`. A later playback state showing them bare doesn't ask again, so those queue
/// rows keep an empty name for the rest of the Daemon's life.
#[tokio::test]
#[ignore = "audit C1: URIs whose description failed are never asked for again (blank queue rows forever)"]
async fn a_description_that_never_arrives_is_asked_for_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sock");
    let listener = singularityd::bind(&path).unwrap();
    // a Catalogue stand-in: it describes only the first URI of every batch it is asked about, and counts
    let (cat_tx, mut cat_rx) = mpsc::channel::<CatCmd>(16);
    let (cat_events, cat_events_rx) = mpsc::channel::<CatEvent>(64);
    let describes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = describes.clone();
    tokio::spawn(async move {
        while let Some(cmd) = cat_rx.recv().await {
            if let CatCmd::Describe(uris) = cmd {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if let Some(first) = uris.first() {
                    let item = Item { uri: first.clone(), name: "Described".into(), artists: vec!["A".into()], ..Default::default() };
                    let _ = cat_events.send(CatEvent::Described(vec![item])).await;
                }
            }
        }
    });
    tokio::spawn(singularityd::serve_with(listener, Box::new(BareQueueEngine), Some((cat_tx, cat_events_rx)), None, singularityd::Extras::default()));
    let mut s = connect(&path).await;
    // the first playback state asks about the 25 bare entries
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(describes.load(std::sync::atomic::Ordering::SeqCst), 1, "the first batch was asked about");
    // a second playback state (a command made the Engine re-emit it) shows the same bare entries
    proto::write_msg(&mut s, &ClientMsg::Command(Command::Pause)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(describes.load(std::sync::atomic::Ordering::SeqCst), 2, "the URIs whose description never came back were never asked for again");
}

// ── C10: repeat-one (or replaying a track) is logged as one play ──────────────────────────────────────

fn playing(uri: &str, position_ms: u64, now_ms: u64) -> PlaybackState {
    PlaybackState {
        status: PlayStatus::Playing,
        item: Some(Item { uri: uri.into(), name: uri.into(), artists: vec!["A".into()], duration_ms: 200_000, ..Default::default() }),
        position_ms,
        timestamp_ms: now_ms,
        speed: 1.0,
        ..Default::default()
    }
}

/// `cargo test -p singularityd --test audit_correctness a_repeated_track -- --ignored`
///
/// Today: a play is identified by track URI only, so a track that starts over (repeat one, or a replay) is
/// taken for the same play; when it finally ends, one play is logged with `played_ms` clamped to the track's
/// length. "played N times", the listening minutes and the streak undercount every looped track.
#[test]
#[ignore = "audit C10: repeat-one is a single play in the listening log"]
fn a_repeated_track_is_logged_as_a_second_play() {
    let mut r = singularityd::store::Recorder::default();
    assert_eq!(r.observe(&playing("spotify:track:a", 0, 0), "here", 0), None, "nothing to log yet");
    // 200 s later the same track starts over from the beginning (repeat one)
    let first = r.observe(&playing("spotify:track:a", 0, 200_000), "here", 200_000);
    let play = first.expect("the first run of the track was never logged as a play");
    assert_eq!((play.item.uri.as_str(), play.played_ms), ("spotify:track:a", 200_000));
}

// ── C14: an LRC timestamp's minutes overflow (handed over by T0024) ──────────────────────────────────

/// `cargo test -p singularityd --test audit_correctness a_hostile_lrc -- --ignored`
///
/// Today: `parse_lrc` computes `m * 60_000 + …` on a `u64` parsed straight out of the text LRCLIB sent. A
/// minute field of 20 digits overflows: debug builds panic (in the Catalogue task: with C2 that is a dead
/// Catalogue), release builds wrap to an arbitrary time and the line is shown at the wrong moment.
#[test]
#[ignore = "audit C14: an LRC timestamp's minutes overflow (panic in debug, a wrong time in release)"]
fn a_hostile_lrc_timestamp_does_not_overflow() {
    let lines = singularityd::lyrics::parse_lrc("[18446744073709551615:59.99] words\n[00:01.00] fine\n");
    assert_eq!(lines.len(), 2);
    // saturated, in time order, and the sane line is untouched
    assert_eq!(lines[1].text, "fine");
    assert_eq!(lines[1].start_ms, 1000);
    assert!(lines[0].start_ms >= lines[1].start_ms, "the overflowing stamp stays the latest, not a wrapped one: {:?}", lines[0].start_ms);
}
