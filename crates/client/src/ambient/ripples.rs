//! ripples: rings spreading out from the middle on each beat, like drops falling on still water, fading as they
//! grow. How bright a ring starts follows the level; a handful at most are alive at once. Paused, no new drops
//! fall and the last rings spread out and fade.

use super::{Canvas, against_peak, ease, over, playing, smoothstep, tick};
use crate::app::App;
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::time::Instant;

/// Ripples' Visual id on the pixel tier.
const ID: u32 = 7;
/// Rings alive at most.
const MAX: usize = 5;
/// Seconds a ring lives, and the least time between two drops.
const LIFE: f32 = 3.6;
const GAP: f32 = 0.22;

#[derive(Default, Clone, Copy)]
struct Ring {
    age: f32,
    /// how bright it started, 0..1
    strength: f32,
    /// where on the ramp its colour sits
    hue: f32,
}

#[derive(Default)]
pub struct State {
    last: Option<Instant>,
    rings: [Option<Ring>; MAX],
    /// since the last drop
    since: f32,
    /// the eased level (the calm glow in the middle) and the loudest lately
    lvl: f32,
    peak: f32,
    drops: u32,
}

pub fn ripples(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.width < 2 || r.height < 1 {
        return;
    }
    let on = playing(app);
    let (level, beat) = app.meters.frame.as_ref().map(|f| (f.out_level as f32, f.beat)).unwrap_or((0.0, false));
    let c = Canvas::new(app, r, 300_000.0);
    let s = &mut app.ambient.ripples;
    let dt = tick(&mut s.last, 0.25);
    let norm = against_peak(&mut s.peak, level, dt, 40.0);
    ease(&mut s.lvl, if on { norm } else { 0.0 }, dt, 0.12, 0.6);
    s.since += dt;
    for ring in s.rings.iter_mut() {
        if let Some(g) = ring {
            g.age += dt;
            if g.age >= LIFE {
                *ring = None;
            }
        }
    }
    // a beat drops a new ring (one beat frame is seen by several draws at a high frame rate: hence the gap)
    if on && beat && s.since >= GAP {
        s.since = 0.0;
        s.drops = s.drops.wrapping_add(1);
        let new = Ring { age: 0.0, strength: 0.35 + 0.65 * norm, hue: (s.drops % 3) as f32 / 2.0 };
        // into a free place, else over the oldest
        let slot = s.rings.iter().position(|g| g.is_none()).unwrap_or_else(|| {
            (0..MAX).max_by(|&a, &b| s.rings[a].map_or(0.0, |g| g.age).total_cmp(&s.rings[b].map_or(0.0, |g| g.age))).unwrap_or(0)
        });
        s.rings[slot] = Some(new);
    }
    let (rings, lvl) = (s.rings, s.lvl);
    let (cx, cy) = (c.w / 2.0, c.h / 2.0);
    // the rings reach just past the nearer edge before they're gone
    let reach = cx.min(cy) * 1.15 + cx.max(cy) * 0.25;
    let px = c.ux.max(c.uy);
    // each ring's radius, width, brightness and colour, once a frame
    let mut live = [(0f32, 1f32, 0f32, [0f32; 3]); MAX];
    let mut n = 0;
    for g in rings.iter().flatten() {
        let a = g.age / LIFE;
        // fast at first, slowing as it spreads, like water
        let radius = reach * (1.0 - (1.0 - a).powf(1.7));
        // swells in over ~300 ms as it leaves the middle (born bright on the centre spot, it was a flash), fades as it grows
        let bright = g.strength * smoothstep(0.0, 0.3, g.age) * (1.0 - a).powf(1.6) * 0.75;
        let width = px * 0.9 + 0.12 + 0.45 * a;
        let col = th.ramp_at(0.45 + 0.4 * g.hue + 0.15 * g.strength);
        live[n] = (radius, width, bright, [col.0 as f32, col.1 as f32, col.2 as f32]);
        n += 1;
    }
    let live = &live[..n];
    let bg = th.bg;
    let centre = th.accent;
    let dot = 0.5 + 0.9 * lvl;
    let glow = 0.18 + 0.45 * lvl;
    c.paint(buf, r, app, th, ID, |_, _, x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let d = (dx * dx + dy * dy).sqrt();
        // the calm spot where the drops fall
        let k = (-(d / dot).powi(2)).exp() * glow;
        let mut acc = [centre.0 as f32 * k, centre.1 as f32 * k, centre.2 as f32 * k];
        for &(radius, width, bright, col) in live {
            // a crest and a fainter trough-and-crest behind it
            // (only near a crest: most of the Pane is far from every ring, and exp isn't free)
            let (e1, e2) = ((d - radius) / width, (d - radius * 0.82) / (width * 0.8));
            if e1.abs().min(e2.abs()) > 3.0 {
                continue;
            }
            let w = ((-(e1 * e1)).exp() + 0.35 * (-(e2 * e2)).exp()) * bright;
            if w > 0.002 {
                for i in 0..3 {
                    acc[i] += col[i] * w;
                }
            }
        }
        over(bg, acc)
    });
}
