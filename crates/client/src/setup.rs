//! Getting started and staying healthy (#5): the first-run setup wizard, the health check, and the stream stats Pane.

use crate::app::{App, Link};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::{CatalogueStatus, EngineKind, EngineStatus, PlayStatus, StreamInfo};

/// How to get lossless once (it's a per-device setting, from a phone).
pub const LOSSLESS_FIX: &str = "On your phone: Spotify → device picker → Singularity → Change quality settings → Lossless";
/// The same for the Spotify app as Engine: its own setting.
pub const APP_LOSSLESS_FIX: &str = "Open the Spotify app once: Settings → Audio quality → Streaming quality → Lossless";

/// librespot never streams lossless: what to do if you want it.
pub const LIBRESPOT_LOSSLESS: &str = "librespot plays at most 320 kbps; for lossless set [engine] kind = \"auto\" in config.toml";

/// How to get lossless with this Engine.
pub fn lossless_fix(app: &App) -> &'static str {
    match app.engine_kind {
        EngineKind::SpotifyApp => APP_LOSSLESS_FIX,
        EngineKind::Librespot => LIBRESPOT_LOSSLESS,
        _ => LOSSLESS_FIX,
    }
}
pub const KEY_URL: &str = "https://developer.spotify.com/dashboard/soloist";

/// What this OS calls its keyring (the Daemon reports "keyring" for either).
const KEYRING: &str = if cfg!(target_os = "macos") { "Keychain" } else { "keyring" };

/// Where a secret lives, as the health view says it.
fn place(at: &str) -> String {
    if at == "keyring" { KEYRING.into() } else { at.into() }
}

#[derive(Default)]
pub struct Setup {
    /// the user closed the wizard this run (the palette's "Setup" reopens it)
    pub dismissed: bool,
    /// forced open from the palette even though everything is set up
    pub forced: bool,
    pub key: String,
    /// the audio check passed: sound came through the tap
    pub heard: bool,
    /// the Engine was ready once this run: if it drops off later (the Spotify app gone from Connect), the status bar
    /// and the health check say so, the wizard doesn't jump up again
    pub engine_was_ready: bool,
    /// asked to enter the key again (Soloist rejected it)
    pub rekey: bool,
}

/// The wizard's steps (each Engine has its own, #66).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Soloist: the API key
    Key,
    /// Soloist: downloading and starting it
    Engine,
    /// Soloist: picked once from a phone
    Pair,
    /// the Spotify app: installed (and this Daemon has a Catalogue to drive it with)
    App,
    Login,
    /// the Spotify app, librespot: its device found on Connect
    Connect,
    Audio,
    Done,
}

impl Step {
    /// This Engine's steps, in order. The Spotify app is driven over Connect, so the login comes before finding it.
    fn all(kind: EngineKind) -> &'static [(Step, &'static str)] {
        match kind {
            EngineKind::SpotifyApp => &[(Step::App, "Spotify app"), (Step::Login, "Log in to Spotify"), (Step::Connect, "Find the app on Connect"), (Step::Audio, "Audio check")],
            EngineKind::Librespot => &[(Step::Login, "Log in to Spotify"), (Step::Connect, "Start the player"), (Step::Audio, "Audio check")],
            _ => &[(Step::Key, "Soloist key"), (Step::Engine, "Download Soloist"), (Step::Pair, "Pair from your phone"), (Step::Login, "Log in to Spotify"), (Step::Audio, "Audio check")],
        }
    }
}

/// Where setup stands, from what the Daemon reports.
pub fn step(app: &App) -> Step {
    if app.engine_kind.via_connect() {
        return app_step(app);
    }
    if app.setup.rekey {
        return Step::Key;
    }
    match &app.engine {
        // no report yet: nothing to ask about
        None => return Step::Done,
        Some(EngineStatus::NotConfigured { .. }) => return Step::Key,
        Some(EngineStatus::Starting) | Some(EngineStatus::Failed { .. }) => return Step::Engine,
        Some(EngineStatus::NeedsPairing { .. }) => return Step::Pair,
        Some(EngineStatus::Ready { .. }) => {}
    }
    match &app.catalogue {
        Some(CatalogueStatus::Ready { .. }) => {}
        _ => return Step::Login,
    }
    if !app.setup.heard { Step::Audio } else { Step::Done }
}

