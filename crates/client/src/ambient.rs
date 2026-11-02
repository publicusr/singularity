//! Ambient Widgets (D36 demos the owner kept, #44): calm, audio-aware pictures for the Slots.
//!
//! - **starfield** (audio Slots): flying through stars, faster with the music, streaking on surges
//! - **marquee** (side Slots): the title in a big pixel font, scrolling when it doesn't fit, brighter on beats
//! - **glow** (side Slots): the cover, with its colour bleeding out into the Pane
//! - **vinyl**: the cover as the label of a record spinning at 33⅓, the tonearm creeping in as the song plays
//! - **waterfall**: a spectrogram scrolling right to left, the last 24 s
//! - **aurora**: slow curtains of light in the cover's colours, swelling with the bass
//! - **ripples**: rings spreading out from the middle on each beat, like drops on water

use crate::app::App;
use crate::theme::{Rgb, Theme};
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

mod aurora;
mod ripples;
mod vinyl;
mod waterfall;

pub use aurora::aurora;
pub use ripples::ripples;
pub use vinyl::vinyl;
pub use waterfall::waterfall;

/// Demo state that lives across frames.
#[derive(Default)]
pub struct State {
    stars: Vec<Star>,
    stars_last: Option<Instant>,
    speed: f32,
    seed: u32,
    marquee_t: f32,
    marquee_last: Option<Instant>,
    /// the title the marquee shows (a new one starts from the beginning)
    marquee_title: String,
    /// eased beat brightness, 0..1
    marquee_glow: f32,
    /// the glow's colour now, easing toward the cover's
    glow_now: Option<[[f32; 3]; 16]>,
    glow_last: Option<Instant>,
    /// the glow's clock, its eased (normalised) bass, the beat pulse, and the loudest bass lately
    glow_t: f32,
    glow_lvl: f32,
    glow_pulse: f32,
    /// the beat that drives the pulse (the pulse eases toward it, so a beat swells in rather than flashing)
    glow_kick: f32,
    glow_peak: f32,
    vinyl: vinyl::State,
    waterfall: waterfall::State,
    aurora: aurora::State,
    ripples: ripples::State,
}

#[derive(Clone, Copy)]
struct Star {
    x: f32,
    y: f32,
    z: f32,
    /// where on the Theme's ramp its colour sits (stars differ a little)
    hue: f32,
    /// its own twinkle phase
    phase: f32,
}

/// A small deterministic random number in 0..1.
fn rnd(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed % 10_000) as f32 / 10_000.0
}

// ── starfield ─────────────────────────────────────────────────────────

