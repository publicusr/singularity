//! Singularity design-direction prototype. Throwaway code: its only job is to be looked at.

mod app;
mod audio;
mod blackhole;
mod directions;
mod gfx;
mod gpu;
mod mock;
mod ui;
mod viz;

use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use ratatui::crossterm::{execute, terminal};
use std::io::{Write, stdout};
use std::time::{Duration, Instant};

fn detect_kitty() -> bool {
    let term = std::env::var("TERM").unwrap_or_default();
    let prog = std::env::var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    std::env::var("SINGULARITY_NO_PIXEL").is_err()
        && (term.contains("kitty")
            || term.contains("ghostty")
            || prog.contains("ghostty")
            || prog.contains("wezterm")
            || std::env::var("KITTY_WINDOW_ID").is_ok()
            || std::env::var("GHOSTTY_RESOURCES_DIR").is_ok())
}

fn cell_px() -> (f32, f32) {
    match terminal::window_size() {
        Ok(ws) if ws.width > 0 && ws.columns > 0 => (ws.width as f32 / ws.columns as f32, ws.height as f32 / ws.rows as f32),
        _ => (9.0, 18.0),
    }
}

fn main() -> std::io::Result<()> {
    if let Some(dir) = std::env::args().nth(2).filter(|_| std::env::args().nth(1).as_deref() == Some("--snapshot")) {
        return snapshot(&dir);
    }
    let mut term = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    let mut app = app::App::new(detect_kitty(), cell_px());
    gfx::USE_SHM.store(
        std::env::var("KITTY_WINDOW_ID").is_ok() && std::env::var("SSH_CONNECTION").is_err() && std::env::var("SINGULARITY_NO_SHM").is_err(),
        std::sync::atomic::Ordering::Relaxed,
    );
    if let Some(g) = &app.gpu {
        app.say(format!("GPU: {}", g.adapter_name));
    }
    let frame_budget = Duration::from_millis(30);
    let mut last = Instant::now();

    let res = (|| -> std::io::Result<()> {
        while !app.quit {
            let now = Instant::now();
            let dt = (now - last).as_secs_f32().min(0.1);
            last = now;
            app.tick(dt);
            app.begin_frame();

            let mut out = stdout().lock();
            out.write_all(b"\x1b[?2026h")?; // synchronized output: no tearing
            drop(out);
            term.draw(|f| directions::draw(f, &mut app))?;
            let mut out = stdout().lock();
            for img in app.kitty_queue.drain(..) {
                gfx::write_kitty(&mut out, &img)?;
                if let Some(k) = app.kitty_keys_this_frame.get(&img.id) {
                    app.kitty_sent.insert(img.id, *k);
                }
            }
            let stale: Vec<u32> = app.kitty_sent.keys().filter(|id| !app.kitty_keys_this_frame.contains_key(id)).cloned().collect();
            for id in stale {
                gfx::kitty_delete(&mut out, id)?;
                app.kitty_sent.remove(&id);
            }
            out.write_all(b"\x1b[?2026l")?;
            out.flush()?;
            drop(out);

            let deadline = now + frame_budget;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if !event::poll(left)? {
                    break;
                }
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => app.key(k),
                    Event::Mouse(m) => app.mouse(m),
                    Event::Resize(..) => {
                        app.cell_px = cell_px();
                        app.kitty_sent.clear();
                        gfx::kitty_delete_all(&mut stdout())?;
                    }
                    _ => {}
                }
                if app.quit {
                    break;
                }
            }
        }
        Ok(())
    })();

    let _ = gfx::kitty_delete_all(&mut stdout());
    gfx::shm_cleanup();
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    res
}

