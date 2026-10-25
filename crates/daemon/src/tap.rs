//! AudioTap (spec D16, #15): listen to the Engine's own audio stream only, and turn it into spectrum frames.
//!
//! - *Source*: the Engine's PipeWire stream through `parec --monitor-stream` (Soloist's shows up as
//!   `application.name="Spotify"` without a process binary, the desktop app's with binary `spotify`), the librespot
//!   Engine's own samples handed over in-process ([`Feed`], same on every OS), or a synthetic groove for
//!   `--fake-engine` and tests.
//! - *Analysis* ([`Analyzer`]): 2048-point FFT every 1470 samples (30 Hz), 64 log-spaced bands, onset detection on
//!   bass flux, and automatic gain. The tap is post-volume, so the bands (for Visuals) are measured against the
//!   track's own recent loudness: the same music looks the same at volume 20 and 100. The `out_*` meters (#29) are
//!   the real output level instead, on a fixed scale, so they follow the volume.
//! - The tap runs only while at least one Client listens; frames stop once the audio has been silent for a second.

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use singularity_proto::SpectrumFrame;
use std::sync::Arc;

pub const BANDS: usize = 64;
pub const RATE: f32 = 44100.0;
const N: usize = 2048;
/// samples per frame: 30 frames a second
pub const HOP: usize = 1470;
const DT: f32 = HOP as f32 / RATE;
/// dynamic range shown, below the recent loudest band
const RANGE_DB: f32 = 48.0;
/// below this the input counts as silence (paused, between tracks)
const GATE_DB: f32 = -75.0;
/// the output meters' scale: this many dB below full scale up to full scale
const OUT_RANGE_DB: f32 = 60.0;
/// a full-scale sine's band magnitude in `frame` (Hann window, /N): bands are shown against it
const BAND_FULL_DB: f32 = -12.0;

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    mono: Vec<f32>,
    pos: usize,
    since_hop: usize,
    peak: [f32; 2],
    bands: [f32; BANDS],
    prev_raw: [f32; BANDS],
    /// automatic gain: a loudness envelope (dB) with fast attack and slow release; None = take the next sound as is
    env_db: Option<f32>,
    level: f32,
    vu: [f32; 2],
    out_level: f32,
    out_vu: [f32; 2],
    out_bands: [f32; 3],
    flux_avg: f32,
    cooldown: f32,
    quiet_frames: u32,
}

impl Default for Analyzer {
    fn default() -> Self {
        Analyzer::new()
    }
}