pub fn starfield(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.width < 4 || r.height < 2 {
        return;
    }
    let energy = app.meters.frame.as_ref().map(|f| f.level as f32 / 255.0).unwrap_or(0.0);
    let beat = app.meters.frame.as_ref().is_some_and(|f| f.beat);
    let aspect = app.cell_px.map(|(w, h)| (h / w) as f32).unwrap_or(2.0);
    let s = &mut app.ambient;
    let dt = s.stars_last.map(|l| l.elapsed().as_secs_f32().min(0.1)).unwrap_or(0.0);
    s.stars_last = Some(Instant::now());
    if s.seed == 0 {
        s.seed = 0x9e37_79b9;
    }
    // calm drift when quiet; a smooth surge with energy, a kick on beats (eased, never a strobe)
    let target = 0.15 + energy * 0.9 + if beat { 0.6 } else { 0.0 };
    s.speed += (target - s.speed) * (dt * 4.0).min(1.0);
    let want = (r.width as usize * r.height as usize / 6).clamp(20, 600);
    while s.stars.len() < want {
        let (x, y, z) = (rnd(&mut s.seed) * 2.0 - 1.0, rnd(&mut s.seed) * 2.0 - 1.0, 0.1 + rnd(&mut s.seed) * 0.9);
        let (hue, phase) = (rnd(&mut s.seed), rnd(&mut s.seed) * std::f32::consts::TAU);
        s.stars.push(Star { x, y, z, hue, phase });
    }
    s.stars.truncate(want);
    let (cx, cy) = (r.width as f32 / 2.0, r.height as f32 / 2.0);
    // a cell is `aspect` times taller than wide: scale y so the field is round
    let (sx, sy) = (cx, cx / aspect);
    let project = |st: &Star, z: f32| (cx + st.x / z * sx * 0.5, cy + st.y / z * sy * 0.5);
    let speed = s.speed;
    let mut seed = s.seed;
    for st in s.stars.iter_mut() {
        let before = *st;
        st.z -= speed * dt;
        let (px, py) = project(st, st.z);
        if st.z <= 0.02 || px < 0.0 || py < 0.0 || px >= r.width as f32 || py >= r.height as f32 {
            *st = Star { x: rnd(&mut seed) * 2.0 - 1.0, y: rnd(&mut seed) * 2.0 - 1.0, z: 1.0, hue: rnd(&mut seed), phase: rnd(&mut seed) * std::f32::consts::TAU };
            continue;
        }
        let near = (1.0 - st.z).clamp(0.0, 1.0);
        // nearer is brighter; each star sits a little apart on the ramp, and far ones twinkle faintly
        st.phase += dt * 3.0;
        let twinkle = if near < 0.4 { 0.75 + 0.25 * st.phase.sin() } else { 1.0 };
        let col = th.ramp_at((0.15 + 0.7 * near + 0.25 * (st.hue - 0.5)).clamp(0.0, 1.0)).scale(twinkle);
        // a streak back to where it was, when it moved more than a cell
        let (ox, oy) = project(&before, before.z.max(0.02));
        let steps = ((px - ox).abs().max((py - oy).abs())) as i32;
        for k in 1..steps.min(8) {
            let t = k as f32 / steps as f32;
            let (tx, ty) = (px + (ox - px) * t, py + (oy - py) * t);
            if tx >= 0.0 && ty >= 0.0 && (tx as u16) < r.width && (ty as u16) < r.height {
                text(buf, r.x + tx as u16, r.y + ty as u16, 1, "·", th.s(col.scale(0.5 * (1.0 - t))));
            }
        }
        let ch = if near > 0.8 { "✦" } else if near > 0.5 { "•" } else if near > 0.25 { "∙" } else { "·" };
        text(buf, r.x + px as u16, r.y + py as u16, 1, ch, th.s(col));
    }
    s.seed = seed;
}

// ── marquee ───────────────────────────────────────────────────────────

/// A 3×5 pixel font: rows top to bottom, `#` lit. Letters without one fall back to their plain form, then `?`.
fn glyph(c: char) -> [&'static str; 5] {
    match c.to_ascii_uppercase() {
        'A' => [".#.", "#.#", "###", "#.#", "#.#"],
        'B' => ["##.", "#.#", "##.", "#.#", "##."],
        'C' => [".##", "#..", "#..", "#..", ".##"],
        'D' => ["##.", "#.#", "#.#", "#.#", "##."],
        'E' => ["###", "#..", "##.", "#..", "###"],
        'F' => ["###", "#..", "##.", "#..", "#.."],
        'G' => [".##", "#..", "#.#", "#.#", ".##"],
        'H' => ["#.#", "#.#", "###", "#.#", "#.#"],
        'I' => ["###", ".#.", ".#.", ".#.", "###"],
        'J' => ["..#", "..#", "..#", "#.#", ".#."],
        'K' => ["#.#", "#.#", "##.", "#.#", "#.#"],
        'L' => ["#..", "#..", "#..", "#..", "###"],
        'M' => ["#.#", "###", "###", "#.#", "#.#"],
        'N' => ["##.", "#.#", "#.#", "#.#", "#.#"],
        'O' => [".#.", "#.#", "#.#", "#.#", ".#."],
        'P' => ["##.", "#.#", "##.", "#..", "#.."],
        'Q' => [".#.", "#.#", "#.#", "##.", ".##"],
        'R' => ["##.", "#.#", "##.", "#.#", "#.#"],
        'S' => [".##", "#..", ".#.", "..#", "##."],
        'T' => ["###", ".#.", ".#.", ".#.", ".#."],
        'U' => ["#.#", "#.#", "#.#", "#.#", "###"],
        'V' => ["#.#", "#.#", "#.#", "#.#", ".#."],
        'W' => ["#.#", "#.#", "###", "###", "#.#"],
        'X' => ["#.#", "#.#", ".#.", "#.#", "#.#"],
        'Y' => ["#.#", "#.#", ".#.", ".#.", ".#."],
        'Z' => ["###", "..#", ".#.", "#..", "###"],
        '0' => [".#.", "#.#", "#.#", "#.#", ".#."],
        '1' => [".#.", "##.", ".#.", ".#.", "###"],
        '2' => ["##.", "..#", ".#.", "#..", "###"],
        '3' => ["##.", "..#", ".#.", "..#", "##."],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "##.", "..#", "##."],
        '6' => [".##", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", ".#.", ".#.", ".#."],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "##."],
        ' ' => ["...", "...", "...", "...", "..."],
        '.' => ["...", "...", "...", "...", ".#."],
        ',' => ["...", "...", "...", ".#.", "#.."],
        '!' => [".#.", ".#.", ".#.", "...", ".#."],
        '?' => ["##.", "..#", ".#.", "...", ".#."],
        '\'' => [".#.", ".#.", "...", "...", "..."],
        '-' => ["...", "...", "###", "...", "..."],
        '&' => [".#.", "#.#", ".#.", "#.#", ".##"],
        '(' => [".#.", "#..", "#..", "#..", ".#."],
        ')' => [".#.", "..#", "..#", "..#", ".#."],
        '/' => ["..#", "..#", ".#.", "#..", "#.."],
        ':' => ["...", ".#.", "...", ".#.", "..."],
        c => match plain(c) {
            Some(p) => glyph(p),
            None => ["##.", "..#", ".#.", "...", ".#."],
        },
    }
}

