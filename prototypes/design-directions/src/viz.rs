//! Visuals. Pixel visuals render into an `Fb` (works for both render tiers);
//! cell visuals draw text characters straight into the ratatui buffer.

use crate::audio::{AudioFrame, BANDS};
use crate::gfx::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::f32::consts::PI;

// ───────────────────────────── warp starfield ─────────────────────────────

pub struct Warp {
    stars: Vec<(f32, f32, f32)>,
}

impl Warp {
    pub fn new() -> Warp {
        let stars = (0..700u32)
            .map(|i| (hash1(i * 3) * 2.0 - 1.0, hash1(i * 3 + 1) * 2.0 - 1.0, hash1(i * 3 + 2) * 4.0 + 0.1))
            .collect();
        Warp { stars }
    }
    pub fn step(&mut self, dt: f32, a: &AudioFrame) {
        let speed = 0.5 + a.energy * 5.0 + a.beat * 2.5;
        for (i, s) in self.stars.iter_mut().enumerate() {
            s.2 -= dt * speed;
            if s.2 < 0.05 {
                let k = (i as u32).wrapping_mul(2654435761).wrapping_add((s.0 * 1e6) as u32);
                *s = (hash1(k) * 2.0 - 1.0, hash1(k + 1) * 2.0 - 1.0, 4.0);
            }
        }
    }
    pub fn render(&self, fb: &mut Fb, a: &AudioFrame, colors: &[Rgb]) {
        for p in fb.px.iter_mut() {
            *p = p.scale(0.0);
        }
        let (w, h) = (fb.w as f32, fb.h as f32);
        let streak = 0.06 + a.energy * 0.25;
        for (i, s) in self.stars.iter().enumerate() {
            let proj = |z: f32| (w * 0.5 + s.0 / z * w * 0.5, h * 0.5 + s.1 / z * w * 0.5);
            let (x0, y0) = proj(s.2 + streak);
            let (x1, y1) = proj(s.2);
            let bright = (1.0 - s.2 / 4.0).powi(2) * 1.6;
            let c = ramp(colors, hash1(i as u32)).scale(bright);
            let n = ((x1 - x0).abs().max((y1 - y0).abs()) as usize).clamp(1, 60);
            for k in 0..=n {
                let t = k as f32 / n as f32;
                let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
                if x >= 0.0 && y >= 0.0 && (x as usize) < fb.w && (y as usize) < fb.h {
                    let idx = y as usize * fb.w + x as usize;
                    fb.px[idx] = fb.px[idx].add(c.scale(0.4 + 0.6 * t));
                }
            }
        }
    }
}

// ───────────────────────────── procedural album covers ─────────────────────────────

pub fn cover_palette(seed: u32) -> [Rgb; 3] {
    let hue = hash1(seed * 7 + 1);
    let shift = [0.08, 0.5, 0.33, 0.6][(seed % 4) as usize];
    [hsv(hue, 0.65, 0.95), hsv(hue + shift, 0.75, 0.6), hsv(hue + shift * 0.5, 0.6, 0.12)]
}

pub fn cover(fb: &mut Fb, seed: u32, t: f32) {
    let [c1, c2, c3] = cover_palette(seed);
    let (w, h) = (fb.w as f32, fb.h as f32);
    let style = seed % 5;
    for y in 0..fb.h {
        for x in 0..fb.w {
            let (u, v) = (x as f32 / w, y as f32 / h);
            let c = match style {
                0 => {
                    let d = ((u - 0.5).powi(2) + (v - 0.45).powi(2)).sqrt();
                    let bg = c3.lerp(c2, v);
                    if d < 0.28 { c1.lerp(c2, (v - 0.17) * 1.8) } else { bg.add(c1.scale((-(d - 0.28) * 9.0).exp() * 0.4)) }
                }
                1 => {
                    let d = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
                    if ((d * 14.0 - t * 0.3).fract()) < 0.5 { c1.lerp(c2, d * 1.5) } else { c3 }
                }
                2 => {
                    if ((u + v) * 7.0).fract() < 0.45 { c1 } else { c2.lerp(c3, v) }
                }
                3 => {
                    let n = fbm(u * 3.0 + seed as f32, v * 3.0, 4);
                    if n > 0.55 { c1 } else if n > 0.45 { c2 } else { c3 }
                }
                _ => {
                    let g = ((u * 6.0).fract() - 0.5).powi(2) + ((v * 6.0).fract() - 0.5).powi(2);
                    let big = (u - 0.3).abs() < 0.12 || (v - 0.7).abs() < 0.08;
                    if big { c1 } else if g < 0.06 { c2 } else { c3 }
                }
            };
            fb.px[y * fb.w + x] = c;
        }
    }
}