impl Analyzer {
    pub fn new() -> Analyzer {
        let window = (0..N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (N - 1) as f32).cos()).collect();
        Analyzer {
            fft: FftPlanner::new().plan_fft_forward(N),
            window,
            mono: vec![0.0; N],
            pos: 0,
            since_hop: 0,
            peak: [0.0; 2],
            bands: [0.0; BANDS],
            prev_raw: [0.0; BANDS],
            env_db: None,
            level: 0.0,
            vu: [0.0; 2],
            out_level: 0.0,
            out_vu: [0.0; 2],
            out_bands: [0.0; 3],
            flux_avg: 0.05,
            cooldown: 0.0,
            quiet_frames: u32::MAX,
        }
    }

    /// Forget the loudness envelope (the volume just changed): the next sound sets it afresh.
    pub fn reset_gain(&mut self) {
        self.env_db = None;
    }

    /// Has it gone quiet (the clearing frame sent), so silence needs no more feeding?
    pub fn is_quiet(&self) -> bool {
        self.quiet_frames > 0
    }

    /// One frame period of silence: while the Engine sends nothing (paused), the bars still fall and the clearing
    /// frame still goes out, instead of the last frame hanging in every Client.
    pub fn silence(&mut self) -> Vec<SpectrumFrame> {
        if self.is_quiet() { vec![] } else { self.push(&[0.0; HOP * 2]) }
    }

    /// Feed interleaved stereo samples; returns the frames completed meanwhile. A silent stretch yields one final
    /// all-zero frame and then nothing until sound returns.
    pub fn push(&mut self, stereo: &[f32]) -> Vec<SpectrumFrame> {
        let mut out = vec![];
        for s in stereo.as_chunks::<2>().0 {
            self.mono[self.pos] = (s[0] + s[1]) * 0.5;
            self.pos = (self.pos + 1) % N;
            self.peak[0] = self.peak[0].max(s[0].abs());
            self.peak[1] = self.peak[1].max(s[1].abs());
            self.since_hop += 1;
            if self.since_hop == HOP {
                self.since_hop = 0;
                if let Some(f) = self.frame() {
                    out.push(f);
                }
            }
        }
        out
    }

    fn frame(&mut self) -> Option<SpectrumFrame> {
        let mut buf: Vec<Complex<f32>> = (0..N).map(|i| Complex::new(self.mono[(self.pos + i) % N] * self.window[i], 0.0)).collect();
        let rms = (self.mono.iter().map(|x| x * x).sum::<f32>() / N as f32).sqrt();
        let silent = 20.0 * (rms + 1e-12).log10() < GATE_DB;
        self.fft.process(&mut buf);
        let mut db = [-120.0f32; BANDS];
        if !silent {
            let (fmin, fmax) = (35.0f32, 16000.0f32);
            for (b, slot) in db.iter_mut().enumerate() {
                let f0 = fmin * (fmax / fmin).powf(b as f32 / BANDS as f32);
                let f1 = fmin * (fmax / fmin).powf((b + 1) as f32 / BANDS as f32);
                let i0 = ((f0 / RATE * N as f32) as usize).max(1);
                let i1 = ((f1 / RATE * N as f32) as usize).max(i0 + 1).min(N / 2);
                let m = buf[i0..i1].iter().map(|c| c.norm()).fold(0.0, f32::max) / N as f32;
                // highs are naturally quieter: tilt them up a little (≈ +8 dB at the top)
                let tilt = 1.0 + b as f32 / BANDS as f32 * 1.5;
                *slot = 20.0 * (m * tilt + 1e-12).log10();
            }
            // automatic gain: follow the loudest band up at once, let go slowly (3 dB/s); silence holds it
            let max = db.iter().cloned().fold(f32::MIN, f32::max);
            self.env_db = Some(match self.env_db {
                None => max,
                Some(env) if max > env => env + (max - env) * 0.5,
                Some(env) => (env - 3.0 * DT).max(max),
            });
        }
        let env = self.env_db.unwrap_or(0.0);
        let rel = |d: f32, env: f32| ((d - (env - RANGE_DB)) / RANGE_DB).clamp(0.0, 1.0);
        let mut raw = [0.0f32; BANDS];
        for (r, d) in raw.iter_mut().zip(db.iter()) {
            // the loudest band lands a little below the top, so peaks never pin
            *r = if silent { 0.0 } else { rel(*d, env + 4.0) };
        }
        let fall = (1.0 - DT * 3.5).clamp(0.0, 1.0);
        for (band, r) in self.bands.iter_mut().zip(raw.iter()) {
            let old = *band;
            *band = if *r > old { old * 0.4 + *r * 0.6 } else { old * fall };
        }
        // loudness in a narrower 24 dB window, so a song's dynamics show: steady music sits near 70 % (the band
        // envelope runs ≈ 13 dB above the broadband rms)
        let lvl = if silent { 0.0 } else { ((20.0 * rms.log10() - env + 30.0) / 24.0).clamp(0.0, 1.0) };
        self.level = self.level * 0.7 + lvl * 0.3;
        for c in 0..2 {
            let p = if silent { 0.0 } else { rel(20.0 * (self.peak[c] + 1e-12).log10() + 3.0, env + 4.0) };
            self.vu[c] = if p > self.vu[c] { p } else { (self.vu[c] - DT * 1.2).max(p) };
        }
        // the real output level: no gain, a fixed scale
        let out = |d: f32| if silent { 0.0 } else { ((d + OUT_RANGE_DB) / OUT_RANGE_DB).clamp(0.0, 1.0) };
        self.out_level = self.out_level * 0.7 + out(20.0 * (rms + 1e-12).log10()) * 0.3;
        for c in 0..2 {
            let p = out(20.0 * (self.peak[c] + 1e-12).log10());
            self.out_vu[c] = if p > self.out_vu[c] { p } else { (self.out_vu[c] - DT * 1.2).max(p) };
        }
        for (g, (a, b)) in [(0, BANDS / 6), (BANDS / 6, BANDS / 2), (BANDS / 2, BANDS)].into_iter().enumerate() {
            let mean = db[a..b].iter().sum::<f32>() / (b - a) as f32;
            let v = out(mean - BAND_FULL_DB);
            self.out_bands[g] = if v > self.out_bands[g] { self.out_bands[g] * 0.4 + v * 0.6 } else { self.out_bands[g] * fall };
        }
        self.peak = [0.0; 2];
        // onsets: positive spectral flux in the bass, against its own running average
        let flux: f32 = (0..10).map(|b| (raw[b] - self.prev_raw[b]).max(0.0)).sum::<f32>() / 10.0;
        self.prev_raw = raw;
        self.cooldown -= DT;
        let beat = !silent && flux > self.flux_avg * 2.2 + 0.03 && self.cooldown <= 0.0;
        if beat {
            // the kick, not its hats: at most ~3 onsets a second
            self.cooldown = 0.3;
        }
        self.flux_avg = self.flux_avg * 0.95 + flux * 0.05;

        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let frame = SpectrumFrame {
            bands: self.bands.iter().map(|v| q(*v)).collect(),
            level: q(self.level),
            vu: [q(self.vu[0]), q(self.vu[1])],
            beat,
            out_level: q(self.out_level),
            out_vu: [q(self.out_vu[0]), q(self.out_vu[1])],
            out_bands: self.out_bands.map(q),
        };
        let quiet = frame.bands.iter().all(|b| *b == 0) && frame.level == 0 && frame.vu == [0, 0] && frame.out_vu == [0, 0] && frame.out_bands == [0; 3];
        if quiet {
            self.quiet_frames = self.quiet_frames.saturating_add(1);
            // one zero frame tells Clients to clear; after that, silence costs nothing
            return (self.quiet_frames == 1).then_some(frame);
        }
        self.quiet_frames = 0;
        Some(frame)
    }
}

