//! Adapter contract test: a recorded Soloist WebSocket transcript (spike 1, anonymised) maps to the
//! PlaybackState we promise Clients, and our commands map to Soloist's. Then whole runs of the Soloist session, on a
//! made-up clock: an update restarted into at a quiet moment, a skip, a resume after the restart, a stuck stream.

use serde_json::{Value, json};
use singularity_proto::{Command, EngineStatus, PlayStatus, PlaybackState, RepeatMode};
use singularityd::engine::{EngineEvent, Resume};
use singularityd::soloist::{Action, Handover, Input, LogNews, Now, Session, Stop};
use std::time::Duration;

fn session(handover: Handover) -> Session {
    Session::new("Singularity".into(), "1.0".into(), Some(55), handover)
}

fn t0() -> Now {
    Now { at: tokio::time::Instant::now(), ms: 1_750_000_000_000 }
}

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn ws(s: &mut Session, ev: Value, now: Now) -> Vec<Action> {
    s.step(Input::Ws(ev), now)
}

/// The commands a step sends to Soloist.
fn sent(a: &[Action]) -> Vec<String> {
    a.iter().filter_map(|a| if let Action::Send(m) = a { m["command"].as_str().map(String::from) } else { None }).collect()
}

/// The playback state a step tells the Daemon about last.
fn playback(a: &[Action]) -> Option<PlaybackState> {
    a.iter().rev().find_map(|a| if let Action::Event(EngineEvent::Playback(p)) = a { Some(p.clone()) } else { None })
}

fn stop(a: &[Action]) -> Option<Stop> {
    a.iter().find_map(|a| if let Action::Stop(s) = a { Some(*s) } else { None })
}

fn resumed(a: &[Action]) -> Option<Resume> {
    a.iter().find_map(|a| if let Action::Event(EngineEvent::Resume(r)) = a { Some(r.clone()) } else { None })
}

fn statuses(a: &[Action]) -> Vec<EngineStatus> {
    a.iter().filter_map(|a| if let Action::Event(EngineEvent::Status(s)) = a { Some(s.clone()) } else { None }).collect()
}

fn notices(a: &[Action]) -> Vec<String> {
    a.iter().filter_map(|a| if let Action::Event(EngineEvent::Notice(n)) = a { Some(n.clone()) } else { None }).collect()
}

fn item(uri: &str) -> Value {
    json!({"uri": uri, "decorations": {"identity": {"name": uri}, "playback": {"duration_ms": 200_000}}})
}

fn track(uri: &str) -> Value {
    json!({"type": "track_changed", "item": item(uri)})
}

/// A full playback state: `status`, `uri` at `pos_ms` as of `now`.
fn state(status: &str, uri: &str, pos_ms: u64, now: Now) -> Value {
    let speed = if status == "playing" { 1.0 } else { 0.0 };
    json!({
        "type": "playback_state", "status": status, "item": item(uri), "context": {"uri": "spotify:album:x"},
        "position": {"position_ms": pos_ms, "timestamp_ms": now.ms, "speed": speed}, "volume": 55, "options": {}
    })
}

/// A logged-in session playing track `uri` at 42 s.
fn playing(uri: &str, now: Now) -> Session {
    let mut s = session(Handover::default());
    ws(&mut s, json!({"type": "auth_state", "logged_in": true, "is_active": true}), now);
    ws(&mut s, state("playing", uri, 42_000, now), now);
    s
}

