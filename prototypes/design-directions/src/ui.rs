//! Shared UI components. Every interactive element registers a hit-box so the mouse works everywhere.

use crate::app::{Act, App, Hero};
use crate::blackhole;
use crate::gfx::{self, Rgb};
use crate::mock::{self, fmt_time};
use crate::viz;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub struct Theme {
    pub bg: Rgb,
    pub panel: Rgb,
    pub fg: Rgb,
    pub dim: Rgb,
    pub accent: Rgb,
    pub accent2: Rgb,
    pub border: Rgb,
    pub sel: Rgb,
    pub ramp: Vec<Rgb>,
    pub borders: border::Set<'static>,
    /// per-box border colours (btop style), derived from the palette variant
    pub boxes: [Rgb; 4],
}

impl Theme {
    pub fn s(&self, c: Rgb) -> Style {
        Style::default().fg(c.color())
    }
    pub fn panel_style(&self) -> Style {
        Style::default().fg(self.fg.color()).bg(self.panel.color())
    }
    pub fn block<'a>(&self, title: &'a str) -> Block<'a> {
        Block::default()
            .borders(Borders::ALL)
            .border_set(self.borders)
            .border_style(self.s(self.border))
            .title(Span::styled(format!(" {title} "), self.s(self.accent).add_modifier(Modifier::BOLD)))
            .style(self.panel_style())
    }
}

pub fn text(buf: &mut Buffer, x: u16, y: u16, max: u16, s: &str, st: Style) {
    if max > 0 {
        buf.set_stringn(x, y, s, max as usize, st);
    }
}

// ───────────────────────────── chrome ─────────────────────────────

pub const NAMES: [&str; 5] = ["Event Horizon", "Classic+", "Cockpit", "Singularity", "Terminal"];

pub fn top_bar(f: &mut Frame, r: Rect, app: &mut App, th: &Theme) {
    let buf = f.buffer_mut();
    gfx::fill(buf, r, th.bg.color());
    let mut x = r.x + 1;
    text(buf, x, r.y, 14, "◉ SINGULARITY", th.s(th.accent).add_modifier(Modifier::BOLD));
    x += 15;
    for (i, name) in NAMES.iter().enumerate() {
        let label = if r.width > 110 { format!(" {} {} ", i + 1, name) } else { format!(" {} ", i + 1) };
        let w = label.chars().count() as u16;
        if x + w >= r.right().saturating_sub(30) {
            break;
        }
        let st = if i == app.dir {
            Style::default().fg(th.bg.color()).bg(th.accent.color()).add_modifier(Modifier::BOLD)
        } else {
            th.s(th.dim)
        };
        text(buf, x, r.y, w, &label, st);
        app.hit(Rect::new(x, r.y, w, 1), Act::Dir(i));
        x += w + 1;
    }
    let chips = [
        (if app.frame.live { "♪ live audio" } else { "♪ synthetic" }, Act::ToggleAudio),
        (if app.pixel_active() { "▣ pixel tier" } else { "▤ cell tier" }, Act::TogglePixel),
    ];
    let mut cx = r.right().saturating_sub(2);
    let fps = format!("{:>3.0}fps", app.fps);
    cx = cx.saturating_sub(fps.len() as u16);
    text(buf, cx, r.y, 7, &fps, th.s(th.dim));
    for (label, act) in chips.iter().rev() {
        let w = label.chars().count() as u16 + 2;
        cx = cx.saturating_sub(w + 1);
        text(buf, cx, r.y, w, &format!(" {label} "), Style::default().fg(th.fg.color()).bg(th.sel.color()));
        app.hit(Rect::new(cx, r.y, w, 1), act.clone());
    }
}

pub fn hint_bar(f: &mut Frame, r: Rect, th: &Theme, hints: &[(&str, &str)]) {
    let buf = f.buffer_mut();
    gfx::fill(buf, r, th.bg.color());
    let mut spans = vec![Span::raw(" ")];
    for (k, d) in hints {
        spans.push(Span::styled(format!(" {k} "), Style::default().fg(th.bg.color()).bg(th.dim.color())));
        spans.push(Span::styled(format!(" {d}   "), th.s(th.dim)));
    }
    Paragraph::new(Line::from(spans)).render_ref(r, buf);
}

