//! Core Audio process tap on its own (macOS, #67): `tap_probe <pid>…` taps those processes for 5 s and prints what
//! came through (rate, samples, loudness). Read-only: the tapped audio still plays as before.
#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use std::sync::{Arc, Mutex};
    let pids: Vec<i32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let objects = singularityd::process_tap::objects_of(&pids);
    println!("audio process objects for {pids:?}: {objects:?}");
    let stats = Arc::new(Mutex::new((0usize, 0f64, 0f64)));
    let s = stats.clone();
    let tap = singularityd::process_tap::ProcessTap::start(&objects, Box::new(move |x, rate| {
        let mut g = s.lock().unwrap();
        g.0 += x.len();
        g.1 += x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>();
        g.2 = rate;
    }))?;
    std::thread::sleep(std::time::Duration::from_secs(5));
    let (n, sq, rate) = *stats.lock().unwrap();
    println!("rate {rate} Hz, {n} samples in 5 s ({:.0}/s), rms {:.4}, heard: {}", n as f64 / 5.0, (sq / n.max(1) as f64).sqrt(), tap.heard());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {}