#[test]
fn transcript_maps_to_playback_state() {
    let mut s = session(Handover::default());
    let now = t0();
    let mut last = None;
    for line in include_str!("fixtures/soloist_transcript.jsonl").lines() {
        let ev: Value = serde_json::from_str(line).unwrap();
        last = playback(&ws(&mut s, ev, now)).or(last);
    }
    let p = last.unwrap();
    let item = p.item.as_ref().unwrap();
    assert_eq!(item.name, "Accretion Disk");
    assert_eq!(item.artists, vec!["Nova Drift"]);
    assert_eq!(item.album.as_deref(), Some("Event Horizon"));
    assert_eq!(item.cover_url.as_deref(), Some("https://i.scdn.co/image/large"));
    assert!(item.explicit);
    assert_eq!(item.duration_ms, 227_000);
    assert_eq!(p.status, PlayStatus::Playing);
    assert_eq!(p.context_name.as_deref(), Some("Event Horizon"));
    assert_eq!((p.position_ms, p.speed), (45_000, 1.0));
    assert_eq!(p.volume, 72);
    assert!(p.shuffle);
    assert_eq!(p.repeat, RepeatMode::Context);
    assert_eq!(p.queue.len(), 2);
    assert_eq!(p.queue[1].source, "autoplay");
}

#[test]
fn commands_map_to_soloist() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    assert_eq!(sent(&s.step(Input::Command(Command::TogglePlay), now)), ["pause"]);
    let rep = sent(&s.step(Input::Command(Command::SetRepeat { mode: RepeatMode::Track }), now));
    assert_eq!(rep, ["set_repeat_context", "set_repeat_track"]);
    let vol = s.step(Input::Command(Command::SetVolume { percent: 150 }), now);
    assert!(matches!(vol[0], Action::SaveVolume(100)), "a restarted Soloist starts at it: {vol:?}");
    assert!(matches!(&vol[1], Action::Send(m) if m["volume"] == 100));
}

/// The volume asked for while Soloist was starting is lost once it logs in: it's set again then, and only then.
#[test]
fn the_volume_is_set_again_on_login() {
    let now = t0();
    let mut s = session(Handover::default());
    let a = ws(&mut s, json!({"type": "auth_state", "logged_in": false}), now);
    assert!(sent(&a).is_empty());
    assert!(matches!(statuses(&a)[..], [EngineStatus::NeedsPairing { .. }]));
    s.step(Input::Command(Command::SetVolume { percent: 30 }), now);
    let a = ws(&mut s, json!({"type": "auth_state", "logged_in": true, "is_active": true}), now);
    assert!(matches!(&a[0], Action::Send(m) if m["command"] == "set_volume" && m["volume"] == 30), "the volume asked for last: {a:?}");
    assert!(matches!(statuses(&a)[..], [EngineStatus::Ready { active: true, .. }]));
    assert!(sent(&ws(&mut s, json!({"type": "device_changed", "is_active": false}), now)).is_empty(), "still logged in: not again");
    // the build's lifetime arrives later: the status says it
    let a = s.step(Input::Log(LogNews::Lifetime(80)), now);
    assert!(matches!(statuses(&a)[..], [EngineStatus::Ready { active: false, expires_in_days: Some(80), .. }]));
}

#[test]
fn a_quick_second_toggle_goes_from_the_first_not_from_a_stale_state() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    assert_eq!(sent(&s.step(Input::Command(Command::TogglePlay), now)), ["pause"]);
    // Soloist hasn't said "paused" yet: the second press still means play
    assert_eq!(sent(&s.step(Input::Command(Command::TogglePlay), now + Duration::from_millis(300))), ["play"]);
    // the press goes stale
    assert_eq!(sent(&s.step(Input::Command(Command::TogglePlay), now + secs(3))), ["pause"], "from Soloist's word again");
    let mut s = session(Handover::default());
    ws(&mut s, json!({"type": "playback_changed", "status": "buffering"}), now);
    assert_eq!(sent(&s.step(Input::Command(Command::TogglePlay), now)), ["pause"], "buffering counts as playing");
}

/// Once Soloist has said what our play/pause asked for, the next toggle goes from what it says, not from the press
/// (2026-11-05: Space pressed fast as the app starts).
#[test]
fn a_toggle_goes_from_soloist_once_it_has_answered() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    assert_eq!(sent(&s.step(Input::Command(Command::Pause), now)), ["pause"]);
    ws(&mut s, json!({"type": "playback_changed", "status": "paused"}), now);
    // then something else (a phone, Spotify restoring a session) starts it: a toggle pauses
    ws(&mut s, json!({"type": "playback_changed", "status": "playing"}), now);
    assert_eq!(sent(&s.step(Input::Command(Command::TogglePlay), now)), ["pause"]);
    // a stale report that contradicts the press doesn't end it
    ws(&mut s, json!({"type": "playback_changed", "status": "playing"}), now);
    assert_eq!(sent(&s.step(Input::Command(Command::TogglePlay), now)), ["play"], "the press still stands");
}

