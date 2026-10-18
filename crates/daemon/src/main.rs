//! `singularityd` binary: parse flags, pick the Engine, serve the socket.

use singularityd::engine::{Engine, FakeEngine};
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
    // --fake-engine: a pretend player, handy for UI work without Soloist or a Spotify account
    let engine: Box<dyn Engine> = if std::env::args().any(|a| a == "--fake-engine") { Box::new(FakeEngine) } else { Box::new(SoloistEngine::from_env()) };
    // --no-catalogue: playback only (no Spotify login / private API)
    // --fake-catalogue <home.json>: a recorded Home feed instead of Spotify (UI work, screenshots)
    let args: Vec<String> = std::env::args().collect();
    let fake_home = args.iter().position(|a| a == "--fake-catalogue").and_then(|i| args.get(i + 1));
    let catalogue = if let Some(path) = fake_home {
        Some(singularityd::catalogue::spawn_fake(serde_json::from_str(&std::fs::read_to_string(path)?)?))
    } else {
        (!args.iter().any(|a| a == "--no-catalogue")).then(|| singularityd::catalogue::spawn(singularityd::catalogue::Dirs::from_env()))
    };
    // the AudioTap listens to Soloist's own stream; the fake Engine "plays" a synthetic groove
    // (SINGULARITY_TAP=off: none, so a test Daemon never listens to the owner's real Soloist)
    let tap = if args.iter().any(|a| a == "--fake-engine") { singularityd::tap::TapSource::Synthetic } else { singularityd::tap::TapSource::PipeWire };
    let tap = (std::env::var("SINGULARITY_TAP").as_deref() != Ok("off")).then_some(tap);
    // MPRIS on the session bus (SINGULARITY_DBUS=<address> picks another bus, =off none). A fake Engine stays off the
    // session bus unless asked, so test Daemons never show up in the desktop's media controls.
    let fake = args.iter().any(|a| a == "--fake-engine");
    let notify = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mpris = match std::env::var("SINGULARITY_DBUS").ok().filter(|v| !v.is_empty()) {
        Some(v) if v == "off" => None,
        Some(address) => Some(singularityd::mpris::Options { address: Some(address), notify: notify.clone() }),
        None if fake => None,
        None => Some(singularityd::mpris::Options { address: None, notify: notify.clone() }),
    };
    // config.toml: what the Daemon uses of it now, and again whenever it's saved
    tokio::spawn(follow_config(notify));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = tokio::select! {
        r = singularityd::serve_with(listener, engine, catalogue, tap, mpris) => r.map(Some),
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