/// Where setup stands with an Engine played over Connect (the Spotify app, librespot).
fn app_step(app: &App) -> Step {
    let librespot = app.engine_kind == EngineKind::Librespot;
    match &app.engine {
        None => return Step::Done,
        // librespot waits for the login; the app may not be installed
        Some(EngineStatus::NotConfigured { .. }) if !librespot => return Step::App,
        // librespot in a Daemon without the Catalogue: nothing to log in to, it says why on its step
        Some(EngineStatus::NotConfigured { .. }) if app.catalogue.is_none() => return Step::Connect,
        _ => {}
    }
    if !matches!(app.catalogue, Some(CatalogueStatus::Ready { .. })) {
        return Step::Login;
    }
    if !matches!(app.engine, Some(EngineStatus::Ready { .. })) {
        return Step::Connect;
    }
    // (the Spotify app's audio isn't tapped on macOS yet, #67: playing counts as the check there)
    let tapped = librespot || !cfg!(target_os = "macos");
    if app.setup.heard || (!tapped && app.playback.status == PlayStatus::Playing) { Step::Done } else { Step::Audio }
}

/// The wizard shows itself while something essential isn't set up (or when asked for).
pub fn wanted(app: &App) -> bool {
    if app.setup.forced {
        return true;
    }
    if app.setup.dismissed || !matches!(app.link, Link::Connected { .. }) {
        return false;
    }
    let missing = match step(app) {
        Step::Key | Step::Engine | Step::Pair | Step::App => true,
        Step::Connect => !app.setup.engine_was_ready,
        _ => false,
    };
    missing || matches!(app.catalogue, Some(CatalogueStatus::LoggedOut))
}

pub fn rect(area: Rect) -> Rect {
    crate::ui::centered(area, 90, 20)
}

