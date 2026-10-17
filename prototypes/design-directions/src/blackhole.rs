//! Black hole renderer, ported to CPU/Rust from s0xDk/ghostty-blackhole (MIT,
//! https://github.com/s0xDk/ghostty-blackhole), itself modelled on Eric Bruneton's black hole shader.
//!
//! Every pixel near the hole integrates its own null geodesic (a = -3/2 h² x / r⁵, leapfrog, ~48 steps),
//! crossing a thin Keplerian disk coloured by a Shakura–Sunyaev temperature profile as blackbody light,
//! Doppler-shifted and beamed by g = √(1 − 1.5/r) / (1 − β·k̂). Photon ring and the Interstellar
//! "disk over and under the shadow" fall out of the tracing.
//!
//! Two uses:
//! - `render` fills a framebuffer (a pane visual), OLED black background.
//! - `render` with alpha + `lens_cells` = a hole floating over the UI that bends the UI's own text.

use crate::audio::AudioFrame;
use crate::gfx::{Fb, Rgb};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use rayon::prelude::*;

pub const B_CRIT: f32 = 2.598_076_2;

#[derive(Clone, Copy)]
pub struct DiskLook {
    pub name: &'static str,
    pub temp: f32,
    pub incl: f32,
    pub roll: f32,
    pub inner: f32,
    pub outer: f32,
    pub opac: f32,
    pub dopp: f32,
    pub beam: f32,
    pub gain: f32,
    pub contr: f32,
    pub wind: f32,
    pub speed: f32,
    pub expo: f32,
    pub star: f32,
}

macro_rules! look {
    ($n:expr, $($v:expr),*) => {{
        let [temp, incl, roll, inner, outer, opac, dopp, beam, gain, contr, wind, speed, expo, star] = [$($v),*];
        DiskLook { name: $n, temp, incl, roll, inner, outer, opac, dopp, beam, gain, contr, wind, speed, expo, star }
    }};
}

//                                temp    incl  roll  inner outer opac  dopp  beam gain contr wind speed expo  star
pub const INFERNO: DiskLook = look!("inferno", 5500.0, 1.50, 0.35, 1.8, 8.0, 0.90, 0.60, 2.5, 2.2, 1.6, 7.0, 5.0, 1.40, 0.25);
pub const GARGANTUA: DiskLook = look!("gargantua", 4500.0, 1.52, 0.10, 2.2, 7.0, 0.85, 0.35, 2.0, 1.4, 0.5, 7.0, 5.0, 1.20, 0.25);
pub const M87: DiskLook = look!("m87* donut", 3800.0, 0.55, -0.30, 2.2, 6.0, 0.45, 0.90, 3.5, 1.6, 0.4, 3.0, 2.5, 1.10, 0.2);
pub const EMBER: DiskLook = look!("face-on ember", 6500.0, 0.30, 0.00, 3.0, 10.0, 0.50, 0.80, 2.5, 1.0, 1.1, 7.0, 5.0, 1.00, 0.2);
pub const QUASAR: DiskLook = look!("quasar", 15000.0, 1.30, 0.35, 3.0, 14.0, 0.35, 1.00, 4.0, 1.2, 1.3, 8.0, 5.0, 0.80, 0.25);
/// Orange/red, rendered at low resolution and dithered onto a 4-colour palette (pixel art).
pub const PIXEL_EMBER: DiskLook = look!("pixel ember", 3600.0, 1.48, 0.30, 1.8, 8.5, 0.90, 0.70, 2.5, 2.4, 1.4, 7.0, 4.0, 1.50, 0.0);
pub const BLAZAR: DiskLook = look!("blazar", 18000.0, 1.05, 0.55, 3.0, 16.0, 0.30, 1.00, 5.0, 1.0, 1.5, 9.0, 6.0, 0.75, 0.25);

pub struct Style {
    pub look: DiskLook,
    /// Optional monochrome tint (e.g. phosphor green): luminance is mapped onto this colour.
    pub tint: Option<Rgb>,
    /// Shadow radius as a fraction of the framebuffer height.
    pub rh: f32,
    /// Hole centre in framebuffer uv (0..1, y down).
    pub center: (f32, f32),
    /// Produce an alpha channel (transparent where the UI should show through).
    pub alpha: bool,
}

/// Fit the hole into a pane: shadow radius so the whole disk is visible.
/// Disk extent in shadow radii (how far the visible disk reaches from the centre).
pub fn reach(look: &DiskLook) -> f32 {
    (look.outer + 3.0) / B_CRIT
}

