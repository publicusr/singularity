//! `singularityd` binary: parse flags, pick the Engine, serve the socket.

use singularityd::engine::{Engine, FakeEngine};
use singularityd::app_engine::AppEngine;
use singularityd::librespot::LibrespotEngine;
use singularityd::soloist::SoloistEngine;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("singularityd {}", singularityd::VERSION);
        return Ok(());
    }
    let path = singularityd::default_socket();
    let listener = match singularityd::bind(&path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            eprintln!("singularityd is already running ({})", path.display());
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    tracing::info!("singularityd {} listening on {}", singularityd::VERSION, path.display());
    // the Engine (D51, D52): --fake-engine, a pretend player for UI work without Soloist or a Spotify account; else
    // config.toml's [engine] (Soloist on Linux, librespot on macOS; any of them where it exists)
    let args: Vec<String> = std::env::args().collect();
    let fake = args.iter().any(|a| a == "--fake-engine");
    let choice = singularity_config::load_config(&singularity_config::config_path()).value.engine;
    // SINGULARITY_ENGINE=soloist|spotify-app|librespot overrides config.toml (tests pin it, whatever the OS)
    use singularity_config::EngineKind as Kind;
    let kind = match std::env::var("SINGULARITY_ENGINE").as_deref() {
        _ if fake => None,
        Ok("soloist") => Some(Kind::Soloist),
        Ok("spotify-app") => Some(Kind::SpotifyApp),
        Ok("librespot") => Some(Kind::Librespot),
        _ => Some(choice.kind.resolved()),
    };
    let app = (kind == Some(Kind::SpotifyApp)).then(|| AppEngine::new(&choice.device, None));
    let librespot = (kind == Some(Kind::Librespot)).then(|| LibrespotEngine::new(&choice.device, None, singularityd::catalogue::Dirs::from_env()));
    let identity = app.as_ref().map(|a| a.identity.clone()).or_else(|| librespot.as_ref().map(|l| l.identity.clone())).unwrap_or_default();
    // --no-catalogue: playback only (no Spotify login / private API)
    // --fake-catalogue <home.json>: a recorded Home feed instead of Spotify (UI work, screenshots)
    let fake_home = args.iter().position(|a| a == "--fake-catalogue").and_then(|i| args.get(i + 1));
    let catalogue = if let Some(path) = fake_home {
        Some(singularityd::catalogue::spawn_fake(serde_json::from_str(&std::fs::read_to_string(path)?)?))
    } else {
        (!args.iter().any(|a| a == "--no-catalogue")).then(|| singularityd::catalogue::spawn(singularityd::catalogue::Dirs::from_env(), identity.clone()))
    };
    // the AudioTap listens to the Engine's own audio: Soloist's PipeWire stream, librespot's samples in-process; the
    // fake Engine "plays" a synthetic groove (SINGULARITY_TAP=off: none, so a test Daemon never listens to the
    // owner's real player). (The Spotify app's audio: its PipeWire stream on Linux, a Core Audio tap on macOS, #67.)
    use singularityd::tap::{StreamOf, TapSource};
    let tap = match (&librespot, kind) {
        _ if fake => Some(TapSource::Synthetic),
        (Some(l), _) => Some(TapSource::Feed(l.feed.clone())),
        (None, Some(Kind::SpotifyApp)) if cfg!(target_os = "macos") => None,
        (None, Some(Kind::SpotifyApp)) => Some(TapSource::PipeWire(StreamOf::SpotifyApp)),
        _ => Some(TapSource::PipeWire(StreamOf::Soloist)),
    };
    let tap = tap.filter(|_| std::env::var("SINGULARITY_TAP").as_deref() != Ok("off"));
    let engine: Box<dyn Engine> = match (app, librespot) {
        _ if fake => Box::new(FakeEngine),
        (_, Some(l)) => {
            tracing::info!("Engine: librespot, {:?} on Connect", l.identity.device_name);
            Box::new(LibrespotEngine { catalogue: catalogue.as_ref().map(|(tx, _)| tx.clone()), ..l })
        }
        (Some(app), _) => {
            tracing::info!("Engine: the Spotify app, {:?} on Connect", app.identity.device_name);
            // SINGULARITY_START_APP=0: never look for or start the real app (tests)
            let start_app = std::env::var("SINGULARITY_START_APP").as_deref() != Ok("0");
            Box::new(AppEngine { catalogue: catalogue.as_ref().map(|(tx, _)| tx.clone()), start_app, ..app })
        }
        (None, None) => Box::new(SoloistEngine::from_env()),
    };
    // MPRIS on the session bus (SINGULARITY_DBUS=<address> picks another bus, =off none). A fake Engine stays off the
    // session bus unless asked, so test Daemons never show up in the desktop's media controls.
    let notify = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mpris = match std::env::var("SINGULARITY_DBUS").ok().filter(|v| !v.is_empty()) {
        Some(v) if v == "off" => None,
        Some(address) => Some(singularityd::mpris::Options { address: Some(address), notify: notify.clone() }),
        None if fake => None,
        None => Some(singularityd::mpris::Options { address: None, notify: notify.clone() }),
    };
    // the listening log and library backups under $XDG_DATA_HOME/singularity (SINGULARITY_DATA=<dir> elsewhere, =off
    // none). A fake Engine keeps none unless asked, so test Daemons never write into the owner's log.
    let store = match std::env::var("SINGULARITY_DATA").ok().filter(|v| !v.is_empty()) {
        Some(v) if v == "off" => None,
        Some(dir) => Some(std::path::PathBuf::from(dir)),
        None if fake => None,
        None => Some(singularityd::store::Store::default_dir()),
    };
    // config.toml: what the Daemon uses of it now, and again whenever it's saved
    tokio::spawn(follow_config(notify));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = tokio::select! {
        r = singularityd::serve_with(listener, engine, catalogue, tap, singularityd::Extras { mpris, store }) => r.map(Some),
        _ = tokio::signal::ctrl_c() => Ok(None),
        _ = term.recv() => Ok(None),
    };
    // dropping the runtime drops the Engine task, and with it Soloist (kill_on_drop + PDEATHSIG)
    let _ = std::fs::remove_file(&path);
    tracing::info!("singularityd stopped");
    if matches!(result, Ok(Some(singularityd::Served::Replaced))) {
        std::process::exit(75); // EX_TEMPFAIL
    }
    result.map(|_| ())
}

/// Apply `config.toml` (desktop notifications) and re-read it whenever it's saved.
async fn follow_config(notify: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let path = singularity_config::config_path();
    let mut watched = singularity_config::Watched::new(path.clone());
    let mut first = true;
    loop {
        if first || watched.changed() {
            first = false;
            let r = singularity_config::load_config(&path);
            for p in &r.problems {
                tracing::warn!("{p}");
            }
            notify.store(r.value.notifications.track_change, std::sync::atomic::Ordering::Relaxed);
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