/// Which PipeWire stream is the Engine's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamOf {
    /// Soloist: `application.name="Spotify"` without a process binary (or binary `soloist`)
    Soloist,
    /// the official desktop app (D51 on Linux): binary `spotify`
    SpotifyApp,
    /// the librespot Engine plays from this Daemon (for the health view; its tap is the [`Feed`])
    Daemon,
}

/// The Engine's stream in `pactl -f json list sink-inputs`: the newest one that matches.
pub fn find_engine_stream(sink_inputs: &serde_json::Value, of: StreamOf) -> Option<u64> {
    sink_inputs
        .as_array()?
        .iter()
        .filter(|si| {
            let p = &si["properties"];
            let binary = p["application.process.binary"].as_str();
            match of {
                StreamOf::Soloist => p["application.name"].as_str() == Some("Spotify") && binary.is_none_or(|b| b == "soloist"),
                StreamOf::SpotifyApp => binary == Some("spotify"),
                StreamOf::Daemon => binary == Some("singularityd"),
            }
        })
        .filter_map(|si| si["index"].as_u64())
        .max()
}

/// Samples handed over in-process by an Engine that plays from this Daemon (librespot): it pushes interleaved
/// stereo at 44.1 kHz while [`Feed::wanted`], the tap takes them. Nothing is copied while nobody listens, and a tap
/// that falls behind drops packets instead of holding up playback.
pub struct Feed {
    wanted: std::sync::atomic::AtomicBool,
    tx: tokio::sync::mpsc::Sender<Vec<f32>>,
    rx: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Vec<f32>>>,
}