/// A letter's plain form for the pixel font (Polish and other common accents).
fn plain(c: char) -> Option<char> {
    Some(match c {
        'ą' | 'Ą' | 'á' | 'à' | 'ä' | 'â' | 'Á' | 'Ä' => 'A',
        'ć' | 'Ć' | 'ç' | 'č' => 'C',
        'ę' | 'Ę' | 'é' | 'è' | 'ë' | 'ê' | 'É' => 'E',
        'ł' | 'Ł' => 'L',
        'ń' | 'Ń' | 'ñ' => 'N',
        'ó' | 'Ó' | 'ö' | 'ô' | 'Ö' => 'O',
        'ś' | 'Ś' | 'š' => 'S',
        'ź' | 'ż' | 'Ź' | 'Ż' | 'ž' => 'Z',
        'ü' | 'ú' | 'Ü' => 'U',
        'í' | 'ï' => 'I',
        _ => return None,
    })
}

/// The title in big pixels (half-blocks: 5 pixel rows in 3 text rows), scrolling when it's wider than the Pane.
pub fn marquee(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    if w < 8 || r.height == 0 {
        return;
    }
    let Some(item) = app.playback.item.clone() else {
        return text(buf, x, r.y, w, "nothing playing", th.s(th.dim));
    };
    let beat = app.meters.frame.as_ref().is_some_and(|f| f.beat);
    let s = &mut app.ambient;
    let dt = s.marquee_last.map(|l| l.elapsed().as_secs_f32().min(0.1)).unwrap_or(0.0);
    s.marquee_last = Some(Instant::now());
    s.marquee_t += dt;
    if s.marquee_title != item.name {
        s.marquee_title = item.name.clone();
        s.marquee_t = 0.0;
    }
    // a beat lifts it a little; it fades back over half a second
    s.marquee_glow = if beat { (s.marquee_glow + 0.5).min(1.0) } else { (s.marquee_glow - dt * 2.0).max(0.0) };
    let lift = s.marquee_glow * 0.25;
    // pixel columns: 3 per letter and a gap
    let title: Vec<char> = item.name.chars().collect();
    let cols = title.len() * 4;
    let big = r.height >= 3;
    let y = r.y + r.height.saturating_sub(if big { 5 } else { 2 }) / 2;
    if big {
        let gap = 8usize;
        let scroll = cols > w as usize;
        // 8 columns a second, with a gap before it comes round again
        let off = if scroll { (s.marquee_t * 8.0) as usize % (cols + gap) } else { 0 };
        let left = if scroll { 0 } else { (w as usize - cols) / 2 };
        let lit = |px: usize, py: usize| -> bool {
            let i = (px + off) % (cols + if scroll { gap } else { 0 });
            let (ch, c) = (i / 4, i % 4);
            c < 3 && ch < title.len() && glyph(title[ch])[py].as_bytes()[c] == b'#'
        };
        for cx in 0..(w as usize - left).min(if scroll { w as usize } else { cols }) {
            for row in 0..3usize {
                let (top, bottom) = (row * 2, row * 2 + 1);
                let a = lit(cx, top);
                let b = bottom < 5 && lit(cx, bottom);
                let ch = match (a, b) {
                    (true, true) => "█",
                    (true, false) => "▀",
                    (false, true) => "▄",
                    _ => continue,
                };
                // the ramp runs across the title, top to bottom brighter
                let col = th.ramp_at((0.45 + 0.55 * (1.0 - row as f32 / 3.0) + lift).min(1.0));
                text(buf, x + (left + cx) as u16, y + row as u16, 1, ch, th.s(col));
            }
        }
    } else {
        text(buf, x, y, w, &item.name, th.s(th.accent).add_modifier(Modifier::BOLD));
    }
    let artists = item.artists.join(", ");
    let ay = y + if big { 4 } else { 1 };
    if ay < r.bottom() && !artists.is_empty() {
        let aw = (artists.width() as u16).min(w);
        text(buf, x + (w - aw) / 2, ay, aw, &artists, th.s(th.dim));
    }
}

