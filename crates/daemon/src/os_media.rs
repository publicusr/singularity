//! OS media integration (D89): what's playing in the desktop's media controls, media keys driving the Daemon, and a
//! notification on each new track. One module the Daemon starts with the Hub; each OS supplies only its adapter:
//!
//! - Linux: [`mpris`] on D-Bus (KDE's media widget, `playerctl`, media keys) and notifications through
//!   `org.freedesktop.Notifications`, with the cover
//! - macOS: [`now_playing`], the Now Playing entry (Control Centre, the menu bar) and remote commands, notifications
//!   through `osascript`, without the cover
//!
//! Shared here: whether it runs at all ([`Options::wanted`]), which track changes are news ([`TrackNews`]), and the
//! main-thread handover ([`main_thread`]: Apple delivers remote commands on the main thread's run loop, so on macOS
//! the Daemon runs on another thread; on Linux the Daemon simply runs).

pub mod mpris;
pub mod now_playing;

use crate::Hub;
use singularity_proto::{PlayStatus, PlaybackState};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// This OS's adapter.
#[cfg(not(target_os = "macos"))]
use mpris as adapter;
#[cfg(target_os = "macos")]
use now_playing as adapter;

/// What the Daemon tells the OS.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// a D-Bus address for MPRIS (`SINGULARITY_DBUS=<address>`; tests use a private bus); `None` is the session bus.
    /// Always `None` for Now Playing (no bus).
    pub address: Option<String>,
    /// a notification on each new track (`notifications.track_change`, changed live)
    pub notify: Arc<AtomicBool>,
}

impl Options {
    /// Whether the Daemon runs OS media integration, from the environment (`env`), whether the Engine is a fake one
    /// (tests, pty) and whether it is the Spotify app:
    ///
    /// - `SINGULARITY_NOWPLAYING=off` (or the older `SINGULARITY_DBUS=off`): none; off wins over on (Q99)
    /// - `SINGULARITY_NOWPLAYING=on`: on, a fake Engine too (Q98)
    /// - `SINGULARITY_DBUS=<address>`: on, on that bus, a fake Engine too, only where the adapter is on a bus (MPRIS);
    ///   Now Playing ignores it (Q99)
    /// - otherwise none for a fake Engine, so test Daemons never show up in the desktop's media controls
    /// - and none where the OS has one entry for everyone and the Spotify app, as the Engine, already has its own
    ///   (Now Playing: a second entry would fight it for the media keys; MPRIS lists every player, Q100)
    pub fn wanted(env: impl Fn(&str) -> Option<String>, fake: bool, app_is_engine: bool, notify: Arc<AtomicBool>) -> Option<Options> {
        choose(env, fake, app_is_engine, Adapter { one_entry: adapter::ONE_ENTRY, on_a_bus: adapter::ON_A_BUS }, notify)
    }
}

/// What the policy needs to know about this OS's adapter.
#[derive(Clone, Copy, Debug)]
struct Adapter {
    /// the OS keeps one media entry for everyone (Now Playing), not one per player (MPRIS)
    one_entry: bool,
    /// it runs on a D-Bus, so `SINGULARITY_DBUS=<address>` means something to it (MPRIS)
    on_a_bus: bool,
}

/// [`Options::wanted`] for `adapter`.
fn choose(env: impl Fn(&str) -> Option<String>, fake: bool, app_is_engine: bool, adapter: Adapter, notify: Arc<AtomicBool>) -> Option<Options> {
    let var = |k: &str| env(k).filter(|v| !v.is_empty());
    let (switch, dbus) = (var("SINGULARITY_NOWPLAYING"), var("SINGULARITY_DBUS"));
    if switch.as_deref() == Some("off") || dbus.as_deref() == Some("off") {
        return None;
    }
    let address = dbus.filter(|_| adapter.on_a_bus);
    let one_entry = adapter.one_entry;
    let asked = switch.as_deref() == Some("on") || address.is_some();
    if !asked && (fake || (app_is_engine && one_entry)) {
        return None;
    }
    Some(Options { address, notify })
}

