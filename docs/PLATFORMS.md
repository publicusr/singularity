# Platforms: Linux and macOS

Owner rule (D49): one codebase, the same features on both, nothing exclusive to either if possible. OS-specific code
lives only behind the platform seams below; everything else (Client, Catalogue, ConnectRemote, LocalStore, Config,
protocol, Visuals) is shared. Windows is out of scope (D10).

Status: ✅ works (verified as noted) · 🔶 built, not yet verified there · 🔨 ticket open · ❌ missing, with the ticket ·
➖ not needed there. Last full audit of this table against the code: 2026-10-19 (every `cfg(target_os)`, `/proc`,
D-Bus, `pactl`/`parec`, `xdg-open`, `systemctl` and subprocess call was checked); rows updated through 2026-10-20.
For users, the README's "Build and run" and "macOS quirks" sections say the same in short.

## The Engines (what plays the music)

`[engine] kind` in `config.toml` (Settings page too), read when the Daemon starts. `auto` = Soloist on Linux, librespot
on macOS (D54, D56: standalone, lightest, the audio Widgets work out of the box; for lossless on macOS, `soloist` (the
VM) or `spotify-app`).

| Engine | Linux | macOS | Lossless | Cost (measured on the M2 Air) |
|--------|-------|-------|----------|-------------------------------|
| Soloist (ADR-0001) | ✅ default; cache capped by `engine.cache_gb` (D61, 5 GB) | ➖ natively (Spotify ships it for Linux only): see the Soloist VM | ✅ | (Linux: not measured yet, #71) |
| Spotify app over Connect (D51, #65) | 🔶 built (`spotify --minimized`), never run | ✅ live-checked, network-change fixes awaiting the owner's re-check | ✅ the app's own setting | app ~600 MB, 5–25 % CPU playing |
| librespot in the Daemon (D52, #70) | 🔶 built, type-checked; needs `alsa-lib` to build; never run | 🔶 default (D54); on Connect live (read-only check); playback awaits the owner | ❌ 320 kbps at most, never | Daemon ~26 MB, ~0–1 % CPU idle; playing ~1 % CPU, 28 MB |
| Soloist in the Soloist VM (D55, #72): `kind = "soloist"` on macOS | ➖ (Soloist runs natively) | 🔶 built; live: VM up and waiting for pairing in 5.6 s, paired from the phone, FLAC on the Mac's speakers, the audio Widgets fed | ✅ FLAC verified | ~300 MB; idle ~1 % CPU (pipe sink over vsock, 1 vCPU); playing FLAC: not measured on this setup yet (the owner was on librespot); first setup ~2 min, ~1.3 GB disk; cache fixed at 200 MB (`engine.cache_gb` doesn't apply: the VM's disk is small) |

Lossless without the Spotify app on macOS: not possible with librespot (Spotify keeps lossless to its own players and
asked librespot not to pursue it), hence Soloist itself in a small Linux VM (D53 spike, D55, #72).

## Features, one by one

| Feature (spec story) | Linux | macOS | Gap and ticket |
|----------------------|-------|-------|----------------|
| Setup wizard per Engine (1, 2) | ✅ Soloist; 🔶 app, librespot | ✅ app (pty + live); ✅ librespot (pty); 🔶 the Soloist VM (Soloist's steps, VM setup in the status bar) | — |
| Stay logged in (3); secrets in the OS store (63) | ✅ Secret Service | ✅ Keychain (#64, live round trip) | — |
| Engine updates itself (4) | ✅ Soloist | ✅ the VM's Soloist (the guest fetches and swaps builds; same quiet-moment restart); ➖ the app updates itself; librespot ships with Singularity | — |
| Health check (5) | ✅ names missing programs (#77) | ✅; audio-output row now names the system output (cpal); names missing programs (#77) | — |
| Lossless notice and fix (6, 26, 27) | ✅ | ✅ the VM's Soloist (its log, as on Linux); ✅ app (quality level from Connect, not the exact bitrate); librespot says "320 kbps, its best" | Stream Pane's exact bitrate: Soloist only (⚠️ the app doesn't report it) |
| Home, search, library, pages, lyrics, devices, queue, playlists, podcasts, stats, Settings (7–25, 31–35, 68–80, 83) | ✅ | ✅ (shared code; pty scenarios pass on macOS) | — |
| Audiobooks (84, #62): book pages, chapters, resume, library, search | ✅ Soloist (live: a chapter played, its resume point recorded); 🔶 app (Spotify's own player: expected to play, not tried); ⚠️ librespot: Spotify refuses it chapter keys, so playing one says "Audiobooks need Soloist or the Spotify app" (D57); pages, library and search work | ✅ the VM's Soloist (live check, 2026-10-20); 🔶 app as on Linux; ⚠️ librespot, the default, as on Linux | inherent (D57): DRM, not ours to get around |
| Download speed and stalls (#45) | ✅ Soloist | ✅ the VM's Soloist; ⚠️ neither the app nor librespot reports it | inherent; noted in the Stream Pane |
| Media keys, OS now-playing (28) | ✅ MPRIS | 🔶 librespot and the Soloist VM: the Daemon's own Now Playing entry + remote commands (play/pause/next/previous/seek; built, awaits the owner's media keys); the app: its own entry | #68 (cover art in Now Playing still to do) |
| Track notifications (29) | ✅ D-Bus, with the cover | 🔶 `osascript` notifications, no cover (a process without an app bundle can't post its own) | #68 |
| Music survives closing the terminal (30) | ✅ | ✅ (own process group) | — |
| Daemon starts at login (optional: a Client starts it anyway) | ✅ systemd user unit | ✅ LaunchAgent (`packaging/launchd/install.sh`; tested under a scratch label: start, exit-75 restart, uninstall) | — |
| **Audio Widgets react to the music (40): spectrum, loudness, VU, rain, scope, the hole's pulse** | ✅ Soloist (PipeWire); 🔶 app (its PipeWire stream); 🔶 librespot (in-process) | 🔶 **librespot: in-process, no permission needed**; 🔶 Soloist VM: in-process (its PCM); 🔶 **app: Core Audio process tap, needs the terminal allowed under "System Audio Recording Only"** (silence otherwise, and Singularity says so) | #67 (live check with the app pending) |
| Visuals on the GPU, paused when hidden (36–42) | ✅ Vulkan | ✅ Metal ("Apple M2") | `tap_cpu` budget on the M2: #71 |
| Pixel tier: kitty graphics, shared memory (43) | ✅ kitty, Ghostty | 🔶 same code (POSIX `shm_open`); not tried in a real kitty/Ghostty on macOS yet | owner's terminal check |
| `top` Widget (Terminal design) | ✅ `/proc` | ✅ libproc (new: was empty on macOS); also shows the Spotify app and its helpers | — |
| Opening the Spotify login page in the browser | ✅ `xdg-open` | ✅ `open` (new: did nothing on macOS before) | — |
| Stale-Daemon check (#25) | ✅ `/proc/<pid>/exe` | ✅ `proc_name` (#63) | — |
| Engine stops with the Daemon | ✅ Soloist (`PR_SET_PDEATHSIG`) | ✅ the VM stops with it (its helpers are the Daemon's children); ➖ the app keeps running (the user's app too); librespot is inside the Daemon | — |
| "Restart the player" | ✅ Soloist; 🔶 app, librespot | ✅ app; 🔶 librespot, the VM's Soloist | — |
| Local data, backups, config, paths (58–62, D50) | ✅ | ✅ same XDG-style paths | — |
| Mini-player, mouse, palette, layouts, themes (44–57, 67) | ✅ | ✅ | — |
| Jam: start on this device, invite link + QR, guests, end (D64) | ✅ Soloist (live: start and end, 2026-10-30; a guest joining not yet tried); 🔶 librespot, app (same Connect code, not tried) | 🔶 same code (Connect, no OS part); not tried | a guest's join and the dealer updates: a second phone |

## The seams (where OS-specific code lives)

| Seam | Linux | macOS | macOS status |
|------|-------|-------|--------------|
| Engine (ADR-0001, D51, D52, D55) | Soloist over its WebSocket; or the app / librespot | librespot; Soloist in the VM (`vm.rs`, `soloist_vm.rs`, `pcm.rs`); the app (started hidden) | see the Engines table |
| AudioTap (D16) | `parec` on the Engine's PipeWire stream (Soloist's, or the app's); librespot in-process | librespot and the Soloist VM in-process; Core Audio process tap on Spotify.app (48 → 44.1 kHz) | 🔶 #67 for the app |
| Secrets (D24) | `secret-tool` | `/usr/bin/security` | ✅ #64 |
| OS media integration | MPRIS on D-Bus | Now Playing + remote commands (MediaPlayer framework; the main thread runs the run loop, tokio another thread) | 🔶 #68 |
| Notifications | D-Bus | `osascript` | 🔶 #68 |
| Autostart | systemd user unit | LaunchAgent | ✅ #69 |
| Process lookup | `/proc` | `proc_name`, libproc | ✅ |
| Programs the Daemon runs (pgrep, pkill, tar, secret-tool, pactl, parec; #77, audit S3) | fixed locations (`/usr/bin`, `/bin`, `/usr/local/bin`; Spotify also `/snap/bin`), never `PATH` | the same, plus `/opt/homebrew/bin` last; `scutil` in `/usr/sbin` | ✅; only `pgrep`, `pkill`, `tar` are needed there |
| The Spotify app's launcher | `spotify --minimized` from `/usr/bin`, `/bin`, `/usr/local/bin` or `/snap/bin` | `open -gj -a Spotify` from `/usr/bin` | ✅; the health view's **Tools** row names it only with `kind = "spotify-app"` |
| Audio output name (health) | `pactl` (the Engine's stream → its sink) | cpal's default output device | ✅ |
| Opening a URL | `xdg-open` from `/usr/bin`, `/bin` or `/usr/local/bin` | `open` | ✅ |
| The Client starting the Daemon | `singularityd` next to the Client (never `PATH`) | the same | ✅ |
| Socket dir | `$XDG_RUNTIME_DIR/singularity/` | `$TMPDIR/singularity-<uid>/singularity/` (per-user `$TMPDIR`; the uid part reads "user" without `/proc`, harmless) | ✅ |

## Not yet run on Linux

Everything since the move to the Mac is type-checked for Linux (`tools/check-linux.sh`) but has not run there:
the Engine choice, librespot (needs `alsa-lib` installed to build), the app on Linux, the tap's stream matching. First
thing back on the Linux machine: `cargo test --workspace`, the pty scenarios, and a session with each Engine.

## Shared, and the same on both

Paths (D50): `~/.config/singularity`, `~/.local/share/singularity`, `~/.local/state/singularity`,
`~/.cache/singularity`, with the `XDG_*` variables respected. Terminals: kitty, Ghostty and WezTerm for the pixel tier
on both; the cell tier everywhere else. Visuals: wgpu on Vulkan (Linux) or Metal (macOS), CPU fallback on both.