// ── glow ──────────────────────────────────────────────────────────────

/// The cover in the middle, its colours bleeding out into the Pane as smooth solid fields (half-block resolution,
/// exponential falloff from the cover's edge) and fading into the Theme's background. Bass swells it, a beat pulses it.
pub fn glow(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.width < 6 || r.height < 3 {
        return;
    }
    let Some(item) = app.playback.item.clone() else {
        return text(buf, r.x + 1, r.y, r.width.saturating_sub(2), "nothing playing", th.s(th.dim));
    };
    let url = item.cover_url.clone().unwrap_or_default();
    // the cover: square in pixels, about half the Pane's height
    let aspect = app.cell_px.map(|(w, h)| h / w).unwrap_or(2.0);
    let ch = (r.height / 2).max(2);
    let cw = ((ch as f64 * aspect).round() as u16).min(r.width.saturating_sub(2));
    let cover = Rect::new(r.x + (r.width - cw) / 2, r.y + (r.height - ch) / 2, cw, ch);
    let tint = app.art_tint.clone();
    // a 4x4 grid of the cover's colours (a little more saturated, so a muddy cover still glows)
    let target = app.covers.scaled(&url, 4, 4, tint.as_ref()).map(|img| {
        let mut g = [[0f32; 3]; 16];
        for (i, p) in img.pixels().take(16).enumerate() {
            let m = [p[0] as f32, p[1] as f32, p[2] as f32];
            let grey = (m[0] + m[1] + m[2]) / 3.0;
            g[i] = m.map(|v| (grey + (v - grey) * 1.6).clamp(0.0, 255.0));
        }
        g
    });
    let (level, beat) = app
        .meters
        .frame
        .as_ref()
        .map(|f| (f.out_bands[0].max(f.out_level / 2) as f32, f.beat))
        .unwrap_or((0.0, false));
    let s = &mut app.ambient;
    let dt = s.glow_last.map(|l| l.elapsed().as_secs_f32().min(0.1)).unwrap_or(1.0);
    s.glow_last = Some(Instant::now());
    s.glow_t += dt;
    // normalise against the loudest lately, so a quiet volume still moves it (the peak forgets over ~10 s)
    s.glow_peak = (s.glow_peak * (1.0 - dt * 0.1)).max(level).max(40.0);
    let norm = (level / s.glow_peak).clamp(0.0, 1.0);
    // eased both ways, never a flash: the level follows over ~200 ms, a beat swells in over ~80 ms and fades over ~400 ms
    // (the owner: "too bright and epilepsy-inducing sometimes, depending on the song")
    s.glow_lvl += (norm - s.glow_lvl) * (dt * 5.0).min(1.0);
    s.glow_kick = if beat { 1.0 } else { s.glow_kick * (-dt / 0.15).exp() };
    s.glow_pulse += (s.glow_kick - s.glow_pulse) * (dt / 0.08).min(1.0);
    // the colours ease toward the cover's (a new song fades over, it doesn't jump)
    if let Some(t) = target {
        let k = (dt * 2.5).min(1.0);
        s.glow_now = Some(match s.glow_now {
            Some(mut now) => {
                for (n, t) in now.iter_mut().zip(&t) {
                    for c in 0..3 {
                        n[c] += (t[c] - n[c]) * k;
                    }
                }
                now
            }
            None => t,
        });
    }
    if let Some(grid) = s.glow_now {
        let (lvl, pulse, t) = (s.glow_lvl, s.glow_pulse, s.glow_t);
        // everything in cell-width units: a cell is 1 wide and `aspect` tall, a half-block row `aspect / 2`
        let ar = aspect as f32;
        let half = ar / 2.0;
        let (mx, my) = (cover.x as f32 + cover.width as f32 / 2.0, (cover.y as f32 + cover.height as f32 / 2.0) * ar);
        let (hw, hh) = (cover.width as f32 / 2.0, cover.height as f32 * half);
        // the room around the cover on each axis: the glow is measured against it, so it fades out before every edge
        // of the Pane, however loud (the owner: the whole Pane should never be filled with colour)
        let (rx, ry) = ((r.width as f32 / 2.0 - hw).max(1.0), (r.height as f32 * half - hh).max(1.0));
        let radius = 0.3 * (0.85 + 0.35 * lvl + 0.2 * pulse);
        let gain = (0.42 + 0.18 * lvl + 0.1 * pulse).min(0.7);
        let bg = [th.bg.0 as f32, th.bg.1 as f32, th.bg.2 as f32];
        let shade = |px: f32, py: f32| -> Rgb {
            let (dx, dy) = (((px - mx).abs() - hw).max(0.0) / rx, ((py - my).abs() - hh).max(0.0) / ry);
            // 0 at the cover, 1 at the Pane's edge
            let d = (dx * dx + dy * dy).sqrt();
            // a slow wobble of the reach, different on each side
            let wob = 1.0 + 0.05 * (t * 0.6 + (px - mx) * 0.12).sin() * (t * 0.4 + (py - my) * 0.08).cos();
            let fall = (-(d / (radius * wob))).exp();
            // gone by 85 % of the way to the edge
            let e = (d / 0.85).clamp(0.0, 1.0);
            let edge = 1.0 - e * e * (3.0 - 2.0 * e);
            let k = (fall * edge * gain).clamp(0.0, 1.0);
            // the colour of the cover's nearest edge point
            let u = ((px - (mx - hw)) / (2.0 * hw)).clamp(0.0, 1.0) * 3.0;
            let v = ((py - (my - hh)) / (2.0 * hh)).clamp(0.0, 1.0) * 3.0;
            let (x0, y0) = (u.floor().min(2.0), v.floor().min(2.0));
            let (fx, fy) = (u - x0, v - y0);
            let at = |x: f32, y: f32| grid[y as usize * 4 + x as usize];
            let (a, b, c, e) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
            let chan = |i: usize| {
                let col = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + e[i] * fx) * fy;
                (bg[i] + (col - bg[i]) * k).round().clamp(0.0, 255.0) as u8
            };
            Rgb(chan(0), chan(1), chan(2))
        };
        // the pixel tier: one smooth image (the terminal scales it), with the cover painted into it
        if let Some((cpw, cph)) = app.pixel_tier_for(r) {
            // one image pixel per 2 screen pixels: smooth once scaled, still cheap
            let (w, h) = (((r.width as f64 * cpw) / 2.0).round().max(8.0) as u32, ((r.height as f64 * cph) / 2.0).round().max(8.0) as u32);
            let (cx0, cy0) = (((cover.x - r.x) as f64 * cpw / 2.0).round() as u32, ((cover.y - r.y) as f64 * cph / 2.0).round() as u32);
            let (ccw, cch) = ((cover.width as f64 * cpw / 2.0).round().max(1.0) as u32, (cover.height as f64 * cph / 2.0).round().max(1.0) as u32);
            let art = app.covers.scaled(&url, ccw, cch, tint.as_ref());
            let (sx, sy) = (r.width as f32 / w as f32, r.height as f32 * ar / h as f32);
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    let inside = x >= cx0 && y >= cy0 && x < cx0 + ccw && y < cy0 + cch;
                    let c = match (&art, inside) {
                        (Some(a), true) => {
                            let p = a.get_pixel((x - cx0).min(a.width() - 1), (y - cy0).min(a.height() - 1));
                            Rgb(p[0], p[1], p[2])
                        }
                        _ => shade(r.x as f32 + (x as f32 + 0.5) * sx, r.y as f32 * ar + (y as f32 + 0.5) * sy),
                    };
                    rgba.extend_from_slice(&[c.0, c.1, c.2, 255]);
                }
            }
            if let Some(k) = app.kitty.as_mut() {
                k.frame(GLOW_ID, r, w, h, std::sync::Arc::new(rgba));
            }
            app.under_image(buf, r, th);
            if art.is_some() {
                glow_label(buf, r, cover, &item, th);
                return;
            }
        } else {
            for yy in r.top()..r.bottom() {
                for xx in r.left()..r.right() {
                    let px = xx as f32 + 0.5;
                    let (top, bot) = (shade(px, yy as f32 * ar + half * 0.5), shade(px, yy as f32 * ar + half * 1.5));
                    if let Some(cell) = buf.cell_mut((xx, yy)) {
                        cell.set_char('\u{2580}').set_fg(top.color()).set_bg(bot.color());
                    }
                }
            }
        }
    }
    crate::home::cover(buf, cover, &url, "♪", app, th);
    glow_label(buf, r, cover, &item, th);
}

