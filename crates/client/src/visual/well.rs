//! The gravity well (#16): the Pane that holds the black hole. Its own frame sags toward the hole, breathing with the
//! bass, and the border lines of neighbouring Panes bend toward it. Only structure moves (box-drawing lines, redrawn as
//! braille dots); text, titles and content never move, and no dot ever lands on text.

use crate::app::{App, Hit};
use crate::theme::{Rgb, Theme};
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::collections::HashMap;

/// kitty image id of the hole (ids below 1000 are Visuals)
const HOLE_ID: u32 = 1;
const BITS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Where the hole sits this frame, for bending the neighbours: the well's rect and its centre in cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hole {
    pub well: Rect,
    pub cx: f32,
    pub cy: f32,
}

/// Draw the well over `outer` (the Pane's full rect, frame included).
pub fn draw(buf: &mut Buffer, outer: Rect, app: &mut App, th: &Theme) {
    for y in outer.top()..outer.bottom() {
        text(buf, outer.x, y, outer.width, &" ".repeat(outer.width as usize), th.base());
    }
    let inner = Rect::new(outer.x + 2, outer.y + 1, outer.width.saturating_sub(4), outer.height.saturating_sub(2));
    app.hit(outer, Hit::Well);
    // its hole is out (idle takeover): the well is empty, and nothing bends
    if app.idle_secs().is_some() {
        crate::visual::idle::empty_well(buf, inner, th);
        warped_frame(buf, outer, 0.0, [th.border, th.dim]);
        text(buf, outer.x + 2, outer.y, 14, " gravity well ", th.s(th.accent));
        return;
    }
    remember_origin(inner, app);
    if render(buf, inner, app) {
        app.hole_at = Some(Hole { well: outer, cx: inner.x as f32 + inner.width as f32 / 2.0, cy: inner.y as f32 + inner.height as f32 / 2.0 });
    }
    let focused = app.focus == "well";
    warped_frame(buf, outer, 0.5 + app.hole.bass() * 0.9, if focused { [th.border_focus, th.accent] } else { [th.border, th.dim] });
    text(buf, outer.x + 2, outer.y, 14, " gravity well ", if focused { th.title() } else { th.s(th.accent) });
    look_label(buf, outer, app, th);
}

/// The black hole on its own (Event Horizon's): a plain Pane, no well, nothing bends. `inner` is inside the frame.
pub fn draw_plain(buf: &mut Buffer, outer: Rect, inner: Rect, app: &mut App, th: &Theme) {
    app.hit(outer, Hit::Well);
    if app.idle_secs().is_some() {
        return crate::visual::idle::empty_well(buf, inner, th);
    }
    remember_origin(inner, app);
    render(buf, inner, app);
    look_label(buf, outer, app, th);
}

/// Where the hole sits, for the idle takeover to break out from: its centre, and its shadow radius in cell widths.
fn remember_origin(inner: Rect, app: &mut App) {
    let rh = crate::visual::blackhole::fit_rh(inner.width as usize, inner.height as usize * 2, app.hole.look());
    app.hole_origin = Some(crate::visual::idle::Origin { cx: inner.x as f32 + inner.width as f32 / 2.0, cy: inner.y as f32 + inner.height as f32 / 2.0, rh: rh * inner.height as f32 * 2.0 });
}

fn look_label(buf: &mut Buffer, outer: Rect, app: &App, th: &Theme) {
    let label = format!(" ◂ {} ▸ ", app.hole.look().name);
    let w = label.chars().count() as u16;
    // the × that closes the Pane takes the last cells of the title
    if outer.width > w + 21 {
        text(buf, outer.right() - w - 5, outer.y, w, &label, th.s(th.dim));
    }
}

