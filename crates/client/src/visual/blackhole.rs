//! The black hole (ADR-0004): the ghostty-blackhole model by s0xDk (MIT, https://github.com/s0xDk/ghostty-blackhole),
//! itself after Eric Bruneton's shader. Every pixel near the hole integrates its own null geodesic
//! (a = -3/2 h² x / r⁵, leapfrog, ≤ 56 steps) through a thin Keplerian disk, coloured as Doppler-shifted, beamed
//! blackbody light. The photon ring and the disk seen over and under the shadow fall out of the tracing.
//!
//! This is the CPU tracer; `blackhole.wgsl` is the same model on the GPU. Both end in the same tone mapping,
//! quantisation and RGBA8 packing, so their outputs can be compared (see the tests).

// the constants match `blackhole.wgsl` digit for digit, so both tracers agree
#![allow(clippy::excessive_precision)]

use rayon::prelude::*;

pub const B_CRIT: f32 = 2.598_076_2;

/// A named preset for the accretion disk.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// > 0: rendered at 1/`pixel` resolution and dithered onto `PIXEL_RAMP` (pixel art)
    pub pixel: usize,
}

macro_rules! look {
    ($n:expr, $px:expr, $($v:expr),*) => {{
        let [temp, incl, roll, inner, outer, opac, dopp, beam, gain, contr, wind, speed, expo, star] = [$($v),*];
        DiskLook { name: $n, temp, incl, roll, inner, outer, opac, dopp, beam, gain, contr, wind, speed, expo, star, pixel: $px }
    }};
}

/// The Disk looks, in the order `v` cycles them (spec story 37); the first is the default, as in the prototype.
//                                     px   temp    incl  roll  inner outer opac  dopp  beam gain contr wind speed expo  star
pub const LOOKS: [DiskLook; 7] = [
    look!("Inferno", 0, 5500.0, 1.50, 0.35, 1.8, 8.0, 0.90, 0.60, 2.5, 2.2, 1.6, 7.0, 5.0, 1.40, 0.35),
    look!("Gargantua", 0, 4500.0, 1.52, 0.10, 2.2, 7.0, 0.85, 0.35, 2.0, 1.4, 0.5, 7.0, 5.0, 1.20, 0.35),
    look!("M87*", 0, 3800.0, 0.55, -0.30, 2.2, 6.0, 0.45, 0.90, 3.5, 1.6, 0.4, 3.0, 2.5, 1.10, 0.35),
    look!("Quasar", 0, 15000.0, 1.30, 0.35, 3.0, 14.0, 0.35, 1.00, 4.0, 1.2, 1.3, 8.0, 5.0, 0.80, 0.35),
    look!("Blazar", 0, 18000.0, 1.05, 0.55, 3.0, 16.0, 0.30, 1.00, 5.0, 1.0, 1.5, 9.0, 6.0, 0.75, 0.35),
    look!("Face-on ember", 0, 6500.0, 0.30, 0.00, 3.0, 10.0, 0.50, 0.80, 2.5, 1.0, 1.1, 7.0, 5.0, 1.00, 0.35),
    look!("Pixel ember", 5, 3600.0, 1.48, 0.30, 1.8, 8.5, 0.90, 0.70, 2.5, 2.4, 1.4, 7.0, 4.0, 1.50, 0.35),
];

/// Pixel-art palette: deep red → orange → amber → pale yellow (never white).
pub const PIXEL_RAMP: [[f32; 3]; 4] = [[0.35, 0.03, 0.02], [0.85, 0.22, 0.04], [1.0, 0.58, 0.12], [1.0, 0.93, 0.62]];
pub const PIXEL_LEVELS: f32 = 5.0;

/// One frame's request.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub look: DiskLook,
    /// animation clock (seconds, may run faster with the music)
    pub t: f32,
    /// brightness response to the audio, ≈ 0.85 … 1.5
    pub gain: f32,
    /// shadow radius as a fraction of the image height
    pub rh: f32,
    /// where the hole sits in the image (0..1, y down); the middle unless it's cut by the screen's edge
    pub center: (f32, f32),
}

/// How far the disk and its halo reach, in shadow radii.
pub fn reach(look: &DiskLook) -> f32 {
    (look.outer + 3.0) / B_CRIT
}