impl Feed {
    pub fn new() -> Arc<Feed> {
        // ~1.5 s of librespot's packets
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        Arc::new(Feed { wanted: Default::default(), tx, rx: tokio::sync::Mutex::new(rx) })
    }

    /// Does the tap want samples now?
    pub fn wanted(&self) -> bool {
        self.wanted.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Hand over samples (from the player's thread; never blocks).
    pub fn push(&self, samples: Vec<f32>) {
        let _ = self.tx.try_send(samples);
    }
}

/// Interleaved stereo at any rate → 44.1 kHz (linear: the analysis only needs bands, not hi-fi), kept across calls.
#[derive(Default)]
pub struct Resampler {
    /// where the next output sample falls, in input frames, relative to the frame `last` holds
    pos: f64,
    last: [f32; 2],
}

impl Resampler {
    pub fn push(&mut self, input: &[f32], rate: f64, out: &mut Vec<f32>) {
        if (rate - RATE as f64).abs() < 1.0 {
            out.extend_from_slice(input);
            return;
        }
        let step = rate / RATE as f64;
        let frames = input.len() / 2;
        // frame -1 is the last one of the previous call
        let at = |i: isize| if i < 0 { self.last } else { [input[i as usize * 2], input[i as usize * 2 + 1]] };
        while self.pos < frames as f64 - 1.0 {
            let i = self.pos.floor() as isize;
            let t = (self.pos - i as f64) as f32;
            let (a, b) = (at(i), at(i + 1));
            out.push(a[0] + (b[0] - a[0]) * t);
            out.push(a[1] + (b[1] - a[1]) * t);
            self.pos += step;
        }
        if frames > 0 {
            self.last = at(frames as isize - 1);
            self.pos -= frames as f64;
        }
    }
}

impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feed").field("wanted", &self.wanted()).finish()
    }
}

/// A 118 bpm groove (kick, hats, bass, pad, arpeggio) for `--fake-engine`: `frames` stereo samples from sample `t0`.
pub fn synth(t0: u64, frames: usize) -> Vec<f32> {
    use std::f32::consts::TAU;
    let hash = |x: u32| {
        let mut h = x.wrapping_mul(374761393) ^ 7919u32.wrapping_mul(668265263);
        h = (h ^ (h >> 13)).wrapping_mul(1274126177);
        ((h ^ (h >> 16)) & 0xffffff) as f32 / 16777215.0
    };
    let beat = 60.0 / 118.0;
    let bassline = [41.2, 41.2, 49.0, 36.7];
    let chords = [[220.0, 261.6, 329.6], [196.0, 246.9, 293.7], [174.6, 220.0, 261.6], [196.0, 246.9, 311.1]];
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let n = t0 + i as u64;
        let ts = n as f32 / RATE;
        let bar = ((ts / (beat * 4.0)).floor() as i64).rem_euclid(4) as usize;
        let tb = ts.rem_euclid(beat);
        let kick = (TAU * (50.0 + 90.0 * (-tb * 30.0).exp()) * tb).sin() * (-tb * 7.0).exp() * 0.9;
        let th = (ts + beat * 0.5).rem_euclid(beat);
        let hat = (hash(n as u32) * 2.0 - 1.0) * (-th * 60.0).exp() * 0.18;
        let bass = ((ts * bassline[bar]).fract() * 2.0 - 1.0) * 0.22 * (0.5 + 0.5 * (-tb * 3.0).exp());
        let pad: f32 = chords[bar].iter().enumerate().map(|(k, f)| (TAU * f * ts + k as f32).sin() * 0.06).sum();
        let ts16 = ts.rem_euclid(beat * 0.25);
        let note = chords[bar][((ts / (beat * 0.25)).floor() as i64).rem_euclid(3) as usize] * 2.0;
        let lead = ((TAU * note * ts).sin() + 0.4 * (TAU * note * 3.0 * ts).sin()) * (-ts16 * 9.0).exp() * 0.16;
        let pan = 0.5 + 0.45 * (ts * 0.9).sin();
        out.push(kick + hat + bass + pad + lead * (1.0 - pan));
        out.push(kick + hat + bass + pad + lead * pan);
    }
    out
}