/// Glow's Visual id on the pixel tier (the hole is 1, the idle takeover 2).
const GLOW_ID: u32 = 3;

/// The song under the cover.
fn glow_label(buf: &mut Buffer, r: Rect, cover: Rect, item: &singularity_proto::Item, th: &Theme) {
    let label = format!("{} · {}", item.name, item.artists.join(", "));
    let ly = cover.bottom() + 1;
    if ly < r.bottom() {
        let lw = (label.width() as u16).min(r.width.saturating_sub(2));
        let label: String = if label.width() as u16 > lw { label.chars().take(lw.saturating_sub(1) as usize).collect::<String>() + "…" } else { label };
        text(buf, r.x + (r.width - lw) / 2, ly, lw, &label, th.s(th.fg));
    }
}

// ── shared by the pictures below ─────────────────────────────────────

/// Where a picture is drawn. Coordinates are in cell widths from the Pane's top left on both axes (a cell is 1 wide
/// and `aspect` tall), so circles come out round; a pixel is `ux` × `uy` of them.
pub(crate) struct Canvas {
    /// pixels across and down: the pixel tier's image, or one per half cell (half-blocks)
    pub pw: u32,
    pub ph: u32,
    pub ux: f32,
    pub uy: f32,
    /// the pixel tier
    pub pixel: bool,
    /// the Pane in units: `r.width` × `r.height · aspect`
    pub w: f32,
    pub h: f32,
}

