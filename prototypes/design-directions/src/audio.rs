//! Audio feed for the visuals.
//!
//! Captures whatever the system is playing from the default PipeWire/Pulse monitor (`parec`),
//! the same trick cava uses. In the real app this would tap only the Soloist node.
//! When nothing is playing it falls back to a synthetic groove so the demo is never dead.

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const BANDS: usize = 64;
const N: usize = 2048;
const RATE: f32 = 44100.0;

#[derive(Clone)]
pub struct AudioFrame {
    pub bands: [f32; BANDS],
    pub peaks: [f32; BANDS],
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub rms: f32,
    /// 1.0 on a detected beat, decays towards 0
    pub beat: f32,
    /// slowly-smoothed overall energy, good for speeds
    pub energy: f32,
    pub wave_l: Vec<f32>,
    pub wave_r: Vec<f32>,
    pub live: bool,
}

impl Default for AudioFrame {
    fn default() -> Self {
        AudioFrame {
            bands: [0.0; BANDS],
            peaks: [0.0; BANDS],
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            rms: 0.0,
            beat: 0.0,
            energy: 0.0,
            wave_l: vec![0.0; 512],
            wave_r: vec![0.0; 512],
            live: false,
        }
    }
}

struct Ring {
    l: Vec<f32>,
    r: Vec<f32>,
    pos: usize,
    last_loud: Option<Instant>,
}

pub struct Audio {
    ring: Arc<Mutex<Ring>>,
    capture_ok: Arc<Mutex<bool>>,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    pub force_synthetic: bool,
    frame: AudioFrame,
    bass_avg: f32,
    gain: f32,
    beat_cooldown: f32,
}

impl Audio {
    pub fn start() -> Audio {
        let ring = Arc::new(Mutex::new(Ring {
            l: vec![0.0; N * 2],
            r: vec![0.0; N * 2],
            pos: 0,
            last_loud: None,
        }));
        let capture_ok = Arc::new(Mutex::new(false));
        {
            let ring = ring.clone();
            let ok = capture_ok.clone();
            std::thread::spawn(move || capture(ring, ok));
        }
        let fft = FftPlanner::new().plan_fft_forward(N);
        let window = (0..N)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (N - 1) as f32).cos())
            .collect();
        Audio {
            ring,
            capture_ok,
            fft,
            window,
            force_synthetic: false,
            frame: AudioFrame::default(),
            bass_avg: 0.1,
            gain: 1.0,
            beat_cooldown: 0.0,
        }
    }

    pub fn is_live(&self) -> bool {
        if self.force_synthetic || !*self.capture_ok.lock().unwrap() {
            return false;
        }
        let ring = self.ring.lock().unwrap();
        ring.last_loud.is_some_and(|t| t.elapsed().as_secs_f32() < 1.5)
    }

    pub fn update(&mut self, t: f32, dt: f32, playing: bool) -> &AudioFrame {
        let live = self.is_live();
        let (mut l, mut r) = (vec![0.0f32; N], vec![0.0f32; N]);
        if live {
            let ring = self.ring.lock().unwrap();
            let len = ring.l.len();
            for i in 0..N {
                let idx = (ring.pos + len - N + i) % len;
                l[i] = ring.l[idx];
                r[i] = ring.r[idx];
            }
        } else if playing {
            synth(t, &mut l, &mut r);
        }

        // spectrum
        let mut buf: Vec<Complex<f32>> =
            (0..N).map(|i| Complex::new((l[i] + r[i]) * 0.5 * self.window[i], 0.0)).collect();
        self.fft.process(&mut buf);
        let mags: Vec<f32> = buf[..N / 2].iter().map(|c| c.norm() / N as f32).collect();

        let (fmin, fmax) = (35.0f32, 16000.0f32);
        let mut raw = [0.0f32; BANDS];
        for (b, slot) in raw.iter_mut().enumerate() {
            let f0 = fmin * (fmax / fmin).powf(b as f32 / BANDS as f32);
            let f1 = fmin * (fmax / fmin).powf((b + 1) as f32 / BANDS as f32);
            let i0 = ((f0 / RATE * N as f32) as usize).max(1);
            let i1 = ((f1 / RATE * N as f32) as usize).max(i0 + 1).min(N / 2);
            let m = mags[i0..i1].iter().cloned().fold(0.0, f32::max);
            // tilt: boost highs a little, they are naturally quieter
            let tilt = 1.0 + b as f32 / BANDS as f32 * 2.5;
            let db = 20.0 * (m * tilt * self.gain + 1e-9).log10();
            *slot = ((db + 62.0) / 52.0).clamp(0.0, 1.0);
        }
        // automatic gain: keep the loudest band around 0.9
        let maxb = raw.iter().cloned().fold(0.0, f32::max);
        if maxb > 0.97 {
            self.gain *= 0.97;
        } else if maxb < 0.7 && maxb > 0.05 {
            self.gain = (self.gain * 1.01).min(40.0);
        }

        let f = &mut self.frame;
        let fall = (1.0 - dt * 3.5).clamp(0.0, 1.0);
        for b in 0..BANDS {
            f.bands[b] = if raw[b] > f.bands[b] { f.bands[b] * 0.4 + raw[b] * 0.6 } else { f.bands[b] * fall };
            f.peaks[b] = if f.bands[b] > f.peaks[b] { f.bands[b] } else { (f.peaks[b] - dt * 0.35).max(0.0) };
        }
        let avg = |a: usize, b: usize| f.bands[a..b].iter().sum::<f32>() / (b - a) as f32;
        f.bass = avg(0, 8);
        f.mid = avg(8, 36);
        f.treble = avg(36, BANDS);
        f.rms = (l.iter().chain(r.iter()).map(|x| x * x).sum::<f32>() / (2 * N) as f32).sqrt();
        f.energy = f.energy * (1.0 - dt * 1.5) + (f.bass * 0.6 + f.mid * 0.4) * dt * 1.5;

        // beat detection on bass flux
        self.beat_cooldown -= dt;
        self.bass_avg = self.bass_avg * (1.0 - dt * 2.0) + f.bass * dt * 2.0;
        if f.bass > self.bass_avg * 1.18 && f.bass > 0.25 && self.beat_cooldown <= 0.0 {
            f.beat = 1.0;
            self.beat_cooldown = 0.22;
        } else {
            f.beat = (f.beat - dt * 4.0).max(0.0);
        }

        let step = N / 512;
        for i in 0..512 {
            f.wave_l[i] = l[N - 512 * step + i * step];
            f.wave_r[i] = r[N - 512 * step + i * step];
        }
        f.live = live;
        &self.frame
    }
}