/// B4 (2026-11-20 bug hunt): a skip kept the old track's position until Soloist's next `position_sync`, and a pause
/// snapped back to the last anchor. Then the `position_sync` and the late answer to our `get_queue` arrive.
#[test]
fn a_skip_starts_the_new_track_at_zero_then_follows_position_sync() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    let p = playback(&ws(&mut s, json!({"type": "playback_changed", "status": "paused"}), now + secs(20))).unwrap();
    assert_eq!(p.position_at((now + secs(25)).ms), 62_000, "paused at 1:02, where it was, not back at 0:42");
    ws(&mut s, json!({"type": "playback_changed", "status": "playing"}), now + secs(21));
    assert_eq!(sent(&s.step(Input::Command(Command::Next), now + secs(22))), ["skip_next"]);
    let a = ws(&mut s, track("spotify:track:b"), now + secs(23));
    assert_eq!(sent(&a), ["get_queue"], "the queue after the change is asked for");
    let p = playback(&a).unwrap();
    assert_eq!(p.item.as_ref().map(|i| i.uri.as_str()), Some("spotify:track:b"));
    assert_eq!(p.position_at((now + secs(23)).ms), 0, "the new track starts at its beginning");
    let sync = json!({"type": "position_sync", "position": {"position_ms": 1_000, "timestamp_ms": (now + secs(24)).ms, "speed": 1.0}});
    let p = playback(&ws(&mut s, sync, now + secs(24))).unwrap();
    assert_eq!(p.position_at((now + secs(26)).ms), 3_000);
    // the answer to our get_queue, a snapshot from before the skip: only its queue counts
    let mut answer = state("playing", "spotify:track:a", 64_000, now + secs(22));
    answer["upcoming"] = json!([{"uid": "u1", "source": "context", "item": item("spotify:track:c")}]);
    let p = playback(&ws(&mut s, answer, now + secs(25))).unwrap();
    assert_eq!(p.item.as_ref().map(|i| i.uri.as_str()), Some("spotify:track:b"), "the song that stopped isn't put back");
    assert_eq!(p.position_at((now + secs(26)).ms), 3_000);
    assert_eq!(p.queue.len(), 1);
    // and a pushed queue change still lands
    assert!(playback(&ws(&mut s, json!({"type": "queue_changed", "upcoming": []}), now + secs(26))).unwrap().queue.is_empty());
}

/// C9: the queue-answer window only takes a state that actually answers `get_queue` (one carrying the queue). A phone
/// pausing inside the window is real news: its status and position must land.
#[test]
fn a_state_without_the_queue_is_applied_normally_inside_the_window() {
    let now = t0();
    let mut s = session(Handover::default());
    assert_eq!(sent(&ws(&mut s, track("spotify:track:a"), now)), ["get_queue"]);
    let p = playback(&ws(&mut s, state("paused", "spotify:track:a", 30_000, now), now + secs(1))).unwrap();
    assert_eq!((p.status, p.position_ms), (PlayStatus::Paused, 30_000), "the pause was dropped");
    let mut answer = state("playing", "spotify:track:old", 1_000, now);
    answer["upcoming"] = json!([{"uid": "u1", "source": "context", "item": item("spotify:track:next")}]);
    let p = playback(&ws(&mut s, answer, now + secs(2))).unwrap();
    assert_eq!(p.status, PlayStatus::Paused, "an answer with the queue still leaves the state alone");
    assert_eq!(p.queue.len(), 1);
}