// ───────────────────────────── cell visuals ─────────────────────────────

const EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn put(buf: &mut Buffer, x: u16, y: u16, ch: char, fg: Color, bg: Option<Color>) {
    if let Some(c) = buf.cell_mut((x, y)) {
        c.set_char(ch).set_fg(fg);
        if let Some(bg) = bg {
            c.set_bg(bg);
        }
    }
}

fn band_at(a: &AudioFrame, frac: f32) -> (f32, f32) {
    let f = frac * (BANDS - 1) as f32;
    let i = f as usize;
    let j = (i + 1).min(BANDS - 1);
    let t = f - i as f32;
    (a.bands[i] * (1.0 - t) + a.bands[j] * t, a.peaks[i].max(a.peaks[j]))
}

/// cava-style bars. Bands are smoothed across neighbours (no jagged single-bar spikes).
/// `mirror` grows from the vertical centre; `peaks` draws slowly falling caps.
pub fn spectrum(buf: &mut Buffer, r: Rect, a: &AudioFrame, colors: &[Rgb], mirror: bool, gap: bool, peaks: bool) {
    if r.width == 0 || r.height == 0 {
        return;
    }
    let step = if gap { 2 } else { 1 };
    let cols = (r.width / step).max(1) as usize;
    let mut vals: Vec<(f32, f32)> = (0..cols).map(|ci| band_at(a, ci as f32 / (cols.max(2) - 1) as f32)).collect();
    for _ in 0..2 {
        let prev = vals.clone();
        for i in 0..cols {
            let l = prev[i.saturating_sub(1)].0;
            let rr = prev[(i + 1).min(cols - 1)].0;
            vals[i].0 = prev[i].0 * 0.5 + (l + rr) * 0.25;
        }
    }
    for (ci, (v, pk)) in vals.into_iter().enumerate() {
        let x = r.x + ci as u16 * step;
        if mirror {
            let half = r.height as f32 / 2.0;
            let lvl = v * half * 8.0;
            for row in 0..r.height {
                let dy = (row as f32 + 0.5 - half).abs();
                let fill = (lvl - (dy - 0.5) * 8.0).clamp(0.0, 8.0);
                if fill >= 1.0 {
                    let col = ramp(colors, dy / half).color();
                    let upper = (row as f32 + 0.5) < half;
                    let ch = if fill >= 8.0 {
                        '█'
                    } else if upper {
                        EIGHTHS[fill as usize]
                    } else if fill >= 4.0 {
                        '▀'
                    } else {
                        '▔'
                    };
                    put(buf, x, r.y + row, ch, col, None);
                }
            }
        } else {
            let total = v * r.height as f32 * 8.0;
            for row in 0..r.height {
                let from_bottom = (r.height - 1 - row) as f32;
                let fill = (total - from_bottom * 8.0).clamp(0.0, 8.0);
                if fill >= 1.0 {
                    let col = ramp(colors, from_bottom / r.height as f32).color();
                    put(buf, x, r.y + row, EIGHTHS[fill.round() as usize], col, None);
                }
            }
            if peaks {
                let prow = ((1.0 - pk) * (r.height as f32 - 1.0)).round() as u16;
                if pk > 0.02 && prow < r.height {
                    let y = r.y + prow;
                    if buf.cell((x, y)).is_some_and(|c| c.symbol() == " ") {
                        put(buf, x, y, '▁', ramp(colors, pk * 0.8).color(), None);
                    }
                }
            }
        }
    }
}

/// Braille dot canvas (2×4 dots per cell) with intensity accumulation.
pub struct Braille {
    w: usize,
    h: usize,
    dots: Vec<f32>,
}

