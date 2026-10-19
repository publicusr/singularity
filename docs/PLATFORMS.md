# Platforms: Linux and macOS

Owner rule (D49): one codebase, the same features on both, nothing exclusive to either if possible. OS-specific code
lives only behind the platform seams below; everything else (Client, Catalogue, ConnectRemote, LocalStore, Config,
protocol, Visuals) is shared and untouched. Windows is out of scope (D10).

Status: ✅ works · 🔨 ticket open · ➖ not needed there (the OS or the Engine does it) · ⚠️ a real gap, with why.

## The seams

| Seam | Linux | macOS | macOS status |
|------|-------|-------|--------------|
| Engine (ADR-0001, D51) | SoloistEngine: Soloist over its local WebSocket | AppEngine: Spotify.app started hidden, driven over Connect | 🔨 #65 |
| AudioTap (D16) | PipeWire monitor of the Soloist stream (`parec`) | Core Audio process tap on Spotify.app (macOS 14.2+) | 🔨 #67 |
| Secrets (D24) | Secret Service (`secret-tool`) | Keychain (`/usr/bin/security`) | ✅ #64 |
| OS media integration | MPRIS on D-Bus (media keys, KDE widget, `playerctl`) | Spotify.app's own Now Playing entry (media keys, Control Centre) | 🔨 #68 |
| Track notifications | D-Bus notifications | macOS notifications | 🔨 #68 |
| Daemon autostart | systemd user unit (`packaging/systemd/`) | LaunchAgent (`packaging/launchd/`) | 🔨 #69 |
| Stale-Daemon check (#25) | `/proc/<pid>/exe` | `proc_name` (the binary may be gone after a rebuild) | ✅ #63 |
| Engine dies with the Daemon | `PR_SET_PDEATHSIG` + `kill_on_drop` | Spotify.app is left running (it's the user's app too) | ➖ |
| Health check / setup wizard | Soloist key, pairing, Spotify login, audio output (`pactl`) | Spotify.app installed and logged in, Spotify login, audio output | 🔨 #66 |

## Features that differ because the Engine differs

| Feature | Linux (Soloist) | macOS (Spotify.app) | Status |
|---------|-----------------|---------------------|--------|
| Lossless | ✅ | ✅ (the desktop app's own setting) | — |
| Stream Pane: codec and bitrate | from Soloist's verbose log | from Connect's `playback_quality` (quality level, not the exact bitrate) | 🔨 #65 |
| Stream Pane: download speed and stalls (#45) | from Soloist's log | buffering from the cluster only; no download speed | ⚠️ the app doesn't report it |
| Engine updates (#6) | Daemon stages and restarts new Soloist builds | Spotify.app updates itself | ➖ |
| Playing without the Catalogue login | ✅ (Soloist is paired on its own) | needs the Catalogue login (Connect goes through it) | ⚠️ inherent to D51; the wizard asks for it first |

## Shared, and the same on both

Paths (D50): `~/.config/singularity`, `~/.local/share/singularity`, `~/.local/state/singularity`,
`~/.cache/singularity`, with the `XDG_*` variables respected. The socket goes in `$XDG_RUNTIME_DIR/singularity/`,
else the per-user temp dir (`$TMPDIR` on macOS). Terminals: kitty, Ghostty and WezTerm for the pixel tier on both;
the cell tier everywhere else. Visuals: wgpu on Vulkan (Linux) or Metal (macOS), CPU fallback on both.