pub const HINTS: &[(&str, &str)] = &[
    ("␣", "play/pause"),
    ("←→", "seek"),
    ("Tab", "next design"),
    ("v", "swap visual"),
    ("t", "palette"),
    ("i", "idle preview"),
    ("p", "pixel/cell"),
    ("^K", "command palette"),
    ("?", "help"),
    ("q", "quit"),
];

trait RenderRef {
    fn render_ref(self, r: Rect, buf: &mut Buffer);
}
impl RenderRef for Paragraph<'_> {
    fn render_ref(self, r: Rect, buf: &mut Buffer) {
        ratatui::widgets::Widget::render(self, r, buf);
    }
}

// ───────────────────────────── now playing ─────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Progress {
    Line,
    Spectrum,
    Waveform,
    Thin,
}

pub fn now_playing(f: &mut Frame, r: Rect, app: &mut App, th: &Theme, style: Progress, art: bool) {
    let tr = app.track();
    let buf = f.buffer_mut();
    gfx::fill(buf, r, th.panel.color());
    if r.height < 2 {
        return;
    }
    let mut left = r.x + 1;
    if art && r.height >= 3 {
        let aw = (r.height * 2).min(12);
        let ar = Rect::new(r.x + 1, r.y, aw, r.height);
        let seed = tr.cover;
        app.pixel_pane(buf, ar, 900, false, 160, |fb| viz::cover(fb, seed, 0.0));
        left += aw + 1;
    }
    // title block
    let ty = r.y + r.height / 2 - 1;
    let tw = (r.width / 4).max(18);
    text(buf, left, ty, tw, tr.title, th.s(th.fg).add_modifier(Modifier::BOLD));
    let heart = if app.liked { "♥" } else { "♡" };
    let hx = left + (tr.title.chars().count() as u16 + 1).min(tw);
    text(buf, hx, ty, 2, heart, th.s(th.accent));
    app.hit(Rect::new(hx, ty, 1, 1), Act::Like);
    text(buf, left, ty + 1, tw, &format!("{} · {}", tr.artist, tr.album), th.s(th.dim));

    // transport, centred
    let mid_w = (r.width / 2).max(30).min(r.width.saturating_sub(left - r.x + 20));
    let mx = r.x + (r.width - mid_w) / 2;
    let buttons = [("⇄", Act::None), ("|◀", Act::Prev), (if app.playing { "❚❚" } else { "▶" }, Act::PlayPause), ("▶|", Act::Next), ("↻", Act::None)];
    let bw = 4u16;
    let bx0 = mx + (mid_w.saturating_sub(bw * buttons.len() as u16)) / 2;
    let by = if r.height >= 3 { r.y + r.height / 2 - 1 } else { r.y };
    for (i, (label, act)) in buttons.iter().enumerate() {
        let x = bx0 + i as u16 * bw;
        let st = if i == 2 { Style::default().fg(th.bg.color()).bg(th.accent.color()).add_modifier(Modifier::BOLD) } else { th.s(th.fg) };
        let lab = format!("{label:^3}");
        text(buf, x, by, 3, &lab, st);
        if !matches!(act, Act::None) {
            app.hit(Rect::new(x, by, 3, 1), act.clone());
        }
    }
    // progress
    let py = by + 1;
    let ph = if style == Progress::Spectrum || style == Progress::Waveform { (r.bottom() - py).min(2) } else { 1 };
    let pos = app.pos;
    let dur = tr.secs as f32;
    let t0 = fmt_time(pos);
    let t1 = fmt_time(dur);
    text(buf, mx, py + ph - 1, 6, &t0, th.s(th.dim));
    let bar = Rect::new(mx + 6, py, mid_w.saturating_sub(12), ph);
    text(buf, bar.right() + 1, py + ph - 1, 6, &t1, th.s(th.dim));
    let prog = pos / dur;
    match style {
        Progress::Line => {
            for x in 0..bar.width {
                let on = (x as f32) < prog * bar.width as f32;
                let ch = if on { '━' } else { '─' };
                let col = if on { th.accent } else { th.dim.scale(0.6) };
                text(buf, bar.x + x, bar.y, 1, &ch.to_string(), th.s(col));
            }
            let kx = bar.x + (prog * bar.width as f32) as u16;
            text(buf, kx.min(bar.right().saturating_sub(1)), bar.y, 1, "●", th.s(th.fg));
        }
        Progress::Thin => {
            let k = (prog * bar.width as f32) as u16;
            for x in 0..bar.width {
                let (ch, col) = if x < k { ("━", th.accent) } else if x == k { ("╸", th.accent) } else { ("─", th.border) };
                text(buf, bar.x + x, bar.y, 1, ch, th.s(col));
            }
        }
        Progress::Spectrum => {
            let fr = app.frame.clone();
            viz::spectrum(buf, bar, &fr, &th.ramp, false, false, false);
            // played part bright, rest dimmed
            let k = bar.x + (prog * bar.width as f32) as u16;
            for y in bar.top()..bar.bottom() {
                for x in k..bar.right() {
                    if let Some(c) = buf.cell_mut((x, y)) {
                        c.set_fg(th.dim.scale(0.55).color());
                    }
                }
            }
        }
        Progress::Waveform => viz::waveform_bar(buf, bar, prog, tr.cover, &th.ramp, th.dim.scale(0.5).color()),
    }
    app.hit(bar, Act::Seek(bar));

    // volume, right
    let vw = 12u16;
    let vx = r.right().saturating_sub(vw + 6);
    if vx > bar.right() + 8 {
        text(buf, vx, by + 1, 3, "vol", th.s(th.dim));
        let vbar = Rect::new(vx + 4, by + 1, vw, 1);
        for x in 0..vw {
            let on = (x as f32) < app.volume * vw as f32;
            text(buf, vbar.x + x, vbar.y, 1, if on { "▬" } else { "─" }, th.s(if on { th.fg } else { th.dim.scale(0.6) }));
        }
        app.hit(vbar, Act::Volume(vbar));
        text(buf, vx, by, vw + 4, "  LOSSLESS 24/44.1", th.s(th.accent2).add_modifier(Modifier::BOLD));
    }
}

