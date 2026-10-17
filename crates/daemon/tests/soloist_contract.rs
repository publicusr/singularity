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
