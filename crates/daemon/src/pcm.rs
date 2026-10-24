//! Audio from the Soloist VM (D55): raw float32 stereo 44.1 kHz from the guest's pipe sink over virtio-vsock (the
//! guest connects; the Daemon listens on a Unix socket vfkit bridges to), played on the Mac's output through cpal and
//! handed to the AudioTap in-process (`tap::Feed`), as librespot's is.
//!
//! - A jitter buffer: playing starts once ~0.3 s is buffered, and again after running dry; past 2 s behind (the two
//!   clocks drift) it skips back to 0.3 s. Without it the network's bursts stutter.
//! - Nothing comes while nothing plays (the guest's sink sleeps): after 3 s without data the output stream is paused
//!   too, so an idle VM costs the Mac's audio nothing; the first data plays it again.
//! - Its own thread (cpal's stream stays on the thread that made it); takes the guest's next connection when one ends.

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

/// Listen on `socket` and play what the guest sends there; `feed` gets the same samples while the tap wants them.
pub fn listen(socket: std::path::PathBuf, feed: Arc<Feed>) -> anyhow::Result<Player> {
    if let Some(dir) = socket.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(&socket);
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    std::thread::Builder::new()
        .name("vm-audio".into())
        .spawn(move || {
            if let Err(e) = run(listener, &feed, &s) {
                tracing::warn!("Soloist VM audio: {e}");
            }
            let _ = std::fs::remove_file(&socket);
        })
        .expect("a thread for the VM's audio");
    Ok(Player { stop })
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

fn run(listener: std::os::unix::net::UnixListener, feed: &Feed, stop: &AtomicBool) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::io::Read;
    use std::time::{Duration, Instant};
    let buf: Arc<Mutex<Jitter>> = Default::default();
    let dev = cpal::default_host().default_output_device().ok_or_else(|| anyhow::anyhow!("no audio output device"))?;
    let cfg = cpal::StreamConfig { channels: 2, sample_rate: cpal::SampleRate(44_100), buffer_size: cpal::BufferSize::Default };
    let b = buf.clone();
    let stream = dev.build_output_stream(&cfg, move |out: &mut [f32], _| b.lock().expect("jitter lock").take(out), |e| tracing::warn!("Soloist VM audio output: {e}"), None)?;
    let _ = stream.pause();
    let mut playing = false;
    let mut last_data = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let mut s = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        tracing::info!("Soloist VM audio: connected");
        s.set_nonblocking(false)?;
        let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
        let mut raw = vec![0u8; 16_384];
        let mut have = 0usize;
        while !stop.load(Ordering::Relaxed) {
            match s.read(&mut raw[have..]) {
                Ok(0) => break,
                Ok(n) => {
                    last_data = Instant::now();
                    if !playing {
                        playing = stream.play().is_ok();
                    }
                    have += n;
                    let whole = have - have % 8;
                    let samples: Vec<f32> = raw[..whole].as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
                    if feed.wanted() {
                        feed.push(samples.clone());
                    }
                    buf.lock().expect("jitter lock").fill(samples);
                    raw.copy_within(whole..have, 0);
                    have -= whole;
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    // the guest's sink sleeps while nothing plays: let the output sleep too
                    if playing && last_data.elapsed() > Duration::from_secs(3) {
                        let _ = stream.pause();
                        playing = false;
                    }
                }
                Err(_) => break,
            }
        }
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