/// CPU fallback for pixel art: ordered dither of luminance onto a small palette.
pub fn quantize(fb: &mut Fb, levels: f32, pal: &[Rgb; 4]) {
    const BAYER: [f32; 16] = [0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0];
    for y in 0..fb.h {
        for x in 0..fb.w {
            let i = y * fb.w + x;
            let p = fb.px[i];
            let lum = (0.2126 * p.0 + 0.7152 * p.1 + 0.0722 * p.2).clamp(0.0, 1.0);
            let d = (BAYER[(y % 4) * 4 + x % 4] + 0.5) / 16.0;
            let q = ((lum.powf(0.8) * levels + d - 0.5).clamp(0.0, levels)).floor() / levels;
            fb.px[i] = if q <= 0.0 { Rgb::BLACK } else { crate::gfx::ramp(pal, q) };
            if !fb.alpha.is_empty() && q <= 0.0 && fb.alpha[i] < 0.99 {
                fb.alpha[i] = 0.0;
            }
        }
    }
}

pub fn fit_rh(w: usize, h: usize, look: &DiskLook) -> f32 {
    let aspect = w as f32 / h as f32;
    let reach = (look.outer + 3.0) / B_CRIT; // disk extent in shadow radii, with lensing margin
    (0.92 * aspect / (2.0 * reach)).min(0.85 / (2.0 * reach * look.incl.cos().abs().max(0.35)))
}

fn hash21(x: f32, y: f32) -> f32 {
    let (mut px, mut py) = ((x * 234.34).fract(), (y * 435.345).fract());
    let d = px * (px + 34.23) + py * (py + 34.23);
    px += d;
    py += d;
    (px * py).fract().abs()
}

