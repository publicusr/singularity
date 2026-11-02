//! aurora: slow curtains of light drifting across a dark sky, in the cover's colours (the Theme's ramp without one).
//! The bass swells them, the treble sends a gentle shimmer along their rays; most of the Pane stays dark, and paused
//! they drift to a standstill.

use super::{Canvas, against_peak, ease, over, playing, smoothstep, tick};
use crate::app::App;
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::f32::consts::TAU;
use std::time::Instant;

/// Aurora's Visual id on the pixel tier.
const ID: u32 = 6;
/// Curtains.
const N: usize = 3;

#[derive(Default)]
pub struct State {
    last: Option<Instant>,
    /// its clock, and how fast it runs (1 playing, 0 paused: eased, so it drifts to a halt)
    t: f32,
    pace: f32,
    /// eased bass and treble (against their loudest lately)
    bass: f32,
    treble: f32,
    peaks: [f32; 2],
    /// the curtains' colours now, easing toward the cover's
    cols: Option<[[f32; 3]; N]>,
    /// per pixel column: each curtain's lower edge (units down) and its strength there
    edge: Vec<[(f32, f32); N]>,
}

pub fn aurora(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.width < 2 || r.height < 1 {
        return;
    }
    let on = playing(app);
    let (bass, treble) = app.meters.frame.as_ref().map(|f| (f.out_bands[0] as f32, f.out_bands[2] as f32)).unwrap_or((0.0, 0.0));
    let target = palette(app, th);
    let c = Canvas::new(app, r, 160_000.0);
    let s = &mut app.ambient.aurora;
    let dt = tick(&mut s.last, 0.25);
    ease(&mut s.pace, if on { 1.0 } else { 0.0 }, dt, 0.8, 0.6);
    s.t += dt * s.pace;
    let nb = against_peak(&mut s.peaks[0], bass, dt, 50.0);
    let nt = against_peak(&mut s.peaks[1], treble, dt, 40.0);
    // the curtains swell over ~a third of a second and settle over a second: breathing, never flashing
    ease(&mut s.bass, if on { nb } else { 0.0 }, dt, 0.25, 1.0);
    ease(&mut s.treble, if on { nt } else { 0.0 }, dt, 0.15, 0.6);
    let k = 1.0 - (-dt / 0.6).exp();
    let cols = s.cols.get_or_insert(target);
    for (n, t) in cols.iter_mut().zip(&target) {
        for i in 0..3 {
            n[i] += (t[i] - n[i]) * k;
        }
    }
    let (t, bass, treble, cols) = (s.t, s.bass, s.treble, *cols);

    // each column's curtains: a wavy lower edge, rays of varying strength, a slow drifting reach along the Pane
    s.edge.clear();
    for i in 0..c.pw {
        let x = (i as f32 + 0.5) * c.ux;
        let u = x / c.w.max(1.0);
        let mut e = [(0.0, 0.0); N];
        for (n, slot) in e.iter_mut().enumerate() {
            let f = n as f32;
            let y = c.h * (0.42 + 0.11 * f)
                + c.h * 0.09 * (u * TAU * 0.8 + t * 0.09 * (1.0 + f * 0.4) + f * 2.1).sin()
                + c.h * 0.04 * (u * TAU * 2.1 - t * 0.13 + f * 1.3).sin();
            let centre = 0.5 + 0.38 * (t * 0.035 * (1.0 + f * 0.5) + f * 2.4).sin();
            let reach = (-((u - centre) / 0.36).powi(2)).exp();
            // rays: vertical streaks that wander slowly, and a treble shimmer travelling along them
            let rays = 0.6 + 0.4 * (0.5 + 0.5 * (x * 1.1 + f * 5.0 + 2.0 * (x * 0.23 - t * 0.21 + f).sin()).sin());
            let shimmer = 1.0 + 0.22 * treble * (x * 2.7 - t * 4.0 + f * 3.0).sin();
            *slot = (y, reach * rays * shimmer);
        }
        s.edge.push(e);
    }
    let edge = std::mem::take(&mut s.edge);
    let bg = th.bg;
    // never too bright: at most about half the colour at the very brightest point
    let gain = 0.36 + 0.18 * bass;
    let up = c.h * (0.20 + 0.12 * bass);
    let down = c.h * 0.035;
    c.paint(buf, r, app, th, ID, |i, _, _, y| {
        let e = &edge[(i as usize).min(edge.len() - 1)];
        let mut acc = [0f32; 3];
        for (n, &(ey, st)) in e.iter().enumerate() {
            let d = y - ey;
            // brightest at the lower edge, fading up the rays, falling off fast below
            let v = if d < 0.0 { (d / up).exp() } else { (-(d / down).powi(2)).exp() };
            // the curtains hang in a darker sky: the top of the Pane stays dim
            let w = v * st * gain * (0.45 + 0.55 * smoothstep(0.0, c.h * 0.3, y));
            if w < 0.002 {
                continue;
            }
            // each curtain turns toward the next colour up its rays
            let mix = (-d / up).clamp(0.0, 1.0);
            let (a, b) = (cols[n], cols[(n + 1) % N]);
            for k in 0..3 {
                acc[k] += (a[k] + (b[k] - a[k]) * mix) * w;
            }
        }
        over(bg, acc)
    });
    app.ambient.aurora.edge = edge;
}

/// Three bright colours from the cover (its most vivid, apart from each other), else from the Theme's ramp.
fn palette(app: &mut App, th: &Theme) -> [[f32; 3]; N] {
    let ramp = |t: f32| {
        let c = th.ramp_at(t);
        [c.0 as f32, c.1 as f32, c.2 as f32]
    };
    let fallback = [ramp(0.45), ramp(0.75), ramp(1.0)];
    let Some(url) = app.playback.item.as_ref().and_then(|i| i.cover_url.clone()) else {
        return fallback;
    };
    let tint = app.art_tint.clone();
    let Some(img) = app.covers.scaled(&url, 4, 4, tint.as_ref()) else {
        return fallback;
    };
    let mut px = [[0f32; 3]; 16];
    for (i, p) in img.pixels().take(16).enumerate() {
        px[i] = [p[0] as f32, p[1] as f32, p[2] as f32];
    }
    let vivid = |c: &[f32; 3]| {
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        (mx - mn) + 0.3 * mx
    };
    let mut out = fallback;
    let mut picked = 0;
    let mut used = [false; 16];
    for _ in 0..N {
        let mut best: Option<usize> = None;
        for (i, c) in px.iter().enumerate() {
            let far = out[..picked].iter().all(|o| (0..3).map(|k| (o[k] - c[k]).abs()).sum::<f32>() > 70.0);
            if !used[i] && far && best.is_none_or(|b| vivid(c) > vivid(&px[b])) {
                best = Some(i);
            }
        }
        let Some(b) = best else { break };
        used[b] = true;
        out[picked] = px[b];
        picked += 1;
    }
    // a grey or dark cover: fall back to the ramp for what's missing, and lift every colour so it glows
    for (i, c) in out.iter_mut().enumerate() {
        if i >= picked || vivid(c) < 40.0 {
            *c = fallback[i];
        }
        let grey = (c[0] + c[1] + c[2]) / 3.0;
        let sat = c.map(|v| (grey + (v - grey) * 1.4).clamp(0.0, 255.0));
        let mx = sat[0].max(sat[1]).max(sat[2]).max(1.0);
        *c = sat.map(|v| v * (220.0 / mx));
    }
    out
}