// ───────────────────────────── sidebar / library ─────────────────────────────

pub fn sidebar(f: &mut Frame, r: Rect, app: &mut App, th: &Theme, title: &str, tree: bool) {
    let block = th.block(title);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let buf = f.buffer_mut();
    let nav = [("⌂", "Home"), ("⌕", "Search")];
    let mut y = inner.y;
    if !tree {
        for (i, (ic, name)) in nav.iter().enumerate() {
            let st = if i == 0 { th.s(th.fg).add_modifier(Modifier::BOLD) } else { th.s(th.dim) };
            text(buf, inner.x + 1, y, inner.width - 1, &format!("{ic}  {name}"), st);
            y += 1;
        }
        y += 1;
        text(buf, inner.x + 1, y, inner.width - 1, "Your Library", th.s(th.dim).add_modifier(Modifier::BOLD));
        y += 1;
    } else {
        text(buf, inner.x, y, inner.width, "~/library", th.s(th.accent).add_modifier(Modifier::BOLD));
        y += 1;
    }
    app.hit(Rect::new(inner.x, y, inner.width, inner.bottom().saturating_sub(y)), Act::SidebarScroll);
    for (i, (ic, name)) in mock::LIBRARY.iter().enumerate() {
        if y >= inner.bottom() {
            break;
        }
        let sel = i == app.sidebar_sel;
        let row = Rect::new(inner.x, y, inner.width, 1);
        if sel {
            gfx::fill(buf, row, th.sel.color());
        }
        let label = if tree {
            let last = i + 1 == mock::LIBRARY.len();
            format!("{}── {}", if last { "└" } else { "├" }, name)
        } else {
            format!(" {ic}  {name}")
        };
        let st = if sel { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.fg) };
        text(buf, inner.x, y, inner.width, &label, st);
        app.hit(row, Act::Sidebar(i));
        y += 1;
    }
}

// ───────────────────────────── home ─────────────────────────────