pub fn draw_wizard(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) {
    let inner = crate::ui::popup(buf, r, th, "Set up Singularity");
    let (x, w) = (inner.x + 2, inner.width.saturating_sub(4));
    let mut y = inner.y;
    let need = match app.engine_kind {
        EngineKind::SpotifyApp => "You need: Spotify Premium · the Spotify app, logged in to the same account",
        EngineKind::Librespot => "You need: Spotify Premium (librespot, built in, plays as your account)",
        _ => "You need: Spotify Premium · a Soloist API key · your phone on the same network, once",
    };
    text(buf, x, y, w, need, th.s(th.dim));
    y += 2;
    let now = step(app);
    let all = Step::all(app.engine_kind);
    let idx = |s: Step| all.iter().position(|(t, _)| *t == s).unwrap_or(all.len());
    for &(s, label) in all {
        let (mark, st) = if idx(s) < idx(now) {
            ("✓", th.s(th.accent2))
        } else if s == now {
            ("›", th.s(th.accent).add_modifier(Modifier::BOLD))
        } else {
            ("·", th.s(th.dim))
        };
        text(buf, x, y, 2, mark, st);
        text(buf, x + 2, y, w.saturating_sub(2), label, st);
        y += 1;
    }
    y += 1;
    let line = |buf: &mut Buffer, y: &mut u16, s: &str, st| {
        text(buf, x, *y, w, s, st);
        *y += 1;
    };
    match now {
        Step::Key => {
            line(buf, &mut y, "1. Open this page, log in with your Premium account, and create a key:", th.s(th.fg));
            line(buf, &mut y, &format!("   {KEY_URL}"), th.s(th.accent2));
            line(buf, &mut y, &format!("2. Paste it here and press Enter (it goes to your {KEYRING}):"), th.s(th.fg));
            let shown = if app.setup.key.is_empty() { "paste the key…".to_string() } else { "•".repeat(app.setup.key.chars().count().min(48)) };
            line(buf, &mut y, &format!("   › {shown}▏"), th.s(if app.setup.key.is_empty() { th.dim } else { th.accent }));
        }
        Step::Engine => match &app.engine {
            Some(EngineStatus::Failed { message }) => {
                line(buf, &mut y, &format!("Soloist couldn't start: {message}"), th.s(th.error));
                line(buf, &mut y, "Singularity retries by itself. A wrong key? Press K to enter it again.", th.s(if message.contains("API key") { th.fg } else { th.dim }));
            }
            _ => line(buf, &mut y, "Downloading Spotify's own player (Soloist) from Spotify and starting it…", th.s(th.fg)),
        },
        Step::Pair => {
            line(buf, &mut y, "On your phone, open Spotify, tap the device picker (bottom), and choose", th.s(th.fg));
            line(buf, &mut y, "   Singularity", th.title());
            line(buf, &mut y, "You only do this once. While you're there: Change quality settings → Lossless.", th.s(th.dim));
        }
        Step::App => match &app.engine {
            Some(EngineStatus::NotConfigured { reason }) => {
                line(buf, &mut y, &format!("Can't play yet: {reason}"), th.s(th.error));
                line(buf, &mut y, "Singularity checks again when the Daemon restarts.", th.s(th.dim));
            }
            _ => line(buf, &mut y, "Looking for the Spotify app…", th.s(th.fg)),
        },
        Step::Connect if app.engine_kind == EngineKind::Librespot => {
            match &app.engine {
                Some(EngineStatus::Failed { message }) => line(buf, &mut y, message, th.s(th.error)),
                Some(EngineStatus::NotConfigured { reason }) => line(buf, &mut y, &format!("Can't play yet: {reason}"), th.s(th.error)),
                _ => line(buf, &mut y, "Starting the built-in player (librespot) and waiting for it on Spotify Connect…", th.s(th.fg)),
            }
            line(buf, &mut y, "Press Enter to restart it.", th.s(th.dim));
        }
        Step::Connect => {
            match &app.engine {
                Some(EngineStatus::Failed { message }) => line(buf, &mut y, message, th.s(th.error)),
                _ => line(buf, &mut y, "Starting the Spotify app (hidden) and waiting for it on Spotify Connect…", th.s(th.fg)),
            }
            line(buf, &mut y, "Press Enter to restart it. If it isn't logged in, open it once and log in with the same account.", th.s(th.dim));
        }
        Step::Login => match &app.catalogue {
            Some(CatalogueStatus::LoginPending { url, code }) => {
                line(buf, &mut y, "Approve this code in your browser (it should have opened):", th.s(th.fg));
                line(buf, &mut y, &format!("   {url}"), th.s(th.accent2));
                text(buf, x + 3, y, 24, &format!(" {code} "), th.chip(th.accent));
                y += 2;
                line(buf, &mut y, "Singularity never sees your password.", th.s(th.dim));
            }
            Some(CatalogueStatus::Unavailable { message }) => line(buf, &mut y, &format!("Spotify browsing is unavailable: {message}"), th.s(th.error)),
            _ => line(buf, &mut y, "Press Enter to log in (for Home, your library and search).", th.s(th.fg)),
        },
        Step::Audio | Step::Done => {
            if app.setup.heard {
                line(buf, &mut y, "✓ Sound is coming through.", th.s(th.accent2));
            } else {
                line(buf, &mut y, "Press Space to play something; this step passes once sound comes through.", th.s(th.fg));
            }
            match &app.stream {
                Some(s) if s.lossless() => line(buf, &mut y, &format!("✓ Lossless: {}", describe(s)), th.s(th.accent2)),
                // as good as this Engine gets: nothing to fix
                Some(s) if !app.engine_kind.lossless() => line(buf, &mut y, &format!("✓ {} (librespot's best; never lossless)", describe(s)), th.s(th.accent2)),
                Some(s) => {
                    line(buf, &mut y, &format!("! Not lossless yet: {}", describe(s)), th.s(th.error));
                    line(buf, &mut y, lossless_fix(app), th.s(th.fg));
                }
                None => {}
            }
        }
    }
    text(buf, x, inner.bottom().saturating_sub(1), w, "Enter: continue · Esc: later (palette → \"Setup\" brings this back)", th.s(th.dim));
}

