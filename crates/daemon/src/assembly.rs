//! Engine assembly (D51, D52, D55): what the Daemon plays with, decided once at start from config.toml's `[engine]`,
//! the environment and the command line.
//!
//! - [`assemble`] is pure: it picks the Engine, its Identity (the name on Connect), the tap's source and the Catalogue,
//!   and gathers the problems found on the way. The machine's facts come in as [`AssemblyFacts`], so both OSes'
//!   choices are tested on either.
//! - [`Assembly::build`] makes the chosen Engine and its tap source: the one Identity names the Engine, so the name in
//!   Connect and the name the Catalogue looks for can't disagree (audit C5), and an Engine that plays from this Daemon
//!   shares its `Feed` with the tap.

use crate::catalogue::{CatCmd, Dirs};
use crate::engine::{Engine, FakeEngine, Identity};
use crate::tap::{StreamOf, TapSource};
use singularity_config::{Config, EngineKind, Read};
use std::path::PathBuf;
use tokio::sync::mpsc;

/// What [`assemble`] needs to know about the machine the Daemon runs on: a few of the
/// [Host](singularity_config::host::Host)'s answers, and the installed app's version. Lookups run only when the choice
/// needs them.
pub struct AssemblyFacts<'a> {
    pub macos: bool,
    /// an OS and CPU Spotify ships Soloist for (Linux on x86_64 and arm64: D73; Apple Silicon Macs: Q188)
    pub soloist_cpu: bool,
    /// this computer's name (what the Spotify app registers under)
    pub computer: &'a dyn Fn() -> String,
    /// the installed Spotify app's version ("" if unknown)
    pub app_version: &'a dyn Fn() -> String,
}

impl AssemblyFacts<'static> {
    /// This machine.
    pub fn this() -> AssemblyFacts<'static> {
        AssemblyFacts {
            macos: singularity_config::host::MACOS,
            soloist_cpu: singularity_config::host::Host::this().soloist_cpu(),
            computer: &|| singularity_config::host::Host::this().computer(),
            app_version: &|| crate::app_engine::installed().map(|a| a.version).unwrap_or_default(),
        }
    }
}

/// Which Engine plays.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// `--fake-engine`: a pretend player for UI work and tests
    Fake,
    /// Soloist on this computer (Linux; Apple Silicon Macs, Q188); `cache_mb` is its `--cache-size` (0: no limit)
    Soloist { cache_mb: u32 },
    /// the official app, driven over Connect; `start_app`: find and start it (off in tests)
    SpotifyApp { start_app: bool },
    /// librespot inside the Daemon
    Librespot,
}

/// What the AudioTap listens to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tap {
    /// the fake Engine's synthetic groove
    Synthetic,
    /// the samples of an Engine that plays from this Daemon (librespot)
    EngineFeed,
    /// macOS: a Core Audio process tap on the Engine's process (the Spotify app, #67; Soloist, Q191)
    Process(StreamOf),
    /// Linux: the Engine's PipeWire stream
    PipeWire(StreamOf),
}

/// Where Home, Search and the library come from.
#[derive(Debug, Clone, PartialEq)]
pub enum CatalogueMode {
    /// Spotify, with the saved login
    Spotify,
    /// `--fake-catalogue <home.json>`: a recorded Home feed (a `fake` build)
    Recorded(PathBuf),
    /// `--no-catalogue`: playback only
    Off,
}

/// The Daemon's Engine, as decided.
#[derive(Debug, Clone)]
pub struct Assembly {
    pub plan: Plan,
    /// the Engine's name on Connect and how it's driven; the Catalogue gets the same one
    pub identity: Identity,
    /// None: no tap (`SINGULARITY_TAP=off`, so a test Daemon never listens to the owner's real player)
    pub tap: Option<Tap>,
    pub catalogue: CatalogueMode,
    /// config.toml's problems and the assembly's own, worded for the user
    pub problems: Vec<String>,
}

