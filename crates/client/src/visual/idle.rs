//! Idle takeover (#17, D8): after a while without input *the* black hole on screen leaves its Pane, grows and drifts
//! over the interface, bending it (each cell near the hole shows the cell its light ray came from). Designs without a
//! hole get one that falls in from the top-right corner. Any input wakes the UI at once and is swallowed.

use crate::app::App;
use crate::theme::Theme;
use crate::ui::text;
use crate::visual::blackhole;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// kitty image id of the hole out of its well (ids below 1000 are Visuals; the well's is 1)
const FREE_ID: u32 = 2;
/// Seconds the hole takes to break out of its Pane onto its drift path.
const BREAK_OUT: f32 = 4.0;

/// Where the hole was before it left: centre (cells) and shadow radius, in cell widths (a cell is ~1:2, so y distances
/// count double).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Origin {
    pub cx: f32,
    pub cy: f32,
    pub rh: f32,
}

/// The hole's place `secs` into the takeover: from `from` onto a slow Lissajous drift around the body.
pub fn place(body: Rect, from: Option<Origin>, secs: f32, look: &blackhole::DiskLook) -> Origin {
    let from = from.unwrap_or(Origin { cx: body.right() as f32 + 6.0, cy: body.y as f32 - 3.0, rh: 0.4 });
    let t = secs * 0.06;
    let lx = 0.75 * (t * 0.37).sin() + 0.25 * (t * 0.83 + 1.0).sin();
    let ly = 0.70 * (t * 0.54 + 2.1).sin() + 0.30 * (t * 1.07).sin();
    let rh_t = (body.height as f32 * 2.0 * 0.07).max(2.5);
    let reach = rh_t * blackhole::reach(look);
    let tx = body.x as f32 + body.width as f32 * 0.5 + lx * (body.width as f32 * 0.5 - reach * 0.6).max(0.0);
    let ty = body.y as f32 + body.height as f32 * 0.45 + ly * (body.height as f32 * 0.5 - reach * 0.3).max(0.0);
    let k = (secs / BREAK_OUT).clamp(0.0, 1.0);
    let e = k * k * (3.0 - 2.0 * k);
    Origin { cx: from.cx + (tx - from.cx) * e, cy: from.cy + (ty - from.cy) * e, rh: from.rh + (rh_t - from.rh) * e }
}

/// Draw the takeover over `body` (after the Panes, before any popup).
pub fn draw(buf: &mut Buffer, body: Rect, app: &mut App, th: &Theme) {
    let Some(secs) = app.idle_secs() else { return };
    if body.width < 20 || body.height < 10 {
        return;
    }
    let look = *app.hole.look();
    let o = place(body, app.hole_origin, secs, &look);
    // the UI dims as the hole takes over
    let e = (secs / BREAK_OUT).clamp(0.0, 1.0);
    let dim = 1.0 - 0.5 * e * e * (3.0 - 2.0 * e);
    for y in body.top()..body.bottom() {
        for x in body.left()..body.right() {
            if let Some(c) = buf.cell_mut((x, y))
                && let Color::Rgb(r, g, b) = c.fg
            {
                let s = |v: u8| (v as f32 * dim) as u8;
                c.set_fg(Color::Rgb(s(r), s(g), s(b)));
            }
        }
    }
    lens_cells(buf, body, o.cx, o.cy, o.rh, th);
    // the hole itself, in a box around it (cut by the body's edges)
    let reach = o.rh * blackhole::reach(&look);
    let x0 = (o.cx - reach).max(body.x as f32) as u16;
    let y0 = (o.cy - reach / 2.0).max(body.y as f32) as u16;
    let x1 = ((o.cx + reach).ceil() as u16).min(body.right());
    let y1 = ((o.cy + reach / 2.0).ceil() as u16).min(body.bottom());
    let r = Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0));
    if r.width < 2 || r.height < 2 {
        return;
    }
    let center = ((o.cx - r.x as f32) / r.width as f32, (o.cy - r.y as f32) / r.height as f32);
    // radius as a fraction of the box's height (radius in cell heights is rh / 2)
    let rh = o.rh / 2.0 / r.height as f32;
    if let Some((cw, ch)) = app.pixel_tier_for(r) {
        let (w, h) = app.hole.render_size((r.width as f64 * cw) as usize, (r.height as f64 * ch) as usize);
        let (px, fresh) = app.hole.free_frame(w, h, rh, center);
        if let Some(k) = app.kitty.as_mut()
            && (fresh || !k.keep(FREE_ID, r))
        {
            k.frame(FREE_ID, r, w as u32, h as u32, px);
        }
    } else {
        // half-blocks where the disk is lit; elsewhere the lensed UI stays readable
        let (w, h) = (r.width as usize, r.height as usize * 2);
        let (px, _) = app.hole.free_frame(w, h, rh, center);
        let at = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            (px[i], px[i + 1], px[i + 2])
        };
        for cy in 0..r.height as usize {
            for cx in 0..w {
                let (t, b) = (at(cx, cy * 2), at(cx, cy * 2 + 1));
                let lit = |p: (u8, u8, u8)| p.0 as u16 + p.1 as u16 + p.2 as u16 > 60;
                if !lit(t) && !lit(b) {
                    continue;
                }
                if let Some(c) = buf.cell_mut((r.x + cx as u16, r.y + cy as u16)) {
                    c.set_symbol("▀").set_fg(Color::Rgb(t.0, t.1, t.2)).set_bg(Color::Rgb(b.0, b.1, b.2));
                }
            }
        }
    }
}