/// Shadow radius that fits the whole disk (with its lensed halo) inside a `w`×`h` image.
pub fn fit_rh(w: usize, h: usize, look: &DiskLook) -> f32 {
    let aspect = w as f32 / h.max(1) as f32;
    let reach = reach(look);
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
    if t > 66.0 {
        [(1.292936 * (t - 60.0).powf(-0.1332047)).clamp(0.0, 1.0), (1.1298909 * (t - 60.0).powf(-0.0755148)).clamp(0.0, 1.0), 1.0]
    } else {
        let b = if t <= 19.0 { 0.0 } else { (0.5432068 * (t - 10.0).ln() - 1.1962540).clamp(0.0, 1.0) };
        [1.0, (0.3900816 * t.ln() - 0.6318414).clamp(0.0, 1.0), b]
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let q = ((x - a) / (b - a)).clamp(0.0, 1.0);
    q * q * (3.0 - 2.0 * q)
}

fn stars(d: [f32; 3], t: f32) -> [f32; 3] {
    let (sx, sy) = (d[0].atan2(-d[2]) * 40.0, d[1].clamp(-1.0, 1.0).asin() * 40.0);
    let (ix, iy) = (sx.floor(), sy.floor());
    let h = hash21(ix, iy);
    if h < 0.92 {
        return [0.0; 3];
    }
    let (fx, fy) = (sx - ix - 0.5, sy - iy - 0.5);
    let (ox, oy) = ((hash21(ix + 17.3, iy + 17.3) - 0.5) * 0.7, (hash21(ix + 31.7, iy + 31.7) - 0.5) * 0.7);
    let spark = smoothstep(0.10, 0.0, ((fx - ox).powi(2) + (fy - oy).powi(2)).sqrt());
    let tw = 0.7 + 0.3 * (t * (0.5 + 2.0 * hash21(ix + 5.1, iy + 5.1)) + 40.0 * h).sin();
    let k = spark * tw * (h - 0.92) / 0.08;
    let m = hash21(ix + 2.9, iy + 2.9);
    [k * (1.0 - 0.25 * m), k * (0.82 + 0.03 * m), k * (0.6 + 0.4 * m)]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// The colour of one pixel (before quantisation), as the shader computes it.
fn trace(f: &Frame, x: usize, y: usize, w: usize, h: usize) -> [f32; 3] {
    let l = &f.look;
    let rin = l.inner.max(1.6);
    let rout = l.outer.max(rin + 0.5);
    let gain = l.gain * f.gain;
    let wk = B_CRIT / f.rh.max(1e-3);
    let px = (x as f32 + 0.5 - f.center.0 * w as f32) / h as f32;
    let py = -((y as f32 + 0.5 - f.center.1 * h as f32) / h as f32);
    let (cr, sr) = (l.roll.cos(), l.roll.sin());
    let (prx, pry) = ((cr * px - sr * py) * wk, (sr * px + cr * py) * wk);
    let b = (prx * prx + pry * pry).sqrt();
    let bmax = rout + 3.0;
    let z0 = 14.0f32.max(rout + 5.0);
    if b >= bmax {
        if l.star <= 0.0 {
            return [0.0; 3];
        }
        let s = stars(norm([-(prx / b) * (2.0 / b), -(pry / b) * (2.0 / b), -1.0]), f.t);
        return [s[0] * l.star, s[1] * l.star, s[2] * l.star];
    }
    let (ci, si) = (l.incl.cos(), l.incl.sin());
    let n = [0.0, si, ci];
    let e2 = [0.0, ci, -si];
    let sdir = if l.speed < 0.0 { -1.0 } else { 1.0 };
    let spd = l.speed.abs();
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
        let k = -1.5 * h2 / (r2b * r2b * r2b.sqrt()) * 0.5 * dt;
        v = [v[0] + xp[0] * k, v[1] + xp[1] * k, v[2] + xp[2] * k];
        let s = dot(xp, n);
        if s * s_prev < 0.0 && trans > 0.02 {
            let tc = s_prev / (s_prev - s);
            let xc = [x_prev[0] + (xp[0] - x_prev[0]) * tc, x_prev[1] + (xp[1] - x_prev[1]) * tc, x_prev[2] + (xp[2] - x_prev[2]) * tc];
            let rc = dot(xc, xc).sqrt();
            if rc > rin && rc < rout {
                let band = smoothstep(rin, rin * 1.25, rc) * (1.0 - smoothstep(rout * 0.70, rout, rc));
                let turns = dot(xc, e2).atan2(xc[0]) / std::f32::consts::TAU;
                let kep = (rin / rc).powf(1.5);
                let gloc = (1.0 - 1.5 / rc).max(0.02).sqrt();
                let swirl = rc * l.wind * 0.12 - f.t * kep * spd * gloc * sdir;
                let streaks = vnoise_wrap_y(rc * 2.8, turns * 19.0 + swirl * 3.0, 19.0) * 0.65 + vnoise_wrap_y(rc, turns * 9.0 + swirl * 1.5 + 7.0, 9.0) * 0.35;
                let streaks = 0.35 + l.contr * streaks * streaks;
                let gas = norm(cross(n, xc));
                let gas = [gas[0] * sdir, gas[1] * sdir, gas[2] * sdir];
                let beta = (1.0 / (2.0 * (rc - 1.0)).max(0.2).sqrt()).clamp(0.0, 0.99);
                let g = gloc / (1.0 + beta * dot(gas, norm(v))).max(0.05);
                let g = 1.0 + (g - 1.0) * l.dopp;
                let xpr = (1.0 - (rin / rc).sqrt()).max(0.0);
                let tprof = (rin / rc).powf(0.75) * xpr.powf(0.25) / 0.488;
                let cbb = blackbody(l.temp * tprof * g);
                let density = band * streaks;
                let kk = trans * gain * 2.2 * density * tprof * tprof * g.powf(l.beam);
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
    let bg = if !captured && l.star > 0.0 {
        let s = stars(norm(v), f.t);
        [s[0] * l.star, s[1] * l.star, s[2] * l.star]
    } else {
        [0.0; 3]
    };
    [0, 1, 2].map(|i| bg[i] * trans + 1.0 - (-emit[i] * l.expo).exp())
}

const BAYER: [f32; 16] = [0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0];

/// The shader's tail: pixel-art quantisation (if `levels` > 0), clamp, pack as RGBA8.
pub fn finish(col: [f32; 3], x: usize, y: usize, levels: f32) -> [u8; 4] {
    let mut col = col;
    if levels > 0.5 {
        let lum = (0.2126 * col[0] + 0.7152 * col[1] + 0.0722 * col[2]).clamp(0.0, 1.0);
        let d = (BAYER[(y % 4) * 4 + x % 4] + 0.5) / 16.0;
        let q = (lum.powf(0.8) * levels + d - 0.5).clamp(0.0, levels).floor() / levels;
        col = if q <= 0.0 {
            [0.0; 3]
        } else {
            let s = q.clamp(0.0, 1.0) * 3.0;
            let (i, t) = ((s as usize).min(2), s - (s as usize).min(2) as f32);
            [0, 1, 2].map(|c| PIXEL_RAMP[i][c] + (PIXEL_RAMP[i + 1][c] - PIXEL_RAMP[i][c]) * t)
        };
    }
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(col[0]), q(col[1]), q(col[2]), 255]
}

/// Render `w`×`h` RGBA8 pixels on the CPU (all cores).
pub fn render_cpu(f: &Frame, w: usize, h: usize) -> Vec<u8> {
    let levels = if f.look.pixel > 0 { PIXEL_LEVELS } else { 0.0 };
    let mut out = vec![0u8; w * h * 4];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            row[x * 4..x * 4 + 4].copy_from_slice(&finish(trace(f, x, y, w, h), x, y, levels));
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(look: usize, w: usize, h: usize) -> Frame {
        let look = LOOKS[look];
        Frame { look, t: 1.0, gain: 1.0, rh: fit_rh(w, h, &look), center: (0.5, 0.5) }
    }

    #[test]
    fn every_look_fits_its_image_and_has_a_shadow() {
        for (i, look) in LOOKS.iter().enumerate() {
            let (w, h) = (96, 48);
            let px = render_cpu(&frame(i, w, h), w, h);
            let at = |x: usize, y: usize| &px[(y * w + x) * 4..(y * w + x) * 4 + 3];
            // the centre is the shadow (black), the disk is lit somewhere, and the edges stay dark (it fits)
            assert!(at(w / 2, h / 2).iter().all(|c| *c < 30) || look.incl < 1.0, "{}: shadow", look.name);
            assert!(px.chunks(4).any(|p| p[0] > 120), "{}: a bright disk", look.name);
            let edge: u32 = (0..w).flat_map(|x| [at(x, 0), at(x, h - 1)]).map(|p| p.iter().map(|c| *c as u32).sum::<u32>()).max().unwrap();
            assert!(edge < 200, "{}: the disk is cut by the edge ({edge})", look.name);
        }
    }

    #[test]
    fn pixel_ember_uses_only_its_palette() {
        let (w, h) = (40, 20);
        let px = render_cpu(&frame(6, w, h), w, h);
        let mut colours: Vec<[u8; 3]> = px.chunks(4).map(|p| [p[0], p[1], p[2]]).collect();
        colours.sort();
        colours.dedup();
        assert!(colours.len() <= 1 + PIXEL_LEVELS as usize, "{colours:?}");
        assert!(colours.iter().all(|c| !(c[0] > 245 && c[1] > 245 && c[2] > 245)), "no white");
    }
}
