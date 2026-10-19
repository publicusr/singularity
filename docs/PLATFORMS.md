# Platforms: Linux and macOS

Owner rule (D49): one codebase, the same features on both, nothing exclusive to either if possible. OS-specific code
lives only behind the platform seams below; everything else (Client, Catalogue, ConnectRemote, LocalStore, Config,
protocol, Visuals) is shared. Windows is out of scope (D10).

Status: ✅ works (verified as noted) · 🔶 built, not yet verified there · 🔨 ticket open · ❌ missing, with the ticket ·
➖ not needed there. Last full audit of this table against the code: 2026-10-19 (every `cfg(target_os)`, `/proc`,
D-Bus, `pactl`/`parec`, `xdg-open`, `systemctl` and subprocess call was checked).

## The Engines (what plays the music)

`[engine] kind` in `config.toml` (Settings page too), read when the Daemon starts. `auto` = Soloist on Linux, librespot
on macOS (D54: standalone, lightest, the audio Widgets work out of the box; the app is the lossless choice).

| Engine | Linux | macOS | Lossless | Cost (measured on the M2 Air) |
|--------|-------|-------|----------|-------------------------------|
| Soloist (ADR-0001) | ✅ default | ➖ Spotify ships it for Linux only | ✅ | (Linux: not measured yet, #71) |
| Spotify app over Connect (D51, #65) | 🔶 built (`spotify --minimized`), never run | ✅ live-checked, network-change fixes awaiting the owner's re-check | ✅ the app's own setting | app ~600 MB, 5–25 % CPU playing |
| librespot in the Daemon (D52, #70) | 🔶 built, type-checked; needs `alsa-lib` to build; never run | 🔶 default (D54); on Connect live (read-only check); playback awaits the owner | ❌ 320 kbps at most, never | Daemon ~26 MB, ~0–1 % CPU idle; playing not measured yet |

Lossless without the Spotify app on macOS: not possible with librespot (Spotify keeps lossless to its own players and
asked librespot not to pursue it). The only other route is running Soloist itself in a small Linux VM (D53, open).

## Features, one by one

| Feature (spec story) | Linux | macOS | Gap and ticket |
|----------------------|-------|-------|----------------|
| Setup wizard per Engine (1, 2) | ✅ Soloist; 🔶 app, librespot | ✅ app (pty + live); ✅ librespot (pty) | — |
| Stay logged in (3); secrets in the OS store (63) | ✅ Secret Service | ✅ Keychain (#64, live round trip) | — |
| Engine updates itself (4) | ✅ Soloist | ➖ the app updates itself; librespot ships with Singularity | — |
| Health check (5) | ✅ | ✅; audio-output row now names the system output (cpal) | — |
| Lossless notice and fix (6, 26, 27) | ✅ | ✅ app (quality level from Connect, not the exact bitrate); librespot says "320 kbps, its best" | Stream Pane's exact bitrate: Soloist only (⚠️ the app doesn't report it) |
| Home, search, library, pages, lyrics, devices, queue, playlists, podcasts, stats, Settings (7–25, 31–35, 68–80, 83) | ✅ | ✅ (shared code; pty scenarios pass on macOS) | — |
| Download speed and stalls (#45) | ✅ Soloist | ⚠️ neither the app nor librespot reports it | inherent; noted in the Stream Pane |
| Media keys, OS now-playing (28) | ✅ MPRIS | 🔶 app: its own Now Playing entry; ❌ librespot: nothing yet | #68 (Now Playing + remote commands from the Daemon, so it works for every Engine) |
| Track notifications (29) | ✅ D-Bus | ❌ | #68 |
| Music survives closing the terminal (30) | ✅ | ✅ (own process group) | — |
| Daemon starts at login | ✅ systemd user unit | ❌ | #69 (LaunchAgent; also the "Incompatible" message still names `systemctl`) |
| **Audio Widgets react to the music (40): spectrum, loudness, VU, rain, scope, the hole's pulse** | ✅ Soloist (PipeWire); 🔶 app (its PipeWire stream); 🔶 librespot (in-process) | 🔶 **librespot: in-process, no permission needed**; ❌ **app: nothing yet, the Widgets stay flat** | #67 (Core Audio process tap on Spotify.app) |
| Visuals on the GPU, paused when hidden (36–42) | ✅ Vulkan | ✅ Metal ("Apple M2") | `tap_cpu` budget on the M2: #71 |
| Pixel tier: kitty graphics, shared memory (43) | ✅ kitty, Ghostty | 🔶 same code (POSIX `shm_open`); not tried in a real kitty/Ghostty on macOS yet | owner's terminal check |
| `top` Widget (Terminal design) | ✅ `/proc` | ✅ libproc (new: was empty on macOS); also shows the Spotify app and its helpers | — |
| Opening the Spotify login page in the browser | ✅ `xdg-open` | ✅ `open` (new: did nothing on macOS before) | — |
| Stale-Daemon check (#25) | ✅ `/proc/<pid>/exe` | ✅ `proc_name` (#63) | — |
| Engine stops with the Daemon | ✅ Soloist (`PR_SET_PDEATHSIG`) | ➖ the app keeps running (the user's app too); librespot is inside the Daemon | — |
| "Restart the player" | ✅ Soloist; 🔶 app, librespot | ✅ app; 🔶 librespot | — |
| Local data, backups, config, paths (58–62, D50) | ✅ | ✅ same XDG-style paths | — |
| Mini-player, mouse, palette, layouts, themes (44–57, 67) | ✅ | ✅ | — |

## The seams (where OS-specific code lives)

| Seam | Linux | macOS | macOS status |
|------|-------|-------|--------------|
| Engine (ADR-0001, D51, D52) | Soloist over its WebSocket; or the app / librespot | the app (started hidden), or librespot | see the Engines table |
| AudioTap (D16) | `parec` on the Engine's PipeWire stream (Soloist's, or the app's); librespot in-process | librespot in-process; Core Audio process tap on Spotify.app | 🔨 #67 for the app |
| Secrets (D24) | `secret-tool` | `/usr/bin/security` | ✅ #64 |
| OS media integration | MPRIS on D-Bus | Now Playing / remote commands | 🔨 #68 |
| Notifications | D-Bus | macOS notifications | 🔨 #68 |
| Autostart | systemd user unit | LaunchAgent | 🔨 #69 |
| Process lookup | `/proc` | `proc_name`, libproc | ✅ |
| Audio output name (health) | `pactl` (the Engine's stream → its sink) | cpal's default output device | ✅ |
| Opening a URL | `xdg-open` | `open` | ✅ |
| Socket dir | `$XDG_RUNTIME_DIR/singularity/` | `$TMPDIR/singularity-<uid>/singularity/` (per-user `$TMPDIR`; the uid part reads "user" without `/proc`, harmless) | ✅ |

## Not yet run on Linux

Everything since the move to the Mac is type-checked for Linux (`tools/check-linux.sh`) but has not run there:
the Engine choice, librespot (needs `alsa-lib` installed to build), the app on Linux, the tap's stream matching. First
thing back on the Linux machine: `cargo test --workspace`, the pty scenarios, and a session with each Engine.

## Shared, and the same on both

Paths (D50): `~/.config/singularity`, `~/.local/share/singularity`, `~/.local/state/singularity`,
`~/.cache/singularity`, with the `XDG_*` variables respected. Terminals: kitty, Ghostty and WezTerm for the pixel tier
on both; the cell tier everywhere else. Visuals: wgpu on Vulkan (Linux) or Metal (macOS), CPU fallback on both.