impl Canvas {
    /// The canvas for `r`: the pixel tier when it can be used, at most `max_px` pixels (the terminal scales the image
    /// smoothly), else half-blocks.
    pub fn new(app: &App, r: Rect, max_px: f64) -> Canvas {
        let aspect = app.cell_px.map(|(w, h)| (h / w) as f32).unwrap_or(2.0);
        let (w, h) = (r.width as f32, r.height as f32 * aspect);
        let (pw, ph, pixel) = match app.pixel_tier_for(r) {
            Some((cpw, cph)) => {
                let (sw, sh) = (r.width as f64 * cpw, r.height as f64 * cph);
                // one image pixel per 2 screen pixels or more, so a big Pane stays cheap
                let div = (sw * sh / max_px).sqrt().max(2.0);
                ((sw / div).round().max(1.0) as u32, (sh / div).round().max(1.0) as u32, true)
            }
            None => (r.width as u32, r.height as u32 * 2, false),
        };
        Canvas { pw, ph, ux: w / pw.max(1) as f32, uy: h / ph.max(1) as f32, pixel, w, h }
    }

    /// Fill the picture from `shade(column, row, x, y)` (x, y the pixel's centre in units). On the pixel tier the
    /// image goes up as Visual `id`; else the Pane is drawn in half-blocks.
    pub fn paint(&self, buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme, id: u32, mut shade: impl FnMut(u32, u32, f32, f32) -> Rgb) {
        if self.pixel {
            let mut rgba = Vec::with_capacity((self.pw * self.ph * 4) as usize);
            for j in 0..self.ph {
                let y = (j as f32 + 0.5) * self.uy;
                for i in 0..self.pw {
                    let c = shade(i, j, (i as f32 + 0.5) * self.ux, y);
                    rgba.extend_from_slice(&[c.0, c.1, c.2, 255]);
                }
            }
            if let Some(k) = app.kitty.as_mut() {
                k.frame(id, r, self.pw, self.ph, std::sync::Arc::new(rgba));
            }
            app.under_image(buf, r, th);
            return;
        }
        for row in 0..r.height {
            let (j0, j1) = (row as u32 * 2, row as u32 * 2 + 1);
            let (y0, y1) = ((j0 as f32 + 0.5) * self.uy, (j1 as f32 + 0.5) * self.uy);
            for col in 0..r.width {
                let x = col as f32 + 0.5;
                let (top, bot) = (shade(col as u32, j0, x, y0), shade(col as u32, j1, x, y1));
                if let Some(cell) = buf.cell_mut((r.x + col, r.y + row)) {
                    cell.set_char('\u{2580}').set_fg(top.color()).set_bg(bot.color());
                }
            }
        }
    }
}