/// Home shelves as cover-art cards (`cards`) or as compact text lists.
pub fn home(f: &mut Frame, r: Rect, app: &mut App, th: &Theme, title: &str, cards: bool) {
    let block = th.block(title);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let buf = f.buffer_mut();
    let mut y = inner.y;
    let greet = "Good evening";
    text(buf, inner.x + 1, y, inner.width, greet, th.s(th.fg).add_modifier(Modifier::BOLD));
    y += 2;
    let card_w = 16u16;
    for (si, shelf) in mock::HOME.iter().enumerate() {
        if y + 2 > inner.bottom() {
            break;
        }
        text(buf, inner.x + 1, y, inner.width, shelf.title, th.s(th.accent).add_modifier(Modifier::BOLD));
        text(buf, inner.right().saturating_sub(9), y, 8, "Show all", th.s(th.dim));
        y += 1;
        if cards {
            let n = ((inner.width.saturating_sub(1)) / (card_w + 2)) as usize;
            let cover_h = card_w / 2;
            if y + cover_h + 2 > inner.bottom() {
                break;
            }
            for (i, (name, sub, seed)) in shelf.items.iter().take(n).enumerate() {
                let x = inner.x + 1 + i as u16 * (card_w + 2);
                let cr = Rect::new(x, y, card_w, cover_h);
                let seed = *seed;
                app.pixel_pane(buf, cr, 100 + (si * 10 + i) as u32, false, 200, |fb| viz::cover(fb, seed, 0.0));
                text(buf, x, y + cover_h, card_w, name, th.s(th.fg).add_modifier(Modifier::BOLD));
                text(buf, x, y + cover_h + 1, card_w, sub, th.s(th.dim));
                app.hit(Rect::new(x, y, card_w, cover_h + 2), Act::PlayPause);
            }
            y += cover_h + 3;
        } else {
            let mut x = inner.x + 1;
            for (name, _, seed) in shelf.items {
                let [c1, ..] = viz::cover_palette(*seed);
                let w = name.chars().count() as u16 + 4;
                if x + w > inner.right() {
                    break;
                }
                text(buf, x, y, 2, "■", th.s(c1));
                text(buf, x + 2, y, w - 2, name, th.s(th.fg));
                x += w + 1;
            }
            y += 2;
        }
    }
}

// ───────────────────────────── queue ─────────────────────────────

pub fn queue(f: &mut Frame, r: Rect, app: &mut App, th: &Theme, title: &str, table: bool) {
    let block = th.block(title);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let buf = f.buffer_mut();
    let mut y = inner.y;
    if table && inner.height > 1 {
        let hdr = format!("{:<3}{:<w$}{:<16}{:>5}", "#", "TITLE", "ARTIST", "TIME", w = (inner.width as usize).saturating_sub(24));
        text(buf, inner.x, y, inner.width, &hdr, th.s(th.dim).add_modifier(Modifier::BOLD));
        y += 1;
    }
    let all = app.upcoming();
    for (i, t) in all.iter().enumerate() {
        if y >= inner.bottom() {
            break;
        }
        let line = if table {
            format!("{:<3}{:<w$}{:<16}{:>5}", i + 1, t.title, t.artist, fmt_time(t.secs as f32), w = (inner.width as usize).saturating_sub(24))
        } else {
            format!("{}  ·  {}", t.title, t.artist)
        };
        let [c1, ..] = viz::cover_palette(t.cover);
        if !table {
            text(buf, inner.x, y, 2, "■", th.s(c1));
            text(buf, inner.x + 2, y, inner.width.saturating_sub(2), &line, th.s(if i == 0 { th.fg } else { th.dim }));
        } else {
            text(buf, inner.x, y, inner.width, &line, th.s(if i == 0 { th.fg } else { th.dim }));
        }
        app.hit(Rect::new(inner.x, y, inner.width, 1), Act::Next);
        y += 1;
    }
}

// ───────────────────────────── lyrics ─────────────────────────────

