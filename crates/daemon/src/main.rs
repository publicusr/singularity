//! `singularityd` binary: parse flags, pick the Engine, serve the socket.

use singularityd::assembly::{AssemblyFacts, CatalogueMode, Plan, assemble};

/// The log names the account and what plays: whoever opened it (a Client, launchd, a shell), it's owner-only.
fn private_log() {
    // SAFETY: fstat fills a zeroed stat for fd 2; fchmod only changes a regular file's mode
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstat(2, &mut st) == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFREG {
            libc::fchmod(2, 0o600);
        }
    }
}

fn main() {
    // OS media integration may need the main thread (macOS's run loop, #68): the Daemon runs where it says
    singularityd::os_media::main_thread(daemon);
}

/// Run the Daemon; its exit code.
fn daemon() -> i32 {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a tokio runtime");
    let code = match rt.block_on(run()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e:?}");
            1
        }
    };
    // dropping the runtime drops the Engine task, and with it Soloist (kill_on_drop + PDEATHSIG)
    rt.shutdown_timeout(std::time::Duration::from_secs(2));
    code
}

async fn run() -> anyhow::Result<i32> {
    private_log();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("singularityd {}", singularityd::VERSION);
        return Ok(0);
    }
    // nowhere to keep settings and data without HOME: stop and say so (Q93), before taking the socket
    let folders = match singularity_config::app_folders::AppFolders::from_env() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("singularityd: {e}");
            return Ok(1);
        }
    };
    let path = singularityd::default_socket();
    let listener = match singularityd::bind(&path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            eprintln!("singularityd is already running ({})", path.display());
            return Ok(0);
        }
        Err(e) => return Err(e.into()),
    };
    // this is the Daemon now: the one before's log is kept aside, and this one's can't grow for ever (Q170)
    let log = folders.state.join(singularityd::daemon_log::NAME);
    if singularityd::daemon_log::take_over(2, &log) {
        tokio::spawn(singularityd::daemon_log::watch(2, log));
    }
    tracing::info!("singularityd {} listening on {}", singularityd::VERSION, path.display());
    // the Engine (D51, D52, D55), its tap and the Catalogue, from config.toml's [engine], the environment and the flags
    // (--fake-engine, --no-catalogue, --fake-catalogue <home.json>): see `assembly`
    let args: Vec<String> = std::env::args().collect();
    let config = singularity_config::load_config(&singularity_config::config_path());
    let assembly = assemble(config, &args, |k| std::env::var(k).ok(), &AssemblyFacts::this());
    // the Client reads the same config.toml and shows its problems (a toast, the Settings page); here they're logged
    for p in &assembly.problems {
        tracing::warn!("{p}");
    }
    let fake = assembly.fake();
    let catalogue = match &assembly.catalogue {
        CatalogueMode::Recorded(path) => Some(fake_catalogue(path)?),
        CatalogueMode::Spotify => Some(singularityd::catalogue::spawn(singularityd::catalogue::Dirs::from_env(), assembly.identity.clone())),
        CatalogueMode::Off => None,
    };
    let (engine, tap) = assembly.build(catalogue.as_ref().map(|(tx, _)| tx.clone()));
    // OS media integration (MPRIS, Now Playing): SINGULARITY_NOWPLAYING=off|on; a fake Engine only when asked, so test
    // Daemons never show up in the desktop's media controls
    let notify = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let os_media = singularityd::os_media::Options::wanted(|k| std::env::var(k).ok(), fake, matches!(assembly.plan, Plan::SpotifyApp { .. }), notify.clone());
    // the listening log and library backups under $XDG_DATA_HOME/singularity (SINGULARITY_DATA=<dir> elsewhere, =off
    // none). A fake Engine keeps none unless asked, so test Daemons never write into the owner's log.
    let store = match std::env::var("SINGULARITY_DATA").ok().filter(|v| !v.is_empty()) {
        Some(v) if v == "off" => None,
        Some(dir) => Some(std::path::PathBuf::from(dir)),
        None if fake => None,
        None => Some(singularityd::store::Store::default_dir()),
    };
    // the last Home and library for the next start (#81), in the user's cache dir: with Spotify's Catalogue, unless
    // SINGULARITY_HOME_CACHE=off; =on keeps one for a fake Catalogue too (in a test's scratch dirs)
    let last_seen = match std::env::var("SINGULARITY_HOME_CACHE").ok().as_deref() {
        Some("off") => false,
        Some("on") => true,
        _ => matches!(assembly.catalogue, CatalogueMode::Spotify),
    };
    let last_seen = last_seen.then(singularityd::last_seen::LastSeen::default_path);
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = tokio::select! {
        // `notify`: the Daemon's config follower keeps notifications.track_change current (A1: supervised like the rest)
        r = singularityd::serve_with(listener, engine, catalogue, tap, singularityd::Extras { os_media, store, notify: Some(notify), last_seen }) => r.map(Some),
        _ = tokio::signal::ctrl_c() => Ok(None),
        _ = term.recv() => Ok(None),
    };
    let _ = std::fs::remove_file(&path);
    tracing::info!("singularityd stopped");
    // replaced by a newer build: EX_TEMPFAIL
    result.map(|r| if matches!(r, Some(singularityd::Served::Replaced)) { 75 } else { 0 })
}

#[cfg(feature = "fake")]
fn fake_catalogue(path: &std::path::Path) -> anyhow::Result<(tokio::sync::mpsc::Sender<singularityd::catalogue::CatCmd>, tokio::sync::mpsc::Receiver<singularityd::catalogue::CatEvent>)> {
    Ok(singularityd::catalogue::spawn_fake(serde_json::from_str(&std::fs::read_to_string(path)?)?))
}

#[cfg(not(feature = "fake"))]
fn fake_catalogue(_: &std::path::Path) -> anyhow::Result<(tokio::sync::mpsc::Sender<singularityd::catalogue::CatCmd>, tokio::sync::mpsc::Receiver<singularityd::catalogue::CatEvent>)> {
    anyhow::bail!("--fake-catalogue is only in builds with the `fake` feature: cargo build -p singularityd --features fake")
}
