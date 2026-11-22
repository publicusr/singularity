//! `singularity`: the terminal Client.

mod app;
mod openlist;
mod receive;
mod arrange;
mod artist;
mod bigtext;
mod conn;
mod ambient;
mod covers;
mod design;
mod gfx;
mod home;
mod jam;
mod keymap;
mod layout;
mod layouts;
mod looks;
mod karaoke;
mod lyrics;
mod meters;
mod mini;
mod pace;
mod cover;
mod playlists;
mod queue;
mod scroll;
mod scrub;
mod saved;
mod search;
mod settings;
mod setup;
mod songlyrics;
mod sidebar;
mod stats;
mod term;
mod termfont;
mod theme;
mod tile;
mod tiling;
mod tracks;
mod ui;
mod visual;
mod wheel;
mod widgets;

use app::App;
use ratatui::crossterm::event::{self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, Event as TermEvent, KeyEventKind, MouseEventKind};
use ratatui::crossterm::execute;
use std::io::{Write, stdout};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("singularity {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (cmd_tx, mut conn_rx) = conn::spawn();
    let mut app = App::new(Some(cmd_tx));
    app.search = search::Search::with_recent(search::load_recent());
    app.covers = covers::Covers::spawn(cache_dir().join("covers"));
    app.cell_px = term::cell_px();
    app.hole.allow_gpu = true;
    app.load_config();
    if term::kitty_graphics_supported() && app.cell_px.is_some() {
        let mut k = gfx::Kitty::new(term::kitty_shm());
        k.below_backgrounds = term::below_backgrounds();
        app.kitty = Some(k);
        // big text draws in the terminal's font: find it while the first frames draw
        bigtext::warm();
    }
    let mut tui = ratatui::init();
    execute!(stdout(), EnableMouseCapture, EnableFocusChange, EnableBracketedPaste)?;
    // the terminal's own pointer, whatever an earlier run (or another app) left
    stdout().write_all(arrange::pointer_shape_seq("").as_bytes())?;
    // frames: 30 a second, `visuals.unfocused_fps` unfocused, `visuals.motion_fps` while something moves (pace.rs)
    let mut focused = true;
    // pixel-precise pointer where the terminal supports it (#24)
    let mut units = term::MouseUnits::detect();
    units.enable(&mut stdout())?;
    // input is read on its own thread so every event carries when it arrived: one wheel notch can come as a burst
    // of reports (Ghostty), told apart from separate notches by their timing; a dragged Pane's dwell needs it too
    let (input_tx, input) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            let ev = event::read();
            let failed = ev.is_err();
            if input_tx.send((Instant::now(), ev)).is_err() || failed {
                break;
            }
        }
    });
    let mut wheel = term::WheelBursts::default();
    let mut pacer = pace::Pacer::new(Instant::now());
    // `SINGULARITY_FRAME_LOG=<file>`: a line per frame (when it started, its rate, how long its parts took)
    let mut log = std::env::var_os("SINGULARITY_FRAME_LOG").and_then(|p| std::fs::File::create(p).ok()).map(std::io::BufWriter::new);
    let t0 = Instant::now();
    let res = (|| -> anyhow::Result<()> {
        // a pointer move waits for the frame: only the last one before it counts (a mouse sends hundreds a second)
        let mut motion: Option<(Instant, TermEvent)> = None;
        while !app.quit {
            // until the frame is due, input is handled as it comes
            let (due, fps) = loop {
                let fps = app.frame_rate(focused);
                let due = pacer.due(fps);
                let Some(wait) = due.checked_duration_since(Instant::now()).filter(|w| !w.is_zero()) else { break (due, fps) };
                let (at, ev) = match input.recv_timeout(wait) {
                    Ok(x) => x,
                    Err(mpsc::RecvTimeoutError::Timeout) => break (due, fps),
                    Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("the terminal input closed"),
                };
                let ev = ev?;
                // input brings the frame forward: to the motion rate, or for a bare hover at most 60 a second
                let hover = matches!(&ev, TermEvent::Mouse(m) if m.kind == MouseEventKind::Moved) && !app.moving_now();
                pacer.input(if hover { 60.min(app.motion_fps()) } else { app.motion_fps() });
                if matches!(&ev, TermEvent::Mouse(m) if matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Drag(_))) {
                    if let Some((pat, pev)) = motion.take()
                        && !same_kind(&pev, &ev)
                    {
                        handle(&mut app, pev, pat, &mut focused, &mut units, &mut wheel)?;
                    }
                    motion = Some((at, ev));
                    continue;
                }
                if let Some((pat, pev)) = motion.take() {
                    handle(&mut app, pev, pat, &mut focused, &mut units, &mut wheel)?;
                }
                handle(&mut app, ev, at, &mut focused, &mut units, &mut wheel)?;
            };
            if let Some((at, ev)) = motion.take() {
                handle(&mut app, ev, at, &mut focused, &mut units, &mut wheel)?;
            }
            let start = Instant::now();
            pacer.begin(start, due, fps);
            while let Ok(ev) = conn_rx.try_recv() {
                app.receive(ev);
            }
            app.tick();
            let ticked = start.elapsed();
            let mut out = stdout().lock();
            out.write_all(b"\x1b[?2026h")?; // synchronized output: the frame and its images appear together
            drop(out);
            let mut built = Duration::ZERO;
            tui.draw(|f| {
                let t = Instant::now();
                ui::draw(f, &mut app);
                built = t.elapsed();
            })?;
            let drawn = start.elapsed();
            let mut out = stdout().lock();
            if let Some(k) = app.kitty.as_mut() {
                k.flush(&mut out)?;
            }
            let shape = app.pointer_shape();
            if shape != app.pointer_shape_sent {
                out.write_all(arrange::pointer_shape_seq(shape).as_bytes())?;
                app.pointer_shape_sent = shape;
            }
            if let Some(url) = app.open_url.take() {
                open_in_browser(&url);
            }
            if let Some(t) = app.clipboard.take() {
                write!(out, "\x1b]52;c;{}\x07", gfx::base64(t.as_bytes()))?;
            }
            out.write_all(b"\x1b[?2026l")?;
            out.flush()?;
            drop(out);
            if let Some(l) = log.as_mut() {
                let us = |d: Duration| d.as_micros();
                // start, rate, then µs: tick, ui::draw, diff and write, images and flush
                writeln!(l, "{} {fps} {} {} {} {}", us(start - t0), us(ticked), us(built), us(drawn.saturating_sub(ticked + built)), us(start.elapsed().saturating_sub(drawn)))?;
            }
        }
        Ok(())
    })();
    if let Some(l) = log.as_mut() {
        let _ = l.flush();
    }
    if let Some(k) = app.kitty.as_mut() {
        let _ = k.reset(&mut stdout());
    }
    let _ = units.disable(&mut stdout());
    // always: a shape we can't know about (a crashed run, another app) mustn't outlive us either
    let _ = stdout().write_all(arrange::pointer_shape_seq("").as_bytes());
    let _ = execute!(stdout(), DisableMouseCapture, DisableFocusChange, DisableBracketedPaste);
    ratatui::restore();
    res
}

