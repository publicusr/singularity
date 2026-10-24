//! Adapter contract test: a recorded Soloist WebSocket transcript (spike 1, anonymised) maps to the
//! PlaybackState we promise Clients, and our commands map to Soloist's.

use serde_json::Value;
use singularity_proto::{Command, PlayStatus, RepeatMode};
use singularityd::soloist::SoloistState;

#[test]
fn transcript_maps_to_playback_state() {
    let mut st = SoloistState::default();
    for line in include_str!("fixtures/soloist_transcript.jsonl").lines() {
        let ev: Value = serde_json::from_str(line).unwrap();
        st.apply(&ev);
    }
    let p = &st.playback;
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
    let mut st = SoloistState::default();
    st.playback.status = PlayStatus::Playing;
    assert_eq!(st.to_soloist(Command::TogglePlay)[0]["command"], "pause");
    let rep = st.to_soloist(Command::SetRepeat { mode: RepeatMode::Track });
    assert_eq!(rep[0]["command"], "set_repeat_context");
    assert_eq!(rep[1]["command"], "set_repeat_track");
    assert_eq!(st.to_soloist(Command::SetVolume { percent: 150 })[0]["volume"], 100);
}

/// Soloist's verbose log, recorded in spike 1 (stream lines only): the first track Vorbis, then FLAC after the
/// device was set to Lossless.
#[test]
fn verbose_log_gives_lifetime_and_stream_format() {
    use singularityd::soloist::{LogNews, LogParser};
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
/// underruns. The recovery line ("updateInternalDriverToPlaying: underrun") isn't a new stall.
#[test]
fn verbose_log_gives_download_speed_and_stalls() {
    use singularityd::soloist::{LogNews, LogParser};
    let log = "16:55:27.737 I [f:1118                          ] FileStreamer bandwidth sample: 224.270345 kBps\x1b[0m
16:55:56.543 I [f:776                           ] audioThreadMain: underrun, _is_processing_audio now false\x1b[0m
16:56:23.708 I [f:1118                          ] FileStreamer bandwidth sample: 121.894370 kBps\x1b[0m
16:53:56.943 I [f:972                           ] updateInternalDriverToPlaying: underrun, _is_processing_audio now true\x1b[0m
16:59:55.638 I [f:1118                          ] FileStreamer bandwidth sample: 43.717687 kBps\x1b[0m";
    let mut p = LogParser::default();
    let news: Vec<LogNews> = log.lines().filter_map(|l| p.line(l)).collect();
    assert_eq!(news, vec![LogNews::Bandwidth(224), LogNews::Stall, LogNews::Bandwidth(122), LogNews::Bandwidth(44)]);
}

#[test]
fn updates_restart_soloist_only_at_quiet_moments() {
    use singularity_proto::{Item, PlayStatus, PlaybackState};
    use singularityd::soloist::{quiet_moment, resume_point};
    let mut p = PlaybackState { status: PlayStatus::Playing, item: Some(Item { uri: "spotify:track:a".into(), duration_ms: 200_000, ..Default::default() }), context_uri: Some("spotify:album:x".into()), position_ms: 42_000, timestamp_ms: singularity_proto::now_ms(), speed: 0.0, ..Default::default() };
    assert!(!quiet_moment(&p, false, false), "never in the middle of a playing track");
    assert!(quiet_moment(&p, true, true), "between tracks, when our own command made the change");
    assert!(!quiet_moment(&p, true, false), "but not when another app (a phone on Connect) changed the track");
    let r = resume_point(&p, true).unwrap();
    assert_eq!((r.track_uri.as_str(), r.position_ms, r.paused), ("spotify:track:a", 0, false), "a track that just started starts over");
    p.status = PlayStatus::Paused;
    assert!(quiet_moment(&p, false, false));
    let r = resume_point(&p, false).unwrap();
    assert_eq!((r.context_uri.as_deref(), r.position_ms, r.paused), (Some("spotify:album:x"), 42_000, true), "paused stays paused, at the same place");
    assert!(quiet_moment(&PlaybackState::default(), false, false) && resume_point(&PlaybackState::default(), false).is_none(), "idle: nothing to pick up");
}

/// A phone (or any other app on Connect) changing the track is news to follow: the Daemon must not take it for a
/// change of its own and restart Soloist over it (which would put its remembered state back).
#[test]
fn only_our_own_commands_explain_a_track_change() {
    use serde_json::json;
    use singularity_proto::Command;
    let track = |uri: &str| json!({"type": "track_changed", "item": {"uri": uri, "decorations": {"identity": {"name": uri}}}});
    let mut st = SoloistState::default();
    // a change nobody here asked for
    st.apply(&track("spotify:track:a"));
    assert!(!st.change_is_ours());
    // a next we sent: whatever the player picks is ours
    st.to_soloist(Command::Next);
    st.apply(&track("spotify:track:b"));
    assert!(st.change_is_ours());
    // (once)
    st.apply(&track("spotify:track:c"));
    assert!(!st.change_is_ours());
    // the track a play asked for is ours; another one isn't
    st.to_soloist(Command::Play { uri: Some("spotify:track:d".into()) });
    st.apply(&track("spotify:track:e"));
    assert!(!st.change_is_ours(), "someone else's track, even right after ours");
    st.to_soloist(Command::Play { uri: Some("spotify:track:f".into()) });
    st.apply(&track("spotify:track:f"));
    assert!(st.change_is_ours());
}

/// The answer to our `get_queue` is a snapshot of the player from before the change we just heard about: taking its
/// track would put the song that just stopped back (in the UI, MPRIS, Now Playing and the listening log).
#[test]
fn a_queue_answer_never_puts_the_previous_track_back() {
    use serde_json::json;
    let track = |uri: &str, name: &str| json!({"uri": uri, "decorations": {"identity": {"name": name}}});
    let mut st = SoloistState::default();
    st.apply(&json!({"type": "context_changed", "context": {"uri": "spotify:playlist:p"}}));
    st.apply(&json!({"type": "track_changed", "item": track("spotify:track:old", "Old")}));
    st.asking_for_queue();
    st.apply(&json!(
        {"type": "track_changed", "item": track("spotify:track:new", "New")}
    ));
    // the answer (the stand-in and the real Soloist send a playback state) still shows the old song
    let changed = st.apply(&json!({
        "type": "playback_state", "status": "playing", "item": track("spotify:track:old", "Old"),
        "context": {"uri": "spotify:playlist:p"}, "position": {"position_ms": 1000, "timestamp_ms": 1, "speed": 1.0},
        "volume": 40, "options": {}, "upcoming": [{"uid": "u1", "source": "context", "item": track("spotify:track:next", "Next")}]
    }));
    assert!(changed, "the queue is news");
    assert_eq!(st.playback.item.as_ref().map(|i| i.uri.as_str()), Some("spotify:track:new"), "the new song stays");
    assert_eq!(st.playback.queue.len(), 1);
    assert_eq!(st.playback.context_uri.as_deref(), Some("spotify:playlist:p"));
    // and a pushed queue change still lands
    assert!(st.apply(&json!({"type": "queue_changed", "upcoming": []})));
    assert!(st.playback.queue.is_empty());
}
