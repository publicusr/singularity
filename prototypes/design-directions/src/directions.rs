//! The six design directions (round 2). All OLED black. Each is a full identity: palette, border
//! style, layout, and a curated set of visuals per slot (see `app::slot_options`).

use crate::app::App;
use crate::gfx::{self, Rgb};
use crate::mock;
use crate::ui::{self, Progress, Theme};
use crate::viz;
use ratatui::Frame;
use ratatui::layout::{Constraint::*, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;

/// Colour variants a design offers (t cycles). Everything (spectrum, graphs, borders, accents) derives
/// from the variant, so recolouring is one switch, not per-widget fiddling.
const COCKPIT_VARIANTS: [(&str, [u32; 3], [u32; 4], u32); 5] = [
    // name, ramp low→high, box borders (spectrum, stream, instruments, lists), accent
    ("btop", [0x77ca9b, 0xcbc06c, 0xdc4c4c], [0x556d59, 0x5c588d, 0x6c6c4b, 0x805252], 0xeeeeee),
    ("ice", [0x1f4f7a, 0x5fb4ff, 0xbfe6ff], [0x1d3b57, 0x24486b, 0x1f4560, 0x2a4f73], 0xbfe6ff),
    ("amber", [0x7a3d00, 0xffa031, 0xffe0a3], [0x5a3a14, 0x6b4416, 0x5e4214, 0x6e3f12], 0xffd28a),
    ("violet", [0x3d2a7a, 0xa98bff, 0xf0d6ff], [0x3a2d63, 0x463572, 0x3b3166, 0x4b3478], 0xe2d4ff),
    ("mono", [0x3a3a3a, 0x9a9a9a, 0xf0f0f0], [0x2e2e2e, 0x343434, 0x2e2e2e, 0x383838], 0xf0f0f0),
];
const TERMINAL_VARIANTS: [(&str, u32, u32, u32); 3] = [
    ("phosphor green", 0x33ff66, 0x7dff9a, 0x15401f),
    ("amber crt", 0xffb000, 0xffd27a, 0x4a3300),
    ("ice", 0x5fd7ff, 0xa8ecff, 0x123a4a),
];

pub fn variant_count(dir: usize) -> usize {
    match dir {
        2 => COCKPIT_VARIANTS.len(),
        4 => TERMINAL_VARIANTS.len(),
        _ => 1,
    }
}

pub fn variant_name(dir: usize, v: usize) -> &'static str {
    match dir {
        2 => COCKPIT_VARIANTS[v % COCKPIT_VARIANTS.len()].0,
        4 => TERMINAL_VARIANTS[v % TERMINAL_VARIANTS.len()].0,
        _ => "default",
    }
}