/// Decide the Engine: `--fake-engine`, else `SINGULARITY_ENGINE=soloist|spotify-app|librespot` (tests pin it,
/// whatever the OS), else config.toml's `[engine] kind` (`auto`: Soloist where Spotify ships it, Linux on x86_64 and
/// arm64 and Apple Silicon Macs; librespot elsewhere, an Intel Mac included). `env` reads an environment variable.
pub fn assemble(config: Read<Config>, args: &[String], env: impl Fn(&str) -> Option<String>, facts: &AssemblyFacts) -> Assembly {
    let mut problems = config.problems;
    let choice = config.value.engine;
    let flag = |f: &str| args.iter().any(|a| a == f);
    let kind = match env("SINGULARITY_ENGINE").as_deref() {
        _ if flag("--fake-engine") => None,
        Some("soloist") => Some(EngineKind::Soloist),
        Some("spotify-app") => Some(EngineKind::SpotifyApp),
        Some("librespot") => Some(EngineKind::Librespot),
        other => {
            if let Some(v) = other.filter(|v| !v.is_empty()) {
                problems.push(format!("SINGULARITY_ENGINE={v} isn't known (soloist, spotify-app or librespot); using config.toml's"));
            }
            Some(choice.kind.resolved_on(facts.soloist_cpu))
        }
    };
    let device = choice.device.trim();
    let named = |default: &dyn Fn() -> String| if device.is_empty() { default() } else { device.to_string() };
    let (plan, identity, tap) = match kind {
        None => (Plan::Fake, Identity::default(), Tap::Synthetic),
        Some(EngineKind::SpotifyApp) => {
            let identity = Identity { device_name: named(facts.computer), via_connect: true, version: (facts.app_version)(), player: "the Spotify app" };
            let tap = if facts.macos { Tap::Process(StreamOf::SpotifyApp) } else { Tap::PipeWire(StreamOf::SpotifyApp) };
            (Plan::SpotifyApp { start_app: env("SINGULARITY_START_APP").as_deref() != Some("0") }, identity, tap)
        }
        Some(EngineKind::Librespot) => {
            let identity = Identity { device_name: named(&|| format!("Singularity on {}", (facts.computer)())), via_connect: true, version: String::new(), player: "librespot" };
            (Plan::Librespot, identity, Tap::EngineFeed)
        }
        // Soloist (Auto is resolved above) on this computer: on macOS Spotify's Mac build (Q188), its sound tapped where
        // it goes, in Core Audio (Q191). A Mac's default name sets it apart from a Linux machine's "Singularity" and
        // this Mac's librespot, which can linger in the device list after a switch, as devices are found by name.
        Some(_) => {
            let device_name = if facts.macos { named(&|| format!("Singularity on {} (lossless)", (facts.computer)())) } else { named(&|| crate::soloist::DEVICE_NAME.into()) };
            let tap = if facts.macos { Tap::Process(StreamOf::Soloist) } else { Tap::PipeWire(StreamOf::Soloist) };
            (Plan::Soloist { cache_mb: choice.soloist_cache_mb() }, Identity { device_name, ..Identity::default() }, tap)
        }
    };
    let catalogue = match args.iter().position(|a| a == "--fake-catalogue").and_then(|i| args.get(i + 1)) {
        Some(path) => CatalogueMode::Recorded(PathBuf::from(path)),
        None if flag("--no-catalogue") => CatalogueMode::Off,
        None => CatalogueMode::Spotify,
    };
    let tap = (env("SINGULARITY_TAP").as_deref() != Some("off")).then_some(tap);
    Assembly { plan, identity, tap, catalogue, problems }
}

impl Assembly {
    pub fn fake(&self) -> bool {
        self.plan == Plan::Fake
    }