/// The hole's picture over `inner`, pixels or half-blocks. False if there's no room for it.
fn render(buf: &mut Buffer, inner: Rect, app: &mut App) -> bool {
    if inner.width >= 4 && inner.height >= 3 {
        if let Some((cw, ch)) = app.pixel_tier_for(inner) {
            let (w, h) = app.hole.render_size((inner.width as f64 * cw) as usize, (inner.height as f64 * ch) as usize);
            let (px, fresh) = app.hole.frame(w, h);
            if let Some(k) = app.kitty.as_mut()
                && (fresh || !k.keep(HOLE_ID, inner))
            {
                k.frame(HOLE_ID, inner, w as u32, h as u32, px);
            }
            let th = app.theme.clone();
            app.under_image(buf, inner, &th);
        } else {
            let (w, h) = (inner.width as usize, inner.height as usize * 2);
            let (px, _) = app.hole.frame(w, h);
            let img = image::RgbImage::from_fn(w as u32, h as u32, |x, y| {
                let i = (y as usize * w + x as usize) * 4;
                image::Rgb([px[i], px[i + 1], px[i + 2]])
            });
            crate::gfx::blit_halfblocks(&img, buf, inner);
        }
        return true;
    }
    false
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Color {
    let m = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * t.clamp(0.0, 1.0)) as u8;
    Color::Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// A rectangle whose edges are pulled toward its centre, drawn with braille dots (denser dots are brighter).
fn warped_frame(buf: &mut Buffer, r: Rect, strength: f32, colors: [Rgb; 2]) {
    let (dw, dh) = (r.width as usize * 2, r.height as usize * 4);
    let mut dots = vec![0.0f32; dw * dh];
    let mut plot = |x: f32, y: f32| {
        if x >= 0.0 && y >= 0.0 && (x as usize) < dw && (y as usize) < dh {
            dots[y as usize * dw + x as usize] += 0.6;
        }
    };
    let (w, h) = (dw as f32 - 1.0, dh as f32 - 1.0);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let sag = |u: f32| (-(u * 2.6).powi(2)).exp(); // u = -0.5..0.5 along an edge
    let steps = ((w + h) * 3.0) as usize;
    for i in 0..=steps {
        let u = i as f32 / steps as f32 - 0.5;
        let dy = sag(u) * strength * h * 0.10;
        let dx = sag(u) * strength * w * 0.035;
        plot(cx + u * w, dy);
        plot(cx + u * w, h - dy);
        plot(dx, cy + u * h);
        plot(w - dx, cy + u * h);
    }
    for cy in 0..r.height as usize {
        for cx in 0..r.width as usize {
            let (mut bits, mut sum) = (0u32, 0.0f32);
            for (dy, row) in BITS.iter().enumerate() {
                for (dx, bit) in row.iter().enumerate() {
                    let v = dots[(cy * 4 + dy) * dw + cx * 2 + dx];
                    if v > 0.0 {
                        bits |= bit;
                        sum += v;
                    }
                }
            }
            if bits != 0
                && let Some(c) = buf.cell_mut((r.x + cx as u16, r.y + cy as u16))
            {
                c.set_char(char::from_u32(0x2800 + bits).unwrap_or(' ')).set_fg(mix(colors[0], colors[1], sum / 4.0));
            }
        }
    }
}

/// Box-drawing line segments of a glyph: left, right, up, down.
fn segments(ch: char) -> Option<[bool; 4]> {
    Some(match ch {
        '─' | '━' | '═' => [true, true, false, false],
        '│' | '┃' | '║' => [false, false, true, true],
        '┌' | '╭' | '╔' | '┏' => [false, true, false, true],
        '┐' | '╮' | '╗' | '┓' => [true, false, false, true],
        '└' | '╰' | '╚' | '┗' => [false, true, true, false],
        '┘' | '╯' | '╝' | '┛' => [true, false, true, false],
        '├' | '╠' => [false, true, true, true],
        '┤' | '╣' => [true, false, true, true],
        '┬' | '╦' => [true, true, false, true],
        '┴' | '╩' => [true, true, true, false],
        '┼' | '╬' => [true, true, true, true],
        _ => return None,
    })
}

/// Bend the border lines near the hole toward it. `body` bounds the effect; the well itself is left alone, with a
/// one-cell moat so neighbours bow toward it without touching it.
pub fn bend_borders(buf: &mut Buffer, body: Rect, hole: Hole, bass: f32) {
    let well = hole.well;
    // gravity reaches about one well-size out; the pull breathes gently with the bass
    let falloff = (well.width as f32).hypot(well.height as f32 * 2.0) * 0.55;
    let amp = 2.2 * (1.0 + bass * 0.3);
    let (cx, cy) = (hole.cx, hole.cy);
    let pull_at = |ux: f32, uy: f32| {
        let (dx, dy) = (ux - cx, uy - cy * 2.0);
        let dist = (dx * dx + dy * dy).sqrt().max(1e-3);
        (dx / dist, dy / dist, amp * (-(dist / falloff).powi(2)).exp())
    };
    let reach = falloff * 2.2;
    let x0 = (cx - reach).max(body.x as f32) as u16;
    let x1 = ((cx + reach) as u16).min(body.right());
    let y0 = (cy - reach / 2.0).max(body.y as f32) as u16;
    let y1 = ((cy + reach / 2.0) as u16).min(body.bottom());
    let inside = |x: u16, y: u16, r: Rect| x >= r.x && x < r.right() && y >= r.y && y < r.bottom();
    // sample points along every border segment, in braille-dot coordinates
    let mut pts: Vec<(f32, f32, Color, bool)> = vec![];
    for y in y0..y1 {
        for x in x0..x1 {
            if inside(x, y, well) {
                continue;
            }
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            let Some(s) = cell.symbol().chars().next().and_then(segments) else { continue };
            // cells that would barely move keep their solid glyph: seamless from straight to curved
            if pull_at(x as f32 + 0.5, (y as f32 + 0.5) * 2.0).2 < 0.45 {
                continue;
            }
            let col = cell.fg;
            let (bx, by) = (x as f32 * 2.0 + 0.5, y as f32 * 4.0 + 1.5);
            let mut add = |dx0: f32, dy0: f32, dx1: f32, dy1: f32| {
                for i in 0..=6 {
                    let t = i as f32 / 6.0;
                    pts.push((bx + dx0 + (dx1 - dx0) * t, by + dy0 + (dy1 - dy0) * t, col, dy0 == dy1));
                }
            };
            if s[0] {
                add(-0.5, 0.0, 0.0, 0.0);
            }
            if s[1] {
                add(0.0, 0.0, 1.5, 0.0);
            }
            if s[2] {
                add(0.0, -1.5, 0.0, 0.0);
            }
            if s[3] {
                add(0.0, 0.0, 0.0, 2.5);
            }
            cell.set_char(' ');
        }
    }
    let moat = Rect::new(well.x.saturating_sub(1), well.y.saturating_sub(1), well.width + 2, well.height + 2);
    let in_moat = |nx: f32, ny: f32| inside((nx / 2.0) as u16, (ny / 4.0) as u16, moat);
    let mut cells: HashMap<(u16, u16), (u32, Color)> = HashMap::new();
    for (px, py, col, horiz) in pts {
        let (ux, uy) = (px / 2.0, py / 2.0);
        let (dxn, dyn_, pull) = pull_at(ux, uy);
        // lines only move sideways (perpendicular to themselves): no gaps, no bunching
        let (mut sx, mut sy) = if horiz { (0.0, dyn_ * pull) } else { (dxn * pull, 0.0) };
        let mut np = ((ux - sx) * 2.0, (uy - sy) * 2.0);
        for _ in 0..5 {
            if !in_moat(np.0, np.1) {
                break;
            }
            sx *= 0.5;
            sy *= 0.5;
            np = ((ux - sx) * 2.0, (uy - sy) * 2.0);
        }
        if np.0 < 0.0 || np.1 < 0.0 {
            continue;
        }
        let key = ((np.0 / 2.0) as u16, (np.1 / 4.0) as u16);
        let e = cells.entry(key).or_insert((0, col));
        e.0 |= BITS[(np.1 as usize) % 4][(np.0 as usize) % 2];
    }
    for ((x, y), (bits, col)) in cells {
        if !inside(x, y, body) || inside(x, y, well) {
            continue;
        }
        if let Some(c) = buf.cell_mut((x, y)) {
            let ch = c.symbol().chars().next().unwrap_or(' ');
            // only blank cells or other braille: never on top of text
            if ch == ' ' || ('\u{2800}'..='\u{28ff}').contains(&ch) {
                let old = if ch == ' ' { 0 } else { ch as u32 - 0x2800 };
                c.set_char(char::from_u32(0x2800 + (old | bits)).unwrap_or(' ')).set_fg(col);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    #[test]
    fn bending_moves_only_lines_never_text() {
        // a well in the middle with a neighbouring Pane border beside it, and text next to the border
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        for y in 0..20 {
            buf.set_string(20, y, "│", Style::default());
        }
        buf.set_string(10, 9, "Daily Mix", Style::default());
        let well = Rect::new(22, 5, 20, 10);
        bend_borders(&mut buf, area, Hole { well, cx: 32.0, cy: 10.0 }, 0.5);
        assert_eq!((10..19).map(|x| buf[(x, 9)].symbol().to_string()).collect::<String>(), "Daily Mix", "text is untouched");
        // near the hole the straight line became braille, bowed toward it; far away it stays a line
        let row = |y: u16| (0..60).map(|x| buf[(x, y)].symbol().chars().next().unwrap()).collect::<Vec<char>>();
        assert!(row(10).iter().any(|c| ('\u{2801}'..='\u{28ff}').contains(c)), "bent near the hole");
        assert_eq!(buf[(20, 0)].symbol(), "│", "straight far away");
        for y in well.top()..well.bottom() {
            for x in well.left()..well.right() {
                assert_eq!(buf[(x, y)].symbol(), " ", "nothing lands in the well");
            }
        }
    }

    #[test]
    fn the_frame_sags_toward_the_centre() {
        let r = Rect::new(0, 0, 30, 12);
        let mut buf = Buffer::empty(r);
        warped_frame(&mut buf, r, 1.0, [Rgb(40, 40, 40), Rgb(90, 90, 90)]);
        // the top edge's middle sits lower than its ends
        let first_row_with_dot = |x: u16| (0..12).find(|&y| buf[(x, y)].symbol() != " ").unwrap();
        assert!(first_row_with_dot(15) > first_row_with_dot(1));
    }
}
