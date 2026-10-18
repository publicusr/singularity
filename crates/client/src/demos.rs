//! Visual demos (D36): Widgets to try in your own layout before any of them becomes a ticket. Each is marked "demo" in
//! its Pane title and sits in the ◂ ▸ cycle of the Slots it fits. The ones the owner doesn't keep are deleted.
//!
//! - **starfield** (audio Slots): flying through stars, faster with the music, streaking on beats
//! - **up next** (side Slots): the queue as a timeline, songs as long as they last
//! - **marquee** (side Slots): the title in a big pixel font, scrolling when it doesn't fit
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
}

#[derive(Clone, Copy)]
struct Star {
    x: f32,
    y: f32,
    z: f32,
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
    let s = &mut app.demos;
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
        s.stars.push(Star { x, y, z });
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
            *st = Star { x: rnd(&mut seed) * 2.0 - 1.0, y: rnd(&mut seed) * 2.0 - 1.0, z: 1.0 };
            continue;
        }
        let near = (1.0 - st.z).clamp(0.0, 1.0);
        let col = th.ramp_at(0.2 + 0.8 * near);
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

// ── up next ───────────────────────────────────────────────────────────

/// The queue as a timeline: what's playing (with its progress), then each coming song as long as it lasts.
pub fn river(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    if w < 10 || r.height == 0 {
        return;
    }
    let Some(now) = app.playback.item.clone() else {
        return text(buf, x, r.y, w, "nothing playing", th.s(th.dim));
    };
    let pos = app.display_position(singularity_proto::now_ms());
    let left = now.duration_ms.saturating_sub(pos);
    // the width covers the next half hour (or less, if the queue is short)
    let mut songs: Vec<(String, u64, Rgb, bool)> = vec![(now.name.clone(), left.max(1), th.accent, true)];
    for e in &app.playback.queue {
        let col = match e.source.as_str() {
            "queue" => th.accent2,
            "autoplay" => th.border,
            _ => th.fg,
        };
        songs.push((e.item.name.clone(), if e.item.duration_ms > 0 { e.item.duration_ms } else { 180_000 }, col, false));
    }
    let span: u64 = songs.iter().map(|s| s.1).sum::<u64>().min(30 * 60_000).max(60_000);
    let per_cell = span as f32 / w as f32;
    // vertically centred: the band, names under it, the time axis
    let y = r.y + r.height.saturating_sub(3) / 2;
    let mut cx = 0f32;
    for (i, (name, ms, col, playing)) in songs.iter().enumerate() {
        let start = cx;
        cx += *ms as f32 / per_cell;
        let (a, b) = (start.round() as u16, (cx.round() as u16).min(w));
        if a >= w {
            break;
        }
        let seg = b.saturating_sub(a).max(1);
        // alternate the shade so neighbours stay apart; the playing song shows how far it is
        let base = if i % 2 == 0 { *col } else { col.scale(0.7) };
        let band = if *playing { "▀".repeat(seg as usize) } else { "▀".repeat(seg.saturating_sub(1) as usize) + " " };
        text(buf, x + a, y, seg, &band, th.s(base));
        if seg > 3 && y + 1 < r.bottom() {
            let label: String = name.chars().take(seg as usize - 1).collect();
            let st = if *playing { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.dim) };
            text(buf, x + a, y + 1, seg - 1, &label, st);
        }
    }
    if y + 2 < r.bottom() {
        // ticks every 5 minutes (every minute when the span is short)
        let step = if span > 12 * 60_000 { 5 * 60_000 } else { 60_000 };
        let mut t = step;
        text(buf, x, y + 2, 3, "now", th.s(th.border));
        while (t as f32) < span as f32 {
            let tx = (t as f32 / per_cell) as u16;
            let label = format!("+{}m", t / 60_000);
            if tx + label.width() as u16 <= w && tx > 4 {
                text(buf, x + tx, y + 2, label.width() as u16, &label, th.s(th.border));
            }
            t += step;
        }
    }
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
    let s = &mut app.demos;
    let dt = s.marquee_last.map(|l| l.elapsed().as_secs_f32().min(0.1)).unwrap_or(0.0);
    s.marquee_last = Some(Instant::now());
    s.marquee_t += dt;
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
                let col = th.ramp_at(0.45 + 0.55 * (1.0 - row as f32 / 3.0));
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

/// The cover in the middle, its average colour (saturated a little) bleeding out into the Pane and fading to black.
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
    let avg = app.covers.scaled(&url, 4, 4, tint.as_ref()).map(|img| {
        let n = img.pixels().len() as f32;
        let sum = img.pixels().fold([0f32; 3], |a, p| [a[0] + p[0] as f32, a[1] + p[1] as f32, a[2] + p[2] as f32]);
        let mean = sum.map(|v| v / n);
        // a little more colour than the average, so a muddy cover still glows
        let grey = (mean[0] + mean[1] + mean[2]) / 3.0;
        let sat = mean.map(|v| (grey + (v - grey) * 1.6).clamp(0.0, 255.0) as u8);
        Rgb(sat[0], sat[1], sat[2])
    });
    if let Some(c) = avg {
        let bass = app.hole.bass();
        let (mx, my) = (cover.x as f32 + cover.width as f32 / 2.0, cover.y as f32 + cover.height as f32 / 2.0);
        let reach = (r.width as f32 / 2.0).max(r.height as f32 * aspect as f32 / 2.0);
        for yy in r.top()..r.bottom() {
            for xx in r.left()..r.right() {
                let d = ((xx as f32 - mx).powi(2) + ((yy as f32 - my) * aspect as f32).powi(2)).sqrt() / reach;
                // breathes gently with the bass
                let k = ((1.0 - d).clamp(0.0, 1.0).powf(2.2) * (0.55 + 0.25 * bass)).clamp(0.0, 1.0);
                if let Some(cell) = buf.cell_mut((xx, yy)) {
                    cell.set_char(' ').set_bg(th.bg.lerp(c, k).color());
                }
            }
        }
    }
    crate::home::cover(buf, cover, &url, "♪", app, th);
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
}
