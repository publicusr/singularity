//! Audio from the Soloist VM (D55): raw float32 stereo 44.1 kHz over TCP from the guest's PCM server, played on the
//! Mac's output through cpal and handed to the AudioTap in-process (`tap::Feed`), as librespot's is.
//!
//! - A jitter buffer: playing starts once ~0.3 s is buffered, and again after running dry; past 2 s behind (the two
//!   clocks drift) it skips back to 0.3 s. Without it the network's bursts stutter.
//! - Its own thread (cpal's stream stays on the thread that made it); reconnects every second until stopped.

use crate::tap::Feed;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// ~0.3 s of interleaved stereo samples
const START: usize = 26_460;
/// ~2 s
const MAX: usize = 176_400;

/// Playing, until dropped.
pub struct Player {
    stop: Arc<AtomicBool>,
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Play what `addr` sends; `feed` gets the same samples while the tap wants them.
pub fn play(addr: String, feed: Arc<Feed>) -> Player {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    std::thread::Builder::new()
        .name("vm-audio".into())
        .spawn(move || {
            if let Err(e) = run(&addr, &feed, &s) {
                tracing::warn!("Soloist VM audio: {e}");
            }
        })
        .expect("a thread for the VM's audio");
    Player { stop }
}

/// The jitter buffer: samples, and whether it's playing (else filling).
#[derive(Default)]
pub struct Jitter {
    q: VecDeque<f32>,
    playing: bool,
}

impl Jitter {
    pub fn fill(&mut self, samples: impl IntoIterator<Item = f32>) {
        self.q.extend(samples);
        if self.q.len() > MAX {
            let extra = self.q.len() - START;
            self.q.drain(..extra);
        }
    }

    pub fn take(&mut self, out: &mut [f32]) {
        if !self.playing && self.q.len() >= START {
            self.playing = true;
        }
        for s in out.iter_mut() {
            *s = if self.playing { self.q.pop_front().unwrap_or(0.0) } else { 0.0 };
        }
        if self.q.is_empty() {
            self.playing = false;
        }
    }
}

fn run(addr: &str, feed: &Feed, stop: &AtomicBool) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::io::Read;
    let buf: Arc<Mutex<Jitter>> = Default::default();
    let dev = cpal::default_host().default_output_device().ok_or_else(|| anyhow::anyhow!("no audio output device"))?;
    let cfg = cpal::StreamConfig { channels: 2, sample_rate: cpal::SampleRate(44_100), buffer_size: cpal::BufferSize::Default };
    let b = buf.clone();
    let stream = dev.build_output_stream(&cfg, move |out: &mut [f32], _| b.lock().expect("jitter lock").take(out), |e| tracing::warn!("Soloist VM audio output: {e}"), None)?;
    stream.play()?;
    while !stop.load(Ordering::Relaxed) {
        let Ok(mut s) = std::net::TcpStream::connect(addr) else {
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        };
        let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(1)));
        let mut raw = vec![0u8; 16_384];
        let mut have = 0usize;
        while !stop.load(Ordering::Relaxed) {
            match s.read(&mut raw[have..]) {
                Ok(0) => break,
                Ok(n) => {
                    have += n;
                    let whole = have - have % 8;
                    let samples: Vec<f32> = raw[..whole].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                    if feed.wanted() {
                        feed.push(samples.clone());
                    }
                    buf.lock().expect("jitter lock").fill(samples);
                    raw.copy_within(whole..have, 0);
                    have -= whole;
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_jitter_buffer_fills_before_playing_and_catches_up_when_far_behind() {
        let mut j = Jitter::default();
        let mut out = vec![1.0; 512];
        j.fill(vec![0.5; START - 1]);
        j.take(&mut out);
        assert!(out.iter().all(|s| *s == 0.0), "still filling");
        j.fill(vec![0.5; 10]);
        j.take(&mut out);
        assert!(out.iter().all(|s| *s == 0.5), "playing");
        j.fill(vec![0.25; MAX * 2]);
        assert_eq!(j.q.len(), START, "too far behind: back to 0.3 s");
        let mut all = vec![0.0; START + 100];
        j.take(&mut all);
        assert!(!j.playing, "ran dry: fills again");
    }
}