/// Where the tap's audio comes from.
#[derive(Clone, Debug)]
pub enum TapSource {
    /// the Engine's PipeWire stream (Linux)
    PipeWire(StreamOf),
    /// samples from an Engine inside this Daemon (librespot, every OS)
    Feed(Arc<Feed>),
    /// macOS: a Core Audio process tap on the Spotify app (#67), delivered like a Feed
    #[cfg(target_os = "macos")]
    App(Arc<Feed>),
    /// the built-in groove, while the (fake) Engine plays
    Synthetic,
}

/// Run the tap for the Daemon: idle until a Client listens, then analyse and broadcast frames.
pub async fn run(hub: Arc<crate::Hub>, source: TapSource) {
    // the Spotify app's tap feeds a Feed of its own while it's wanted
    #[cfg(target_os = "macos")]
    let source = match source {
        TapSource::App(feed) => {
            tokio::spawn(crate::process_tap::manage(hub.clone(), feed.clone()));
            TapSource::Feed(feed)
        }
        s => s,
    };
    use std::sync::atomic::Ordering;
    let mut an = Analyzer::new();
    let mut volume = None;
    let listening = |hub: &crate::Hub| hub.spectrum_listeners.load(Ordering::SeqCst) > 0;
    loop {
        while !listening(&hub) {
            hub.spectrum_wake.notified().await;
        }
        let pb = hub.playback.lock().await.clone();
        if volume.is_some_and(|v| v != pb.volume) {
            an.reset_gain();
        }
        volume = Some(pb.volume);
        let playing = pb.status == singularity_proto::PlayStatus::Playing;
        match &source {
            TapSource::Feed(feed) => {
                feed.wanted.store(true, Ordering::Relaxed);
                let mut rx = feed.rx.lock().await;
                while listening(&hub) {
                    // paused: no packets. Let the bars fall a frame period at a time until they're down, then only
                    // look at the listeners again every second
                    let wait = if an.is_quiet() { std::time::Duration::from_secs(1) } else { std::time::Duration::from_secs_f32(DT) };
                    let Ok(Some(samples)) = tokio::time::timeout(wait, rx.recv()).await else {
                        for f in an.silence() {
                            let _ = hub.spectrum.send(f);
                        }
                        continue;
                    };
                    let v = hub.playback.lock().await.volume;
                    if volume.is_some_and(|old| old != v) {
                        an.reset_gain();
                    }
                    volume = Some(v);
                    for f in an.push(&samples) {
                        let _ = hub.spectrum.send(f);
                    }
                }
                feed.wanted.store(false, Ordering::Relaxed);
                // what came in meanwhile is stale by the next listen
                while rx.try_recv().is_ok() {}
            }
            #[cfg(target_os = "macos")]
            TapSource::App(_) => unreachable!("turned into a Feed when the tap starts"),
            TapSource::Synthetic => {
                // one frame's worth of audio per frame period
                let mut t = 0u64;
                let mut tick = tokio::time::interval(std::time::Duration::from_secs_f32(DT));
                while listening(&hub) {
                    tick.tick().await;
                    let pb = hub.playback.lock().await.clone();
                    if volume.is_some_and(|v| v != pb.volume) {
                        an.reset_gain();
                    }
                    volume = Some(pb.volume);
                    // after the volume, as Soloist's stream is (a square-law curve)
                    let gain = (pb.volume as f32 / 100.0).powi(2);
                    let samples = if pb.status == singularity_proto::PlayStatus::Playing { synth(t, HOP).into_iter().map(|x| x * gain).collect() } else { vec![0.0; HOP * 2] };
                    t += HOP as u64;
                    for f in an.push(&samples) {
                        let _ = hub.spectrum.send(f);
                    }
                }
            }
            TapSource::PipeWire(_) if !playing => {
                // not playing: let the bars fall, then wait
                let frames = an.silence();
                for f in frames.iter().cloned() {
                    let _ = hub.spectrum.send(f);
                }
                tokio::time::sleep(if frames.is_empty() { std::time::Duration::from_secs(1) } else { std::time::Duration::from_secs_f32(DT) }).await;
            }
            TapSource::PipeWire(of) => {
                if let Err(e) = capture(&hub, &mut an, *of).await {
                    tracing::debug!("audio tap: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }
}

/// Capture the Engine's stream until it ends or nobody listens any more.
async fn capture(hub: &crate::Hub, an: &mut Analyzer, of: StreamOf) -> anyhow::Result<()> {
    use std::sync::atomic::Ordering;
    use tokio::io::AsyncReadExt;
    // (no child inherits the Daemon's secret: audit S10)
    let out = crate::programs::tokio_command(crate::programs::Tool::Pactl.path()?).args(["-f", "json", "list", "sink-inputs"]).output().await?;
    let idx = find_engine_stream(&serde_json::from_slice(&out.stdout)?, of).ok_or_else(|| anyhow::anyhow!("no Engine stream in PipeWire yet"))?;
    tracing::info!("audio tap on PipeWire stream #{idx}");
    let mut child = crate::programs::tokio_command(crate::programs::Tool::Parec.path()?)
        .args([&format!("--monitor-stream={idx}"), "--format=float32le", "--rate=44100", "--channels=2", "--raw", "--latency-msec=30", "--client-name=singularity-tap"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("parec has no stdout"))?;
    let mut bytes = vec![0u8; HOP * 8];
    let mut have = 0;
    let mut volume = hub.playback.lock().await.volume;
    while hub.spectrum_listeners.load(Ordering::SeqCst) > 0 {
        let wait = if an.is_quiet() { std::time::Duration::from_secs(1) } else { std::time::Duration::from_secs_f32(DT) };
        let n = match tokio::time::timeout(wait, stdout.read(&mut bytes[have..])).await {
            Ok(r) => r?,
            // paused: no data, but keep the stream open (and let the bars fall)
            Err(_) => {
                for f in an.silence() {
                    let _ = hub.spectrum.send(f);
                }
                continue;
            }
        };
        if n == 0 {
            anyhow::bail!("the Engine stream ended");
        }
        have += n;
        let whole = have - have % 8;
        let samples: Vec<f32> = bytes[..whole].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        bytes.copy_within(whole..have, 0);
        have -= whole;
        let v = hub.playback.lock().await.volume;
        if v != volume {
            volume = v;
            an.reset_gain();
        }
        for f in an.push(&samples) {
            let _ = hub.spectrum.send(f);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, amp: f32, secs: f32) -> Vec<f32> {
        (0..(RATE * secs) as usize).flat_map(|i| [(std::f32::consts::TAU * freq * i as f32 / RATE).sin() * amp; 2]).collect()
    }

    fn run(a: &mut Analyzer, s: &[f32]) -> Vec<SpectrumFrame> {
        s.chunks(4096).flat_map(|c| a.push(c)).collect()
    }

    #[test]
    fn thirty_frames_a_second_and_a_sine_lights_its_band() {
        let mut a = Analyzer::new();
        let frames = run(&mut a, &sine(1000.0, 0.3, 2.0));
        assert_eq!(frames.len(), 60);
        let last = frames.last().unwrap();
        let top = (0..BANDS).max_by_key(|&b| last.bands[b]).unwrap();
        let f0 = 35.0f32 * (16000.0f32 / 35.0).powf(top as f32 / BANDS as f32);
        let f1 = 35.0f32 * (16000.0f32 / 35.0).powf((top + 1) as f32 / BANDS as f32);
        assert!(f0 <= 1100.0 && f1 >= 900.0, "1 kHz lands in band {top} ({f0:.0}–{f1:.0} Hz)");
        assert!(last.bands[top] > 200 && last.bands[top] < 255, "loud but never pinned: {}", last.bands[top]);
    }

    #[test]
    fn automatic_gain_makes_volume_20_look_like_volume_100() {
        let music = synth(0, (RATE * 6.0) as usize);
        let quiet: Vec<f32> = music.iter().map(|s| s * 0.04).collect(); // ≈ volume 20 on a cubic curve
        let (mut a, mut b) = (Analyzer::new(), Analyzer::new());
        let fa = run(&mut a, &music);
        let fb = run(&mut b, &quiet);
        // after a second of settling, frame by frame the bands differ only a little
        let diff: f32 = fa.iter().zip(&fb).skip(30).map(|(x, y)| x.bands.iter().zip(&y.bands).map(|(p, q)| (*p as f32 - *q as f32).abs()).sum::<f32>() / BANDS as f32).sum::<f32>()
            / (fa.len() - 30) as f32;
        assert!(diff < 8.0, "mean band difference {diff:.1}/255");
        let la: f32 = fa.iter().skip(30).map(|f| f.level as f32).sum::<f32>() / (fa.len() - 30) as f32;
        let lb: f32 = fb.iter().skip(30).map(|f| f.level as f32).sum::<f32>() / (fb.len() - 30) as f32;
        assert!((la - lb).abs() < 10.0 && la > 40.0, "levels {la:.0} vs {lb:.0}");
    }

    #[test]
    fn the_output_meters_follow_the_volume() {
        let music = synth(0, (RATE * 4.0) as usize);
        let mean = |fs: &[SpectrumFrame], f: fn(&SpectrumFrame) -> f32| fs.iter().skip(30).map(f).sum::<f32>() / (fs.len() - 30) as f32;
        let at = |gain: f32| {
            let s: Vec<f32> = music.iter().map(|x| x * gain).collect();
            let fs = run(&mut Analyzer::new(), &s);
            (mean(&fs, |f| f.out_level as f32), mean(&fs, |f| f.out_vu[0] as f32), mean(&fs, |f| f.out_bands[1] as f32), mean(&fs, |f| f.level as f32))
        };
        let (full, tenth, hundredth) = (at(1.0), at(0.1), at(0.01));
        // 20 dB quieter is a third of the 60 dB scale lower (255 / 3 = 85), on every output meter
        for (a, b, name) in [(full.0, tenth.0, "level"), (full.1, tenth.1, "vu"), (full.2, tenth.2, "mid")] {
            assert!((a - b - 85.0).abs() < 12.0, "{name}: {a:.0} at full, {b:.0} at -20 dB");
        }
        assert!(full.1 > 200.0, "full scale peaks near the top: {:.0}", full.1);
        assert!(hundredth.1 < 90.0, "-40 dB: low: {:.0}", hundredth.1);
        // while the gained loudness (Visuals) stays put
        assert!((full.3 - hundredth.3).abs() < 12.0, "gained level {:.0} vs {:.0}", full.3, hundredth.3);
    }

    #[test]
    fn beats_are_found_at_the_kick() {
        let mut a = Analyzer::new();
        let frames = run(&mut a, &synth(0, (RATE * 8.0) as usize));
        let beats = frames.iter().filter(|f| f.beat).count();
        // 118 bpm over 8 s ≈ 15.7 kicks
        assert!((12..=19).contains(&beats), "{beats} beats");
    }

    #[test]
    fn silence_sends_one_clearing_frame_then_nothing() {
        let mut a = Analyzer::new();
        run(&mut a, &sine(200.0, 0.3, 1.0));
        let frames = run(&mut a, &vec![0.0; (RATE * 2.0) as usize * 2]);
        let zeros = frames.iter().filter(|f| f.bands.iter().all(|b| *b == 0)).count();
        assert_eq!(zeros, 1);
        assert!(frames.len() < 55 && frames.last().unwrap().bands.iter().all(|b| *b == 0), "bars fall within ~1.5 s, then frames stop");
        assert!(!run(&mut a, &sine(200.0, 0.3, 0.2)).is_empty(), "and come back with the sound");
    }

    #[test]
    fn when_the_samples_stop_the_bars_still_fall_and_clear() {
        let mut a = Analyzer::new();
        run(&mut a, &sine(200.0, 0.3, 1.0));
        assert!(!a.is_quiet());
        // paused: no samples at all, a frame period of silence at a time
        let mut frames = vec![];
        for _ in 0..90 {
            frames.extend(a.silence());
        }
        assert!(a.is_quiet(), "down within 3 s");
        assert!(frames.last().unwrap().bands.iter().all(|b| *b == 0), "the last frame clears every Client");
        assert!(a.silence().is_empty(), "then silence costs nothing");
    }

    #[test]
    fn the_resampler_turns_48_khz_into_44_1_and_keeps_the_tone() {
        let mut r = Resampler::default();
        let mut out = vec![];
        // 2 s of a 1 kHz sine at 48 kHz, in uneven chunks as Core Audio hands them over
        let input: Vec<f32> = (0..96_000).flat_map(|i| [(std::f32::consts::TAU * 1000.0 * i as f32 / 48_000.0).sin() * 0.3; 2]).collect();
        for c in input.chunks(1_234 * 2) {
            r.push(c, 48_000.0, &mut out);
        }
        assert!((out.len() as i64 - 2 * 88_200).abs() < 8, "2 s at 44.1 kHz: {}", out.len() / 2);
        let frames = Analyzer::new().push(&out);
        let last = frames.last().unwrap();
        let top = (0..BANDS).max_by_key(|&b| last.bands[b]).unwrap();
        let f0 = 35.0f32 * (16000.0f32 / 35.0).powf(top as f32 / BANDS as f32);
        let f1 = 35.0f32 * (16000.0f32 / 35.0).powf((top + 1) as f32 / BANDS as f32);
        assert!(f0 <= 1100.0 && f1 >= 900.0, "still 1 kHz: band {top} ({f0:.0}–{f1:.0} Hz)");
    }

    #[test]
    fn picks_soloist_not_the_desktop_app() {
        let v = serde_json::json!([
            {"index": 5, "properties": {"application.name": "Spotify", "application.process.binary": "spotify"}},
            {"index": 9, "properties": {"application.name": "Spotify"}},
            {"index": 7, "properties": {"application.name": "Spotify"}},
            {"index": 11, "properties": {"application.name": "mpv"}}
        ]);
        assert_eq!(find_engine_stream(&v, StreamOf::Soloist), Some(9));
        assert_eq!(find_engine_stream(&v, StreamOf::SpotifyApp), Some(5), "the desktop app as Engine (D51 on Linux)");
        assert_eq!(find_engine_stream(&v, StreamOf::Daemon), None);
        assert_eq!(find_engine_stream(&serde_json::json!([]), StreamOf::Soloist), None);
    }

    #[tokio::test]
    async fn a_feed_takes_samples_only_while_wanted_and_never_blocks() {
        let feed = Feed::new();
        assert!(!feed.wanted());
        feed.wanted.store(true, std::sync::atomic::Ordering::Relaxed);
        // far more than it holds: the extra packets are dropped, push returns at once
        for _ in 0..1000 {
            feed.push(vec![0.5; 8]);
        }
        let mut rx = feed.rx.lock().await;
        let mut n = 0;
        while rx.try_recv().is_ok() {
            n += 1;
        }
        assert_eq!(n, 64);
    }

    /// Budget check (spec: benchmarks, not unit tests): `cargo test --release -p singularityd tap_cpu -- --ignored`
    #[test]
    #[ignore]
    fn tap_cpu_budget() {
        let audio = synth(0, (RATE * 30.0) as usize);
        let mut a = Analyzer::new();
        let t = std::time::Instant::now();
        run(&mut a, &audio);
        let share = t.elapsed().as_secs_f32() / 30.0;
        println!("analysis: {:.3}% of a core", share * 100.0);
        assert!(share < 0.02);
    }
}
