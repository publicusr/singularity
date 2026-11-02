//! vinyl: a record seen from above, the cover as its label, turning at a steady 33⅓ (never faster with the music:
//! the owner). It coasts to a halt over about a second on pause and spins up again on play; the tonearm creeps in
//! from the edge as the song plays, and a beat brushes a faint shimmer over the grooves.

use super::{Canvas, against_peak, ease, playing, smoothstep, tick};
use crate::app::App;
use crate::theme::{Rgb, Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::f32::consts::{PI, TAU};
use std::time::Instant;

/// Vinyl's Visual id on the pixel tier.
const ID: u32 = 4;
/// 33⅓ turns a minute.
const TURN: f32 = 33.333 / 60.0 * TAU;
/// Seconds to spin up or coast to a halt.
const SPIN_UP: f32 = 1.0;

#[derive(Default)]
pub struct State {
    last: Option<Instant>,
    /// the record's angle, radians
    angle: f32,
    /// 0 still … 1 at speed (moves linearly; the speed is its smoothstep)
    spin: f32,
    /// the beat's shimmer, eased, and the loudest level lately
    shimmer: f32,
    kick: f32,
    peak: f32,
    /// where the stylus sits, 0 the outer edge … 1 the run-out (eased, so a seek glides)
    arm: f32,
}

pub fn vinyl(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.width < 3 || r.height < 2 {
        return;
    }
    let on = playing(app);
    let (level, beat) = app.meters.frame.as_ref().map(|f| (f.out_level as f32, f.beat)).unwrap_or((0.0, false));
    let progress = app.playback.item.as_ref().map(|i| {
        let pos = app.playback.position_at(singularity_proto::now_ms()) as f32;
        if i.duration_ms > 0 { (pos / i.duration_ms as f32).clamp(0.0, 1.0) } else { 0.0 }
    });
    let url = app.playback.item.as_ref().and_then(|i| i.cover_url.clone()).unwrap_or_default();
    let tint = app.art_tint.clone();
    let c = Canvas::new(app, r, 250_000.0);

    let s = &mut app.ambient.vinyl;
    let dt = tick(&mut s.last, 0.25);
    let step = dt / SPIN_UP;
    s.spin = if on { (s.spin + step).min(1.0) } else { (s.spin - step).max(0.0) };
    s.angle = (s.angle + TURN * smoothstep(0.0, 1.0, s.spin) * dt) % TAU;
    let norm = against_peak(&mut s.peak, level, dt, 40.0);
    s.kick = if beat && on { 0.5 + 0.5 * norm } else { s.kick * (-dt / 0.15).exp() };
    ease(&mut s.shimmer, s.kick, dt, 0.08, 0.45);
    // parked off the record when nothing plays
    let arm_to = progress.map(|p| 0.08 + 0.84 * p).unwrap_or(-0.35);
    if s.arm == 0.0 && progress.is_some() {
        s.arm = arm_to;
    }
    ease(&mut s.arm, arm_to, dt, 0.4, 0.4);
    let (angle, shimmer, arm) = (s.angle, s.shimmer, s.arm);

    // the record: as big as fits, nudged left to leave the arm room
    let rad = (c.h / 2.0).min(c.w / 2.6) * 0.94;
    let (cx, cy) = ((c.w / 2.0 - rad * 0.2).max(rad + 0.3), c.h / 2.0);
    let label = rad * 0.36;
    let groove_in = rad * 0.42;
    let aa = c.ux.max(c.uy);
    // the label: the cover's middle, turning with the record
    let side = ((2.0 * label / c.ux).round() as u32).clamp(2, 512);
    let art = if url.is_empty() { None } else { app.covers.scaled(&url, side, side, tint.as_ref()) };
    // the tonearm: its pivot up and to the right of the record, the stylus on a groove to the right of the spindle
    let pivot = ((cx + rad * 1.18).min(c.w - rad * 0.12), cy - rad * 0.86);
    let reach_r = rad * (0.96 - 0.52 * arm.clamp(0.0, 1.0)) + if arm < 0.0 { -arm * rad * 1.2 } else { 0.0 };
    let stylus = stylus_at(pivot, (cx, cy), reach_r);

    let bg = th.bg;
    let vinyl = th.bg.lerp(Rgb(24, 24, 28), 0.9);
    let sheen_col = th.fg.lerp(th.ramp_at(0.6), 0.5);
    let metal = th.dim.lerp(th.fg, 0.55);
    let label_plain = th.accent.scale(0.55);
    let (sa, ca) = (-angle).sin_cos();
    let (hx, hy) = (stylus.0 - pivot.0, stylus.1 - pivot.1);
    let hl = (hx * hx + hy * hy).sqrt().max(1e-3);
    let head_from = (stylus.0 - hx / hl * rad * 0.12, stylus.1 - hy / hl * rad * 0.12);
    let head_col = metal.lerp(th.fg, 0.3);
    c.paint(buf, r, app, th, ID, |_, _, x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let d = (dx * dx + dy * dy).sqrt();
        let mut col = bg;
        if d < rad + aa {
            let phi = dy.atan2(dx);
            col = if d < label {
                // turn the point back by the record's angle and read the cover there
                let (u, v) = (dx * ca - dy * sa, dx * sa + dy * ca);
                let lab = match &art {
                    Some(a) => {
                        let (w, h) = (a.width() as f32, a.height() as f32);
                        let px = ((u / label * 0.5 + 0.5) * w).clamp(0.0, w - 1.0) as u32;
                        let py = ((v / label * 0.5 + 0.5) * h).clamp(0.0, h - 1.0) as u32;
                        let p = a.get_pixel(px, py);
                        Rgb(p[0], p[1], p[2])
                    }
                    None => label_plain.scale(0.8 + 0.2 * (u / label).abs()),
                };
                // the spindle hole, and a soft edge to the label
                let hole = smoothstep(rad * 0.035, rad * 0.035 + aa, d);
                vinyl.lerp(lab, smoothstep(label, label - aa, d)).lerp(metal, 1.0 - hole)
            } else {
                // grooves: fine rings, lighter ones every so often (the gaps between tracks), the bare land at the
                // edge and around the label
                let fine = 0.5 + 0.5 * (d * TAU / (2.5 * aa)).sin();
                let gaps = (1.0 - ((d - groove_in) / (rad * 0.115) % 1.0 - 0.5).abs() * 14.0).clamp(0.0, 1.0);
                let land = if d < groove_in || d > rad * 0.97 { 1.0 } else { 0.0 };
                let rel = (phi - angle).rem_euclid(TAU);
                // the sheen: two soft wedges turning with the record, a little brighter on a beat
                let wedge = (rel * 2.0).cos().max(0.0).powi(4) * (0.17 + 0.10 * shimmer) * (0.8 + 0.2 * rel.cos());
                let groove = 0.05 * fine * (1.0 - land) + 0.06 * gaps * (1.0 - land) + 0.04 * land;
                let shine = groove * (1.0 + 1.5 * shimmer * (0.5 + 0.5 * (d * 3.0 - rel * 2.0).sin())) + wedge * (1.0 - land * 0.5);
                vinyl.lerp(sheen_col, shine.min(0.5))
            };
            // the record's edge, anti-aliased
            col = bg.lerp(col, smoothstep(rad + aa * 0.5, rad - aa * 0.5, d));
        }
        // the tonearm over everything: a base, a thin tube, a headshell at the stylus
        let base = dist(x, y, pivot.0, pivot.1) - rad * 0.09;
        if base < aa {
            col = col.lerp(metal.scale(0.7), smoothstep(aa, 0.0, base));
        }
        let tube = seg(x, y, pivot, stylus) - rad * 0.018;
        if tube < aa {
            col = col.lerp(metal, smoothstep(aa, 0.0, tube));
        }
        let head = seg(x, y, head_from, stylus) - rad * 0.035;
        if head < aa {
            col = col.lerp(head_col, smoothstep(aa, 0.0, head));
        }
        col
    });
}

