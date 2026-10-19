//! The Daemon's part of the health view (#5): where the secrets live and where the Engine's audio goes.

use singularity_proto::Health;

/// Where a secret lives, without reading it out.
fn location(account: &str, env: Option<&str>) -> String {
    if env.is_some_and(|v| std::env::var(v).is_ok_and(|k| !k.trim().is_empty())) {
        return format!("environment ({})", env.unwrap_or_default());
    }
    // "keyring" means the OS's own store on either platform (the Client names it)
    if crate::secrets::in_keyring(account).is_some() {
        return "keyring".into();
    }
    match crate::secrets::get(account) {
        Some(_) => format!("owner-only file (no {} found)", crate::secrets::KEYRING),
        None => "missing".into(),
    }
}

/// The output device the Engine's stream plays to: `pactl` sink-inputs → its sink's description.
pub fn audio_output(sink_inputs: &serde_json::Value, sinks: &serde_json::Value, of: crate::tap::StreamOf) -> Option<String> {
    let idx = crate::tap::find_engine_stream(sink_inputs, of)?;
    let si = sink_inputs.as_array()?.iter().find(|s| s["index"].as_u64() == Some(idx))?;
    let sink = si["sink"].as_u64()?;
    let s = sinks.as_array()?.iter().find(|s| s["index"].as_u64() == Some(sink))?;
    s["description"].as_str().or(s["name"].as_str()).map(String::from)
}

/// Where this Engine's audio goes. Linux: the sink its PipeWire stream plays to.
#[cfg(not(target_os = "macos"))]
fn engine_output(kind: singularity_proto::EngineKind) -> Option<String> {
    use crate::tap::StreamOf;
    use std::process::{Command, Stdio};
    let pactl = |what: &str| -> serde_json::Value {
        Command::new("pactl")
            .args(["-f", "json", "list", what])
            .stderr(Stdio::null())
            .output()
            .ok()
            .and_then(|o| serde_json::from_slice(&o.stdout).ok())
            .unwrap_or_default()
    };
    let of = match kind {
        singularity_proto::EngineKind::SpotifyApp => StreamOf::SpotifyApp,
        singularity_proto::EngineKind::Librespot => StreamOf::Daemon,
        _ => StreamOf::Soloist,
    };
    audio_output(&pactl("sink-inputs"), &pactl("sinks"), of)
}

/// Where this Engine's audio goes. macOS: the system output device (librespot plays to it, and so does the Spotify
/// app unless it was pointed elsewhere in its own settings).
#[cfg(target_os = "macos")]
fn engine_output(_: singularity_proto::EngineKind) -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    #[allow(deprecated)] // `description()` is 0.17's; 0.16 has `name()`
    cpal::default_host().default_output_device()?.name().ok()
}

pub fn report(kind: singularity_proto::EngineKind) -> Health {
    Health {
        soloist_key: location("soloist", Some("SOLOIST_API_KEY")),
        spotify_login: location("spotify", None),
        audio_output: engine_output(kind),
        engine_expires_in_days: None,
        engine_build: crate::update::Builds { root: crate::soloist::SoloistEngine::from_env().data_dir }.installed().and_then(|b| b.date()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_sink_soloist_plays_to() {
        let inputs = serde_json::json!([
            {"index": 3, "sink": 1, "properties": {"application.name": "mpv"}},
            {"index": 9, "sink": 2, "properties": {"application.name": "Spotify"}}
        ]);
        let sinks = serde_json::json!([{"index": 1, "description": "Speakers"}, {"index": 2, "description": "DAC Pro"}]);
        assert_eq!(audio_output(&inputs, &sinks, crate::tap::StreamOf::Soloist).as_deref(), Some("DAC Pro"));
        assert_eq!(audio_output(&serde_json::json!([]), &sinks, crate::tap::StreamOf::Soloist), None);
    }
}
