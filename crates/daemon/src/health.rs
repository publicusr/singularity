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

/// The tools a Daemon on macOS (`macos`) or Linux needs for `kind`: its own, plus the app's launcher when the app is
/// the Engine (a Soloist or librespot setup has no use for `spotify`/`open`).
fn required_tools(macos: bool, kind: singularity_proto::EngineKind) -> Vec<crate::programs::Tool> {
    let mut tools = crate::programs::Tool::required(macos).to_vec();
    if kind == singularity_proto::EngineKind::SpotifyApp {
        tools.push(crate::programs::app_tool(macos));
    }
    tools
}

/// The tools this Daemon needs but can't find at their fixed locations (`pactl`, `secret-tool`…). The keyring's
/// client counts only while the keyring is enabled.
fn missing_tools(kind: singularity_proto::EngineKind) -> Vec<String> {
    missing_tools_with(singularity_config::host::MACOS, kind, |t| t.find())
}

/// [`missing_tools`] for macOS (`macos`) or Linux, looking in `dirs` instead (tests pass an empty scratch directory).
#[cfg(test)]
fn missing_tools_in(dirs: &[&str], macos: bool, kind: singularity_proto::EngineKind) -> Vec<String> {
    missing_tools_with(macos, kind, |t| t.find_in(dirs))
}

fn missing_tools_with(macos: bool, kind: singularity_proto::EngineKind, find: impl Fn(crate::programs::Tool) -> Option<std::path::PathBuf>) -> Vec<String> {
    let keyring = crate::secrets::keyring_enabled();
    required_tools(macos, kind)
        .into_iter()
        .filter(|t| *t != crate::programs::Tool::SecretTool || keyring)
        .filter(|t| find(*t).is_none())
        .map(|t| t.name().to_string())
        .collect()
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
    use std::process::Stdio;
    let pactl = |what: &str| -> serde_json::Value {
        let Some(exe) = crate::programs::Tool::Pactl.find() else { return serde_json::Value::default() };
        crate::programs::command(exe)
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
        engine_build: crate::update::Builds { root: crate::soloist::SoloistEngine::from_env().data_dir().to_path_buf() }.installed().and_then(|b| b.date()),
        missing_tools: missing_tools(kind),
        // filled in by the Daemon, which knows which of its tasks are running
        stopped_tasks: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tools_are_named() {
        use singularity_proto::EngineKind;
        let empty = tempfile::tempdir().unwrap();
        let dirs = [empty.path().to_str().unwrap()];
        let macos = singularity_config::host::MACOS;
        let missing = missing_tools_in(&dirs, macos, EngineKind::Soloist);
        for t in crate::programs::Tool::required(macos) {
            if *t == crate::programs::Tool::SecretTool && !crate::secrets::keyring_enabled() {
                continue;
            }
            assert!(missing.contains(&t.name().to_string()), "{} not named in {missing:?}", t.name());
        }
        // the app's launcher is named only for the app (a Soloist setup has no use for it)
        let app = crate::programs::app_tool(macos).name().to_string();
        assert!(!missing.contains(&app));
        let with_app = missing_tools_in(&dirs, macos, EngineKind::SpotifyApp);
        assert!(with_app.contains(&app), "{app} not named in {with_app:?}");
    }

    /// #98, Q102: Linux unpacks Soloist's builds with the `tar` crate, and nothing runs `pgrep`/`pkill` any more (the
    /// Host finds and stops processes); only the macOS VM still runs `tar`. Named here even with every program
    /// missing.
    #[test]
    fn only_the_programs_the_daemon_runs_are_asked_for() {
        use singularity_proto::EngineKind;
        let empty = tempfile::tempdir().unwrap();
        let dirs = [empty.path().to_str().unwrap()];
        let not_used = ["tar", "pgrep", "pkill"];
        for kind in [EngineKind::Soloist, EngineKind::Librespot, EngineKind::SpotifyApp] {
            let linux = missing_tools_in(&dirs, false, kind);
            assert!(linux.iter().all(|t| !not_used.contains(&t.as_str())), "Linux, {kind:?}: {linux:?}");
            assert!(linux.iter().any(|t| t == "pactl") && linux.iter().any(|t| t == "parec"), "Linux, {kind:?}: {linux:?}");
            let mac = missing_tools_in(&dirs, true, kind);
            assert!(mac.iter().any(|t| t == "tar"), "macOS, {kind:?}: {mac:?}");
            assert!(mac.iter().all(|t| t != "pgrep" && t != "pkill"), "macOS, {kind:?}: {mac:?}");
        }
        assert_eq!(missing_tools_in(&dirs, false, EngineKind::SpotifyApp).last().map(String::as_str), Some("spotify"));
        assert_eq!(missing_tools_in(&dirs, true, EngineKind::SpotifyApp), ["tar", "open"]);
    }

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