/// An answer that comes too late isn't taken for the answer: it's a full state.
#[test]
fn a_queue_answer_window_closes() {
    let now = t0();
    let mut s = session(Handover::default());
    ws(&mut s, track("spotify:track:a"), now);
    let mut late = state("paused", "spotify:track:z", 1_000, now + secs(6));
    late["upcoming"] = json!([]);
    let p = playback(&ws(&mut s, late, now + secs(6))).unwrap();
    assert_eq!(p.item.as_ref().map(|i| i.uri.as_str()), Some("spotify:track:z"));
}

/// An update staged while playing waits: not in the middle of a track, not over a change a phone made, only when our
/// own skip changes the track. The new track then starts over in the new build, once it has logged in.
#[test]
fn an_update_staged_during_play_restarts_at_our_skip_and_resumes() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    assert_eq!(stop(&s.step(Input::Staged, now + secs(1))), None, "never in the middle of a playing track");
    assert_eq!(stop(&ws(&mut s, track("spotify:track:b"), now + secs(2))), None, "a phone changed the track: follow it");
    s.step(Input::Command(Command::Next), now + secs(3));
    let a = ws(&mut s, track("spotify:track:c"), now + secs(4));
    assert_eq!(stop(&a), Some(Stop::Update), "between tracks, after our own skip");
    let h = s.into_handover();
    assert_eq!(h.resume, Some(Resume { context_uri: Some("spotify:album:x".into()), track_uri: "spotify:track:c".into(), position_ms: 0, paused: false }), "the track that just started starts over");

    // the new build
    let later = now + secs(10);
    let mut s = session(h);
    assert_eq!(s.deadline(), None);
    let a = ws(&mut s, json!({"type": "auth_state", "logged_in": true, "is_active": true}), later);
    assert_eq!(sent(&a), ["set_volume"]);
    assert_eq!(resumed(&a), None, "the new Soloist gets a moment to say what it plays");
    assert_eq!(s.deadline(), Some((later + Duration::from_millis(1500)).at));
    assert_eq!(resumed(&s.step(Input::Tick, later + secs(1))), None);
    let r = resumed(&s.step(Input::Tick, later + secs(2))).expect("picked up where it was");
    assert_eq!(r.track_uri, "spotify:track:c");
    assert_eq!(s.deadline(), None);
    assert!(s.into_handover().resume.is_none(), "once");
}

/// A play of ours names its track: someone else's track right after it isn't ours. And our skip counts once.
#[test]
fn only_our_own_commands_explain_a_track_change() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    // our skip, before the build was staged: used up by its own change
    s.step(Input::Command(Command::Next), now);
    ws(&mut s, track("spotify:track:b"), now + secs(1));
    s.step(Input::Staged, now + secs(2));
    assert_eq!(stop(&ws(&mut s, track("spotify:track:c"), now + secs(3))), None, "(once)");
    s.step(Input::Command(Command::Play { uri: Some("spotify:track:d".into()) }), now + secs(4));
    assert_eq!(stop(&ws(&mut s, track("spotify:track:e"), now + secs(5))), None, "someone else's track, even right after ours");
    s.step(Input::Command(Command::Play { uri: Some("spotify:track:f".into()) }), now + secs(6));
    assert_eq!(stop(&ws(&mut s, track("spotify:track:f"), now + secs(40))), None, "too late to be ours");
    s.step(Input::Command(Command::Play { uri: Some("spotify:track:g".into()) }), now + secs(41));
    assert_eq!(stop(&ws(&mut s, track("spotify:track:g"), now + secs(42))), Some(Stop::Update));
}

/// Paused or idle is a quiet moment at once: paused stays paused, at the same place; idle has nothing to pick up.
#[test]
fn an_update_staged_while_paused_or_idle_restarts_at_once() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    ws(&mut s, json!({"type": "playback_changed", "status": "paused"}), now + secs(3));
    assert_eq!(stop(&s.step(Input::Staged, now + secs(60))), Some(Stop::Update));
    let r = s.into_handover().resume.unwrap();
    assert_eq!((r.context_uri.as_deref(), r.track_uri.as_str(), r.position_ms, r.paused), (Some("spotify:album:x"), "spotify:track:a", 45_000, true));
    let mut s = session(Handover::default());
    assert_eq!(stop(&s.step(Input::Staged, now)), Some(Stop::Update));
    assert!(s.into_handover().resume.is_none(), "idle: nothing to pick up");
}