/// Start OS media integration on `hub` (`None`: the Daemon runs without it), supervised like the Daemon's other
/// background tasks.
pub fn start(hub: &Arc<Hub>, opts: Option<Options>) {
    let Some(opts) = opts else {
        adapter::not_wanted();
        return;
    };
    let h = hub.clone();
    crate::supervise(hub.clone(), adapter::NAME, move || adapter::run(h.clone(), opts.clone()));
}

/// The process's main thread: hand it to the OS where the adapter needs it (macOS: the Daemon (`daemon`, returning its
/// exit code) runs on another thread, the main one serves remote commands until the Daemon ends the process); on Linux
/// just run the Daemon. Never returns.
pub fn main_thread(daemon: impl FnOnce() -> i32 + Send + 'static) -> ! {
    #[cfg(target_os = "macos")]
    {
        std::thread::spawn(move || std::process::exit(daemon()));
        now_playing::main_thread();
    }
    #[cfg(not(target_os = "macos"))]
    std::process::exit(daemon());
}

/// Decides when a track change is worth a notification: once per new track, when it starts playing, and not for the
/// track that was already playing when the Daemon started.
#[derive(Default)]
pub struct TrackNews {
    last_uri: Option<String>,
}

/// A notification: the track's name, and "artists — album".
#[derive(Debug, Clone, PartialEq)]
pub struct News {
    pub title: String,
    pub body: String,
    pub cover_url: Option<String>,
}