/// Karaoke lyrics: the current line sweeps from accent to fg as it plays. Click a line to seek.
pub fn lyrics(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme, centered: bool, spacing: u16) {
    let cur = mock::lyric_index(app.pos);
    let prog = mock::lyric_progress(app.pos);
    let rows = (r.height / spacing.max(1)) as i32;
    let start = cur as i32 - rows / 3;
    for k in 0..rows {
        let i = start + k;
        if i < 0 || i as usize >= mock::LYRICS.len() {
            continue;
        }
        let (sec, line) = mock::LYRICS[i as usize];
        let y = r.y + k as u16 * spacing.max(1);
        let len = line.chars().count() as u16;
        let x = if centered { r.x + r.width.saturating_sub(len) / 2 } else { r.x + 1 };
        let dist = (i - cur as i32).unsigned_abs() as f32;
        if i as usize == cur {
            let split = (prog * len as f32) as usize;
            let (a, b): (String, String) = (line.chars().take(split).collect(), line.chars().skip(split).collect());
            text(buf, x, y, r.width, &a, th.s(th.accent).add_modifier(Modifier::BOLD));
            text(buf, x + split as u16, y, r.width.saturating_sub(split as u16), &b, th.s(th.fg).add_modifier(Modifier::BOLD));
        } else {
            let c = th.dim.lerp(th.bg, (dist / rows as f32 * 1.4).min(0.85));
            text(buf, x, y, r.width, line, th.s(c));
        }
        let loop_base = (app.pos / 110.0).floor() * 110.0;
        app.hit(Rect::new(r.x, y, r.width, 1), Act::SeekTo(loop_base + sec as f32));
    }
}

// ───────────────────────────── heroes ─────────────────────────────

/// A swappable widget slot. Click it (or press v) to cycle through the visuals this design allows.
pub fn hero(f: &mut Frame, r: Rect, app: &mut App, th: &Theme, kind: Hero, label: bool) {
    let buf = f.buffer_mut();
    if r.width < 2 || r.height < 2 {
        return;
    }
    let fr = app.frame.clone();
    match kind {
        Hero::BlackHole(look) => {
            let probe = app.fb_for(r, true, 1280);
            let rh = blackhole::fit_rh(probe.w, probe.h, &crate::app::LOOKS[look % crate::app::LOOKS.len()]);
            let st = app.bh_style(look, rh, (0.5, 0.5), false);
            let fb = app.render_bh(r, 1280, st);
            app.present(buf, r, 1, true, fb);
        }
        Hero::Warp => {
            let ramp = th.ramp.clone();
            let mut fb = app.fb_for(r, true, 900);
            app.warp.render(&mut fb, &fr, &ramp);
            app.present(buf, r, 3, true, fb);
        }
        Hero::Spectrum => {
            gfx::fill(buf, r, th.bg.color());
            let mirror = app.dir == 2;
            viz::spectrum(buf, r, &fr, &th.ramp, mirror, true, false);
        }
        Hero::Matrix => {
            gfx::fill(buf, r, th.bg.color());
            app.matrix.step(r, app.dt, &fr);
            let title = app.track().title.to_uppercase();
            app.matrix.render(buf, r, app.t, th.accent2, th.accent, &title);
        }
        Hero::Scope => {
            gfx::fill(buf, r, th.bg.color());
            app.scope.render(buf, r, &fr, &th.ramp);
        }
        Hero::History => {
            gfx::fill(buf, r, th.bg.color());
            app.history.render(buf, r, &th.ramp);
        }
    }
    app.hit(r, Act::CycleHero);
    if label && r.width > 24 {
        let s = format!(" ◂ {} ▸ ", kind.name());
        let w = s.chars().count() as u16;
        text(buf, r.right().saturating_sub(w + 1), r.y, w, &s, Style::default().fg(th.dim.color()));
    }
}

