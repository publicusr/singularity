//! T0025 audit (correctness): the bug-proving tests. The ones T0027 fixed (C2/A1, C1/A8) run for real; C10 and C14
//! belong to other tickets and stay `#[ignore]`d until they are fixed.

mod common;

use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg, EngineKind, Event, Item, PlayStatus, PlaybackState, QueueEntry, PROTOCOL_VERSION};
use singularityd::catalogue::{CatCmd, CatEvent};
use singularityd::engine::{Engine, EngineEvent};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

// ── C2/A1: a dead Engine task is silent; the Daemon never reports it ────────────────────────────────

/// Counts how many Engine tasks were started, so a restart can be seen.
#[derive(Clone, Default)]
struct Started(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Started {
    fn count(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// An Engine whose task has already ended (a panic or an error path in the real ones): its event channel's
/// sender is dropped before `serve` even starts the command pump. It can be built again, as the real ones can.
struct DeadEngine(Started);

impl Engine for DeadEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Fake
    }

    fn respawn(&self) -> Option<Box<dyn Engine>> {
        self.0.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(Box::new(RevivedEngine))
    }

    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (commands, _dropped) = mpsc::channel(4);
        let (events_tx, events) = mpsc::channel(4);
        drop(events_tx);
        (commands, events)
    }
}

/// The Engine after the restart: alive, ready, and it answers commands.
struct RevivedEngine;

impl Engine for RevivedEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Fake
    }

    fn respawn(&self) -> Option<Box<dyn Engine>> {
        Some(Box::new(RevivedEngine))
    }

    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (commands, mut cmd_rx) = mpsc::channel(16);
        let (events, ev_rx) = mpsc::channel(16);
        tokio::spawn(async move {
            let _ = events.send(EngineEvent::Status(singularity_proto::EngineStatus::Ready { device_name: "Revived".into(), active: true, version: "test".into(), expires_in_days: None })).await;
            while let Some(cmd) = cmd_rx.recv().await {
                if cmd == (Command::Play { uri: None }) {
                    let _ = events.send(EngineEvent::Playback(PlaybackState { status: PlayStatus::Playing, timestamp_ms: singularity_proto::now_ms(), ..Default::default() })).await;
                }
            }
        });
        (commands, ev_rx)
    }
}

async fn connect(path: &std::path::Path) -> UnixStream {
    let mut s = UnixStream::connect(path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: "audit".into() }).await.unwrap();
    let _welcome: DaemonMsg = proto::read_msg(&mut s).await.unwrap().unwrap();
    s
}