    /// Make the Engine (commands go to `catalogue`, if there is one) and the tap's source. Only for an assembly made
    /// with [`AssemblyFacts::this`]: off macOS there is no process tap (none).
    pub fn build(&self, catalogue: Option<mpsc::Sender<CatCmd>>) -> (Box<dyn Engine>, Option<TapSource>) {
        let identity = self.identity.clone();
        // the samples of an Engine that plays from this Daemon, for the tap
        let mut feed = None;
        let engine: Box<dyn Engine> = match self.plan {
            Plan::Fake => Box::new(FakeEngine),
            Plan::Librespot => {
                tracing::info!("Engine: librespot, {:?} on Connect", identity.device_name);
                let f = feed.insert(crate::tap::Feed::new()).clone();
                Box::new(crate::librespot::LibrespotEngine { identity, catalogue, dirs: Dirs::from_env(), feed: f })
            }
            Plan::SpotifyApp { start_app } => {
                tracing::info!("Engine: the Spotify app, {:?} on Connect", identity.device_name);
                Box::new(crate::app_engine::AppEngine { identity, catalogue, start_app })
            }
            Plan::Soloist { cache_mb } => {
                Box::new(crate::soloist::SoloistEngine::local(identity.device_name, cache_mb))
            }
        };
        let tap = self.tap.and_then(|t| match t {
            Tap::Synthetic => Some(TapSource::Synthetic),
            Tap::EngineFeed => feed.map(TapSource::Feed),
            #[cfg(target_os = "macos")]
            Tap::Process(of) => Some(TapSource::Process(of, crate::tap::Feed::new())),
            #[cfg(not(target_os = "macos"))]
            Tap::Process(_) => None,
            Tap::PipeWire(of) => Some(TapSource::PipeWire(of)),
        });
        (engine, tap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_config::{CONFIG_MIGRATIONS, CONFIG_SCHEMA};

    fn linux() -> AssemblyFacts<'static> {
        AssemblyFacts { macos: false, soloist_cpu: true, computer: &|| "desk".into(), app_version: &|| "1.2.3".into() }
    }

    fn mac() -> AssemblyFacts<'static> {
        AssemblyFacts { macos: true, ..linux() }
    }