/// The new Soloist came back playing something (a phone put it there, or Spotify restored it): putting our remembered
/// state back would undo that.
#[test]
fn a_resume_is_dropped_when_the_player_comes_back_with_its_own_track() {
    let now = t0();
    let resume = Resume { context_uri: None, track_uri: "spotify:track:a".into(), position_ms: 9_000, paused: false };
    let mut s = session(Handover { resume: Some(resume), unstuck_at: None });
    ws(&mut s, json!({"type": "auth_state", "logged_in": true}), now);
    ws(&mut s, track("spotify:track:phone"), now + Duration::from_millis(500));
    assert_eq!(s.deadline(), None);
    assert_eq!(resumed(&s.step(Input::Tick, now + secs(2))), None);
}

/// A remembered resume waits for login: a run that dies before it keeps it for the next.
#[test]
fn a_resume_waits_for_login() {
    let now = t0();
    let resume = Resume { context_uri: None, track_uri: "spotify:track:a".into(), position_ms: 9_000, paused: false };
    let mut s = session(Handover { resume: Some(resume.clone()), unstuck_at: None });
    ws(&mut s, json!({"type": "auth_state", "logged_in": false}), now);
    assert_eq!(s.deadline(), None);
    assert_eq!(s.into_handover().resume, Some(resume));
}

/// 2026-11-02: Soloist sat "playing" at 0:09 for minutes with no data arriving. A dry spell of 20 s restarts it (once
/// per three minutes), picking up where it was.
#[test]
fn a_stream_dry_for_20_s_restarts_soloist_once() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    s.step(Input::Log(LogNews::Stall), now + secs(1));
    s.step(Input::Log(LogNews::Stall), now + secs(5));
    assert_eq!(s.deadline(), Some((now + secs(21)).at), "the spell counts from its first stall");
    s.step(Input::Log(LogNews::Bandwidth(900)), now + secs(6));
    assert_eq!(s.deadline(), None, "data arriving ends it");
    s.step(Input::Log(LogNews::Stall), now + secs(7));
    s.step(Input::Log(LogNews::Flowing), now + secs(8));
    assert_eq!(s.deadline(), None, "audio flowing again ends it");
    s.step(Input::Log(LogNews::Stall), now + secs(10));
    ws(&mut s, json!({"type": "playback_changed", "status": "buffering"}), now + secs(11));
    assert_eq!(stop(&s.step(Input::Tick, now + secs(29))), None, "not yet");
    let a = s.step(Input::Tick, now + secs(30));
    assert_eq!(stop(&a), Some(Stop::Restart), "buffering is still trying to play");
    assert_eq!(notices(&a).len(), 1);
    let h = s.into_handover();
    assert_eq!(h.resume.as_ref().map(|r| (r.track_uri.as_str(), r.paused)), Some(("spotify:track:a", false)));

    // the fresh Soloist is just as dry: it's the connection, not Soloist
    let mut s = session(h);
    ws(&mut s, json!({"type": "auth_state", "logged_in": true}), now + secs(32));
    ws(&mut s, state("playing", "spotify:track:a", 30_000, now + secs(33)), now + secs(33));
    s.step(Input::Log(LogNews::Stall), now + secs(34));
    let a = s.step(Input::Tick, now + secs(54));
    assert_eq!(stop(&a), None);
    assert!(notices(&a)[0].contains("still isn't sending audio"));
}

/// A pause ends a dry spell: a paused player isn't stuck.
#[test]
fn a_pause_ends_a_dry_spell() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    s.step(Input::Log(LogNews::Stall), now + secs(1));
    ws(&mut s, json!({"type": "playback_changed", "status": "paused"}), now + secs(2));
    assert_eq!(s.deadline(), None);
}

