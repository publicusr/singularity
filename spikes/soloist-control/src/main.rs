//! Phase 1 spike (a + c): run Spotify Soloist, pair it, control it over its local WebSocket, and tap
//! its audio stream. Throwaway code. Its job is to turn unknowns into facts (see FINDINGS.md).
//!
//! Needs a Soloist API key, from the environment (SOLOIST_API_KEY) or the system keyring:
//!   secret-tool store --label="Singularity: Soloist API key" service singularity account soloist

use rustfft::{FftPlanner, num_complex::Complex};
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

const CDN: &str = "https://soloist-builds.spotifycdn.com/soloist_release_x86_64.tar.gz";
const DEVICE: &str = "Singularity (spike)";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn say(s: &str) {
    println!("\x1b[38;5;180m▸\x1b[0m {s}");
}

fn api_key() -> Option<String> {
    if let Ok(k) = std::env::var("SOLOIST_API_KEY") {
        if !k.trim().is_empty() {
            return Some(k.trim().to_string());
        }
    }
    let out = Command::new("secret-tool").args(["lookup", "service", "singularity", "account", "soloist"]).output().ok()?;
    let k = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!k.is_empty()).then_some(k)
}

/// Download Soloist from Spotify's CDN into ./bin (never committed, never redistributed).
fn ensure_binary() -> std::io::Result<PathBuf> {
    let bin = root().join("bin");
    let exe = bin.join("soloist");
    if exe.exists() {
        return Ok(exe);
    }
    std::fs::create_dir_all(&bin)?;
    say("Downloading Spotify Soloist from Spotify's CDN…");
    let tgz = bin.join("soloist.tar.gz");
    let ok = Command::new("curl").args(["--fail", "--location", "--silent", "--show-error", "-o"]).arg(&tgz).arg(CDN).status()?.success()
        && Command::new("tar").arg("-xzf").arg(&tgz).arg("-C").arg(&bin).status()?.success();
    let _ = std::fs::remove_file(&tgz);
    if !ok || !exe.exists() {
        return Err(std::io::Error::other("download or extraction failed"));
    }
    Ok(exe)
}

fn findings(line: &str) {
    let p = root().join("FINDINGS.md");
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap();
    let _ = writeln!(f, "{line}");
}

fn start(exe: &Path, key: &str, state: &Path) -> std::io::Result<Child> {
    std::fs::create_dir_all(state.join("cache"))?;
    let log = std::fs::File::create(state.join("soloist.log"))?;
    let _ = std::fs::remove_file(state.join("data/ws.port"));
    let mut cmd = Command::new(exe);
    if std::env::args().any(|a| a == "--verbose") {
        cmd.arg("--verbose");
    }
    cmd.args(["--device-name", DEVICE, "--api-key", key, "--ws", "127.0.0.1:0"])
        .arg("--data-dir")
        .arg(state.join("data"))
        .arg("--cache-dir")
        .arg(state.join("cache"))
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
}