    /// config.toml with this `[engine]` (and anything else after it).
    fn config(engine: &str) -> Read<Config> {
        singularity_config::parse("config.toml", &format!("schema = 7\n[engine]\n{engine}\n"), CONFIG_MIGRATIONS, CONFIG_SCHEMA)
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn with<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn soloist_on_linux_is_named_singularity_and_tapped_through_pipewire() {
        let a = assemble(config("kind = \"auto\""), &[], no_env, &linux());
        assert_eq!(a.plan, Plan::Soloist { cache_mb: 5120 });
        assert_eq!((a.identity.device_name.as_str(), a.identity.via_connect, a.identity.player), ("Singularity", false, "Soloist"));
        assert_eq!((a.tap, a.catalogue, a.problems.len()), (Some(Tap::PipeWire(StreamOf::Soloist)), CatalogueMode::Spotify, 0));
    }

    /// C5: `[engine] device` names Soloist on Linux too, and an empty setting keeps "Singularity".
    #[test]
    fn the_configured_device_names_soloist() {
        let name = |d: &str| assemble(config(&format!("kind = \"soloist\"\ndevice = \"{d}\"")), &[], no_env, &linux()).identity.device_name;
        assert_eq!((name("Kitchen"), name("  Kitchen  "), name(""), name("   ")), ("Kitchen".into(), "Kitchen".into(), "Singularity".into(), "Singularity".into()));
    }

    /// Q188, Q190, Q191: Soloist on an Apple Silicon Mac, `auto` too, runs here and is tapped in Core Audio.
    #[test]
    fn soloist_on_macos_runs_here_and_is_tapped_in_core_audio() {
        for kind in ["soloist", "auto"] {
            let a = assemble(config(&format!("kind = \"{kind}\"")), &[], no_env, &mac());
            assert_eq!((a.plan, a.tap), (Plan::Soloist { cache_mb: 5120 }, Some(Tap::Process(StreamOf::Soloist))), "{kind}");
            assert_eq!(a.identity.device_name, "Singularity on desk (lossless)");
        }
        let named = assemble(config("kind = \"soloist\"\ndevice = \"Den\""), &[], no_env, &mac());
        assert_eq!(named.identity.device_name, "Den");
    }

    #[test]
    fn auto_is_librespot_where_soloist_has_no_build() {
        for facts in [AssemblyFacts { soloist_cpu: false, ..mac() }, AssemblyFacts { soloist_cpu: false, ..linux() }] {
            let a = assemble(config("kind = \"auto\""), &[], no_env, &facts);
            assert_eq!((a.plan, a.tap), (Plan::Librespot, Some(Tap::EngineFeed)));
            assert_eq!((a.identity.device_name.as_str(), a.identity.via_connect, a.identity.player), ("Singularity on desk", true, "librespot"));
        }
    }

    #[test]
    fn the_spotify_app_goes_by_this_computers_name_and_is_tapped_per_os() {
        let a = assemble(config("kind = \"spotify-app\""), &[], no_env, &linux());
        assert_eq!((a.plan, a.tap), (Plan::SpotifyApp { start_app: true }, Some(Tap::PipeWire(StreamOf::SpotifyApp))));
        assert_eq!((a.identity.device_name.as_str(), a.identity.version.as_str(), a.identity.player), ("desk", "1.2.3", "the Spotify app"));
        let m = assemble(config("kind = \"spotify-app\"\ndevice = \"Den\""), &[], with(&[("SINGULARITY_START_APP", "0")]), &mac());
        assert_eq!((m.plan, m.tap, m.identity.device_name.as_str()), (Plan::SpotifyApp { start_app: false }, Some(Tap::Process(StreamOf::SpotifyApp)), "Den"));
    }

    #[test]
    fn the_fake_engine_wins_over_everything_and_plays_a_synthetic_groove() {
        let a = assemble(config("kind = \"librespot\""), &["--fake-engine".into()], with(&[("SINGULARITY_ENGINE", "spotify-app")]), &linux());
        assert!(a.fake());
        assert_eq!((a.plan, a.tap, a.identity.device_name.as_str()), (Plan::Fake, Some(Tap::Synthetic), "Singularity"));
    }

    #[test]
    fn the_environment_pins_the_engine_and_a_bad_pin_is_reported() {
        let a = assemble(config("kind = \"soloist\""), &[], with(&[("SINGULARITY_ENGINE", "librespot")]), &linux());
        assert_eq!(a.plan, Plan::Librespot);
        let bad = assemble(config("kind = \"soloist\""), &[], with(&[("SINGULARITY_ENGINE", "vlc")]), &linux());
        assert_eq!(bad.plan, Plan::Soloist { cache_mb: 5120 });
        assert_eq!(bad.problems, ["SINGULARITY_ENGINE=vlc isn't known (soloist, spotify-app or librespot); using config.toml's"]);
        let off = assemble(config("kind = \"librespot\""), &[], with(&[("SINGULARITY_TAP", "off")]), &linux());
        assert_eq!(off.tap, None);
    }

    #[test]
    fn config_problems_reach_the_caller_and_a_typo_elsewhere_keeps_the_engine() {
        let r = singularity_config::parse("config.toml", "schema = 7\n[ui]\ntransparent = \"yes\"\n[engine]\nkind = \"librespot\"\n", CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        let a = assemble(r, &[], no_env, &linux());
        assert_eq!(a.plan, Plan::Librespot);
        assert!(a.problems.len() == 1 && a.problems[0].starts_with("config.toml: ui.transparent = \"yes\""), "{:?}", a.problems);
        let typo = assemble(config("kind = \"x\""), &[], no_env, &linux());
        assert_eq!(typo.problems, ["config.toml: engine.kind = \"x\" isn't known; using auto"]);
    }

    #[test]
    fn the_command_line_picks_the_catalogue() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let cat = |a: &[&str]| assemble(config(""), &args(a), no_env, &linux()).catalogue;
        assert_eq!(cat(&["--no-catalogue"]), CatalogueMode::Off);
        assert_eq!(cat(&["--fake-catalogue", "home.json"]), CatalogueMode::Recorded("home.json".into()));
        assert_eq!(cat(&[]), CatalogueMode::Spotify);
    }

    #[tokio::test]
    async fn an_engine_that_plays_here_shares_its_samples_with_the_tap() {
        let a = assemble(config("kind = \"librespot\""), &[], no_env, &linux());
        let (engine, tap) = a.build(None);
        assert_eq!(engine.kind(), singularity_proto::EngineKind::Librespot);
        assert!(matches!(tap, Some(TapSource::Feed(_))));
        let (engine, tap) = assemble(config("kind = \"soloist\""), &[], no_env, &linux()).build(None);
        assert_eq!(engine.kind(), singularity_proto::EngineKind::Soloist);
        assert!(matches!(tap, Some(TapSource::PipeWire(StreamOf::Soloist))));
    }
}
