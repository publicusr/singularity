//! The Daemon's part of the health view (#5): where the secrets live and where the Engine's audio goes.

use singularity_proto::Health;
use std::process::{Command, Stdio};

/// Where a secret lives, without reading it out.
fn location(account: &str, env: Option<&str>) -> String {
    if env.is_some_and(|v| std::env::var(v).is_ok_and(|k| !k.trim().is_empty())) {
        return format!("environment ({})", env.unwrap_or_default());
    }
    let in_keyring = std::env::var_os("SINGULARITY_NO_KEYRING").is_none() && Command::new("secret-tool")
        .args(["lookup", "service", "singularity", "account", account])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
    if in_keyring {
        return "keyring".into();
    }
    match crate::secrets::get(account) {
        Some(_) => "owner-only file (no keyring found)".into(),
        None => "missing".into(),
    }
}

/// The output device Soloist's stream plays to: `pactl` sink-inputs → its sink's description.
pub fn audio_output(sink_inputs: &serde_json::Value, sinks: &serde_json::Value) -> Option<String> {
    let idx = crate::tap::find_engine_stream(sink_inputs)?;
    let si = sink_inputs.as_array()?.iter().find(|s| s["index"].as_u64() == Some(idx))?;
    let sink = si["sink"].as_u64()?;
    let s = sinks.as_array()?.iter().find(|s| s["index"].as_u64() == Some(sink))?;
    s["description"].as_str().or(s["name"].as_str()).map(String::from)
}

pub fn report() -> Health {
    let pactl = |what: &str| -> serde_json::Value {
        Command::new("pactl")
            .args(["-f", "json", "list", what])
            .stderr(Stdio::null())
            .output()
            .ok()
            .and_then(|o| serde_json::from_slice(&o.stdout).ok())
            .unwrap_or_default()
    };
    Health {
        soloist_key: location("soloist", Some("SOLOIST_API_KEY")),
        spotify_login: location("spotify", None),
        audio_output: audio_output(&pactl("sink-inputs"), &pactl("sinks")),
        engine_expires_in_days: None,
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
        assert_eq!(audio_output(&inputs, &sinks).as_deref(), Some("DAC Pro"));
        assert_eq!(audio_output(&serde_json::json!([]), &sinks), None);
    }
}