/// Ease `v` toward `to`: rising over about `up` seconds, falling over about `down` (never a jump: no flashes).
pub(crate) fn ease(v: &mut f32, to: f32, dt: f32, up: f32, down: f32) {
    let tau = if to > *v { up } else { down };
    *v += (to - *v) * (1.0 - (-dt / tau.max(1e-3)).exp());
}

/// `v` against the loudest lately (`peak` forgets over ~10 s, never below `floor`), so a quiet volume still moves
/// things: 0..1.
pub(crate) fn against_peak(peak: &mut f32, v: f32, dt: f32, floor: f32) -> f32 {
    *peak = (*peak * (1.0 - dt * 0.1).max(0.0)).max(v).max(floor);
    (v / *peak).clamp(0.0, 1.0)
}

/// Seconds since `last` (now on return), at most `max`; 0 the first time.
pub(crate) fn tick(last: &mut Option<Instant>, max: f32) -> f32 {
    let now = Instant::now();
    let dt = last.map(|l| now.duration_since(l).as_secs_f32().min(max)).unwrap_or(0.0);
    *last = Some(now);
    dt
}

/// Whether the music is playing (not paused, stopped or nothing at all).
pub(crate) fn playing(app: &App) -> bool {
    app.playback.item.is_some() && matches!(app.playback.status, singularity_proto::PlayStatus::Playing | singularity_proto::PlayStatus::Buffering)
}

/// `bg` with `c` added on top at `k` (0..1 a channel, saturating).
pub(crate) fn over(bg: Rgb, c: [f32; 3]) -> Rgb {
    let f = |b: u8, v: f32| (b as f32 + v).round().clamp(0.0, 255.0) as u8;
    Rgb(f(bg.0, c[0]), f(bg.1, c[1]), f(bg.2, c[2]))
}