pub fn theme(app: &App) -> Theme {
    let h = Rgb::hex;
    let black = h(0x000000);
    let v = app.variant[app.dir];
    match app.dir {
        // cream + ice: the colours of the disk itself (blackbody cream, Doppler-blue approaching side)
        0 => Theme {
            bg: black, panel: black, fg: h(0xe8e6e3), dim: h(0x5f5d5a), accent: h(0xf2d3a0),
            accent2: h(0x9cc4ff), border: h(0x1e1e1e), sel: h(0x161412),
            ramp: vec![h(0x3a2c1c), h(0xa8783e), h(0xf2d3a0), h(0xf8ead0)], borders: border::ROUNDED,
            boxes: [h(0x1e1e1e); 4],
        },
        1 => Theme {
            bg: black, panel: black, fg: h(0xffffff), dim: h(0xa7a7a7), accent: h(0x1db954),
            accent2: h(0x1ed760), border: h(0x1f1f1f), sel: h(0x1a1a1a),
            ramp: vec![h(0x0d4a24), h(0x169c46), h(0x1db954), h(0x4fe387)], borders: border::ROUNDED,
            boxes: [h(0x1f1f1f); 4],
        },
        2 => {
            let (_, ramp, boxes, acc) = COCKPIT_VARIANTS[v % COCKPIT_VARIANTS.len()];
            Theme {
                bg: black, panel: black, fg: h(0xcccccc), dim: h(0x6a6a6a), accent: h(acc),
                accent2: h(ramp[2]), border: h(boxes[3]), sel: h(boxes[3]).scale(0.7),
                ramp: ramp.iter().map(|c| h(*c)).collect(), borders: border::ROUNDED,
                boxes: [h(boxes[0]), h(boxes[1]), h(boxes[2]), h(boxes[3])],
            }
        }
        // the synthesis: terminal-driven layout, cockpit instruments, a gravity well
        3 => Theme {
            bg: black, panel: black, fg: h(0xd6d6d6), dim: h(0x585858), accent: h(0xf2d3a0),
            accent2: h(0x8fb8ff), border: h(0x2a2a2a), sel: h(0x141414),
            ramp: vec![h(0x26324d), h(0x5f86c9), h(0xb9c9e6), h(0xf2d3a0)], borders: border::PLAIN,
            boxes: [h(0x2a2a2a); 4],
        },
        _ => {
            let (_, acc, fg, bord) = TERMINAL_VARIANTS[v % TERMINAL_VARIANTS.len()];
            let a = h(acc);
            Theme {
                bg: black, panel: black, fg: h(fg), dim: a.scale(0.42), accent: a,
                accent2: h(fg).lerp(h(0xffffff), 0.4), border: h(bord), sel: h(bord).scale(0.6),
                ramp: vec![a.scale(0.3), a.scale(0.65), a, h(fg).lerp(h(0xffffff), 0.3)], borders: border::PLAIN,
                boxes: [h(bord); 4],
            }
        }
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let th = theme(app);
    let area = f.area();
    gfx::fill(f.buffer_mut(), area, th.bg.color());
    let np_h = if app.dir == 2 { 3 } else { 4 };
    let [top, body, bottom, hints] = Layout::vertical([Length(1), Fill(1), Length(np_h), Length(1)]).areas(area);
    ui::top_bar(f, top, app, &th);
    match app.dir {
        0 => event_horizon(f, body, bottom, app, &th),
        1 => classic(f, body, bottom, app, &th),
        2 => cockpit(f, body, bottom, app, &th),
        3 => singularity(f, body, bottom, app, &th),
        _ => terminal(f, body, bottom, app, &th),
    }
    if app.idle() {
        ui::idle_hole(f, body, app, 6);
    }
    ui::hint_bar(f, hints, &th, ui::HINTS);
    if app.palette.is_some() {
        ui::palette(f, area, app, &th);
    }
    if app.help {
        ui::help(f, area, &th);
    }
    if let Some((msg, _)) = app.toast.clone() {
        ui::toast(f, area, &th, &msg);
    }
}

fn boxed(f: &mut Frame, th: &Theme, r: Rect, title: &str, col: Option<usize>) -> Rect {
    let mut b = th.block(title);
    if let Some(i) = col {
        b = b.border_style(Style::default().fg(th.boxes[i % 4].color()));
    }
    let inner = b.inner(r);
    f.render_widget(b, r);
    inner
}

/// 1 · Event Horizon: OLED black, cream + ice. The black hole owns the top-right quadrant.
fn event_horizon(f: &mut Frame, body: Rect, np: Rect, app: &mut App, th: &Theme) {
    let [side, main, right] = Layout::horizontal([Length(28), Fill(1), Percentage(46)]).areas(body);
    ui::sidebar(f, side, app, th, "library", false);
    let [home, queue] = Layout::vertical([Percentage(62), Fill(1)]).areas(main);
    ui::home(f, home, app, th, "home", true);
    ui::queue(f, queue, app, th, "up next", false);
    let [hero, lyr] = Layout::vertical([Percentage(64), Fill(1)]).areas(right);
    let kind = app.hero();
    ui::hero(f, hero, app, th, kind, true);
    let inner = boxed(f, th, lyr, "lyrics", None);
    ui::lyrics(f.buffer_mut(), inner, app, th, true, 1);
    ui::now_playing(f, np, app, th, Progress::Spectrum, true);
}

/// 2 · Classic+: the Spotify desktop app on OLED black.
fn classic(f: &mut Frame, body: Rect, np: Rect, app: &mut App, th: &Theme) {
    let [side, main, right] = Layout::horizontal([Length(30), Fill(1), Length(38)]).areas(body);
    ui::sidebar(f, side, app, th, "Your Library", false);
    ui::home(f, main, app, th, "Home", true);
    let inner = boxed(f, th, right, "Now playing", None);
    let tr = app.track();
    let cw = inner.width.saturating_sub(2);
    let ch = (cw / 2).min(inner.height.saturating_sub(14));
    let cover = Rect::new(inner.x + 1, inner.y, cw, ch);
    app.pixel_pane(f.buffer_mut(), cover, 901, false, 480, |fb| viz::cover(fb, tr.cover, 0.0));
    let buf = f.buffer_mut();
    let mut y = cover.bottom() + 1;
    ui::text(buf, inner.x + 1, y, cw, tr.title, th.s(th.fg).add_modifier(Modifier::BOLD));
    ui::text(buf, inner.x + 1, y + 1, cw, tr.artist, th.s(th.dim));
    y += 3;
    let viz_h = 6.min(inner.bottom().saturating_sub(y));
    let vr = Rect::new(inner.x + 1, y, cw, viz_h);
    let kind = app.hero();
    ui::hero(f, vr, app, th, kind, false);
    y += viz_h + 1;
    let buf = f.buffer_mut();
    if y < inner.bottom() {
        ui::text(buf, inner.x + 1, y, cw, "Next in queue", th.s(th.fg).add_modifier(Modifier::BOLD));
        y += 1;
        for t in app.upcoming().iter().take(inner.bottom().saturating_sub(y) as usize) {
            let [c1, ..] = viz::cover_palette(t.cover);
            ui::text(buf, inner.x + 1, y, 2, "■", th.s(c1));
            ui::text(buf, inner.x + 3, y, cw - 2, &format!("{} · {}", t.title, t.artist), th.s(th.dim));
            y += 1;
        }
    }
    ui::now_playing(f, np, app, th, Progress::Line, true);
}

/// 3 · Cockpit: btop energy on OLED black.
fn cockpit(f: &mut Frame, body: Rect, np: Rect, app: &mut App, th: &Theme) {
    let [r1, r2, r3] = Layout::vertical([Percentage(44), Percentage(24), Fill(1)]).areas(body);
    let [spec, stats] = Layout::horizontal([Percentage(66), Fill(1)]).areas(r1);
    let [loud, vu, heat] = Layout::horizontal([Percentage(30), Percentage(25), Fill(1)]).areas(r2);
    let [queue, lib] = Layout::horizontal([Percentage(62), Fill(1)]).areas(r3);
    let i = boxed(f, th, spec, "¹spectrum", Some(0));
    let kind = app.hero();
    ui::hero(f, i, app, th, kind, true);
    let i = boxed(f, th, stats, "²stream", Some(1));
    stream_stats(f, i, app, th);
    let i = boxed(f, th, loud, "³loudness · 60s", Some(2));
    app.history.render(f.buffer_mut(), i, &th.ramp);
    let i = boxed(f, th, vu, "⁴levels", Some(2));
    let fr = app.frame.clone();
    viz::vu(f.buffer_mut(), Rect::new(i.x, i.y + 1, i.width, i.height.saturating_sub(1)), &fr, &th.ramp, th.dim.scale(0.5).color());
    for (k, (name, v)) in [("bass", fr.bass), ("mid", fr.mid), ("high", fr.treble)].iter().enumerate() {
        let y = i.y + 5 + k as u16;
        if y < i.bottom() {
            let w = i.width.saturating_sub(7);
            let lit = (v * w as f32) as u16;
            let bar: String = (0..w).map(|x| if x < lit { '■' } else { '·' }).collect();
            ui::text(f.buffer_mut(), i.x, y, 6, name, th.s(th.dim));
            ui::text(f.buffer_mut(), i.x + 6, y, w, &bar, th.s(gfx::ramp(&th.ramp, *v)));
        }
    }
    let i = boxed(f, th, heat, "⁵listening · last 26 weeks", Some(3));
    viz::heatmap(f.buffer_mut(), Rect::new(i.x + 1, i.y + 1, i.width.saturating_sub(2), 7), &th.ramp, Rgb::hex(0x141414).color());
    if i.height > 9 {
        ui::text(f.buffer_mut(), i.x + 1, i.y + 9, i.width, "412 h this year · top: Nova Drift · streak 23 days", th.s(th.dim));
    }
    ui::queue(f, queue, app, th, "⁶queue", true);
    ui::sidebar(f, lib, app, th, "⁷library", false);
    ui::now_playing(f, np, app, th, Progress::Waveform, false);
}

fn stream_stats(f: &mut Frame, r: Rect, app: &App, th: &Theme) {
    let buf = f.buffer_mut();
    let kbps = 1650.0 + (app.t * 0.3).sin() * 240.0;
    let buffer = 0.78 + (app.t * 0.2).sin() * 0.1;
    let rows: Vec<(&str, String)> = vec![
        ("engine", "Soloist 1.3.7 · official".into()),
        ("codec", "FLAC · 24-bit / 44.1 kHz".into()),
        ("bitrate", format!("{kbps:.0} kbps")),
        ("buffer", format!("{} {:.0}%", bar(buffer, 10), buffer * 100.0)),
        ("normal.", "-3.1 dB · loud".into()),
        ("output", "PipeWire → Built-in Audio".into()),
        ("latency", "18 ms".into()),
        ("cache", "412 MB / 2.0 GB".into()),
        ("device", "singularity · active".into()),
        ("source", if app.frame.live { "live monitor".into() } else { "synthetic".into() }),
    ];
    let badge = " ◆ LOSSLESS ";
    ui::text(buf, r.right().saturating_sub(badge.len() as u16 + 1), r.y, 14, badge, Style::default().fg(Color::Black).bg(th.ramp[0].color()).add_modifier(Modifier::BOLD));
    for (i, (k, v)) in rows.iter().enumerate() {
        let y = r.y + 1 + i as u16;
        if y >= r.bottom() {
            break;
        }
        ui::text(buf, r.x + 1, y, 9, k, th.s(th.dim));
        ui::text(buf, r.x + 10, y, r.width.saturating_sub(11), v, th.s(th.fg));
    }
}

fn bar(v: f32, w: usize) -> String {
    let k = (v.clamp(0.0, 1.0) * w as f32) as usize;
    "█".repeat(k) + &"░".repeat(w - k)
}

fn lyrics_log(f: &mut Frame, li: Rect, app: &App, th: &Theme) {
    let cur = mock::lyric_index(app.pos);
    let prog = mock::lyric_progress(app.pos);
    let first = (cur + 1).saturating_sub(li.height as usize);
    let buf = f.buffer_mut();
    for (row, i) in (first..=cur).enumerate() {
        let (s, line) = mock::LYRICS[i];
        let ts = format!("[{:02}:{:02}]", s / 60, s % 60);
        let n = line.chars().count();
        let shown: String = if i == cur { line.chars().take(((prog * 2.5).min(1.0) * n as f32) as usize).collect() } else { line.to_string() };
        let y = li.y + row as u16;
        ui::text(buf, li.x, y, 8, &ts, th.s(th.dim));
        ui::text(buf, li.x + 8, y, li.width.saturating_sub(8), &format!(" > {shown}"), th.s(if i == cur { th.accent } else { th.fg }));
        if i == cur && (app.t * 2.0) as i32 % 2 == 0 {
            ui::text(buf, li.x + 11 + shown.chars().count() as u16, y, 1, "█", th.s(th.accent));
        }
    }
}

fn prompt_player(f: &mut Frame, np: Rect, app: &mut App, th: &Theme, host: &str) {
    let tr = app.track();
    let buf = f.buffer_mut();
    gfx::fill(buf, np, th.bg.color());
    let p = format!("{host}:~$ ");
    let pw = p.chars().count() as u16;
    ui::text(buf, np.x + 1, np.y, np.width, &p, th.s(th.accent).add_modifier(Modifier::BOLD));
    ui::text(buf, np.x + 1 + pw, np.y, np.width.saturating_sub(pw + 1), &format!("now-playing --follow  # {} — {}", tr.title, tr.artist), th.s(th.fg));
    let rest = Rect::new(np.x, np.y + 1, np.width, np.height.saturating_sub(1));
    ui::now_playing(f, rest, app, th, Progress::Thin, false);
}

/// 4 · Singularity: the synthesis. Terminal-driven layout and cockpit instruments around a gravity
/// well: the black hole has its own pane (nothing important is ever under it), and its gravity bends
/// the borders of the panes around it. Content never moves. v changes the disk look.
fn singularity(f: &mut Frame, body: Rect, np: Rect, app: &mut App, th: &Theme) {
    let [left, mid, right] = Layout::horizontal([Length(30), Fill(1), Percentage(38)]).areas(body);
    let [tree, stats] = Layout::vertical([Fill(1), Length(13)]).areas(left);
    ui::sidebar(f, tree, app, th, "~/library", true);
    let i = boxed(f, th, stats, "stream", None);
    stream_stats(f, i, app, th);
    let [home, queue] = Layout::vertical([Percentage(52), Fill(1)]).areas(mid);
    ui::home(f, home, app, th, "home", false);
    ui::queue(f, queue, app, th, "queue", true);
    let [well, lyr, low] = Layout::vertical([Percentage(44), Percentage(32), Fill(1)]).areas(right);
    let [loud, fft] = Layout::horizontal([Percentage(50), Fill(1)]).areas(low);
    let li = boxed(f, th, lyr, "tail -f lyrics.log", None);
    lyrics_log(f, li, app, th);
    let i = boxed(f, th, loud, "loudness", None);
    app.history.render(f.buffer_mut(), i, &th.ramp);
    let i = boxed(f, th, fft, "fft", None);
    let fr = app.frame.clone();
    viz::spectrum(f.buffer_mut(), i, &fr, &th.ramp, false, true, false);
    prompt_player(f, np, app, th, "singularity");
    let look = match app.hero() {
        crate::app::Hero::BlackHole(l) => l,
        _ => 0,
    };
    ui::gravity_well(f, well, app, th, look);
}

/// 5 · Terminal: green phosphor on OLED black, file-tree library, rain, lyrics as a live log, scanlines.
fn terminal(f: &mut Frame, body: Rect, np: Rect, app: &mut App, th: &Theme) {
    let [side, mid, right] = Layout::horizontal([Length(30), Fill(1), Length(36)]).areas(body);
    ui::sidebar(f, side, app, th, "tree", true);
    let [hero, log] = Layout::vertical([Percentage(55), Fill(1)]).areas(mid);
    let hi = boxed(f, th, hero, "rain --audio", None);
    let kind = app.hero();
    ui::hero(f, hi, app, th, kind, true);
    let li = boxed(f, th, log, "tail -f lyrics.log", None);
    lyrics_log(f, li, app, th);
    let [sc, procs, sp] = Layout::vertical([Percentage(35), Percentage(30), Fill(1)]).areas(right);
    let si = boxed(f, th, sc, "osc", None);
    let fr = app.frame.clone();
    app.scope.render(f.buffer_mut(), si, &fr, &th.ramp);
    let pi = boxed(f, th, procs, "top", None);
    let rows = [
        ("PID", "CPU%".to_string(), "MEM", "CMD"),
        ("4211", format!("{:.1}", 2.0 + fr.energy * 2.0), "61M", "soloist"),
        ("4188", format!("{:.1}", 3.0 + fr.energy * 3.0), "38M", "singularityd"),
        ("4190", "0.8".to_string(), "24M", "singularity"),
        ("1290", "1.2".to_string(), "22M", "pipewire"),
    ];
    let buf = f.buffer_mut();
    for (i, (a, b2, c, d)) in rows.iter().enumerate() {
        let y = pi.y + i as u16;
        if y < pi.bottom() {
            let st = if i == 0 { th.s(th.dim).add_modifier(Modifier::BOLD) } else { th.s(th.fg) };
            ui::text(buf, pi.x, y, pi.width, &format!("{a:<6}{b2:>5} {c:>5} {d}"), st);
        }
    }
    let fi = boxed(f, th, sp, "fft", None);
    viz::spectrum(f.buffer_mut(), fi, &fr, &th.ramp, false, true, false);
    prompt_player(f, np, app, th, "singularity@spotify");
    // scanlines across the body
    let buf = f.buffer_mut();
    for y in (body.y..body.bottom()).filter(|y| y % 2 == 1) {
        for x in body.x..body.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                if let Color::Rgb(r, g, b) = c.fg {
                    c.set_fg(Color::Rgb((r as f32 * 0.75) as u8, (g as f32 * 0.75) as u8, (b as f32 * 0.75) as u8));
                }
            }
        }
    }
}
