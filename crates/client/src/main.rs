//! `singularity`: the terminal Client.

mod app;
mod artist;
mod conn;
mod ambient;
mod covers;
mod design;
mod gfx;
mod home;
mod keymap;
mod layout;
mod layouts;
mod looks;
mod lyrics;
mod meters;
mod mini;
mod queue;
mod scrub;
mod saved;
mod search;
mod setup;
mod sidebar;
mod stats;
mod term;
mod theme;
mod tracks;
mod ui;
mod visual;
mod waveform;
mod widgets;

use app::{App, Link};
use conn::ConnEvent;
use ratatui::crossterm::event::{self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, Event as TermEvent, KeyEventKind};
use ratatui::crossterm::execute;
use singularity_proto::{DaemonMsg, Event};
use std::io::{Write, stdout};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("singularity {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (cmd_tx, mut conn_rx) = conn::spawn();
    let mut app = App::new(Some(cmd_tx));
    app.recent = search::load_recent();
    app.covers = covers::Covers::spawn(cache_dir().join("covers"));
    app.waveform = waveform::Waveform::with_cache(cache_dir().join("waves"));
    app.cell_px = term::cell_px();
    app.hole.allow_gpu = true;
    app.load_config();
    if term::kitty_graphics_supported() && app.cell_px.is_some() {
        let mut k = gfx::Kitty::new(term::kitty_shm());
        k.below_backgrounds = term::below_backgrounds();
        app.kitty = Some(k);
    }
    let mut tui = ratatui::init();
    execute!(stdout(), EnableMouseCapture, EnableFocusChange, EnableBracketedPaste)?;
    // unfocused, the whole Client (and so the Visuals) runs slower: `visuals.unfocused_fps` (10) instead of 30
    let mut focused = true;
    let mut last_draw = std::time::Instant::now();
    // pixel-precise pointer where the terminal supports it (#24)
    let mut units = term::MouseUnits::detect();
    units.enable(&mut stdout())?;
    let res = (|| -> anyhow::Result<()> {
        while !app.quit {
            while let Ok(ev) = conn_rx.try_recv() {
                apply(&mut app, ev);
            }
            // at most ~60 redraws a second, however fast input arrives (a moving mouse sends hundreds of events)
            if let Some(wait) = Duration::from_millis(16).checked_sub(last_draw.elapsed()) {
                std::thread::sleep(wait);
            }
            last_draw = std::time::Instant::now();
            app.tick();
            let mut out = stdout().lock();
            out.write_all(b"\x1b[?2026h")?; // synchronized output: the frame and its images appear together
            drop(out);
            tui.draw(|f| ui::draw(f, &mut app))?;
            let mut out = stdout().lock();
            if let Some(k) = app.kitty.as_mut() {
                k.flush(&mut out)?;
            }
            if let Some(t) = app.clipboard.take() {
                write!(out, "\x1b]52;c;{}\x07", gfx::base64(t.as_bytes()))?;
            }
            out.write_all(b"\x1b[?2026l")?;
            out.flush()?;
            drop(out);
            let fps = if focused { 30 } else { app.config.visuals.unfocused_fps.clamp(1, 30) as u64 };
            if event::poll(Duration::from_millis(1000 / fps))? {
                loop {
                    match event::read()? {
                        TermEvent::Key(k) if k.kind != KeyEventKind::Release => app.key(k),
                        TermEvent::Mouse(m) => {
                            let (m, fx, precise) = units.to_cells(m);
                            app.pointer(m, fx, precise);
                        }
                        TermEvent::Paste(s) => app.paste(&s),
                        TermEvent::FocusLost => focused = false,
                        TermEvent::FocusGained => focused = true,
                        TermEvent::Resize(..) => {
                            if units != term::MouseUnits::Cells {
                                units = term::MouseUnits::measure().unwrap_or(units);
                            }
                            app.cell_px = term::cell_px();
                            // the resize clears the screen, and with it every placement
                            if let Some(k) = app.kitty.as_mut() {
                                k.reset(&mut stdout())?;
                            }
                        }
                        _ => {}
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
        }
        Ok(())
    })();
    if let Some(k) = app.kitty.as_mut() {
        let _ = k.reset(&mut stdout());
    }
    let _ = units.disable(&mut stdout());
    let _ = execute!(stdout(), DisableMouseCapture, DisableFocusChange, DisableBracketedPaste);
    ratatui::restore();
    res
}

fn apply(app: &mut App, ev: ConnEvent) {
    match ev {
        ConnEvent::Connecting => {}
        ConnEvent::Updating => app.say("Updating singularityd…"),
        ConnEvent::Disconnected => app.link = Link::Connecting,
        ConnEvent::Connected { daemon_version } => {
            app.link = Link::Connected { version: daemon_version };
            app.on_connected();
        }
        ConnEvent::Incompatible(m) => {
            app.say(m.clone());
            app.link = Link::Incompatible(m);
        }
        ConnEvent::Msg(DaemonMsg::Event(Event::Engine(s))) => app.engine = Some(s),
        ConnEvent::Msg(DaemonMsg::Event(Event::Playback(p))) => app.playback = p,
        ConnEvent::Msg(DaemonMsg::Event(Event::Error { message })) => app.say(message),
        ConnEvent::Msg(DaemonMsg::Event(Event::Catalogue(c))) => {
            if let singularity_proto::CatalogueStatus::LoginPending { url, .. } = &c {
                if !matches!(app.catalogue, Some(singularity_proto::CatalogueStatus::LoginPending { .. })) {
                    app.login_hidden = false;
                    // open the pairing page in the user's browser (the code is also shown in the UI)
                    let _ = std::process::Command::new("xdg-open").arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
                }
            }
            app.catalogue = Some(c);
        }
        ConnEvent::Msg(DaemonMsg::Event(Event::Home(h))) => app.home = Some(h),
        ConnEvent::Msg(DaemonMsg::Event(Event::Library(l))) => app.library = Some(l),
        ConnEvent::Msg(DaemonMsg::Event(Event::Tracks(p))) => app.track_page(p),
        ConnEvent::Msg(DaemonMsg::Event(Event::Spectrum(f))) => {
            // the setup's audio check passes once real sound comes through
            if f.level > 20 {
                app.setup.heard = true;
            }
            // the seek bar's waveform grows as the track plays (#46)
            if app.playback.status == singularity_proto::PlayStatus::Playing
                && let Some(i) = app.playback.item.as_ref()
            {
                let (uri, dur) = (i.uri.clone(), i.duration_ms);
                let pos = app.display_position(singularity_proto::now_ms());
                app.waveform.follow(&uri, dur);
                app.waveform.hear(pos, f.level);
            }
            app.meters.push(f);
        }
        ConnEvent::Msg(DaemonMsg::Event(Event::Stream(s))) => app.stream = Some(s),
        ConnEvent::Msg(DaemonMsg::Event(Event::Delivery(d))) => app.delivery_event(d),
        ConnEvent::Msg(DaemonMsg::Event(Event::Health(h))) => app.health = Some(h),
        ConnEvent::Msg(DaemonMsg::Event(Event::Artist(p))) => app.artist_page(p),
        ConnEvent::Msg(DaemonMsg::Event(Event::Devices { devices })) => app.devices = devices,
        ConnEvent::Msg(DaemonMsg::Event(Event::Search(r))) => {
            if let Some(s) = app.search.as_mut() {
                s.set_results(r);
            }
        }
        ConnEvent::Msg(DaemonMsg::Event(Event::Lyrics(l))) => {
            if app.playback.item.as_ref().is_some_and(|i| i.uri == l.uri) {
                app.lyrics_scroll = 0;
                app.lyrics = Some(l);
            }
        }
        ConnEvent::Msg(DaemonMsg::Event(Event::Notice { message })) => app.say(message),
        ConnEvent::Msg(DaemonMsg::Event(Event::Saved { entries })) => app.saved_event(entries),
        ConnEvent::Msg(DaemonMsg::Event(Event::Listening(s))) => {
            app.listening = Some(s);
            // a Play was logged (or old plays got their links): stats on screen ask again
            app.refresh_stats();
        }
        ConnEvent::Msg(DaemonMsg::Event(Event::Stats(s))) => {
            app.stats.insert(s.period, s);
        }
        ConnEvent::Msg(_) => {}
    }
}

fn cache_dir() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("singularity")
}