impl Braille {
    pub fn new(r: Rect) -> Braille {
        let (w, h) = (r.width as usize * 2, r.height as usize * 4);
        Braille { w, h, dots: vec![0.0; w * h] }
    }
    pub fn plot(&mut self, x: f32, y: f32, v: f32) {
        if x >= 0.0 && y >= 0.0 && (x as usize) < self.w && (y as usize) < self.h {
            self.dots[y as usize * self.w + x as usize] += v;
        }
    }
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, v: f32) {
        let n = ((x1 - x0).abs().max((y1 - y0).abs()) as usize).max(1);
        for k in 0..=n {
            let t = k as f32 / n as f32;
            self.plot(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, v);
        }
    }
    pub fn render(&self, buf: &mut Buffer, r: Rect, colors: &[Rgb]) {
        const BITS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
        for cy in 0..r.height as usize {
            for cx in 0..r.width as usize {
                let (mut bits, mut sum) = (0u32, 0.0f32);
                for (dy, row) in BITS.iter().enumerate() {
                    for (dx, bit) in row.iter().enumerate() {
                        let v = self.dots[(cy * 4 + dy) * self.w + cx * 2 + dx];
                        if v > 0.0 {
                            bits |= bit;
                            sum += v;
                        }
                    }
                }
                if bits != 0 {
                    let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                    put(buf, r.x + cx as u16, r.y + cy as u16, ch, ramp(colors, (sum / 4.0).min(1.0)).color(), None);
                }
            }
        }
    }
}

/// A calm oscilloscope: triggered on a rising zero crossing (so the waveform stands still instead of
/// scrolling), lightly low-passed, and blended with the previous frame (phosphor persistence).
pub struct Scope {
    disp: Vec<f32>,
}

impl Scope {
    pub fn new() -> Scope {
        Scope { disp: vec![] }
    }
    pub fn render(&mut self, buf: &mut Buffer, r: Rect, a: &AudioFrame, colors: &[Rgb]) {
        if r.width < 2 || r.height < 2 {
            return;
        }
        let w = r.width as usize * 2;
        let h = r.height as f32 * 4.0;
        let wave = &a.wave_l;
        let n = wave.len();
        let trig = (1..n / 2).find(|&i| wave[i - 1] < 0.0 && wave[i] >= 0.0).unwrap_or(0);
        let span = n / 2;
        let gain = 0.42 / (a.rms * 3.0).max(0.12);
        if self.disp.len() != w {
            self.disp = vec![0.0; w];
        }
        for i in 0..w {
            let k = trig + i * span / w;
            let s = (wave[k.saturating_sub(1)] + wave[k] * 2.0 + wave[(k + 1).min(n - 1)]) * 0.25 * gain;
            self.disp[i] = self.disp[i] * 0.72 + s * 0.28;
        }
        let mut b = Braille::new(r);
        let mut prev: Option<f32> = None;
        for (i, s) in self.disp.iter().enumerate() {
            let y = h * 0.5 - s.clamp(-1.0, 1.0) * h * 0.45;
            if let Some(py) = prev {
                b.line(i as f32 - 1.0, py, i as f32, y, 1.0);
            }
            prev = Some(y);
        }
        b.render(buf, r, colors);
    }
}

/// btop-style scrolling history graph (braille, filled), e.g. loudness over the last minute.
pub struct History {
    vals: Vec<f32>,
}

impl History {
    pub fn new() -> History {
        History { vals: vec![] }
    }
    pub fn push(&mut self, v: f32) {
        self.vals.push(v.clamp(0.0, 1.0));
        if self.vals.len() > 600 {
            self.vals.remove(0);
        }
    }
    pub fn render(&self, buf: &mut Buffer, r: Rect, colors: &[Rgb]) {
        let w = r.width as usize * 2;
        let h = r.height as usize * 4;
        if w == 0 || h == 0 {
            return;
        }
        const BITS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
        let start = self.vals.len().saturating_sub(w);
        let pad = w.saturating_sub(self.vals.len());
        for cy in 0..r.height as usize {
            for cx in 0..r.width as usize {
                let mut bits = 0u32;
                for dx in 0..2 {
                    let col = cx * 2 + dx;
                    if col < pad {
                        continue;
                    }
                    let v = self.vals[start + col - pad];
                    let top = h as f32 * (1.0 - v);
                    for (dy, row) in BITS.iter().enumerate() {
                        if (cy * 4 + dy) as f32 >= top {
                            bits |= row[dx];
                        }
                    }
                }
                if bits != 0 {
                    let level = 1.0 - cy as f32 / r.height as f32;
                    let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                    put(buf, r.x + cx as u16, r.y + cy as u16, ch, ramp(colors, level).color(), None);
                }
            }
        }
    }
}

