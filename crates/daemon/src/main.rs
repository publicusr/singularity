//! `singularityd` binary: parse flags, pick the Engine, serve the socket.

use singularityd::engine::{Engine, FakeEngine};
use singularityd::app_engine::AppEngine;
use singularityd::librespot::LibrespotEngine;
use singularityd::soloist::SoloistEngine;

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
    // macOS delivers media-key and Control Centre commands on the main thread's run loop (#68): the Daemon runs on
    // another thread and the main thread serves those, until the Daemon ends the process
    #[cfg(target_os = "macos")]
    {
        std::thread::spawn(|| std::process::exit(daemon()));
        singularityd::nowplaying::main_thread();
    }
    #[cfg(not(target_os = "macos"))]
    std::process::exit(daemon());
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
    let path = singularityd::default_socket();
    let listener = match singularityd::bind(&path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            eprintln!("singularityd is already running ({})", path.display());
            return Ok(0);
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
    // macOS: Soloist runs in the Soloist VM (D55); its audio comes to the Daemon, which plays it and taps it
    #[cfg(target_os = "macos")]
    // (SINGULARITY_SOLOIST=local: a local stand-in instead, as the pty harness's tests use on any OS)
    let soloist_vm = (kind == Some(Kind::Soloist) && std::env::var("SINGULARITY_SOLOIST").as_deref() != Ok("local")).then(|| {
        let feed = singularityd::tap::Feed::new();
        (SoloistEngine::in_vm(&choice.device, feed.clone()), feed)
    });
    #[cfg(not(target_os = "macos"))]
    let soloist_vm: Option<(SoloistEngine, std::sync::Arc<singularityd::tap::Feed>)> = None;
    // `[engine] device` names Soloist too, on Linux as in the VM (audit C5): the same name in Connect and in the identity
    let local_soloist = SoloistEngine { cache_mb: choice.soloist_cache_mb(), device_name: SoloistEngine::named(&choice.device), ..SoloistEngine::from_env() };
    let soloist_identity = soloist_vm.as_ref().map(|(s, _)| s.device_name.clone()).unwrap_or_else(|| local_soloist.device_name.clone());
    let identity = app.as_ref().map(|a| a.identity.clone()).or_else(|| librespot.as_ref().map(|l| l.identity.clone())).or_else(|| (kind == Some(Kind::Soloist)).then(|| singularityd::engine::Identity { device_name: soloist_identity, ..Default::default() })).unwrap_or_default();
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
    // owner's real player). The Spotify app's audio: its PipeWire stream on Linux, a Core Audio process tap on macOS (#67).
    use singularityd::tap::{StreamOf, TapSource};
    let tap = match (&librespot, kind) {
        _ if fake => Some(TapSource::Synthetic),
        (Some(l), _) => Some(TapSource::Feed(l.feed.clone())),
        (None, _) if soloist_vm.is_some() => soloist_vm.as_ref().map(|(_, f)| TapSource::Feed(f.clone())),
        #[cfg(target_os = "macos")]
        (None, Some(Kind::SpotifyApp)) => Some(TapSource::App(singularityd::tap::Feed::new())),
        #[cfg(not(target_os = "macos"))]
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
        (None, None) => match soloist_vm {
            Some((s, _)) => {
                tracing::info!("Engine: Soloist in the Soloist VM, {:?} on Connect", s.device_name);
                Box::new(s)
            }
            None => Box::new(local_soloist),
        },
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
    // macOS: Now Playing, media keys and notifications (#68), for an Engine that plays from the Daemon (the Spotify app
    // has its own Now Playing entry). SINGULARITY_NOWPLAYING=off: none; a fake Engine only with =on (tests, pty).
    let now_playing = match std::env::var("SINGULARITY_NOWPLAYING").as_deref() {
        _ if !cfg!(target_os = "macos") => None,
        Ok("off") => None,
        Ok("on") => Some(singularityd::nowplaying::Options { notify: notify.clone() }),
        _ if fake || kind == Some(Kind::SpotifyApp) => None,
        _ => Some(singularityd::nowplaying::Options { notify: notify.clone() }),
    };
    // MPRIS is D-Bus: Linux's (a macOS session has no session bus)
    let mpris = mpris.filter(|_| !cfg!(target_os = "macos"));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = tokio::select! {
        // `notify`: the Daemon's config follower keeps notifications.track_change current (A1: supervised like the rest)
        r = singularityd::serve_with(listener, engine, catalogue, tap, singularityd::Extras { mpris, now_playing, store, notify: Some(notify) }) => r.map(Some),
        _ = tokio::signal::ctrl_c() => Ok(None),
        _ = term.recv() => Ok(None),
    };
    let _ = std::fs::remove_file(&path);
    tracing::info!("singularityd stopped");
    // replaced by a newer build: EX_TEMPFAIL
    result.map(|r| if matches!(r, Some(singularityd::Served::Replaced)) { 75 } else { 0 })
}
