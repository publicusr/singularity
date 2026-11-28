//! `singularity`: the terminal Client.

mod app;
mod openlist;
mod pane;
mod receive;
mod arrange;
mod arrangement;
mod artist;
mod bigtext;
mod cli;
mod conn;
mod ambient;
mod balance;
mod covers;
mod demo;
mod design;
mod gesture;
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
mod needs;
mod pace;
mod panemenu;
mod paneless;
mod cover;
mod playlists;
mod queue;
mod scroll;
mod screen_plan;
mod scrub;
mod saved;
mod search;
mod settings;
mod shown;
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
mod widget_catalogue;
mod widgets;

use app::App;
use ratatui::crossterm::event::{self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, Event as TermEvent, KeyEventKind, MouseEventKind};
use ratatui::crossterm::execute;
use std::io::{Write, stdout};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let demo_env = std::env::var_os("SINGULARITY_DEMO").map(|v| v.to_string_lossy().into_owned());
    let demo = match cli::read(&args, demo_env.as_deref()) {
        cli::Start::Run { demo } => demo,
        cli::Start::Version => {
            println!("singularity {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        cli::Start::Help => {
            print!("{}", cli::HELP);
            return Ok(());
        }
        cli::Start::Wrong(what) => {
            eprintln!("singularity: {what} (see singularity --help)");
            std::process::exit(2);
        }
    };
    // nowhere to keep settings and data without HOME: stop and say so (Q93), before a Daemon is started
    let folders = match singularity_config::app_folders::AppFolders::from_env() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("singularity: {e}");
            std::process::exit(1);
        }
    };
    // demo mode (D90): before anything is read (the recent searches have their own file) or sent (the Hello says so)
    if demo {
        demo::turn_on();
    }
    let (cmd_tx, mut conn_rx) = conn::spawn();
    let mut app = App::new(Some(cmd_tx));
    app.search = search::Search::with_recent(search::load_recent());
    app.covers = covers::Covers::spawn(folders.cache.join("covers"));
    app.cell_px = term::cell_px();
    app.hole.allow_gpu = true;
    // (demo mode first: what loading the config says is redacted too)
    if demo::on() {
        app.start_demo(demo::file_path());
    }
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
    let mut wheel = term::WheelSteps::default();
    let mut pacer = pace::Pacer::new(Instant::now());
    // `SINGULARITY_FRAME_LOG=<file>`: a line per frame (when it started, its rate, how long its parts took)
    let mut log = std::env::var_os("SINGULARITY_FRAME_LOG").and_then(|p| std::fs::File::create(p).ok()).map(std::io::BufWriter::new);
    let t0 = Instant::now();
    let res = (|| -> anyhow::Result<()> {
        // a pointer move waits for the frame: only the last one before it counts (a mouse sends hundreds a second)
        let mut moves = pace::Moves::default();
        while !app.quit {
            // until the frame is due, input is handled as it comes
            let (due, fps) = loop {
                let fps = app.frame_rate(focused);
                let due = pacer.due(fps);
                let now = Instant::now();
                let Some(wait) = due.checked_duration_since(now).filter(|w| !w.is_zero()) else { break (due, fps) };
                // a group of wheel reports under way ends when no report has come in its time (judged by when the
                // reports came, not by when they're handled: a slow frame mustn't split a notch's three)
                let group = wheel.deadline().map(|d| d.saturating_duration_since(now));
                let (at, ev) = match input.recv_timeout(group.map_or(wait, |g| g.min(wait))) {
                    Ok(x) => x,
                    Err(mpsc::RecvTimeoutError::Timeout) if group.is_some_and(|g| g < wait) => {
                        // the steps it held back (a touchpad's), now
                        steps(&mut app, wheel.end(), Instant::now(), &units);
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => break (due, fps),
                    Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("the terminal input closed"),
                };
                let ev = ev?;
                // input brings the frame forward: to the motion rate, or for a bare hover at most 60 a second
                let hover = matches!(&ev, TermEvent::Mouse(m) if m.kind == MouseEventKind::Moved) && !app.moving_now();
                pacer.input(if hover { 60.min(app.motion_fps()) } else { app.motion_fps() });
                for (at, ev) in moves.push(at, ev).into_iter().flatten() {
                    handle(&mut app, ev, at, &mut focused, &mut units, &mut wheel)?;
                }
            };
            if let Some((at, ev)) = moves.flush() {
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
            let mut framed = Duration::ZERO;
            tui.draw(|f| {
                let t = Instant::now();
                ui::frame(f, &mut app);
                framed = t.elapsed();
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
                // start, rate, then µs: tick, the frame (App::frame_begins and ui::draw), diff and write, images and flush
                writeln!(l, "{} {fps} {} {} {} {}", us(start - t0), us(ticked), us(framed), us(drawn.saturating_sub(ticked + framed)), us(start.elapsed().saturating_sub(drawn)))?;
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
fn handle(app: &mut App, ev: TermEvent, at: Instant, focused: &mut bool, units: &mut term::MouseUnits, wheel: &mut term::WheelSteps) -> anyhow::Result<()> {
    app.input_at = Some(at);
    match ev {
        TermEvent::Key(k) if k.kind != KeyEventKind::Release => app.key(k),
        TermEvent::Mouse(m) => steps(app, wheel.push(m, at), at, units),
        TermEvent::Paste(s) => app.paste(&s),
        TermEvent::FocusLost => {
            *focused = false;
            app.pointer_gone();
        }
        TermEvent::FocusGained => *focused = true,
        TermEvent::Resize(w, h) => {
            // the borders move: the one under the pointer is found again on its next move
            app.pointer_gone();
            app.window_resized(w, h);
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

/// Mouse events to act on (a wheel report can be none, or come after steps held back, `term::WheelSteps`).
fn steps(app: &mut App, events: Vec<ratatui::crossterm::event::MouseEvent>, at: Instant, units: &term::MouseUnits) {
    for m in events {
        app.input_at = Some(at);
        let (m, fx, precise) = units.to_cells(m);
        app.pointer(m, fx, precise);
        app.input_at = None;
    }
}

/// Open the pairing page in the user's browser (the code is also shown in the UI): the Host's opener, from a fixed
/// system location, never a `PATH` lookup (audit S3), and without the Daemon's secret (S10).
fn open_in_browser(url: &str) {
    singularity_config::host::Host::this().open(url);
}
