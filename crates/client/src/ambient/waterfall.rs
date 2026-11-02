//! waterfall: a spectrogram. Time runs right to left (the newest sound at the right edge, the last 24 s across the
//! Pane), frequency bottom to top over the Daemon's log-spaced bands, so the bass gets as much room as the treble.
//! It scrolls at a steady real-time pace whatever the frame rate, and holds still while paused.

use super::{Canvas, against_peak, playing, smoothstep, tick};
use crate::app::App;
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::collections::VecDeque;
use std::time::Instant;

/// Waterfall's Visual id on the pixel tier.
const ID: u32 = 5;
/// Slices a second, and the seconds across the Pane.
const RATE: f32 = 20.0;
const SPAN: f32 = 24.0;
const LEN: usize = (RATE * SPAN) as usize + 2;
/// Bands kept per slice.
const BANDS: usize = 64;
/// A faint mark every this many seconds, scrolling with the picture.
const MARK: f32 = 5.0;

#[derive(Default)]
pub struct State {
    last: Option<Instant>,
    /// finished slices, oldest first, each normalised to 0..255
    hist: VecDeque<[u8; BANDS]>,
    /// the slice being gathered (the loudest of each band since the last one)
    acc: Option<[u8; BANDS]>,
    /// seconds since the last slice
    since: f32,
    /// slices ever taken (for the marks)
    taken: u64,
    peak: f32,
}

pub fn waterfall(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.width < 2 || r.height < 1 {
        return;
    }
    let on = playing(app);
    let c = Canvas::new(app, r, 300_000.0);
    let s = &mut app.ambient.waterfall;
    // a long gap (unfocused at 2 fps, or swapped out) still keeps time, within reason
    let dt = tick(&mut s.last, 1.0);
    if on {
        let mut now = [0u8; BANDS];
        let mut loud = 0f32;
        if let Some(f) = app.meters.frame.as_ref().filter(|f| !f.bands.is_empty()) {
            let n = f.bands.len();
            for (i, v) in now.iter_mut().enumerate() {
                let x = i as f32 / (BANDS - 1) as f32 * (n - 1) as f32;
                let (a, k) = (x.floor() as usize, x.fract());
                *v = (f.bands[a] as f32 * (1.0 - k) + f.bands[(a + 1).min(n - 1)] as f32 * k) as u8;
                loud = loud.max(*v as f32);
            }
        }
        // against the loudest lately, so a quiet song still fills the picture
        let norm = 255.0 / s.peak.max(1.0);
        against_peak(&mut s.peak, loud, dt, 80.0);
        let fresh = now.map(|v| (v as f32 * norm).min(255.0) as u8);
        let acc = s.acc.get_or_insert([0; BANDS]);
        for (a, v) in acc.iter_mut().zip(fresh) {
            *a = (*a).max(v);
        }
        s.since += dt;
        let step = 1.0 / RATE;
        while s.since >= step {
            s.since -= step;
            if s.hist.len() == LEN {
                s.hist.pop_front();
            }
            s.hist.push_back(*acc);
            s.taken += 1;
            // the next slice starts from what sounds now (not from silence: the edge would blink)
            *acc = fresh;
        }
    }
    let frac = s.since * RATE;
    let taken = (s.taken % (RATE * MARK) as u64) as f32;
    let hist = std::mem::take(&mut s.hist);
    let live = s.acc.unwrap_or([0; BANDS]);
    let len = hist.len();
    let at = |k: usize| -> &[u8; BANDS] { if k == 0 { &live } else { &hist[len - k] } };
    let bg = th.bg;
    let mark = th.dim;
    // slices per unit across
    let per = RATE * SPAN / c.w;
    c.paint(buf, r, app, th, ID, |_, _, x, y| {
        // how many slices back this column is (0 the one gathering now, at the right edge; centres blend)
        // measured from the pixel's right edge, so the newest sound shows at once even in a coarse Pane
        let p = (c.w - x - c.ux * 0.5).max(0.0) * per;
        let back = (p - frac + 0.5).max(0.0);
        let k = back.floor() as usize;
        if k > len {
            return bg;
        }
        let kf = back.fract();
        let band = ((1.0 - y / c.h) * (BANDS - 1) as f32).clamp(0.0, (BANDS - 1) as f32);
        let (b, bf) = (band.floor() as usize, band.fract());
        let b2 = (b + 1).min(BANDS - 1);
        let sample = |s: &[u8; BANDS]| s[b] as f32 * (1.0 - bf) + s[b2] as f32 * bf;
        let next = if k < len { sample(at(k + 1)) } else { 0.0 };
        let v = (sample(at(k)) * (1.0 - kf) + next * kf) / 255.0;
        let lit = smoothstep(0.06, 1.0, v).powf(1.15);
        let mut col = bg.lerp(th.ramp_at(0.12 + 0.88 * v), lit);
        // a faint mark every MARK seconds, moving with the picture
        let m = (taken + frac - p) / (RATE * MARK);
        let off = (m - m.round()).abs() * RATE * MARK / per;
        if off < c.ux * 0.6 && len > 0 {
            col = col.lerp(mark, 0.12);
        }
        col
    });
    app.ambient.waterfall.hist = hist;
}