/// Bend the UI around a hole at (`cx`, `cy`) with shadow radius `rh` (cell widths; y counts double): every cell near it
/// shows the cell its light ray came from, mirrored at the edges like the shader's. Inside the shadow: black.
pub fn lens_cells(buf: &mut Buffer, area: Rect, cx: f32, cy: f32, rh: f32, th: &Theme) {
    let src = buf.clone();
    let reach = rh * 6.5;
    let x0 = ((cx - reach).floor().max(area.x as f32)) as u16;
    let x1 = ((cx + reach).ceil().min(area.right() as f32)).max(area.x as f32) as u16;
    let y0 = ((cy - reach / 2.0).floor().max(area.y as f32)) as u16;
    let y1 = ((cy + reach / 2.0).ceil().min(area.bottom() as f32)).max(area.y as f32) as u16;
    let mirror = |v: f32, lo: f32, hi: f32| {
        let span = hi - lo;
        let u = (v - lo).rem_euclid(2.0 * span);
        lo + if u > span { 2.0 * span - u } else { u }
    };
    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5 - cx;
            let py = (y as f32 + 0.5 - cy) * 2.0;
            let plen = (px * px + py * py).sqrt().max(1e-4);
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            if plen < rh * 1.02 {
                cell.set_char(' ').set_bg(th.bg.color());
                continue;
            }
            let window = (-(plen / (4.5 * rh)).powi(2)).exp();
            let defl = 5.5 * rh * rh / plen * window;
            let sx = mirror(cx + px - px / plen * defl, area.x as f32, area.right() as f32 - 0.01) as u16;
            let sy = mirror(cy + (py - py / plen * defl) / 2.0, area.y as f32, area.bottom() as f32 - 0.01) as u16;
            if let Some(s) = src.cell((sx, sy)) {
                *cell = s.clone();
            }
        }
    }
}

/// What the well shows while its hole is out.
pub fn empty_well(buf: &mut Buffer, inner: Rect, th: &Theme) {
    let msg = "· the well is empty ·";
    let w = msg.chars().count() as u16;
    if inner.width > w {
        text(buf, inner.x + (inner.width - w) / 2, inner.y + inner.height / 2, w, msg, th.s(th.dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hole_breaks_out_then_drifts_inside_the_body() {
        let body = Rect::new(0, 1, 160, 43);
        let look = blackhole::LOOKS[0];
        let from = Origin { cx: 140.0, cy: 10.0, rh: 3.0 };
        assert_eq!(place(body, Some(from), 0.0, &look), from, "starts where it was");
        for s in [10.0, 100.0, 1000.0] {
            let o = place(body, Some(from), s, &look);
            assert!(o.cx > 0.0 && o.cx < 160.0 && o.cy > 1.0 && o.cy < 44.0, "{o:?} after {s}s");
        }
        // with no hole on screen it falls in from the top-right corner
        let corner = place(body, None, 0.0, &look);
        assert!(corner.cx > 160.0 && corner.cy < 1.0);
    }

    #[test]
    fn lensing_blacks_out_the_shadow_and_leaves_far_cells_alone() {
        let th = Theme::singularity();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        for y in 0..24 {
            text(&mut buf, 0, y, 80, &format!("{y:02}").repeat(40), th.s(th.fg));
        }
        let before = buf.clone();
        lens_cells(&mut buf, area, 40.0, 12.0, 3.0, &th);
        assert_eq!(buf[(40, 12)].symbol(), " ", "the shadow");
        assert_eq!(buf[(1, 1)], before[(1, 1)], "far away nothing moves");
        let moved = (0..24).flat_map(|y| (0..80).map(move |x| (x, y))).filter(|&(x, y)| buf[(x, y)] != before[(x, y)]).count();
        assert!(moved > 40, "the neighbourhood bends ({moved} cells)");
    }
}