fn capture(ring: Arc<Mutex<Ring>>, ok: Arc<Mutex<bool>>) {
    let child = Command::new("parec")
        .args([
            "--device=@DEFAULT_MONITOR@",
            "--format=float32le",
            "--rate=44100",
            "--channels=2",
            "--raw",
            "--latency-msec=15",
            "--client-name=singularity-demo",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return };
    let Some(mut out) = child.stdout.take() else { return };
    *ok.lock().unwrap() = true;
    let mut bytes = [0u8; 4096];
    loop {
        let Ok(n) = out.read(&mut bytes) else { break };
        if n == 0 {
            break;
        }
        let mut ring = ring.lock().unwrap();
        let len = ring.l.len();
        let mut loud = false;
        for frame in bytes[..n - n % 8].chunks_exact(8) {
            let l = f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
            let r = f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
            let p = ring.pos;
            ring.l[p] = l;
            ring.r[p] = r;
            ring.pos = (p + 1) % len;
            loud |= l.abs() > 0.002 || r.abs() > 0.002;
        }
        if loud {
            ring.last_loud = Some(Instant::now());
        }
    }
    *ok.lock().unwrap() = false;
    let _ = child.kill();
}

/// A little 118 bpm groove: kick, offbeat hats, bassline and a pad.
fn synth(t: f32, l: &mut [f32], r: &mut [f32]) {
    use std::f32::consts::TAU;
    let beat = 60.0 / 118.0;
    let bassline = [41.2, 41.2, 49.0, 36.7];
    let chords = [[220.0, 261.6, 329.6], [196.0, 246.9, 293.7], [174.6, 220.0, 261.6], [196.0, 246.9, 311.1]];
    for i in 0..l.len() {
        let ts = t - (l.len() - i) as f32 / RATE;
        let bar = ((ts / (beat * 4.0)).floor() as i64).rem_euclid(4) as usize;
        let tb = ts.rem_euclid(beat);
        let kick_f = 50.0 + 90.0 * (-tb * 30.0).exp();
        let kick = (TAU * kick_f * tb).sin() * (-tb * 7.0).exp() * 0.9;
        let th = (ts + beat * 0.5).rem_euclid(beat);
        let hat = (hash1((ts * RATE) as u32) * 2.0 - 1.0) * (-th * 60.0).exp() * 0.18;
        let bf = bassline[bar];
        let bass = ((ts * bf).fract() * 2.0 - 1.0) * 0.22 * (0.5 + 0.5 * (-tb * 3.0).exp());
        let mut pad = 0.0;
        for (k, f) in chords[bar].iter().enumerate() {
            pad += (TAU * f * ts + k as f32).sin() * 0.06;
        }
        let lfo = 0.5 + 0.5 * (ts * 0.7).sin();
        // arpeggiated lead, an octave up, panned slowly
        let step = (ts / (beat * 0.25)).floor() as i64;
        let ts16 = ts.rem_euclid(beat * 0.25);
        let note = chords[bar][(step.rem_euclid(3)) as usize] * 2.0;
        let lead = ((TAU * note * ts).sin() + 0.4 * (TAU * note * 3.0 * ts).sin() + 0.2 * (TAU * note * 5.0 * ts).sin())
            * (-ts16 * 9.0).exp() * 0.16;
        let pan = 0.5 + 0.45 * (ts * 0.9).sin();
        let shimmer = (hash1((ts * RATE) as u32 ^ 0x55) * 2.0 - 1.0) * 0.025 * (1.0 + (ts * 4.0).sin());
        l[i] = kick + hat * (1.0 - lfo * 0.5) + bass + pad * lfo + lead * (1.0 - pan) + shimmer;
        r[i] = kick + hat * lfo + bass + pad * (1.0 - lfo * 0.6) + lead * pan - shimmer;
    }
}

fn hash1(x: u32) -> f32 {
    crate::gfx::hash1(x)
}
