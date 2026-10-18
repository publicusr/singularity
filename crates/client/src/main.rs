//! `singularity`: the terminal Client.

mod app;
mod artist;
mod conn;
mod covers;
mod gfx;
mod home;
mod layout;
mod lyrics;
mod meters;
mod queue;
mod scrub;
mod setup;
mod sidebar;
mod term;
mod theme;
mod tracks;
mod ui;
mod visual;

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
    app.covers = covers::Covers::spawn(cache_dir().join("covers"));
    app.cell_px = term::cell_px();
    app.hole.allow_gpu = true;
    if term::kitty_graphics_supported() && app.cell_px.is_some() {
        app.kitty = Some(gfx::Kitty::new(term::kitty_shm()));
    }
    let mut tui = ratatui::init();
    execute!(stdout(), EnableMouseCapture, EnableFocusChange, EnableBracketedPaste)?;
    // unfocused, the whole Client (and so the Visuals) runs at 10 fps instead of 30
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
            if event::poll(Duration::from_millis(if focused { 33 } else { 100 }))? {
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
            app.meters.push(f);
        }
        ConnEvent::Msg(DaemonMsg::Event(Event::Stream(s))) => app.stream = Some(s),
        ConnEvent::Msg(DaemonMsg::Event(Event::Health(h))) => app.health = Some(h),
        ConnEvent::Msg(DaemonMsg::Event(Event::Artist(p))) => app.artist_page(p),
        ConnEvent::Msg(DaemonMsg::Event(Event::Lyrics(l))) => {
            if app.playback.item.as_ref().is_some_and(|i| i.uri == l.uri) {
                app.lyrics_scroll = 0;
                app.lyrics = Some(l);
            }
        }
        ConnEvent::Msg(DaemonMsg::Event(Event::Notice { message })) => app.say(message),
        ConnEvent::Msg(_) => {}
    }
}

fn cache_dir() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("singularity")
}