/// One input event, handled at once (`at`: when it arrived).
fn handle(app: &mut App, ev: TermEvent, at: Instant, focused: &mut bool, units: &mut term::MouseUnits, wheel: &mut term::WheelBursts) -> anyhow::Result<()> {
    app.input_at = Some(at);
    match ev {
        TermEvent::Key(k) if k.kind != KeyEventKind::Release => app.key(k),
        TermEvent::Mouse(m) => {
            if wheel.keep(m.kind, at) {
                let (m, fx, precise) = units.to_cells(m);
                app.pointer(m, fx, precise);
            }
        }
        TermEvent::Paste(s) => app.paste(&s),
        TermEvent::FocusLost => {
            *focused = false;
            app.pointer_gone();
        }
        TermEvent::FocusGained => *focused = true,
        TermEvent::Resize(..) => {
            // the borders move: the one under the pointer is found again on its next move
            app.pointer_gone();
            if *units != term::MouseUnits::Cells {
                *units = term::MouseUnits::measure().unwrap_or(*units);
            }
            app.cell_px = term::cell_px();
            // the resize clears the screen, and with it every placement
            if let Some(k) = app.kitty.as_mut() {
                k.reset(&mut stdout())?;
            }
        }
        _ => {}
    }
    app.input_at = None;
    Ok(())
}

/// Two pointer moves of the same kind (both hovering, or dragging with the same button and keys): the later one
/// replaces the earlier.
fn same_kind(a: &TermEvent, b: &TermEvent) -> bool {
    matches!((a, b), (TermEvent::Mouse(x), TermEvent::Mouse(y)) if x.kind == y.kind && x.modifiers == y.modifiers)
}

/// Open the pairing page in the user's browser (the code is also shown in the UI): a fixed system location, never a
/// `PATH` lookup (audit S3), and without the Daemon's secret (S10).
fn open_in_browser(url: &str) {
    if let Some(opener) = opener() {
        let mut cmd = std::process::Command::new(opener);
        cmd.env_remove(conn::SECRET_ENV).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        let _ = cmd.spawn();
    }
}

/// The browser opener at a fixed system location: `open` on a Mac, `xdg-open` on Linux.
fn opener() -> Option<std::path::PathBuf> {
    let name = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    ["/usr/bin", "/bin", "/usr/local/bin"].iter().map(|d| std::path::Path::new(d).join(name)).find(|p| p.is_file())
}

fn cache_dir() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("singularity")
}