/// Idle screensaver: after a while without input the hole drifts in over the whole body and bends
/// everything, text included. Any key or click wakes the UI (and is swallowed).
pub fn idle_hole(f: &mut Frame, area: Rect, app: &mut App, look: usize) {
    if area.width < 20 || area.height < 10 {
        return;
    }
    let lk = crate::app::LOOKS[look % crate::app::LOOKS.len()];
    let t = app.t * 0.06;
    let lx = 0.75 * (t * 0.37).sin() + 0.25 * (t * 0.83 + 1.0).sin();
    let ly = 0.70 * (t * 0.54 + 2.1).sin() + 0.30 * (t * 1.07).sin();
    let rh = (area.height as f32 * 2.0 * 0.07).max(2.5);
    let reach = rh * blackhole::reach(&lk);
    let cx = area.x as f32 + area.width as f32 * 0.5 + lx * (area.width as f32 * 0.5 - reach * 0.6).max(0.0);
    let cy = area.y as f32 + area.height as f32 * 0.45 + ly * (area.height as f32 * 0.5 - reach * 0.3).max(0.0);
    let buf = f.buffer_mut();
    // dim everything a little, then bend it
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                if let ratatui::style::Color::Rgb(r, g, b) = c.fg {
                    c.set_fg(ratatui::style::Color::Rgb(r / 2, g / 2, b / 2));
                }
            }
        }
    }
    blackhole::lens_cells(buf, area, cx, cy, rh, false);
    overlay_hole(buf, area, app, look, cx, cy, rh, 8);
}

/// Draw the disk + shadow centred at (cx, cy) as a transparent overlay, clipped to `area`.
fn overlay_hole(buf: &mut Buffer, area: Rect, app: &mut App, look: usize, cx: f32, cy: f32, rh: f32, id: u32) {
    let lk = crate::app::LOOKS[look % crate::app::LOOKS.len()];
    let reach = rh * blackhole::reach(&lk);
    let x0 = (cx - reach).max(area.x as f32) as u16;
    let y0 = (cy - reach / 2.0).max(area.y as f32) as u16;
    let x1 = ((cx + reach).ceil() as u16).min(area.right());
    let y1 = ((cy + reach / 2.0).ceil() as u16).min(area.bottom());
    let rect = Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0));
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let center = ((cx - rect.x as f32) / rect.width as f32, (cy - rect.y as f32) / rect.height as f32);
    let st = app.bh_style(look, rh / (rect.height as f32 * 2.0), center, true);
    let fb = app.render_bh(rect, 1280, st);
    app.present(buf, rect, id, true, fb);
}

/// The gravity well: the hole lives in its own pane, so nothing you need is ever under it. Its
/// gravity shows on the pane's own frame, which sags toward the centre (smooth braille curves,
/// breathing with the bass), and on the starfield inside, which the tracer lenses for real.
pub fn gravity_well(f: &mut Frame, outer: Rect, app: &mut App, th: &Theme, look: usize) {
    let lk = crate::app::LOOKS[look % crate::app::LOOKS.len()];
    let inner = Rect::new(outer.x + 2, outer.y + 1, outer.width.saturating_sub(4), outer.height.saturating_sub(2));
    if inner.width < 4 || inner.height < 3 {
        return;
    }
    let buf = f.buffer_mut();
    gfx::fill(buf, outer, th.bg.color());
    let probe = app.fb_for(inner, true, 1280);
    let mut st = app.bh_style(look, blackhole::fit_rh(probe.w, probe.h, &lk), (0.5, 0.5), false);
    st.look.star = st.look.star.max(0.35);
    let fb = app.render_bh(inner, 1280, st);
    app.present(buf, inner, 1, true, fb);
    warped_frame(buf, outer, 0.5 + app.frame.bass * 0.9, &[th.border, th.dim]);
    text(buf, outer.x + 2, outer.y, 14, " gravity well ", th.s(th.accent));
    app.hit(outer, Act::CycleHero);
    let label = format!(" ◂ {} ▸ ", lk.name);
    let w = label.chars().count() as u16;
    if outer.width > w + 18 {
        text(buf, outer.right() - w - 2, outer.y, w, &label, th.s(th.dim));
    }
}

/// A rectangle whose edges are pulled toward its centre, drawn with braille dots.
fn warped_frame(buf: &mut Buffer, r: Rect, strength: f32, colors: &[Rgb]) {
    let mut b = viz::Braille::new(r);
    let (w, h) = (r.width as f32 * 2.0 - 1.0, r.height as f32 * 4.0 - 1.0);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let sag = |u: f32| (-(u * 2.6).powi(2)).exp(); // u = -0.5..0.5 along an edge
    let steps = ((w + h) * 3.0) as usize;
    for i in 0..=steps {
        let u = i as f32 / steps as f32 - 0.5;
        // top & bottom edges pulled toward cy, left & right toward cx
        let dy = sag(u) * strength * h * 0.10;
        let dx = sag(u) * strength * w * 0.035;
        b.plot(cx + u * w, 0.0 + dy, 0.6);
        b.plot(cx + u * w, h - dy, 0.6);
        b.plot(0.0 + dx, cy + u * h, 0.6);
        b.plot(w - dx, cy + u * h, 0.6);
    }
    b.render(buf, r, colors);
}

