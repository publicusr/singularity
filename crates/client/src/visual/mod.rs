//! The Visual engine (ADR-0004): the black hole on the GPU (wgpu compute) with a CPU fallback, and the gravity well
//! it lives in (#16). Visuals react to the Daemon's spectrum frames and never cost more than they must: at most one
//! frame per Client frame (≤ 30 fps), a resolution cap, and a slower pace when the terminal isn't focused.

pub mod blackhole;
mod gpu;
pub mod well;

use blackhole::{Frame, LOOKS};
use singularity_proto::SpectrumFrame;

/// Widest image the GPU renders (wider Panes are upscaled by the terminal); the CPU gets far fewer pixels.
const MAX_W_GPU: usize = 1280;
const MAX_W_CPU: usize = 420;

pub struct BlackHole {
    /// None until the first frame; `Some(None)` = no usable GPU (or not allowed)
    gpu: Option<Option<gpu::Gpu>>,
    pub allow_gpu: bool,
    pub look: usize,
    t: f32,
    beat: f32,
    bass: f32,
}

impl Default for BlackHole {
    fn default() -> Self {
        BlackHole { gpu: None, allow_gpu: false, look: 0, t: 0.0, beat: 0.0, bass: 0.0 }
    }
}

impl BlackHole {
    pub fn look(&self) -> &'static blackhole::DiskLook {
        &LOOKS[self.look % LOOKS.len()]
    }

    pub fn next_look(&mut self) {
        self.look = (self.look + 1) % LOOKS.len();
    }

    /// Which renderer is in use, once known.
    pub fn backend(&self) -> Option<String> {
        match &self.gpu {
            Some(Some(g)) => Some(format!("GPU: {}", g.adapter)),
            Some(None) => Some("CPU".into()),
            None => None,
        }
    }

    /// Advance the clock by `dt` seconds; the disk turns a little faster and glows with the bass.
    pub fn advance(&mut self, dt: f32, audio: Option<&SpectrumFrame>) {
        let bass = audio.map(|f| f.bands.iter().take(8).map(|b| *b as f32).sum::<f32>() / (8.0 * 255.0)).unwrap_or(0.0);
        // calm: the response is smoothed, never a strobe
        self.bass += (bass - self.bass) * (dt * 6.0).min(1.0);
        self.beat = if audio.is_some_and(|f| f.beat) { 1.0 } else { (self.beat - dt * 3.0).max(0.0) };
        self.t += dt * (0.8 + 0.5 * self.bass);
    }

    /// The low end right now (0..1), for things that breathe with the music.
    pub fn bass(&self) -> f32 {
        self.bass
    }

    /// The largest size worth rendering for a `w`×`h` pixel area (aspect kept).
    pub fn render_size(&mut self, w: usize, h: usize) -> (usize, usize) {
        let max = if self.gpu_ready() { MAX_W_GPU } else { MAX_W_CPU };
        let k = (max as f32 / w.max(1) as f32).min(1.0);
        (((w as f32 * k) as usize).max(1), ((h as f32 * k) as usize).max(1))
    }

    fn gpu_ready(&mut self) -> bool {
        if self.gpu.is_none() {
            self.gpu = Some(if self.allow_gpu { gpu::Gpu::new() } else { None });
        }
        matches!(self.gpu, Some(Some(_)))
    }

    /// Render a `w`×`h` RGBA8 frame. Pixel-art looks render at 1/5 resolution and are scaled up crisply.
    pub fn render(&mut self, w: usize, h: usize) -> Vec<u8> {
        let look = *self.look();
        let k = if look.pixel > 0 && w >= look.pixel * 8 { look.pixel } else { 1 };
        let (rw, rh_px) = ((w / k).max(1), (h / k).max(1));
        let f = Frame { look, t: self.t, gain: 0.85 + 0.45 * self.bass + 0.25 * self.beat, rh: blackhole::fit_rh(rw, rh_px, &look) };
        let small = match self.gpu_ready().then(|| self.gpu.as_mut().and_then(|g| g.as_mut()).and_then(|g| g.render(&f, rw, rh_px))).flatten() {
            Some(px) => px,
            None => blackhole::render_cpu(&f, rw, rh_px),
        };
        if k == 1 {
            return small;
        }
        // nearest-neighbour upscale: pixel art stays crisp
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            let sy = (y / k).min(rh_px - 1);
            for x in 0..w {
                let sx = (x / k).min(rw - 1);
                out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&small[(sy * rw + sx) * 4..(sy * rw + sx) * 4 + 4]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GPU and CPU trace the same model: their images agree within tolerance (skipped without a GPU).
    #[test]
    fn gpu_matches_cpu_within_tolerance() {
        let Some(mut gpu) = gpu::Gpu::new() else {
            eprintln!("no GPU adapter: skipping the GPU/CPU comparison");
            return;
        };
        for i in [0usize, 2, 6] {
            let look = LOOKS[i];
            let (w, h) = (128, 64);
            let f = Frame { look, t: 2.5, gain: 1.1, rh: blackhole::fit_rh(w, h, &look) };
            let g = gpu.render(&f, w, h).expect("GPU frame");
            let c = blackhole::render_cpu(&f, w, h);
            let mean = g.iter().zip(&c).map(|(a, b)| (*a as f32 - *b as f32).abs()).sum::<f32>() / g.len() as f32;
            let off = g.chunks(4).zip(c.chunks(4)).filter(|(a, b)| a.iter().zip(b.iter()).any(|(p, q)| (*p as i32 - *q as i32).abs() > 24)).count();
            println!("{}: mean |Δ| {mean:.2}, {off} of {} pixels differ by > 24", look.name, w * h);
            assert!(mean < 2.0, "{}: mean difference {mean}", look.name);
            assert!(off * 100 < w * h, "{}: {off} pixels differ visibly", look.name);
        }
    }

    /// Budget (spec: benchmarks, not unit tests): `cargo test --release -p singularity hole_cpu -- --ignored --nocapture`.
    /// Renders a full-resolution well at a paced 30 fps and hands every frame to the kitty shm transfer.
    #[test]
    #[ignore]
    fn hole_cpu_budget() {
        fn cpu_secs() -> f64 {
            let mut u: libc::rusage = unsafe { std::mem::zeroed() };
            unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
            let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
            t(u.ru_utime) + t(u.ru_stime)
        }
        let mut bh = BlackHole { allow_gpu: true, ..Default::default() };
        let mut kitty = crate::gfx::Kitty::new(true);
        let rect = ratatui::layout::Rect::new(0, 0, 128, 36);
        for look in [0usize, 6] {
            bh.look = look;
            let (w, h) = bh.render_size(1280, 720);
            let _ = bh.render(w, h); // warm-up (shader compile)
            let (c0, t0) = (cpu_secs(), std::time::Instant::now());
            for i in 0..150 {
                bh.advance(1.0 / 30.0, None);
                let px = bh.render(w, h);
                kitty.frame(1, rect, w as u32, h as u32, px);
                kitty.flush(&mut std::io::sink()).unwrap();
                // pace to 30 fps
                let next = std::time::Duration::from_secs_f64((i + 1) as f64 / 30.0);
                if let Some(d) = next.checked_sub(t0.elapsed()) {
                    std::thread::sleep(d);
                }
            }
            let share = (cpu_secs() - c0) / t0.elapsed().as_secs_f64();
            println!("{} at {w}×{h}, {}: {:.1}% of a core", bh.look().name, bh.backend().unwrap_or_default(), share * 100.0);
            assert!(share < 0.20);
        }
    }

    #[test]
    fn audio_brightens_gently() {
        let mut bh = BlackHole::default();
        let loud = SpectrumFrame { bands: vec![255; 64], level: 255, vu: [255, 255], beat: true };
        bh.advance(1.0 / 30.0, Some(&loud));
        assert!(bh.bass < 0.25, "one loud frame doesn't flash the hole: {}", bh.bass);
        for _ in 0..60 {
            bh.advance(1.0 / 30.0, Some(&loud));
        }
        assert!(bh.bass > 0.9);
    }
}