fn vnoise_wrap_y(x: f32, y: f32, per: f32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (fx, fy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let y0 = iy.rem_euclid(per);
    let y1 = (iy + 1.0).rem_euclid(per);
    let a = hash21(ix, y0) + (hash21(ix + 1.0, y0) - hash21(ix, y0)) * fx;
    let b = hash21(ix, y1) + (hash21(ix + 1.0, y1) - hash21(ix, y1)) * fx;
    a + (b - a) * fy
}

fn blackbody(t: f32) -> [f32; 3] {
    let t = t.clamp(1500.0, 40000.0) / 100.0;
    let r = if t <= 66.0 { 1.0 } else { (1.292936 * (t - 60.0).powf(-0.1332047)).clamp(0.0, 1.0) };
    let g = if t <= 66.0 {
        (0.3900816 * t.ln() - 0.6318414).clamp(0.0, 1.0)
    } else {
        (1.1298909 * (t - 60.0).powf(-0.0755148)).clamp(0.0, 1.0)
    };
    let b = if t >= 66.0 {
        1.0
    } else if t <= 19.0 {
        0.0
    } else {
        (0.5432068 * (t - 10.0).ln() - 1.1962540).clamp(0.0, 1.0)
    };
    [r, g, b]
}

fn stars(d: [f32; 3], t: f32) -> [f32; 3] {
    let sx = d[0].atan2(-d[2]) * 40.0;
    let sy = d[1].clamp(-1.0, 1.0).asin() * 40.0;
    let (ix, iy) = (sx.floor(), sy.floor());
    let h = hash21(ix, iy);
    if h < 0.92 {
        return [0.0; 3];
    }
    let (fx, fy) = (sx - ix - 0.5, sy - iy - 0.5);
    let (ox, oy) = ((hash21(ix + 17.3, iy + 17.3) - 0.5) * 0.7, (hash21(ix + 31.7, iy + 31.7) - 0.5) * 0.7);
    let dist = ((fx - ox).powi(2) + (fy - oy).powi(2)).sqrt();
    let spark = (1.0 - dist / 0.10).clamp(0.0, 1.0);
    let spark = spark * spark * (3.0 - 2.0 * spark);
    let tw = 0.7 + 0.3 * (t * (0.5 + 2.0 * hash21(ix + 5.1, iy + 5.1)) + 40.0 * h).sin();
    let k = spark * tw * (h - 0.92) / 0.08;
    let m = hash21(ix + 2.9, iy + 2.9);
    [k * (1.0 - 0.25 * m), k * (0.82 + 0.03 * m), k * (0.6 + 0.4 * m)]
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// `t` is the (audio-modulated) animation clock; `a` adds a gentle brightness response.
pub fn render(fb: &mut Fb, t: f32, a: &AudioFrame, st: &Style) {
    let (w, h) = (fb.w, fb.h);
    let l = st.look;
    let rin = l.inner.max(1.6);
    let rout = l.outer.max(rin + 0.5);
    let gain = l.gain * (0.85 + 0.45 * a.bass + 0.25 * a.beat);
    let rh = st.rh.max(1e-3);
    let wk = B_CRIT / rh;
    let bmax = rout + 3.0;
    let z0 = 14.0f32.max(rout + 5.0);
    let (ci, si) = (l.incl.cos(), l.incl.sin());
    let n = [0.0, si, ci];
    let e2 = [0.0, ci, -si];
    let sdir = if l.speed < 0.0 { -1.0 } else { 1.0 };
    let spd = l.speed.abs();
    let (cr, sr) = (l.roll.cos(), l.roll.sin());
    if st.alpha && fb.alpha.len() != w * h {
        fb.alpha = vec![0.0; w * h];
    }
    let want_alpha = st.alpha;

    let rows: Vec<(Vec<Rgb>, Vec<f32>)> = (0..h)
        .into_par_iter()
        .map(|y| {
            let mut row = vec![Rgb::BLACK; w];
            let mut arow = vec![0.0f32; if want_alpha { w } else { 0 }];
            for x in 0..w {
                // p: offset from centre in units of fb height (square pixels), y up
                let px = (x as f32 + 0.5 - st.center.0 * w as f32) / h as f32;
                let py = -((y as f32 + 0.5 - st.center.1 * h as f32) / h as f32);
                let (prx, pry) = ((cr * px - sr * py) * wk, (sr * px + cr * py) * wk);
                let b = (prx * prx + pry * pry).sqrt();
                if b >= bmax {
                    if !want_alpha && l.star > 0.0 {
                        let dd = norm([-(prx / b) * (2.0 / b), -(pry / b) * (2.0 / b), -1.0]);
                        let s = stars(dd, t);
                        row[x] = Rgb(s[0] * l.star, s[1] * l.star, s[2] * l.star);
                    }
                    continue;
                }
                let mut xp = [prx, pry, z0];
                let mut v = [0.0f32, 0.0, -1.0];
                let h2 = prx * prx + pry * pry;
                let mut emit = [0.0f32; 3];
                let mut trans = 1.0f32;
                let mut captured = false;
                let mut s_prev = dot(xp, n);
                let mut x_prev = xp;
                for _ in 0..56 {
                    let r2 = dot(xp, xp);
                    if r2 < 1.0 {
                        captured = true;
                        break;
                    }
                    if xp[2] < -z0 && v[2] < 0.0 || r2 > 4.0 * z0 * z0 {
                        break;
                    }
                    let r = r2.sqrt();
                    let dt = (0.16 * r).clamp(0.03, 1.5);
                    let k = -1.5 * h2 / (r2 * r2 * r) * 0.5 * dt;
                    v = [v[0] + xp[0] * k, v[1] + xp[1] * k, v[2] + xp[2] * k];
                    xp = [xp[0] + v[0] * dt, xp[1] + v[1] * dt, xp[2] + v[2] * dt];
                    let r2b = dot(xp, xp);
                    let rb = r2b.sqrt();
                    let k = -1.5 * h2 / (r2b * r2b * rb) * 0.5 * dt;
                    v = [v[0] + xp[0] * k, v[1] + xp[1] * k, v[2] + xp[2] * k];
                    let s = dot(xp, n);
                    if s * s_prev < 0.0 && trans > 0.02 {
                        let tc = s_prev / (s_prev - s);
                        let xc = [
                            x_prev[0] + (xp[0] - x_prev[0]) * tc,
                            x_prev[1] + (xp[1] - x_prev[1]) * tc,
                            x_prev[2] + (xp[2] - x_prev[2]) * tc,
                        ];
                        let rc = dot(xc, xc).sqrt();
                        if rc > rin && rc < rout {
                            let sm = |a: f32, b: f32, x: f32| {
                                let q = ((x - a) / (b - a)).clamp(0.0, 1.0);
                                q * q * (3.0 - 2.0 * q)
                            };
                            let band = sm(rin, rin * 1.25, rc) * (1.0 - sm(rout * 0.70, rout, rc));
                            let phi = dot(xc, e2).atan2(xc[0]);
                            let turns = phi / std::f32::consts::TAU;
                            let kep = (rin / rc).powf(1.5);
                            let gloc = (1.0 - 1.5 / rc).max(0.02).sqrt();
                            let swirl = rc * l.wind * 0.12 - t * kep * spd * gloc * sdir;
                            let mut streaks = vnoise_wrap_y(rc * 2.8, turns * 19.0 + swirl * 3.0, 19.0) * 0.65
                                + vnoise_wrap_y(rc * 1.0, turns * 9.0 + swirl * 1.5 + 7.0, 9.0) * 0.35;
                            streaks = 0.35 + l.contr * streaks * streaks;
                            let gas = norm(cross(n, xc));
                            let gas = [gas[0] * sdir, gas[1] * sdir, gas[2] * sdir];
                            let beta = (1.0 / (2.0 * (rc - 1.0)).max(0.2).sqrt()).clamp(0.0, 0.99);
                            let g = gloc / (1.0 + beta * dot(gas, norm(v))).max(0.05);
                            let g = 1.0 + (g - 1.0) * l.dopp;
                            let xpr = (1.0 - (rin / rc).sqrt()).max(0.0);
                            let tprof = (rin / rc).powf(0.75) * xpr.powf(0.25) / 0.488;
                            let cbb = blackbody(l.temp * tprof * g);
                            let boost = g.powf(l.beam);
                            let density = band * streaks;
                            let kk = trans * gain * 2.2 * density * tprof * tprof * boost;
                            emit = [emit[0] + cbb[0] * kk, emit[1] + cbb[1] * kk, emit[2] + cbb[2] * kk];
                            trans *= 1.0 - (l.opac * density).clamp(0.0, 1.0);
                        }
                    }
                    s_prev = s;
                    x_prev = xp;
                }
                if !captured && dot(xp, xp) < 4.0 {
                    captured = true;
                }
                let mut bg = [0.0f32; 3];
                if !captured && !want_alpha && l.star > 0.0 {
                    bg = stars(norm(v), t);
                    bg = [bg[0] * l.star, bg[1] * l.star, bg[2] * l.star];
                }
                let c = [
                    bg[0] * trans + 1.0 - (-emit[0] * l.expo).exp(),
                    bg[1] * trans + 1.0 - (-emit[1] * l.expo).exp(),
                    bg[2] * trans + 1.0 - (-emit[2] * l.expo).exp(),
                ];
                let mut col = Rgb(c[0], c[1], c[2]);
                if let Some(tint) = st.tint {
                    let lum = (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).min(1.0);
                    col = tint.scale(lum * 1.15).lerp(Rgb(0.92, 1.0, 0.94), (lum - 0.75).max(0.0) * 2.0);
                }
                row[x] = col;
                if want_alpha {
                    let m = c[0].max(c[1]).max(c[2]).min(1.0);
                    arow[x] = if captured { 1.0 } else { (1.0 - trans).max(m).min(1.0) };
                }
            }
            (row, arow)
        })
        .collect();
    for (y, (row, arow)) in rows.into_iter().enumerate() {
        fb.px[y * w..(y + 1) * w].copy_from_slice(&row);
        if want_alpha {
            fb.alpha[y * w..(y + 1) * w].copy_from_slice(&arow);
        }
    }
}

/// Bend the UI around a hole: every cell near the hole shows the cell its light ray came from.
/// Units: x in cell widths, y in cell heights × 2 (cells are ~1:2), so distances are isotropic.
/// `rh` is the shadow radius in those units. Inside the Einstein ring the image flips, as it should.
pub fn lens_cells(buf: &mut Buffer, area: Rect, cx: f32, cy: f32, rh: f32, borders_only: bool) {
    let src = buf.clone();
    let reach = rh * 6.5;
    let x0 = ((cx - reach).floor().max(area.x as f32)) as u16;
    let x1 = ((cx + reach).ceil().min(area.right() as f32)) as u16;
    let y0 = ((cy - reach / 2.0).floor().max(area.y as f32)) as u16;
    let y1 = ((cy + reach / 2.0).ceil().min(area.bottom() as f32)) as u16;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5 - cx;
            let py = (y as f32 + 0.5 - cy) * 2.0;
            let plen = (px * px + py * py).sqrt().max(1e-4);
            let structural = |c: &ratatui::buffer::Cell| {
                c.symbol().chars().next().is_some_and(|ch| ch == ' ' || ('\u{2500}'..='\u{257f}').contains(&ch))
            };
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            if borders_only && !structural(cell) {
                continue;
            }
            if plen < rh * 1.02 {
                if borders_only {
                    continue;
                }
                cell.set_char(' ').set_bg(Color::Rgb(0, 0, 0));
                continue;
            }
            let window = (-(plen / (4.5 * rh)).powi(2)).exp();
            let defl = 5.5 * rh * rh / plen * window;
            let sx = cx + px - px / plen * defl;
            let sy = cy + (py - py / plen * defl) / 2.0;
            // mirror at the area edges, like the shader's mirrorUV
            let mirror = |v: f32, lo: f32, hi: f32| {
                let span = hi - lo;
                let u = (v - lo).rem_euclid(2.0 * span);
                lo + if u > span { 2.0 * span - u } else { u }
            };
            let sx = mirror(sx, area.x as f32, area.right() as f32 - 0.01) as u16;
            let sy = mirror(sy, area.y as f32, area.bottom() as f32 - 0.01) as u16;
            if let Some(s) = src.cell((sx, sy)) {
                // borders-only: space bends (lines, gaps), content never moves
                if !borders_only || structural(s) {
                    *cell = s.clone();
                }
            }
        }
    }
}