pub(crate) fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_is_three_by_five_and_accents_fall_back() {
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .,!?'-&()/:ąęłóśżźćń".chars() {
            let g = glyph(c);
            assert!(g.iter().all(|row| row.len() == 3 && row.chars().all(|p| p == '#' || p == '.')), "{c}");
        }
        assert_eq!(glyph('ł'), glyph('L'));
        assert_eq!(glyph('ż'), glyph('Z'));
    }

    type Draw = fn(&mut Buffer, Rect, &mut App, &Theme);
    pub(crate) const PICTURES: [(&str, Draw); 4] = [("vinyl", vinyl), ("waterfall", waterfall), ("aurora", aurora), ("ripples", ripples)];

    /// An App with a song playing (a red cover) and a spectrum frame with a beat.
    pub(crate) fn app_playing() -> App {
        let mut app = App::new(None);
        app.covers.insert("https://i.scdn.co/image/c", image::RgbImage::from_fn(64, 64, |x, y| image::Rgb([200, (x * 3) as u8, (y * 3) as u8])));
        app.playback.item = Some(singularity_proto::Item { name: "Song".into(), duration_ms: 200_000, cover_url: Some("https://i.scdn.co/image/c".into()), ..Default::default() });
        app.playback.status = singularity_proto::PlayStatus::Playing;
        app.meters.frame = Some(singularity_proto::SpectrumFrame { bands: (0..64).map(|i| 255 - i * 3).collect(), level: 180, vu: [180, 170], beat: true, out_level: 120, out_vu: [120, 110], out_bands: [150, 90, 60] });
        app
    }

    #[test]
    fn the_pictures_stay_inside_their_pane_at_any_size_playing_paused_or_idle() {
        let th = Theme::singularity();
        for (name, draw) in PICTURES {
            for state in 0..3 {
                let mut app = app_playing();
                match state {
                    1 => {
                        app.playback.status = singularity_proto::PlayStatus::Paused;
                        app.meters.frame = None;
                    }
                    2 => {
                        app.playback.item = None;
                        app.meters.frame = None;
                    }
                    _ => {}
                }
                for (w, h) in [(1u16, 1u16), (2, 1), (1, 3), (3, 2), (8, 3), (20, 5), (40, 8), (80, 14), (200, 60)] {
                    let mut buf = Buffer::empty(Rect::new(0, 0, w + 2, h + 2));
                    for _ in 0..3 {
                        draw(&mut buf, Rect::new(1, 1, w, h), &mut app, &th);
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    for y in 0..h + 2 {
                        for x in 0..w + 2 {
                            if x == 0 || y == 0 || x == w + 1 || y == h + 1 {
                                assert_eq!(buf[(x, y)], ratatui::buffer::Cell::default(), "{name} state {state} {w}x{h}: drew outside at ({x},{y})");
                            }
                        }
                    }
                    // something is drawn once there's room (a picture, not a blank Pane)
                    if w >= 20 && state == 0 {
                        let bg = th.bg.color();
                        let lit = (1..=h).flat_map(|y| (1..=w).map(move |x| (x, y))).filter(|&(x, y)| buf[(x, y)].fg != bg || buf[(x, y)].bg != bg).count();
                        assert!(lit > 0, "{name} {w}x{h}: nothing drawn");
                    }
                }
            }
        }
    }

    #[test]
    fn the_pictures_go_up_as_their_own_visual_on_the_pixel_tier() {
        let th = Theme::singularity();
        for (i, (name, draw)) in PICTURES.into_iter().enumerate() {
            let mut app = app_playing();
            app.cell_px = Some((10.0, 20.0));
            app.kitty = Some(crate::gfx::Kitty::new(false));
            for (w, h) in [(1u16, 1u16), (3, 2), (40, 12), (120, 40)] {
                let mut buf = Buffer::empty(Rect::new(0, 0, w + 2, h + 2));
                let t0 = Instant::now();
                draw(&mut buf, Rect::new(1, 1, w, h), &mut app, &th);
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                let mut out = vec![];
                app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
                let apc = String::from_utf8_lossy(&out);
                // ids 4–7, each sent double-buffered as 2·id or 2·id + 1
                let id = 4 + i as u32;
                assert!(w < 40 || apc.contains(&format!("i={},", id * 2)) || apc.contains(&format!("i={},", id * 2 + 1)), "{name} {w}x{h}: no frame for Visual {id}");
                // a debug build is far slower than release; this only catches a mistake that makes one heavy
                assert!(ms < 400.0, "{name} {w}x{h}: {ms:.1} ms a frame");
            }
        }
    }
}
