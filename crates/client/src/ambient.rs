//! Ambient Widgets (D36 demos the owner kept, #44): calm, audio-aware pictures for the Slots.
//!
//! - **starfield** (audio Slots): flying through stars, faster with the music, streaking on surges
//! - **marquee** (side Slots): the title in a big pixel font, scrolling when it doesn't fit, brighter on beats
//! - **glow** (side Slots): the cover, with its colour bleeding out into the Pane

use crate::app::App;
use crate::theme::{Rgb, Theme};
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

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
        // the pixel tier: the glow is one smooth image at half resolution (the terminal scales it; a soft field loses
        // nothing), and the cover goes over it as its own sharp, full-resolution image (home::cover below), uploaded
        // once per song (the owner: baked into the glow at half resolution it looked "kinda low res")
        if let Some((cpw, cph)) = app.pixel_tier_for(r) {
            let (w, h) = (((r.width as f64 * cpw) / 2.0).round().max(8.0) as u32, ((r.height as f64 * cph) / 2.0).round().max(8.0) as u32);
            let (sx, sy) = (r.width as f32 / w as f32, r.height as f32 * ar / h as f32);
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                let py = r.y as f32 * ar + (y as f32 + 0.5) * sy;
                for x in 0..w {
                    let c = shade(r.x as f32 + (x as f32 + 0.5) * sx, py);
                    rgba.extend_from_slice(&[c.0, c.1, c.2, 255]);
                }
            }
            if let Some(k) = app.kitty.as_mut() {
                k.keep_under_stills(GLOW_ID);
                k.frame(GLOW_ID, r, w, h, std::sync::Arc::new(rgba));
            }
            app.under_image(buf, r, th);
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

    #[test]
    fn glow_puts_the_cover_over_its_field_as_a_sharp_image_of_its_own() {
        let th = Theme::singularity();
        let mut app = App::new(None);
        app.covers.insert("https://i.scdn.co/image/c", image::RgbImage::from_fn(64, 64, |x, y| image::Rgb([200, (x * 3) as u8, (y * 3) as u8])));
        app.playback.item = Some(singularity_proto::Item { name: "Song".into(), cover_url: Some("https://i.scdn.co/image/c".into()), ..Default::default() });
        app.playback.status = singularity_proto::PlayStatus::Playing;
        app.cell_px = Some((10.0, 20.0));
        app.kitty = Some(crate::gfx::Kitty::new(false));
        app.kitty.as_mut().unwrap().below_backgrounds = true;
        let r = Rect::new(1, 1, 60, 20);
        let mut buf = Buffer::empty(Rect::new(0, 0, 62, 22));
        glow(&mut buf, r, &mut app, &th);
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        let apc = String::from_utf8_lossy(&out);
        // the glow field, at half resolution (60·10/2 × 20·20/2), one layer under the stills
        assert!(apc.contains(&format!("i={},s=300,v=200", GLOW_ID * 2)), "{apc:.300}");
        assert!(apc.contains("z=-1073741826"), "the glow sits under the cover");
        // the cover: 10 rows × 20 cells, sent at full resolution (200×200) and placed at the default z
        assert!(apc.contains("s=200,v=200"), "the cover goes up at full resolution");
        assert!(apc.contains("a=p,i=1000,p=1,c=20,r=10,C=1,z=-1073741825"), "the cover is placed over the glow");
    }
}