fn mbit(bps: u32) -> String {
    format!("{:.2} Mbit/s", bps as f64 / 1e6)
}

/// "FLAC · 1.40 Mbit/s (avg 1.69) · 44.1 kHz"
pub fn describe(s: &StreamInfo) -> String {
    let avg = s.average_bitrate.map(|a| format!(" (avg {:.2})", a as f64 / 1e6)).unwrap_or_default();
    format!("{} · {}{avg} · {:.1} kHz", s.codec.to_uppercase(), mbit(s.bitrate), s.sample_rate as f64 / 1000.0)
}

/// The stream stats Pane.
pub fn draw_stream(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    let mut y = r.y;
    let row = |buf: &mut Buffer, y: &mut u16, k: &str, v: &str, st| {
        if *y < r.bottom() {
            text(buf, x, *y, 9, k, th.s(th.dim));
            text(buf, x + 9, *y, w.saturating_sub(9), v, st);
            *y += 1;
        }
    };
    match &app.stream {
        Some(s) => {
            if s.lossless() {
                text(buf, x, y, 10, " LOSSLESS ", th.chip(th.accent2));
            } else if !app.engine_kind.lossless() {
                text(buf, x, y, 11, " 320 KBPS ", th.chip(th.dim));
            } else {
                text(buf, x, y, 14, " NOT LOSSLESS ", th.chip(th.error));
            }
            y += 2;
            row(buf, &mut y, "codec", &s.codec.to_uppercase(), th.s(th.fg));
            row(buf, &mut y, "bitrate", &mbit(s.bitrate), th.s(th.fg));
            if let Some(a) = s.average_bitrate {
                row(buf, &mut y, "average", &mbit(a), th.s(th.fg));
            }
            row(buf, &mut y, "rate", &format!("{:.1} kHz", s.sample_rate as f64 / 1000.0), th.s(th.fg));
            row(buf, &mut y, "decoded", &s.sample_format.replace("-pcm", "").replace('-', " "), th.s(th.dim));
            // how well the download keeps up (#45): red when it's below what this stream needs
            let now = singularity_proto::now_ms();
            if let Some(d) = &app.delivery {
                if let (Some(k), true) = (d.bandwidth_kbps, now.saturating_sub(d.measured_ms) < 180_000) {
                    let need = app.need_kbps().unwrap_or(0);
                    let short = need > 0 && k < need;
                    let v = if short { format!("{k} kB/s, need {need}") } else { format!("{k} kB/s") };
                    row(buf, &mut y, "download", &v, th.s(if short { th.error } else { th.fg }));
                }
                let n = d.stalls_last_minute(now);
                if n > 0 {
                    row(buf, &mut y, "stalls", &format!("{n} in the last 60s"), th.s(th.error));
                }
            }
        }
        None => {
            text(buf, x, y, w, "play something to see the stream", th.s(th.dim));
            y += 2;
        }
    }
    y += 1;
    // (the build updates itself, so its expiry isn't news here; the health check has it)
    if let Some(EngineStatus::Ready { device_name, active, version, .. }) = &app.engine {
        row(buf, &mut y, "device", &format!("{device_name}{}", if *active { " · active" } else { "" }), th.s(th.fg));
        if !version.is_empty() {
            row(buf, &mut y, "build", version, th.s(th.dim));
        }
    }
    if app.stream.as_ref().is_some_and(|s| !s.lossless()) && y + 1 < r.bottom() {
        y += 1;
        // the fix, wrapped to the Pane
        let mut line = String::new();
        let fix = match app.engine_kind {
            EngineKind::SpotifyApp => APP_LOSSLESS_FIX,
            EngineKind::Librespot => LIBRESPOT_LOSSLESS,
            _ => "Fix it once from your phone: device picker → Singularity → Change quality settings → Lossless",
        };
        for word in fix.split(' ') {
            if line.chars().count() + word.chars().count() + 1 > w as usize {
                if y >= r.bottom() {
                    return;
                }
                text(buf, x, y, w, &line, th.s(th.fg));
                y += 1;
                line.clear();
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if y < r.bottom() {
            text(buf, x, y, w, &line, th.s(th.fg));
        }
    }
}

/// One line of the health check: ✓ fine, ! needs attention, ✗ broken.
fn health_rows(app: &App) -> Vec<(char, &'static str, String)> {
    let mut rows = vec![];
    rows.push(match &app.link {
        Link::Connected { version } => ('✓', "Daemon", format!("singularityd {version}")),
        Link::Connecting => ('✗', "Daemon", "not reachable; starting it…".into()),
        Link::Incompatible(m) => ('✗', "Daemon", m.clone()),
    });
    let spotify_app = app.engine_kind.via_connect();
    rows.push(match &app.engine {
        Some(EngineStatus::Ready { version, device_name, active, .. }) if spotify_app => {
            let what = if app.engine_kind == EngineKind::Librespot { "librespot (built in)".to_string() } else { format!("Spotify app {version}") };
            ('✓', "Engine", format!("{what} · \"{device_name}\" on Connect{}", if *active { "" } else { " · not the active device" }))
        }
        Some(EngineStatus::Ready { version, expires_in_days, active, .. }) => {
            let built = app.health.as_ref().and_then(|h| h.engine_build.clone()).map(|d| format!(", built {d}")).unwrap_or_default();
            let left = expires_in_days.map(|d| format!(", {d} days left (updates itself)")).unwrap_or_default();
            // a build this close to expiring means updating has failed for days
            (if expires_in_days.is_some_and(|d| d < 3) { '!' } else { '✓' }, "Engine", format!("{version}{built}{left}{}", if *active { "" } else { " · not the active device" }))
        }
        Some(EngineStatus::NeedsPairing { device_name }) => ('!', "Engine", format!("pick \"{device_name}\" once from your phone's device picker")),
        Some(EngineStatus::Starting) => ('!', "Engine", "starting…".into()),
        Some(EngineStatus::NotConfigured { reason }) => ('✗', "Engine", reason.clone()),
        Some(EngineStatus::Failed { message }) => ('✗', "Engine", message.clone()),
        None => ('!', "Engine", "no status yet".into()),
    });
    if let Some(h) = app.health.as_ref().filter(|_| !spotify_app) {
        rows.push((if h.soloist_key == "keyring" || h.soloist_key.starts_with("environment") { '✓' } else if h.soloist_key == "missing" { '✗' } else { '!' }, "Soloist key", place(&h.soloist_key)));
    }
    rows.push(match &app.catalogue {
        Some(CatalogueStatus::Ready { username, premium, .. }) => (if *premium { '✓' } else { '!' }, "Catalogue", format!("logged in as {username}{}", if *premium { " · Premium" } else { " · not Premium" })),
        Some(CatalogueStatus::LoggedOut) => ('!', "Catalogue", "logged out (Ctrl+K → Log in to Spotify)".into()),
        Some(CatalogueStatus::LoginPending { code, .. }) => ('!', "Catalogue", format!("waiting for the code {code}")),
        Some(CatalogueStatus::Connecting) => ('!', "Catalogue", "connecting…".into()),
        Some(CatalogueStatus::Unavailable { message }) => ('✗', "Catalogue", message.clone()),
        None => ('!', "Catalogue", "disabled in this Daemon".into()),
    });
    if let Some(h) = &app.health {
        rows.push((if h.spotify_login == "keyring" { '✓' } else if h.spotify_login == "missing" { '!' } else { '!' }, "Spotify login", place(&h.spotify_login)));
    }
    let net = match &app.catalogue {
        Some(CatalogueStatus::Ready { .. }) => ('✓', "Network", "Spotify reachable".to_string()),
        Some(CatalogueStatus::Unavailable { .. }) => ('✗', "Network", "Spotify unreachable from the Catalogue".into()),
        _ => ('·', "Network", "unknown until logged in".into()),
    };
    rows.push(net);
    if let Some(h) = &app.health {
        rows.push(match &h.audio_output {
            Some(o) => ('✓', "Audio output", o.clone()),
            None => ('·', "Audio output", "nothing playing right now".into()),
        });
    }
    rows.push(match &app.stream {
        Some(s) if s.lossless() => ('✓', "Stream", describe(s)),
        Some(s) if !app.engine_kind.lossless() => ('✓', "Stream", format!("{}: librespot's best (never lossless)", describe(s))),
        Some(s) => ('!', "Stream", format!("{}: not lossless", describe(s))),
        None => ('·', "Stream", "play something to check the quality".into()),
    });
    rows
}

pub fn health_rect(area: Rect) -> Rect {
    crate::ui::centered(area, 86, 16)
}

pub fn draw_health(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) {
    let inner = crate::ui::popup(buf, r, th, "Health check");
    let (x, w) = (inner.x + 2, inner.width.saturating_sub(4));
    let mut y = inner.y;
    for (mark, k, v) in health_rows(app) {
        let col = match mark {
            '✓' => th.accent2,
            '✗' => th.error,
            '!' => th.accent,
            _ => th.dim,
        };
        text(buf, x, y, 2, &mark.to_string(), th.s(col));
        text(buf, x + 2, y, 14, k, th.s(th.dim));
        text(buf, x + 16, y, w.saturating_sub(16), v.as_str(), th.s(th.fg));
        y += 1;
    }
    if app.stream.as_ref().is_some_and(|s| !s.lossless()) {
        y += 1;
        text(buf, x, y, w, LOSSLESS_FIX, th.s(th.accent));
    }
    let foot = "Soloist takes its key only on its command line: `ps` shows it to local users.";
    text(buf, x, inner.bottom().saturating_sub(2), w, foot, th.s(th.border));
    text(buf, x, inner.bottom().saturating_sub(1), w, "Esc closes · refreshes every few seconds", th.s(th.dim));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_reads_like_the_spec() {
        let s = StreamInfo { codec: "flac".into(), bitrate: 1_400_000, average_bitrate: Some(1_689_310), sample_rate: 44_100, sample_format: "32-bit-float-pcm".into() };
        assert_eq!(describe(&s), "FLAC · 1.40 Mbit/s (avg 1.69) · 44.1 kHz");
    }

    #[test]
    fn steps_follow_the_daemons_reports() {
        let mut app = App::new(None);
        app.link = Link::Connected { version: "t".into() };
        app.engine = Some(EngineStatus::NotConfigured { reason: "no key".into() });
        assert_eq!(step(&app), Step::Key);
        assert!(wanted(&app));
        app.engine = Some(EngineStatus::NeedsPairing { device_name: "Singularity".into() });
        assert_eq!(step(&app), Step::Pair);
        app.engine = Some(EngineStatus::Ready { device_name: "Singularity".into(), active: true, version: "v".into(), expires_in_days: Some(80) });
        app.catalogue = Some(CatalogueStatus::LoggedOut);
        assert_eq!(step(&app), Step::Login);
        assert!(wanted(&app));
        app.catalogue = Some(CatalogueStatus::Ready { username: "u".into(), country: "SE".into(), premium: true });
        assert_eq!(step(&app), Step::Audio);
        assert!(!wanted(&app), "a set-up user is never interrupted");
        app.setup.dismissed = true;
        app.engine = Some(EngineStatus::NotConfigured { reason: "x".into() });
        assert!(!wanted(&app), "dismissed stays dismissed");
    }

    #[test]
    fn the_spotify_app_logs_in_before_it_is_found_on_connect() {
        let mut app = App::new(None);
        app.link = Link::Connected { version: "t".into() };
        app.engine_kind = EngineKind::SpotifyApp;
        app.engine = Some(EngineStatus::NotConfigured { reason: "the Spotify app isn't installed".into() });
        assert_eq!(step(&app), Step::App);
        assert!(wanted(&app));
        // starting, and not logged in: the login first, as the app is driven over Connect
        app.engine = Some(EngineStatus::Starting);
        app.catalogue = Some(CatalogueStatus::LoggedOut);
        assert_eq!(step(&app), Step::Login);
        app.catalogue = Some(CatalogueStatus::Ready { username: "u".into(), country: "PL".into(), premium: true });
        assert_eq!(step(&app), Step::Connect);
        assert!(wanted(&app), "no Engine yet: the wizard says why");
        app.engine = Some(EngineStatus::Ready { device_name: "Mac".into(), active: false, version: "1.2.94".into(), expires_in_days: None });
        assert_eq!(step(&app), Step::Audio);
        assert!(!wanted(&app));
        // its audio is tapped on Linux (its PipeWire stream), not yet on macOS (#67): there playing counts
        app.playback.status = PlayStatus::Playing;
        assert_eq!(step(&app) == Step::Done, cfg!(target_os = "macos"));
        app.setup.heard = true;
        assert_eq!(step(&app), Step::Done);
        // the health check names the app, and has no Soloist key row
        app.health = Some(singularity_proto::Health { soloist_key: "missing".into(), spotify_login: "keyring".into(), ..Default::default() });
        let rows = health_rows(&app);
        let engine = rows.iter().find(|r| r.1 == "Engine").unwrap();
        assert_eq!(engine.2, "Spotify app 1.2.94 · \"Mac\" on Connect · not the active device");
        assert!(rows.iter().all(|r| r.1 != "Soloist key"));
        assert_eq!(rows.iter().find(|r| r.1 == "Spotify login").unwrap().2, KEYRING);
        // later it drops off Connect (a network change): the health check says so, the wizard stays away
        app.setup.engine_was_ready = true;
        app.engine = Some(EngineStatus::Failed { message: "\"Mac\" isn't on Spotify Connect".into() });
        assert_eq!(step(&app), Step::Connect);
        assert!(!wanted(&app));
    }

    #[test]
    fn librespot_needs_only_the_login_and_never_nags_about_lossless() {
        let mut app = App::new(None);
        app.link = Link::Connected { version: "t".into() };
        app.engine_kind = EngineKind::Librespot;
        // it waits for the Catalogue's login: that's the first step, not "install the app"
        app.engine = Some(EngineStatus::NotConfigured { reason: "log in to Spotify first".into() });
        app.catalogue = Some(CatalogueStatus::LoggedOut);
        assert_eq!(step(&app), Step::Login);
        assert!(wanted(&app));
        assert_eq!(Step::all(app.engine_kind).iter().map(|s| s.0).collect::<Vec<_>>(), [Step::Login, Step::Connect, Step::Audio]);
        app.catalogue = Some(CatalogueStatus::Ready { username: "u".into(), country: "PL".into(), premium: true });
        app.engine = Some(EngineStatus::Starting);
        assert_eq!(step(&app), Step::Connect);
        app.engine = Some(EngineStatus::Ready { device_name: "Singularity on Mac".into(), active: true, version: String::new(), expires_in_days: None });
        // its audio is always tapped (in-process): playing isn't enough, sound must come through
        app.playback.status = PlayStatus::Playing;
        assert_eq!(step(&app), Step::Audio);
        app.setup.heard = true;
        assert_eq!(step(&app), Step::Done);
        app.stream = Some(crate::setup::tests::vorbis());
        let rows = health_rows(&app);
        assert_eq!(rows.iter().find(|r| r.1 == "Engine").unwrap().2, "librespot (built in) · \"Singularity on Mac\" on Connect");
        let stream = rows.iter().find(|r| r.1 == "Stream").unwrap();
        assert_eq!(stream.0, '✓', "320 kbps is as good as librespot gets: {stream:?}");
        assert_eq!(lossless_fix(&app), LIBRESPOT_LOSSLESS);
    }

    pub fn vorbis() -> StreamInfo {
        StreamInfo { codec: "vorbis".into(), bitrate: 320_000, average_bitrate: None, sample_rate: 44_100, sample_format: String::new() }
    }
}