/// The next event that is one of `wanted`, within `wait`.
async fn next_of(s: &mut UnixStream, wait: Duration, mut wanted: impl FnMut(&Event) -> bool) -> bool {
    tokio::time::timeout(wait, async {
        loop {
            match proto::read_msg::<_, DaemonMsg>(s).await {
                Ok(Some(DaemonMsg::Event(ev))) if wanted(&ev) => return true,
                Ok(Some(_)) => continue,
                _ => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// `cargo test -p singularityd --test audit_correctness an_engine_that_dies`
///
/// Today (before the fix): the Engine's task ending makes `run_engine` return, which drops the command receiver;
/// `transport()` then threw every player command away and nobody told the Client, restarted the Engine or showed a
/// status. Now: the Client hears `Engine(Failed)` and an error, commands fail loudly instead of vanishing, and the
/// Engine is built again — after which playback works.
#[tokio::test]
async fn an_engine_that_dies_is_reported_and_playback_commands_do_not_vanish() {
    let dir = common::scratch();
    let path = dir.path().join("d.sock");
    let listener = singularityd::bind(&path).unwrap();
    let restarts = Started::default();
    tokio::spawn(singularityd::serve(listener, Box::new(DeadEngine(restarts.clone())), None, None));
    let mut s = connect(&path).await;
    // the failing status (the Engine is reported dead as soon as its task is seen to be gone)
    assert!(
        next_of(&mut s, Duration::from_secs(2), |e| matches!(e, Event::Engine(singularity_proto::EngineStatus::Failed { .. }))).await,
        "a dead Engine was never reported: the UI keeps saying it's fine"
    );
    // while it's down, a command fails loudly rather than vanishing
    proto::write_msg(&mut s, &ClientMsg::Command(Command::Play { uri: Some("spotify:track:x".into()) })).await.unwrap();
    assert!(next_of(&mut s, Duration::from_secs(2), |e| matches!(e, Event::Error { .. })).await, "the command went nowhere, silently");
    // ... and the Engine is started again (its fresh status says so)
    assert!(
        next_of(&mut s, Duration::from_secs(10), |e| matches!(e, Event::Engine(singularity_proto::EngineStatus::Ready { device_name, .. }) if device_name == "Revived")).await,
        "the dead Engine was never started again"
    );
    assert!(restarts.count() >= 1, "no new Engine was built");
    // playback works again
    proto::write_msg(&mut s, &ClientMsg::Command(Command::Play { uri: None })).await.unwrap();
    assert!(next_of(&mut s, Duration::from_secs(2), |e| matches!(e, Event::Playback(p) if p.status == PlayStatus::Playing)).await, "commands still don't reach the restarted Engine");
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

/// `cargo test -p singularityd --test audit_correctness a_description_that`
///
/// Before the fix: `fill_queue` marked all 25 URIs as being asked; the Catalogue answered for the first one only (as
/// the real one did for the 21st+ of a batch, and after the first `getTrack` failure), and nothing ever cleared the
/// other 24 from `Hub::describing`, so a later playback state showing them bare didn't ask again and those queue rows
/// kept an empty name for the rest of the Daemon's life. Now the Catalogue reports what it couldn't look up
/// (`DescribeFailed`, which the real one sends too), and the Hub asks again.
#[tokio::test]
async fn a_description_that_never_arrives_is_asked_for_again() {
    let dir = common::scratch();
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
                let mut rest = vec![];
                for (i, uri) in uris.into_iter().enumerate() {
                    if i == 0 {
                        let item = Item { uri: uri.clone(), name: "Described".into(), artists: vec!["A".into()], ..Default::default() };
                        let _ = cat_events.send(CatEvent::Described(vec![item])).await;
                    } else {
                        // what the real Catalogue says about the URIs it couldn't look up (A8)
                        rest.push(uri);
                    }
                }
                if !rest.is_empty() {
                    let _ = cat_events.send(CatEvent::DescribeFailed(rest)).await;
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

/// C10: before the fix a play was identified by track URI only, so a track that started over (repeat one, a replay)
/// was taken for the same play; when it finally ended, one play was logged with `played_ms` clamped to the track's
/// length. "played N times", the listening minutes and the streak undercounted every looped track.
#[test]
fn a_repeated_track_is_logged_as_a_second_play() {
    let mut r = singularityd::store::Recorder::default();
    assert_eq!(r.observe(&playing("spotify:track:a", 0, 0), "here", 0), None, "nothing to log yet");
    // 200 s later the same track starts over from the beginning (repeat one)
    let first = r.observe(&playing("spotify:track:a", 0, 200_000), "here", 200_000);
    let play = first.expect("the first run of the track was never logged as a play");
    assert_eq!((play.item.uri.as_str(), play.played_ms), ("spotify:track:a", 200_000));
}

/// B8: a seek back to hear a part again is the same play; only a jump back to near the start (the first 20 s) is a
/// replay. Before, any jump back of more than 5 s was logged as a new play.
#[test]
fn a_seek_back_mid_track_is_the_same_play_but_back_to_the_start_is_a_replay() {
    let mut r = singularityd::store::Recorder::default();
    r.observe(&playing("spotify:track:a", 0, 0), "here", 0);
    // at 2:00, back to 1:00: hearing the chorus again
    assert_eq!(r.observe(&playing("spotify:track:a", 60_000, 120_000), "here", 120_000), None, "a seek back mid-track is still the first play");
    // 1:30 later (at 2:30 of the track), back to 0:10: it started over
    let play = r.observe(&playing("spotify:track:a", 10_000, 210_000), "here", 210_000).expect("a jump back to the start is a replay");
    assert_eq!(play.played_ms, 200_000, "the whole run, capped at the track's length");
}

// ── C14: an LRC timestamp's minutes overflow (handed over by T0024) ──────────────────────────────────

/// C14: before the fix `parse_lrc` computed `m * 60_000 + …` on a `u64` parsed straight out of the text LRCLIB sent.
/// A minute field of 20 digits overflowed: debug builds panicked (in the Catalogue task), release builds wrapped to
/// an arbitrary time and the line was shown at the wrong moment. Now it saturates (still the latest line) and a
/// non-finite or negative stamp is skipped.
#[test]
fn a_hostile_lrc_timestamp_does_not_overflow() {
    let lines = singularityd::lyrics::parse_lrc("[18446744073709551615:59.99] words\n[00:01.00] fine\n[00:inf] no\n[00:-2.5] no\n");
    assert_eq!(lines.len(), 2, "the non-finite and negative stamps are skipped");
    // saturated, in time order, and the sane line is untouched
    assert_eq!((lines[0].text.as_str(), lines[0].start_ms), ("fine", 1000));
    assert_eq!((lines[1].text.as_str(), lines[1].start_ms), ("words", u64::MAX), "the overflowing stamp stays the latest, not a wrapped one");
}

// ── B2 (2026-11-20 bug hunt): the Client keeps showing a device that stopped playing ────────────────

/// The computer's own player: one paused state whose anchor is a minute old (Soloist keeps its last anchor while it
/// sits paused), then nothing.
struct PausedHere;

impl Engine for PausedHere {
    fn kind(&self) -> EngineKind {
        EngineKind::Fake
    }

    fn respawn(&self) -> Option<Box<dyn Engine>> {
        None
    }

    fn spawn(self: Box<Self>) -> (mpsc::Sender<Command>, mpsc::Receiver<EngineEvent>) {
        let (commands, mut cmd_rx) = mpsc::channel(16);
        let (events, ev_rx) = mpsc::channel(16);
        tokio::spawn(async move {
            let item = Item { uri: "spotify:track:here".into(), name: "here".into(), ..Default::default() };
            let at = singularity_proto::now_ms() - 60_000;
            let _ = events.send(EngineEvent::Playback(PlaybackState { status: PlayStatus::Paused, item: Some(item), position_ms: 1_000, timestamp_ms: at, ..Default::default() })).await;
            while cmd_rx.recv().await.is_some() {}
        });
        (commands, ev_rx)
    }
}

/// B2: a session dropped every playback state older than the newest one it had forwarded, for its whole life. A
/// remote device's states are stamped when they arrive, so when the phone stopped and the Daemon went back to its own
/// player's state (anchored when it last moved), every connected Client dropped it and kept showing the phone's track.
/// Now only what was queued while the snapshot was read is checked against it.
#[tokio::test]
async fn when_another_device_stops_the_client_goes_back_to_this_one() {
    let dir = common::scratch();
    let path = dir.path().join("d.sock");
    let listener = singularityd::bind(&path).unwrap();
    let (cat_tx, mut cat_rx) = mpsc::channel::<CatCmd>(16);
    let (cat_events, cat_events_rx) = mpsc::channel::<CatEvent>(16);
    tokio::spawn(async move { while cat_rx.recv().await.is_some() {} });
    tokio::spawn(singularityd::serve_with(listener, Box::new(PausedHere), Some((cat_tx, cat_events_rx)), None, singularityd::Extras::default()));
    let mut s = connect(&path).await;
    let is = |ev: &Event, uri: &str| matches!(ev, Event::Playback(p) if p.item.as_ref().is_some_and(|i| i.uri == uri));
    assert!(next_of(&mut s, Duration::from_secs(2), |ev| is(ev, "spotify:track:here")).await, "this computer's state first");
    // a phone plays …
    let phone = PlaybackState { status: PlayStatus::Playing, item: Some(Item { uri: "spotify:track:phone".into(), ..Default::default() }), timestamp_ms: singularity_proto::now_ms(), speed: 1.0, ..Default::default() };
    cat_events.send(CatEvent::Remote(Some(phone))).await.unwrap();
    assert!(next_of(&mut s, Duration::from_secs(2), |ev| is(ev, "spotify:track:phone")).await, "the phone's track");
    // … and stops being the active device
    cat_events.send(CatEvent::Remote(None)).await.unwrap();
    assert!(next_of(&mut s, Duration::from_secs(2), |ev| is(ev, "spotify:track:here")).await, "the Client was left showing the phone");
}
