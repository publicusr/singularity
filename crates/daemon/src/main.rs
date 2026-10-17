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
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = tokio::select! {
        r = singularityd::serve(listener, engine) => r,
        _ = tokio::signal::ctrl_c() => Ok(()),
        _ = term.recv() => Ok(()),
    };
    // dropping the runtime drops the Engine task, and with it Soloist (kill_on_drop + PDEATHSIG)
    let _ = std::fs::remove_file(&path);
    tracing::info!("singularityd stopped");
    result
}