/// LED-ladder VU meters for L and R.
pub fn vu(buf: &mut Buffer, r: Rect, a: &AudioFrame, colors: &[Rgb], dim: Color) {
    let peak = |w: &[f32]| w.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let levels = [peak(&a.wave_l), peak(&a.wave_r)];
    for (i, lvl) in levels.iter().enumerate() {
        let y = r.y + i as u16 * 2;
        if y >= r.bottom() {
            break;
        }
        let label = if i == 0 { "L " } else { "R " };
        buf.set_string(r.x, y, label, ratatui::style::Style::default().fg(dim));
        let width = r.width.saturating_sub(2);
        let db = (20.0 * (lvl + 1e-6).log10() + 48.0) / 48.0;
        let lit = (db.clamp(0.0, 1.0) * width as f32) as u16;
        for x in 0..width {
            let col = if x < lit { ramp(colors, x as f32 / width as f32).color() } else { dim };
            put(buf, r.x + 2 + x, y, if x < lit { '▮' } else { '▯' }, col, None);
        }
    }
}

pub struct Matrix {
    cols: Vec<(f32, f32, u16)>, // head y, speed, trail length
    width: u16,
}

impl Matrix {
    pub fn new() -> Matrix {
        Matrix { cols: vec![], width: 0 }
    }
    pub fn step(&mut self, r: Rect, dt: f32, a: &AudioFrame) {
        if self.width != r.width {
            self.width = r.width;
            self.cols = (0..r.width as u32)
                .map(|i| (-(hash1(i) * r.height as f32 * 2.0), 6.0 + hash1(i + 99) * 14.0, 4 + (hash1(i + 7) * 18.0) as u16))
                .collect();
        }
        let boost = 1.0 + a.energy * 2.5 + a.beat * 1.5;
        for (i, c) in self.cols.iter_mut().enumerate() {
            c.0 += c.1 * dt * boost;
            if c.0 - c.2 as f32 > r.height as f32 {
                let k = (i as u32).wrapping_add((c.0 * 977.0) as u32);
                *c = (-(hash1(k) * 10.0), 6.0 + hash1(k + 1) * 14.0, 4 + (hash1(k + 2) * 18.0) as u16);
            }
        }
    }
    pub fn render(&self, buf: &mut Buffer, r: Rect, t: f32, head: Rgb, body: Rgb, inject: &str) {
        let glyphs: Vec<char> = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789Z:.=*+<>".chars().collect();
        let inj: Vec<char> = inject.chars().collect();
        for (x, c) in self.cols.iter().enumerate() {
            let hy = c.0 as i32;
            for k in 0..c.2 as i32 {
                let y = hy - k;
                if y < 0 || y >= r.height as i32 {
                    continue;
                }
                let seed = hash2(x as i32, y + (t * 6.0) as i32 * (k == 0) as i32);
                let mut ch = glyphs[(seed * glyphs.len() as f32) as usize % glyphs.len()];
                if !inj.is_empty() && x % 9 == 4 && hash2(x as i32, y) > 0.55 {
                    ch = inj[(y as usize) % inj.len()];
                }
                let fade = 1.0 - k as f32 / c.2 as f32;
                let col = if k == 0 { head } else { body.scale(fade.powf(1.5)) };
                put(buf, r.x + x as u16, r.y + y as u16, ch, col.color(), None);
            }
        }
    }
}

/// GitHub-style listening heatmap (fake data).
pub fn heatmap(buf: &mut Buffer, r: Rect, colors: &[Rgb], empty: Color) {
    let weeks = (r.width / 2).min(52);
    for wk in 0..weeks {
        for d in 0..7u16.min(r.height) {
            let v = hash2(wk as i32, d as i32);
            let v = if v < 0.25 { 0.0 } else { (v - 0.25) / 0.75 * (0.4 + 0.6 * wk as f32 / weeks as f32) };
            let col = if v == 0.0 { empty } else { ramp(colors, v).color() };
            put(buf, r.x + wk * 2, r.y + d, '■', col, None);
        }
    }
}

/// A static pseudo-waveform used as a seek bar; played part in `on`, rest in `off`.
pub fn waveform_bar(buf: &mut Buffer, r: Rect, progress: f32, seed: u32, on: &[Rgb], off: Color) {
    for x in 0..r.width {
        let u = x as f32 / r.width as f32;
        let v = 0.25 + 0.75 * (fbm(u * 18.0 + seed as f32, 0.5, 3) * 1.4 - 0.2).clamp(0.0, 1.0) * (0.4 + 0.6 * (u * PI).sin());
        let total = v * r.height as f32 * 8.0;
        for row in 0..r.height {
            let fb = (r.height - 1 - row) as f32;
            let fill = (total - fb * 8.0).clamp(0.0, 8.0) as usize;
            if fill > 0 {
                let col = if u <= progress { ramp(on, u).color() } else { off };
                put(buf, r.x + x, r.y + row, EIGHTHS[fill], col, None);
            }
        }
    }
}