impl TrackNews {
    /// The notification for `s`, if it's news (and notifications are `on`).
    pub fn news(&mut self, s: &PlaybackState, on: bool) -> Option<News> {
        let i = s.item.as_ref().filter(|_| s.status == PlayStatus::Playing)?;
        if self.last_uri.as_deref() == Some(i.uri.as_str()) {
            return None;
        }
        let first = self.last_uri.is_none();
        self.last_uri = Some(i.uri.clone());
        if !on || (first && s.position_at(singularity_proto::now_ms()) > 5000) {
            return None; // off, or the Daemon just started into a playing track: not news
        }
        let body = match &i.album {
            Some(a) if !a.is_empty() => format!("{} — {a}", i.artists.join(", ")),
            _ => i.artists.join(", "),
        };
        Some(News { title: i.name.clone(), body, cover_url: i.cover_url.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::Item;

    fn playing(uri: &str, pos: u64) -> PlaybackState {
        let now = singularity_proto::now_ms();
        PlaybackState { status: PlayStatus::Playing, item: Some(Item { uri: uri.into(), name: "Song".into(), artists: vec!["A".into(), "B".into()], album: Some("LP".into()), ..Default::default() }), position_ms: pos, timestamp_ms: now, speed: 1.0, ..Default::default() }
    }

    #[test]
    fn a_new_track_is_news_once_and_the_one_playing_at_start_is_not() {
        let mut n = TrackNews::default();
        assert_eq!(n.news(&playing("t:1", 60_000), true), None, "already playing when the Daemon started");
        let first = n.news(&playing("t:2", 0), true).unwrap();
        assert_eq!((first.title.as_str(), first.body.as_str()), ("Song", "A, B — LP"));
        assert_eq!(n.news(&playing("t:2", 1000), true), None, "the same track again");
        assert_eq!(n.news(&playing("t:3", 0), false), None, "notifications off");
        assert_eq!(n.news(&playing("t:3", 0), true), None, "seen while off: not news later");
        let mut paused = playing("t:4", 0);
        paused.status = PlayStatus::Paused;
        assert_eq!(n.news(&paused, true), None, "only once it plays");
        assert!(n.news(&playing("t:4", 0), true).is_some());
    }

    /// Linux's adapter and macOS's, as the policy sees them
    const MPRIS: Adapter = Adapter { one_entry: false, on_a_bus: true };
    const NOW_PLAYING: Adapter = Adapter { one_entry: true, on_a_bus: false };

    /// `choose` with these environment variables; `Some(address)` when it runs.
    fn run(vars: &[(&str, &str)], fake: bool, app: bool, adapter: Adapter) -> Option<Option<String>> {
        let env = |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        choose(env, fake, app, adapter, Default::default()).map(|o| o.address)
    }

    #[test]
    fn the_adapters_are_what_the_policy_tests_assume() {
        let this = if cfg!(target_os = "macos") { NOW_PLAYING } else { MPRIS };
        assert_eq!((adapter::ONE_ENTRY, adapter::ON_A_BUS), (this.one_entry, this.on_a_bus));
    }

    #[test]
    fn it_runs_for_a_real_engine_and_not_for_a_fake_one() {
        for a in [MPRIS, NOW_PLAYING] {
            assert_eq!(run(&[], false, false, a), Some(None), "the session bus / Now Playing");
            assert_eq!(run(&[], true, false, a), None, "test Daemons stay out of the desktop's media controls");
            assert_eq!(run(&[("SINGULARITY_NOWPLAYING", "")], true, false, a), None, "empty is unset");
        }
    }

    #[test]
    fn a_bus_address_never_turns_now_playing_on() {
        let bus = ("SINGULARITY_DBUS", "unix:path=/x");
        assert_eq!(run(&[bus], true, false, NOW_PLAYING), None, "a fake Engine");
        assert_eq!(run(&[bus], false, true, NOW_PLAYING), None, "the app as Engine");
        assert_eq!(run(&[bus], false, false, NOW_PLAYING), Some(None), "a real Engine: on as without it, no address");
        assert_eq!(run(&[bus, ("SINGULARITY_NOWPLAYING", "on")], true, false, NOW_PLAYING), Some(None), "asked by its own name");
    }

    #[test]
    fn this_oss_adapter_keeps_or_drops_its_entry_for_the_app_as_today() {
        let wanted = Options::wanted(|_| None, false, true, Default::default()).is_some();
        // Linux: MPRIS stays next to the app's own entry; macOS: Now Playing leaves the media keys to the app
        assert_eq!(wanted, !cfg!(target_os = "macos"));
    }

    #[test]
    fn the_app_as_engine_keeps_the_one_entry_an_os_has_for_everyone() {
        assert_eq!(run(&[], false, true, NOW_PLAYING), None, "Now Playing: the app's own entry has the media keys");
        assert_eq!(run(&[], false, true, MPRIS), Some(None), "MPRIS lists every player");
        assert_eq!(run(&[("SINGULARITY_NOWPLAYING", "on")], false, true, NOW_PLAYING), Some(None), "unless asked");
    }

    #[test]
    fn both_switch_names_turn_it_off_and_on() {
        for a in [MPRIS, NOW_PLAYING] {
            for off in [("SINGULARITY_NOWPLAYING", "off"), ("SINGULARITY_DBUS", "off")] {
                assert_eq!(run(&[off], false, false, a), None, "{off:?}");
            }
            assert_eq!(run(&[("SINGULARITY_NOWPLAYING", "on")], true, false, a), Some(None), "a fake Engine, asked for");
            assert_eq!(run(&[("SINGULARITY_DBUS", "off"), ("SINGULARITY_NOWPLAYING", "on")], false, false, a), None, "off wins");
            assert_eq!(run(&[("SINGULARITY_NOWPLAYING", "off"), ("SINGULARITY_DBUS", "unix:path=/x")], false, false, a), None, "off wins");
        }
        assert_eq!(run(&[("SINGULARITY_DBUS", "unix:path=/x")], true, false, MPRIS), Some(Some("unix:path=/x".into())), "a bus picked: on, on that bus");
    }
}
