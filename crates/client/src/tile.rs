//! Drawing pieces the tiling Widgets share (stream, top, listening, today, top this week): a three-row figure for a
//! headline number, horizontal bars and gauges in eighths, column charts and sparklines. They write cells directly
//! (no Strings per frame) and stay inside the Rect they are given.
#![allow(clippy::too_many_arguments)] // positions, sizes and colours, like ratatui's own drawing calls

use crate::theme::{Rgb, Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

const H_EIGHTHS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
const V_EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn put(buf: &mut Buffer, x: u16, y: u16, ch: char, st: Style) {
    if let Some(c) = buf.cell_mut((x, y)) {
        c.set_char(ch).set_style(st);
    }
}

/// The figure's glyphs: three rows, three columns (`.` and `:` one column).
fn glyph(c: char) -> Option<[&'static str; 3]> {
    Some(match c {
        '0' => ["█▀█", "█ █", "▀▀▀"],
        '1' => ["▀█ ", " █ ", "▀▀▀"],
        '2' => ["▀▀█", "█▀▀", "▀▀▀"],
        '3' => ["▀▀█", " ▀█", "▀▀▀"],
        '4' => ["█ █", "▀▀█", "  ▀"],
        '5' => ["█▀▀", "▀▀█", "▀▀▀"],
        '6' => ["█▀▀", "█▀█", "▀▀▀"],
        '7' => ["▀▀█", "  █", "  ▀"],
        '8' => ["█▀█", "█▀█", "▀▀▀"],
        '9' => ["█▀█", "▀▀█", "▀▀▀"],
        '.' => [" ", " ", "▀"],
        ':' => [" ", "▀", "▀"],
        '-' => ["   ", "▀▀▀", "   "],
        _ => return None,
    })
}

/// Width of `s` as a figure: digits 3 + 1 gap, `.`/`:` 1 + 1; anything else counts as a small letter (1).
pub fn figure_width(s: &str) -> u16 {
    s.chars().map(|c| glyph(c).map(|g| g[0].chars().count() as u16 + 1).unwrap_or(1)).sum::<u16>().saturating_sub(1)
}

/// `s` as a three-row figure from (`x`, `y`): digits big in `big`, other characters (units) small on the bottom row in
/// `small`. Returns the width drawn. Stops at `max` columns.
pub fn figure(buf: &mut Buffer, x: u16, y: u16, max: u16, s: &str, big: Style, small: Style) -> u16 {
    let mut cx = x;
    let end = x.saturating_add(max);
    for c in s.chars() {
        match glyph(c) {
            Some(g) => {
                let gw = g[0].chars().count() as u16;
                if cx + gw > end {
                    break;
                }
                for (row, line) in g.iter().enumerate() {
                    for (i, ch) in line.chars().enumerate() {
                        if ch != ' ' {
                            put(buf, cx + i as u16, y + row as u16, ch, big);
                        }
                    }
                }
                cx += gw + 1;
            }
            None => {
                if cx >= end {
                    break;
                }
                // a unit sits on the bottom row, right after the digit's gap is taken back
                let ux = if cx > x { cx - 1 } else { cx };
                put(buf, ux, y + 2, c, small);
                cx = ux + 2;
            }
        }
    }
    cx.saturating_sub(x).saturating_sub(1).min(max)
}

/// A horizontal bar of `cells` cells filled to `t` (eighths) in `fill`, the rest a faint track in `track` (if any).
pub fn hbar(buf: &mut Buffer, x: u16, y: u16, cells: u16, t: f32, fill: Rgb, track: Option<Rgb>, th: &Theme) {
    let eighths = (t.clamp(0.0, 1.0) * cells as f32 * 8.0).round() as u16;
    for i in 0..cells {
        let e = eighths.saturating_sub(i * 8).min(8) as usize;
        let st = th.s(fill);
        match (e, track) {
            (0, Some(tr)) => put(buf, x + i, y, '─', th.s(tr)),
            (8, _) => put(buf, x + i, y, '█', st),
            // a partial cell on the track's background, so the bar's tip doesn't float
            (e, _) => put(buf, x + i, y, H_EIGHTHS[e], st),
        }
    }
}

/// A gauge: a bar of `cells` cells coloured along the theme's ramp as it fills (each cell its own colour).
pub fn gauge(buf: &mut Buffer, x: u16, y: u16, cells: u16, t: f32, th: &Theme) {
    let eighths = (t.clamp(0.0, 1.0) * cells as f32 * 8.0).round() as u16;
    let track = th.bg.lerp(th.border, 0.6);
    for i in 0..cells {
        let e = eighths.saturating_sub(i * 8).min(8) as usize;
        let col = th.ramp_at(0.25 + 0.75 * (i as f32 + 0.5) / cells.max(1) as f32);
        if e == 0 {
            put(buf, x + i, y, '▬', th.s(track));
        } else {
            put(buf, x + i, y, H_EIGHTHS[e], th.s(col));
        }
    }
}

/// A column `rows` tall at (`x`, bottom row `y_bottom`), filled to `t` in eighths, `w` cells wide.
pub fn column(buf: &mut Buffer, x: u16, y_bottom: u16, w: u16, rows: u16, t: f32, col: Rgb, th: &Theme) {
    let eighths = (t.clamp(0.0, 1.0) * rows as f32 * 8.0).round() as u16;
    for r in 0..rows {
        let e = eighths.saturating_sub(r * 8).min(8) as usize;
        if e == 0 {
            break;
        }
        for i in 0..w {
            put(buf, x + i, y_bottom.saturating_sub(r), V_EIGHTHS[e], th.s(col));
        }
    }
}

/// A one-row sparkline of `vals` (the newest at the right edge), scaled to `max`, each cell coloured along the ramp
/// by its height. Fewer values than cells leave the left side empty.
pub fn sparkline(buf: &mut Buffer, r: Rect, vals: impl DoubleEndedIterator<Item = f32> + ExactSizeIterator, max: f32, th: &Theme) {
    let n = vals.len().min(r.width as usize) as u16;
    let start = r.right() - n;
    for (i, v) in vals.rev().take(n as usize).enumerate() {
        let t = (v / max.max(f32::EPSILON)).clamp(0.0, 1.0);
        let e = ((t * 8.0).round() as usize).max(usize::from(v > 0.0));
        let ch = if e == 0 { '▁' } else { V_EIGHTHS[e] };
        let st = if e == 0 { th.s(th.bg.lerp(th.border, 0.7)) } else { th.s(th.ramp_at(0.3 + 0.7 * t)) };
        put(buf, start + n - 1 - i as u16, r.y, ch, st);
    }
}

/// An area graph of `vals` over the whole Rect (newest at the right), scaled to `max`: each column a bar in eighths,
/// coloured by `colour(value)`.
pub fn area(buf: &mut Buffer, r: Rect, vals: impl ExactSizeIterator<Item = f32>, max: f32, th: &Theme, colour: impl Fn(f32) -> Rgb) {
    let len = vals.len();
    let n = len.min(r.width as usize);
    let x0 = r.right() - n as u16;
    for (i, v) in vals.skip(len - n).enumerate() {
        let t = (v / max.max(f32::EPSILON)).clamp(0.0, 1.0);
        column(buf, x0 + i as u16, r.bottom() - 1, 1, r.height, t, colour(v), th);
    }
}

/// A dim, small-caps-like label ("CODEC") and its value under it, bold. Returns nothing; clips to `w`.
pub fn stat(buf: &mut Buffer, x: u16, y: u16, w: u16, label: &str, value: &str, value_st: Style, th: &Theme) {
    crate::ui::text(buf, x, y, w, label, th.s(th.dim));
    crate::ui::text(buf, x, y + 1, w, value, value_st.add_modifier(Modifier::BOLD));
}

/// A thin rule across `w` cells, for separating sections of a tall Widget.
pub fn rule(buf: &mut Buffer, x: u16, y: u16, w: u16, th: &Theme) {
    let st = th.s(th.bg.lerp(th.border, 0.7));
    for i in 0..w {
        put(buf, x + i, y, '─', st);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_figure_is_three_rows_and_measures_what_it_draws() {
        let th = Theme::singularity();
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 3));
        let w = figure(&mut buf, 0, 0, 30, "12h05m", th.s(th.accent), th.s(th.dim));
        assert_eq!(w, figure_width("12h05m"));
        let row = |y| (0..30).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert!(row(0).starts_with("▀█  ▀▀█"), "{:?}", row(0));
        assert!(row(2).contains('h') && row(2).contains('m'));
        // cut, not overflowing
        let mut small = Buffer::empty(Rect::new(0, 0, 5, 3));
        figure(&mut small, 0, 0, 5, "1234", th.s(th.accent), th.s(th.dim));
    }

    #[test]
    fn graphs_stay_in_their_rect() {
        let th = Theme::singularity();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let vals: Vec<f32> = (0..30).map(|i| i as f32).collect();
        area(&mut buf, Rect::new(2, 1, 5, 3), vals.iter().copied(), 29.0, &th, |_| th.accent);
        sparkline(&mut buf, Rect::new(0, 0, 10, 1), vals.iter().copied(), 29.0, &th);
        hbar(&mut buf, 0, 3, 10, 0.55, th.accent, Some(th.border), &th);
        gauge(&mut buf, 0, 3, 10, 1.5, &th);
        assert_eq!(buf[(6, 1)].symbol(), "█", "the newest, tallest value at the right");
        assert_eq!(buf[(1, 1)].symbol(), " ");
    }
}