#[test]
fn restart_and_a_new_key_stop_soloist() {
    let now = t0();
    let mut s = playing("spotify:track:a", now);
    let a = s.step(Input::Command(Command::RestartEngine), now);
    assert_eq!((notices(&a), stop(&a)), (vec!["Restarting Soloist…".to_string()], Some(Stop::Restart)));
    let a = s.step(Input::Command(Command::SetSoloistKey { key: "k".into() }), now);
    assert!(matches!(&a[..], [Action::StoreKey(k), Action::Stop(Stop::Restart)] if k == "k"), "stored, then used: {a:?}");
}

/// C4: a rejected key (or a second Soloist on the same data) ends the run once the WebSocket is up as well, so the
/// Daemon says Failed and waits for a new key, instead of restarting into the same refusal every 2–60 s.
#[test]
fn a_rejected_key_or_a_locked_data_dir_stops_soloist_at_any_time() {
    let now = t0();
    let mut s = session(Handover::default());
    ws(&mut s, json!({"type": "auth_state", "logged_in": false}), now);
    assert_eq!(stop(&s.step(Input::Log(LogNews::BadKey), now)), Some(Stop::BadKey));
    let mut s = playing("spotify:track:a", now);
    assert_eq!(stop(&s.step(Input::Log(LogNews::Locked), now)), Some(Stop::Locked));
}

/// Soloist's verbose log, recorded in spike 1 (stream lines only): the first track Vorbis, then FLAC after the
/// device was set to Lossless.
#[test]
fn verbose_log_gives_lifetime_and_stream_format() {
    use singularityd::soloist::LogParser;
    let log = include_str!("fixtures/soloist_verbose.log");
    let mut p = LogParser::default();
    let news: Vec<LogNews> = log.lines().filter_map(|l| p.line(l)).collect();
    assert_eq!(news[0], LogNews::Lifetime(88));
    let streams: Vec<_> = news.iter().filter_map(|n| if let LogNews::Stream(s) = n { Some(s.clone()) } else { None }).collect();
    assert_eq!(streams.len(), 2);
    assert_eq!((streams[0].codec.as_str(), streams[0].bitrate, streams[0].lossless()), ("vorbis", 160_000, false));
    let flac = &streams[1];
    assert_eq!((flac.codec.as_str(), flac.bitrate, flac.average_bitrate, flac.sample_rate), ("flac", 1_400_000, Some(1_689_310), 44_100));
    assert_eq!(flac.sample_format, "32-bit-float-pcm");
    assert!(flac.lossless());
    assert_eq!(p.line("Error: another session is running for data directory \"/x\"."), Some(LogNews::Locked));
    assert_eq!(p.line("login failed: make sure --api-key is valid"), Some(LogNews::BadKey));
}

/// Lines from a real stall episode (2026-10-18, the connection saturated by another program): speed samples and
/// underruns. The recovery line ("updateInternalDriverToPlaying: underrun") isn't a new stall: audio flows again.
#[test]
fn verbose_log_gives_download_speed_and_stalls() {
    use singularityd::soloist::LogParser;
    let log = "16:55:27.737 I [f:1118                          ] FileStreamer bandwidth sample: 224.270345 kBps\x1b[0m
16:55:56.543 I [f:776                           ] audioThreadMain: underrun, _is_processing_audio now false\x1b[0m
16:56:23.708 I [f:1118                          ] FileStreamer bandwidth sample: 121.894370 kBps\x1b[0m
16:53:56.943 I [f:972                           ] updateInternalDriverToPlaying: underrun, _is_processing_audio now true\x1b[0m
16:59:55.638 I [f:1118                          ] FileStreamer bandwidth sample: 43.717687 kBps\x1b[0m";
    let mut p = LogParser::default();
    let news: Vec<LogNews> = log.lines().filter_map(|l| p.line(l)).collect();
    assert_eq!(news, vec![LogNews::Bandwidth(224), LogNews::Stall, LogNews::Bandwidth(122), LogNews::Flowing, LogNews::Bandwidth(44)]);
}