/// Where the stylus sits when it's `reach` from the record's centre: on the circle around the pivot that the arm
/// sweeps (its length fixed so the outer edge lands right of the spindle).
fn stylus_at(pivot: (f32, f32), centre: (f32, f32), reach: f32) -> (f32, f32) {
    let (px, py) = (pivot.0 - centre.0, pivot.1 - centre.1);
    let pd = (px * px + py * py).sqrt().max(1e-3);
    // the arm is as long as the pivot is from the centre, a touch less: the stylus reaches past the spindle line
    let len = pd * 0.98;
    // the triangle centre–pivot–stylus: the angle at the centre between the pivot and the stylus
    let cos = ((reach * reach + pd * pd - len * len) / (2.0 * reach.max(1e-3) * pd)).clamp(-1.0, 1.0);
    let a = py.atan2(px) + cos.acos();
    let a = if a.is_finite() { a } else { PI / 4.0 };
    (centre.0 + reach * a.cos(), centre.1 + reach * a.sin())
}

fn dist(x: f32, y: f32, ax: f32, ay: f32) -> f32 {
    ((x - ax).powi(2) + (y - ay).powi(2)).sqrt()
}

/// Distance from (x, y) to the segment a–b.
fn seg(x: f32, y: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let l2 = (vx * vx + vy * vy).max(1e-6);
    let t = (((x - a.0) * vx + (y - a.1) * vy) / l2).clamp(0.0, 1.0);
    dist(x, y, a.0 + vx * t, a.1 + vy * t)
}