fn connect(state: &Path, child: &mut Child) -> Option<WebSocket<MaybeTlsStream<TcpStream>>> {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(20) {
        if let Ok(Some(st)) = child.try_wait() {
            say(&format!("Soloist exited early ({st}). Last log lines:"));
            let log = std::fs::read_to_string(state.join("soloist.log")).unwrap_or_default();
            for l in log.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
                println!("    {l}");
            }
            return None;
        }
        let port = std::fs::read_to_string(state.join("data/ws.port")).ok();
        let addr = std::fs::read_to_string(state.join("data/ws.addr")).ok();
        if let (Some(p), Some(a)) = (port, addr) {
            if let Ok((ws, _)) = tungstenite::connect(format!("ws://{}:{}", a.trim(), p.trim())) {
                if let MaybeTlsStream::Plain(s) = ws.get_ref() {
                    let _ = s.set_read_timeout(Some(Duration::from_millis(80)));
                }
                return Some(ws);
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    None
}

fn name_of(e: &Value) -> String {
    let n = e["decorations"]["identity"]["name"].as_str().unwrap_or("?");
    let by: Vec<&str> = e["decorations"]["creators"].as_array().map(|a| a.iter().filter_map(|c| c["entity"]["decorations"]["identity"]["name"].as_str()).collect()).unwrap_or_default();
    if by.is_empty() { n.to_string() } else { format!("{n} — {}", by.join(", ")) }
}

fn show(ev: &Value, seen: &mut std::collections::BTreeSet<String>) {
    let ty = ev["type"].as_str().unwrap_or("?");
    seen.insert(ty.to_string());
    let line = match ty {
        "auth_state" => {
            if ev["logged_in"].as_bool() == Some(true) {
                format!("logged in · active device: {}", ev["is_active"])
            } else {
                format!("NOT paired yet: on your phone, open Spotify → device picker → \"{DEVICE}\"")
            }
        }
        "playback_state" => format!("{} · {} · vol {} · ctx {}", ev["status"], name_of(&ev["item"]), ev["volume"], ev["context"]["uri"]),
        "track_changed" => format!("now playing: {}", name_of(&ev["item"])),
        "playback_changed" => format!("status: {}", ev["status"]),
        "volume_changed" => format!("volume: {}", ev["volume"]),
        "position_sync" => return, // too chatty; the real app interpolates from these
        "queue_changed" => {
            let up: Vec<String> = ev["upcoming"].as_array().map(|a| a.iter().take(5).map(|q| format!("{} [{}]", name_of(&q["item"]), q["source"].as_str().unwrap_or(""))).collect()).unwrap_or_default();
            format!("queue (next {}): {}", up.len(), up.join(" | "))
        }
        "command_result" => format!("ok: {}", ev["command"]),
        "error" => format!("\x1b[31merror: {}\x1b[0m", ev["message"]),
        _ => ev.to_string(),
    };
    println!("  \x1b[2m{ty:>16}\x1b[0m {line}");
}

/// Find Soloist's playback stream in PipeWire (via pipewire-pulse) and report its sample format.
fn find_stream() -> Option<(u64, String)> {
    let out = Command::new("pactl").args(["-f", "json", "list", "sink-inputs"]).output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    for si in v.as_array()? {
        let props = &si["properties"];
        let hay = format!("{} {} {}", props["application.name"], props["application.process.binary"], props["media.name"]).to_lowercase();
        if hay.contains("soloist") || hay.contains("spotify") {
            let spec = si["sample_specification"].as_str().unwrap_or("?").to_string();
            return Some((si["index"].as_u64()?, format!("{spec} · app={} · binary={}", props["application.name"], props["application.process.binary"])));
        }
    }
    None
}

/// Spike (c): capture only Soloist's stream for a few seconds and draw a live spectrum.
fn tap(secs: u64) {
    let Some((idx, desc)) = find_stream() else {
        say("No Soloist stream found in PipeWire. Is something playing on the spike device?");
        return;
    };
    say(&format!("Soloist stream #{idx}: {desc}"));
    findings(&format!("- PipeWire stream: `{desc}`"));
    let child = Command::new("parec")
        .args([&format!("--monitor-stream={idx}"), "--format=float32le", "--rate=44100", "--channels=2", "--raw", "--latency-msec=20"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return say("parec failed to start") };
    let mut out = child.stdout.take().unwrap();
    let n = 2048;
    let fft = FftPlanner::new().plan_fft_forward(n);
    let mut mono = vec![0.0f32; n];
    let mut buf = vec![0u8; n * 8];
    let t0 = Instant::now();
    let mut peak = 0.0f32;
    while t0.elapsed().as_secs() < secs {
        if out.read_exact(&mut buf).is_err() {
            break;
        }
        for i in 0..n {
            let l = f32::from_le_bytes(buf[i * 8..i * 8 + 4].try_into().unwrap());
            let r = f32::from_le_bytes(buf[i * 8 + 4..i * 8 + 8].try_into().unwrap());
            mono[i] = (l + r) * 0.5;
            peak = peak.max(l.abs()).max(r.abs());
        }
        let mut c: Vec<Complex<f32>> = mono.iter().enumerate().map(|(i, s)| Complex::new(s * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()), 0.0)).collect();
        fft.process(&mut c);
        let bars: String = (0..48)
            .map(|b| {
                let f0 = 40.0f32 * (16000.0f32 / 40.0).powf(b as f32 / 48.0);
                let i = ((f0 / 44100.0 * n as f32) as usize).clamp(1, n / 2 - 1);
                let db = 20.0 * (c[i].norm() / n as f32 + 1e-9).log10();
                let v = ((db + 70.0) / 55.0).clamp(0.0, 1.0);
                [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][(v * 8.0) as usize]
            })
            .collect();
        print!("\r  \x1b[38;5;180m{bars}\x1b[0m  peak {peak:.2}   ");
        let _ = std::io::stdout().flush();
    }
    println!();
    let _ = child.kill();
    findings(&format!("- Audio tap of only the Soloist stream works: peak level {peak:.2} over {secs}s"));
}

fn help() {
    println!(
        "  commands: play <spotify-uri> · pause · resume · next · prev · seek <sec> · vol <0-100>\n            \
         shuffle on|off · queue · state · add <track-uri> · tap [secs] · codec · help · quit"
    );
}

fn main() {
    println!("\n\x1b[1mSingularity · Phase 1 spike: Soloist control + audio tap\x1b[0m\n");
    let Some(key) = api_key() else {
        say("No Soloist API key found. Two ways to provide it:");
        println!("    1) keyring (recommended):  secret-tool store --label=\"Singularity: Soloist API key\" service singularity account soloist");
        println!("    2) environment:            SOLOIST_API_KEY=… cargo run --release");
        return;
    };
    let exe = match ensure_binary() {
        Ok(e) => e,
        Err(e) => return say(&format!("Couldn't get Soloist: {e}")),
    };
    let ver = Command::new(&exe).arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    say(&format!("Soloist: {ver}"));
    let help_txt = Command::new(&exe).arg("--help").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    std::fs::write(root().join("soloist-help.txt"), &help_txt).ok();
    findings(&format!("\n## Run {}\n\n- Version: `{ver}`", chrono_like_now()));

    let state = root().join("state");
    let mut child = match start(&exe, &key, &state) {
        Ok(c) => c,
        Err(e) => return say(&format!("Couldn't start Soloist: {e}")),
    };
    let Some(mut ws) = connect(&state, &mut child) else {
        let _ = child.kill();
        return say("Couldn't reach Soloist's WebSocket (see state/soloist.log).");
    };
    let log = std::fs::read_to_string(state.join("soloist.log")).unwrap_or_default();
    for l in log.lines().filter(|l| l.to_lowercase().contains("expir") || l.to_lowercase().contains("lifetime")) {
        say(&format!("build lifetime: {l}"));
        findings(&format!("- Build lifetime log: `{l}`"));
    }
    say("Connected to Soloist's local WebSocket. Type `help` for commands.");

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut seen = std::collections::BTreeSet::new();
    let mut logged_in = false;
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                if let Ok(ev) = serde_json::from_str::<Value>(t.as_str()) {
                    if ev["type"] == "auth_state" && ev["logged_in"] == true && !logged_in {
                        logged_in = true;
                        findings("- Pairing from the phone worked; session stored");
                    }
                    show(&ev, &mut seen);
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => {
                say(&format!("WebSocket closed: {e}"));
                break;
            }
        }
        if let Ok(Some(st)) = child.try_wait() {
            say(&format!("Soloist exited: {st}"));
            break;
        }
        let Ok(line) = rx.try_recv() else { continue };
        let mut it = line.split_whitespace();
        let cmd = it.next().unwrap_or("");
        let arg = it.next().unwrap_or("");
        let msg = match cmd {
            "play" if arg.is_empty() => json!({"type":"command","command":"play"}),
            "play" => json!({"type":"command","command":"play","uri":arg}),
            "resume" => json!({"type":"command","command":"play"}),
            "pause" => json!({"type":"command","command":"pause"}),
            "next" => json!({"type":"command","command":"skip_next"}),
            "prev" => json!({"type":"command","command":"skip_prev"}),
            "seek" => json!({"type":"command","command":"seek","position_ms": arg.parse::<u64>().unwrap_or(0) * 1000}),
            "vol" => json!({"type":"command","command":"set_volume","volume": arg.parse::<u64>().unwrap_or(50)}),
            "shuffle" => json!({"type":"command","command":"set_shuffle","enabled": arg == "on"}),
            "queue" => json!({"type":"command","command":"get_queue","limit":10}),
            "state" => json!({"type":"command","command":"get_state"}),
            "add" => json!({"type":"command","command":"add_to_queue","uri":arg}),
            "tap" => {
                tap(arg.parse().unwrap_or(10));
                continue;
            }
            "codec" => {
                // grep Soloist's own log for anything that reveals the stream format
                let log = std::fs::read_to_string(state.join("soloist.log")).unwrap_or_default();
                let hits: Vec<&str> = log.lines().filter(|l| { let l = l.to_lowercase(); ["flac", "lossless", "bitrate", "quality", "codec", "vorbis", "format", "kbps", "bit depth", "24-bit", "16-bit"].iter().any(|k| l.contains(k)) }).collect();
                if hits.is_empty() {
                    say("No codec/quality lines in the log yet (try running with --verbose).");
                }
                for h in hits.iter().rev().take(15).rev() {
                    println!("    {h}");
                }
                if let Some(h) = hits.last() {
                    findings(&format!("- Codec/quality log line: `{h}`"));
                }
                continue;
            }
            "help" | "" => {
                help();
                continue;
            }
            "quit" | "exit" | "q" => break,
            _ => {
                say("unknown command");
                help();
                continue;
            }
        };
        let _ = ws.send(Message::text(msg.to_string()));
    }
    findings(&format!("- Event types observed: {}", seen.into_iter().collect::<Vec<_>>().join(", ")));
    let _ = child.kill();
    let _ = child.wait();
    say("Soloist stopped. Results appended to FINDINGS.md, full CLI help saved to soloist-help.txt.");
}

fn chrono_like_now() -> String {
    Command::new("date").arg("+%Y-%m-%d %H:%M").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}