/// Headless: render every direction (cell tier) to cell dumps + the pixel visuals to PPM, for review.
fn snapshot(dir: &str) -> std::io::Result<()> {
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    std::fs::create_dir_all(dir)?;
    let (w, h) = (std::env::var("W").ok().and_then(|v| v.parse().ok()).unwrap_or(200u16), std::env::var("H").ok().and_then(|v| v.parse().ok()).unwrap_or(54u16));
    let mut app = app::App::new(false, (9.0, 18.0));
    app.audio.force_synthetic = true;
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("test backend");
    for d in 0..ui::NAMES.len() + 2 {
        // two extra shots: Cockpit "ice" and Terminal "amber" palette variants
        app.dir = match d { 5 => 2, 6 => 4, _ => d };
        if d >= 5 {
            app.variant[app.dir] = 1;
        }
        for _ in 0..90 {
            app.tick(1.0 / 30.0);
            app.begin_frame();
            term.draw(|f| directions::draw(f, &mut app)).expect("draw");
        }
        let buf = term.backend().buffer();
        let col = |c: Color, dflt: &str| match c {
            Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
            Color::Reset => dflt.to_string(),
            other => format!("{other:?}"),
        };
        let mut out = String::new();
        for y in 0..h {
            for x in 0..w {
                let c = &buf[(x, y)];
                out.push_str(&format!("{}\t{}\t{}\t{}\n", c.symbol().replace('\t', " "), col(c.fg, "dddddd"), col(c.bg, "000000"), c.modifier.contains(ratatui::style::Modifier::BOLD) as u8));
            }
        }
        std::fs::write(format!("{dir}/dir{}.cells", d + 1), format!("{w} {h}\n{out}"))?;
        app.variant = [0; 6];
    }
    // idle break-out, ~2 s in (half way out of the gravity well) and fully out
    for (name, ticks) in [("idle_mid", 60), ("idle_full", 150)] {
        app.dir = 3;
        app.idle_forced = true;
        app.idle_t = 0.0;
        for _ in 0..ticks {
            app.tick(1.0 / 30.0);
            app.begin_frame();
            term.draw(|f| directions::draw(f, &mut app)).expect("draw");
        }
        let buf = term.backend().buffer();
        let col = |c: ratatui::style::Color| match c {
            ratatui::style::Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
            _ => "000000".into(),
        };
        let mut out = String::new();
        for y in 0..h {
            for x in 0..w {
                let c = &buf[(x, y)];
                out.push_str(&format!("{}\t{}\t{}\t0\n", c.symbol().replace('\t', " "), col(c.fg), col(c.bg)));
            }
        }
        std::fs::write(format!("{dir}/{name}.cells"), format!("{w} {h}\n{out}"))?;
    }
    app.idle_forced = false;
    let fr = app.frame.clone();
    let write_ppm = |name: &str, fb: &gfx::Fb| -> std::io::Result<()> {
        let mut data = format!("P6 {} {} 255\n", fb.w, fb.h).into_bytes();
        for p in &fb.px {
            data.extend_from_slice(&p.to_u8());
        }
        std::fs::write(format!("{dir}/{name}.ppm"), data)
    };
    for (i, look) in app::LOOKS.iter().enumerate() {
        let mut fb = gfx::Fb::new(900, 500);
        app.dir = 0;
        let rh = blackhole::fit_rh(fb.w, fb.h, look);
        blackhole::render(&mut fb, app.phase, &fr, &app.bh_style(i, rh, (0.5, 0.5), false));
        write_ppm(&format!("bh{i}"), &fb)?;
        let st = app.bh_style(i, rh, (0.5, 0.5), false);
        let phase = app.phase;
        if let Some(g) = app.gpu.as_mut() {
            let mut gfb = gfx::Fb::new(900, 500);
            let levels = if look.name == "pixel ember" { 5.0 } else { 0.0 };
            let t0 = std::time::Instant::now();
            g.render(&mut gfb, phase, &fr, &st, levels);
            eprintln!("gpu {} 900x500: {:?}", look.name, t0.elapsed());
            let mut data = format!("P6 {} {} 255\n", gfb.w, gfb.h).into_bytes();
            for px in gfb.raw.chunks(4) {
                data.extend_from_slice(&px[..3]);
            }
            std::fs::write(format!("{dir}/gpu_bh{i}.ppm"), data)?;
        }
    }
    for (i, seed) in [10u32, 21, 32, 43, 54].iter().enumerate() {
        let mut fb = gfx::Fb::new(200, 200);
        viz::cover(&mut fb, *seed, 0.0);
        write_ppm(&format!("cover{i}"), &fb)?;
    }
    Ok(())
}
