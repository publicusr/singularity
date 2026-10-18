//! Audio Widgets (#15), drawn from the Daemon's spectrum frames: a cava-style spectrum, a loudness history and VU
//! meters. Colours come from the theme's ramp, which never tops out in white; motion is smoothed in the Daemon. The
//! loudness history and VU meters show the real output level (`out_*`), so they follow the volume (#29).

use crate::theme::{Rgb, Theme};
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use singularity_proto::SpectrumFrame;
use std::collections::VecDeque;

/// Loudness history: one sample per three frames (10 a second), a minute of it.
const HISTORY: usize = 600;
const EIGHTHS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

#[derive(Default)]
pub struct Meters {
    pub frame: Option<SpectrumFrame>,
    pub history: VecDeque<u8>,
    acc: (u32, u32),
}

impl Meters {
    pub fn push(&mut self, f: SpectrumFrame) {
        self.acc.0 += f.out_level as u32;
        self.acc.1 += 1;
        if self.acc.1 == 3 {
            self.history.push_back((self.acc.0 / 3) as u8);
            if self.history.len() > HISTORY {
                self.history.pop_front();
            }
            self.acc = (0, 0);
        }
        self.frame = Some(f);
    }
}

/// The ramp colour at `t` in 0..=1.
fn ramp(th: &Theme, t: f32) -> Rgb {
    let r = &th.ramp;
    if r.is_empty() {
        return th.accent;
    }
    let x = t.clamp(0.0, 1.0) * (r.len() - 1) as f32;
    let (i, f) = (x.floor() as usize, x.fract());
    let (a, b) = (r[i], r[(i + 1).min(r.len() - 1)]);
    let mix = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * f).round() as u8;
    Rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

/// A column `h` rows tall filled to `v` (0..=1) from the bottom, in eighths.
fn column(buf: &mut Buffer, x: u16, bottom: u16, h: u16, v: f32, th: &Theme) {
    let eighths = (v.clamp(0.0, 1.0) * h as f32 * 8.0).round() as u16;
    for row in 0..h {
        let fill = eighths.saturating_sub(row * 8).min(8) as usize;
        let y = bottom - 1 - row;
        if fill == 0 {
            continue;
        }
        // colour by height in the column, so tall bars warm up towards the top
        let c = ramp(th, (row as f32 + 0.5) / h as f32 * 0.85 + 0.15);
        text(buf, x, y, 1, EIGHTHS[fill], th.s(c));
    }
}

/// The spectrum: bands stretched across the width (linear interpolation between neighbours).
pub fn spectrum(buf: &mut Buffer, r: Rect, m: &Meters, th: &Theme) {
    if r.width == 0 || r.height == 0 {
        return;
    }
    let Some(f) = m.frame.as_ref().filter(|f| !f.bands.is_empty()) else {
        return baseline(buf, r, th);
    };
    let n = f.bands.len();
    for x in 0..r.width {
        let pos = (x as f32 + 0.5) / r.width as f32 * (n - 1) as f32;
        let (i, t) = (pos.floor() as usize, pos.fract());
        let v = f.bands[i] as f32 * (1.0 - t) + f.bands[(i + 1).min(n - 1)] as f32 * t;
        column(buf, r.x + x, r.bottom(), r.height, v / 255.0, th);
    }
    if f.bands.iter().all(|b| *b == 0) {
        baseline(buf, r, th);
    }
}

fn baseline(buf: &mut Buffer, r: Rect, th: &Theme) {
    for x in 0..r.width {
        text(buf, r.x + x, r.bottom() - 1, 1, "▁", th.s(th.border));
    }
}

/// VU meters (two rows) above a loudness history (newest on the right).
pub fn loudness(buf: &mut Buffer, r: Rect, m: &Meters, th: &Theme) {
    if r.width < 4 || r.height == 0 {
        return;
    }
    let vu = m.frame.as_ref().map(|f| f.out_vu).unwrap_or([0, 0]);
    let mut y = r.y;
    if r.height >= 4 {
        for (label, v) in [("L", vu[0]), ("R", vu[1])] {
            text(buf, r.x, y, 1, label, th.s(th.dim));
            let w = r.width - 2;
            let lit = (v as f32 / 255.0 * w as f32).round() as u16;
            for x in 0..w {
                let st = if x < lit { th.s(ramp(th, x as f32 / w as f32 * 0.85 + 0.15)) } else { th.s(th.border) };
                text(buf, r.x + 2 + x, y, 1, if x < lit { "▬" } else { "─" }, st);
            }
            y += 1;
        }
        y += 1;
    }
    let h = r.bottom().saturating_sub(y);
    if h == 0 {
        return;
    }
    let w = r.width as usize;
    let hist: Vec<u8> = m.history.iter().rev().take(w).rev().copied().collect();
    let x0 = r.right() - hist.len() as u16;
    for (i, v) in hist.iter().enumerate() {
        column(buf, x0 + i as u16, r.bottom(), h, *v as f32 / 255.0, th);
    }
    if hist.is_empty() {
        baseline(buf, Rect::new(r.x, y, r.width, h), th);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(level: u8, band: u8) -> SpectrumFrame {
        SpectrumFrame { bands: vec![band; 64], level, vu: [200, 100], beat: false, out_level: level, out_vu: [200, 100], out_bands: [band; 3] }
    }

    #[test]
    fn history_takes_one_sample_per_three_frames_and_keeps_a_minute() {
        let mut m = Meters::default();
        for i in 0..(HISTORY * 3 + 30) {
            m.push(frame((i % 255) as u8, 0));
        }
        assert_eq!(m.history.len(), HISTORY);
    }

    #[test]
    fn spectrum_fills_by_height_and_never_white() {
        let th = Theme::singularity();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut m = Meters::default();
        m.push(frame(0, 128));
        spectrum(&mut buf, Rect::new(0, 0, 10, 4), &m, &th);
        // half height: the bottom two rows full, the top two empty
        assert_eq!(buf[(3, 3)].symbol(), "█");
        assert_eq!(buf[(3, 2)].symbol(), "█");
        assert_eq!(buf[(3, 1)].symbol(), " ");
        m.push(frame(0, 255));
        spectrum(&mut buf, Rect::new(0, 0, 10, 4), &m, &th);
        for y in 0..4 {
            if let ratatui::style::Color::Rgb(r, g, b) = buf[(5, y)].fg {
                assert!(!(r > 240 && g > 240 && b > 240), "white at row {y}");
            }
        }
    }

    #[test]
    fn loudness_draws_vu_and_history() {
        let th = Theme::singularity();
        let r = Rect::new(0, 0, 20, 8);
        let mut buf = Buffer::empty(r);
        let mut m = Meters::default();
        for _ in 0..30 {
            m.push(frame(200, 0));
        }
        loudness(&mut buf, r, &m, &th);
        let row = |y| (0..20).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert!(row(0).starts_with("L ▬") && row(1).starts_with("R ▬"));
        assert!(row(0).matches('▬').count() > row(1).matches('▬').count(), "left is louder");
        assert!(row(7).ends_with("██████████"), "ten history samples, newest right: {:?}", row(7));
    }
}