// ───────────────────────────── overlays ─────────────────────────────

pub fn centered(r: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(r.width);
    let h = h.min(r.height);
    Rect::new(r.x + (r.width - w) / 2, r.y + (r.height - h) / 3, w, h)
}

pub fn palette(f: &mut Frame, area: Rect, app: &mut App, th: &Theme) {
    let Some(p) = app.palette.as_ref() else { return };
    let items = app.palette_items();
    let query = p.query.clone();
    let selected = p.sel;
    let r = centered(area, 64, 18);
    f.render_widget(Clear, r);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(th.s(th.accent))
        .title(Span::styled(" Command palette ", th.s(th.accent).add_modifier(Modifier::BOLD)))
        .style(Style::default().bg(th.panel.color()).fg(th.fg.color()));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let buf = f.buffer_mut();
    text(buf, inner.x + 1, inner.y, inner.width - 2, &format!("› {query}▏"), th.s(th.fg).add_modifier(Modifier::BOLD));
    text(buf, inner.x + 1, inner.y + 1, inner.width - 2, &"─".repeat(inner.width as usize - 2), th.s(th.dim));
    for (i, (label, key, act)) in items.iter().enumerate() {
        let y = inner.y + 2 + i as u16;
        if y >= inner.bottom() {
            break;
        }
        let row = Rect::new(inner.x, y, inner.width, 1);
        if i == selected {
            gfx::fill(buf, row, th.sel.color());
        }
        text(buf, inner.x + 2, y, inner.width - 12, label, th.s(if i == selected { th.accent } else { th.fg }));
        text(buf, inner.right().saturating_sub(key.chars().count() as u16 + 2), y, 10, key, th.s(th.dim));
        app.hit(row, Act::Palette(Box::new(act.clone())));
    }
}

pub fn help(f: &mut Frame, area: Rect, th: &Theme) {
    let r = centered(area, 70, 22);
    f.render_widget(Clear, r);
    let block = th.block("You only need these").border_set(border::ROUNDED).style(Style::default().bg(th.panel.color()).fg(th.fg.color()));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let rows = [
        ("Mouse", "click anything · drag seek/volume · scroll the library · click a lyric to jump"),
        ("Ctrl+K  :", "command palette: type to find any action, no memorising"),
        ("Space", "play / pause"),
        ("← →", "seek 5s          n / b   next / previous"),
        ("1-6  Tab", "switch design direction (prototype only)"),
        ("v", "swap the visual in the hero slot (or click it)"),
        ("p", "toggle pixel tier (kitty graphics) / cell tier"),
        ("a", "force synthetic audio (otherwise: whatever your PC plays)"),
        ("l", "like current track"),
        ("q / Esc", "quit / close"),
        ("", ""),
        ("Audio", "visuals react to your system audio via the PipeWire monitor."),
        ("", "Play music in any app (or the real Spotify) to see it live."),
    ];
    let buf = f.buffer_mut();
    for (i, (k, d)) in rows.iter().enumerate() {
        let y = inner.y + 1 + i as u16;
        if y >= inner.bottom() {
            break;
        }
        text(buf, inner.x + 2, y, 12, k, th.s(th.accent).add_modifier(Modifier::BOLD));
        text(buf, inner.x + 14, y, inner.width - 15, d, th.s(th.fg));
    }
}

pub fn toast(f: &mut Frame, area: Rect, th: &Theme, msg: &str) {
    let w = msg.chars().count() as u16 + 4;
    let r = Rect::new(area.right().saturating_sub(w + 2), area.bottom().saturating_sub(3), w, 1);
    let buf = f.buffer_mut();
    text(buf, r.x, r.y, w, &format!("  {msg}  "), Style::default().fg(th.bg.color()).bg(th.accent.color()).add_modifier(Modifier::BOLD));
}
