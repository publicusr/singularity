# Platforms: Linux and macOS

Owner rule (D49): one codebase, the same features on both, nothing exclusive to either if possible. OS-specific code
lives only behind the platform seams below; everything else (Client, Catalogue, ConnectRemote, LocalStore, Config,
protocol, Visuals) is shared. Windows is out of scope (D10).

Status: ✅ works (verified as noted) · 🔶 built, not yet verified there · 🔨 ticket open · ❌ missing, with the ticket ·
➖ not needed there. **On macOS, a row is 🔶 wherever its code moved or changed since the last run on a Mac
(2026-10-20)**, with what was verified then in brackets; CI's `macos-latest` job doesn't count as a Mac run. Since a
Mac's Daemon couldn't make its socket from 2026-10-25 until `b2c8d15` (2026-11-20), which no Mac has run, and every
area's code has changed since, every macOS mark is 🔶 until the macOS session (#85) runs "Not yet run on macOS" below.
Last full audit of this page against the code: **2026-11-25** (D89 wave 1, #99: every `cfg(target_os)`,
`cfg!(target_os)` and `cfg_attr(target_os)` site, every `host::MACOS` branch and the CPU checks; each belongs to one
seam below). Issue numbers written `singularity-archive#N` are the old repo's; a bare `#N` is this repo's.
For users, the README's "Build and run" and "macOS quirks" sections say the same in short.

## The Engines (what plays the music)

**Linux on ARM (D73):** aarch64 Linux is the same build as x86_64 Linux: no `target_arch` code; the CPU is a Host
fact (Rust's `std::env::consts::ARCH`). The Daemon downloads Soloist's arm64 build (Spotify publishes it next to the
x86_64 one, `host::soloist_build`, `update::cdn_for`), and on a Linux CPU with no Soloist build `auto` means
librespot. **CI builds and tests it** on every push and PR of the public repo (GitHub's `ubuntu-24.04-arm` runner), a
release carries an `aarch64-unknown-linux-gnu` archive, and on a Mac `tools/check-linux.sh` type-checks it next to
x86_64. Run once on hardware (2026-11-23, the Mac on Asahi Linux): `tools/check.sh` green, the Visuals on Asahi's
Vulkan, librespot played and fed the audio Widgets, MPRIS paused it; Soloist on ARM was not tried. Status: 🔶
**parked** (Q83, 2026-11-25: no ARM hardware now that the Mac runs macOS again; it stays a one-session job for when
there is). The feature rows below are for x86_64 Linux.

`[engine] kind` in `config.toml` (Settings page too), read when the Daemon starts. `auto` = Soloist on Linux, librespot
on macOS (D54, D56: standalone, lightest, the audio Widgets work out of the box; for lossless on macOS, `soloist` (the
VM) or `spotify-app`).

| Engine | Linux | macOS | Lossless | Cost (measured on the Mac, an M2) |
|--------|-------|-------|----------|-----------------------------------|
| Soloist (ADR-0001) | ✅ default; cache capped by `engine.cache_gb` (D61, 5 GB) | ➖ natively (Spotify ships it for Linux only): see the Soloist VM | ✅ | (Linux: not measured yet, singularity-archive#71) |
| Spotify app over Connect (D51, singularity-archive#65) | 🔶 built (`spotify --minimized`), never run | 🔶 (✅ live-checked by 2026-10-20; network-change fixes awaiting the owner's re-check) | ✅ the app's own setting | app ~600 MB, 5–25 % CPU playing |
| librespot in the Daemon (D52, singularity-archive#70) | 🔶 built, type-checked; needs `alsa-lib` to build; never run | 🔶 default (D54); on Connect live (read-only check); playback awaits the owner | ❌ 320 kbps at most, never | Daemon ~26 MB, ~0–1 % CPU idle; playing ~1 % CPU, 28 MB |
| Soloist in the Soloist VM (D55, singularity-archive#72): `kind = "soloist"` on macOS | ➖ (Soloist runs natively) | 🔶 built (behind the Soloist runner since D89, #94: not run on the Mac since); live (2026-10): VM up and waiting for pairing in 5.6 s, paired from the phone, FLAC on the Mac's speakers, the audio Widgets fed | ✅ FLAC verified | ~300 MB; idle ~1 % CPU (pipe sink over vsock, 1 vCPU); playing FLAC: not measured on this setup yet (the owner was on librespot); first setup ~2 min, ~1.3 GB disk; cache fixed at 200 MB (`engine.cache_gb` doesn't apply: the VM's disk is small) |

Lossless without the Spotify app on macOS: not possible with librespot (Spotify keeps lossless to its own players and
asked librespot not to pursue it), hence Soloist itself in a small Linux VM (D53 spike, D55, singularity-archive#72).

## Features, one by one

| Feature (spec story) | Linux | macOS | Gap and ticket |
|----------------------|-------|-------|----------------|
| Setup wizard per Engine (1, 2) | ✅ Soloist; 🔶 app, librespot | 🔶 (✅ by 2026-10-20: app (pty + live), librespot (pty)); 🔶 the Soloist VM (Soloist's steps, VM setup in the status bar) | — |
| Stay logged in (3); secrets in the OS store (63) | ✅ Secret Service | 🔶 Keychain (✅ by 2026-10-20: singularity-archive#64, live round trip) | — |
| Engine updates itself (4) | ✅ Soloist | 🔶 the VM's Soloist (✅ by 2026-10-20: the guest fetches and swaps builds; same quiet-moment restart); ➖ the app updates itself; librespot ships with Singularity | — |
| Health check (5) | ✅ names missing programs (singularity-archive#77): only those the Daemon runs (#98) | 🔶 (✅ by 2026-10-20: the audio-output row names the system output (cpal), missing programs named); since #98 it names only `tar`, and `open` with the app | — |
| Lossless notice and fix (6, 26, 27) | ✅ | 🔶 (✅ by 2026-10-20: the VM's Soloist (its log, as on Linux); the app (quality level from Connect, not the exact bitrate)); librespot says "320 kbps, its best" | Stream Pane's exact bitrate: Soloist only (⚠️ the app doesn't report it) |
| Home, search, library, pages, lyrics, devices, queue, playlists, podcasts, stats, Settings (7–25, 31–35, 68–80, 83) | ✅ | 🔶 shared code, much of it new or rebuilt since (✅ by 2026-10-20: pty scenarios passed on macOS) | — |
| Audiobooks (84, singularity-archive#62): book pages, chapters, resume, library, search | ✅ Soloist (live: a chapter played, its resume point recorded); 🔶 app (Spotify's own player: expected to play, not tried); ⚠️ librespot: Spotify refuses it chapter keys, so playing one says "Audiobooks need Soloist or the Spotify app" (D57); pages, library and search work | 🔶 the VM's Soloist (✅ live check, 2026-10-20); 🔶 app as on Linux; ⚠️ librespot, the default, as on Linux | inherent (D57): DRM, not ours to get around |
| Download speed and stalls (singularity-archive#45) | ✅ Soloist | 🔶 the VM's Soloist (✅ by 2026-10-20); ⚠️ neither the app nor librespot reports it | inherent; noted in the Stream Pane |
| Media keys, OS now-playing (28) | ✅ MPRIS | 🔶 librespot and the Soloist VM: the Daemon's own Now Playing entry + remote commands (play/pause/next/previous/seek; built, awaits the owner's media keys); the app: its own entry | singularity-archive#68 (cover art in Now Playing still to do) |
| Track notifications (29) | ✅ D-Bus, with the cover | 🔶 `osascript` notifications, no cover (a process without an app bundle can't post its own) | singularity-archive#68 |
| Music survives closing the terminal (30) | ✅ | 🔶 (✅ by 2026-10-20: own process group) | — |
| Daemon starts at login (optional: a Client starts it anyway) | ✅ systemd user unit | 🔶 LaunchAgent (✅ by 2026-10-20: `packaging/launchd/install.sh` under a scratch label: start, exit-75 restart, uninstall) | — |
| **Audio Widgets react to the music (40): spectrum, loudness, VU, rain, scope, the hole's pulse** | ✅ Soloist (PipeWire); 🔶 app (its PipeWire stream); 🔶 librespot (in-process) | 🔶 **librespot: in-process, no permission needed**; 🔶 Soloist VM: in-process (its PCM); 🔶 **app: Core Audio process tap, needs the terminal allowed under "System Audio Recording Only"** (silence otherwise, and Singularity says so) | singularity-archive#67 (live check with the app pending) |
| Visuals on the GPU, paused when hidden (36–42) | ✅ Vulkan | 🔶 Metal (✅ by 2026-10-20: "Apple M2") | `tap_cpu` budget on the M2: singularity-archive#71 |
| Pixel tier: kitty graphics, shared memory (43) | ✅ kitty, Ghostty | 🔶 same code (POSIX `shm_open`); not tried in a real kitty/Ghostty on macOS yet | owner's terminal check |
| `top` Widget (Terminal design) | ✅ `/proc` | 🔶 libproc, behind the Host since #96 (✅ by 2026-10-20: it listed processes, the Spotify app and its helpers too) | — |
| Opening the Spotify login page in the browser | ✅ `xdg-open` | 🔶 `open`, behind the Host since #96 (✅ by 2026-10-20) | — |
| Stale-Daemon check (singularity-archive#25) | ✅ `/proc/<pid>/exe` | 🔶 `proc_name`, behind the Host since #96 (✅ by 2026-10-20: singularity-archive#63) | — |
| Engine stops with the Daemon | ✅ Soloist (`PR_SET_PDEATHSIG`) | 🔶 the VM stops when the Daemon quits (✅ by 2026-10-20; its helpers are the Daemon's children; after a SIGKILL: not tried, see the seams); ➖ the app keeps running (the user's app too); librespot is inside the Daemon | — |
| "Restart the player" | ✅ Soloist; 🔶 app, librespot | 🔶 app (✅ by 2026-10-20); 🔶 librespot, the VM's Soloist | — |
| Local data, backups, config, paths (58–62, D50) | ✅ | 🔶 same XDG-style paths, decided in one place since D89 (App folders, #97) (✅ by 2026-10-20) | — |
| Mini-player, mouse, palette, layouts, themes (44–57, 67) | ✅ | 🔶 rebuilt since (free tiling, Arrange mode, the wheel) (✅ by 2026-10-20) | — |
| Demo mode (D90): the filter where events enter the Client, the redactor, the demo Listening log | ✅ pty `demo` and `PTY_DEMO=1 wheel-all`: no fake secret drawn | 🔶 shared code (the computer's name from the Host); not run on the Mac yet | — |
| Jam: start on this device, invite link + QR, guests, end (D64) | ✅ Soloist (live: start and end, 2026-10-30; a guest joining not yet tried); 🔶 librespot, app (same Connect code, not tried) | 🔶 same code (Connect, no OS part); not tried | a guest's join and the dealer updates: a second phone |

## The seams (where OS-specific code lives)

Every OS-specific line of code belongs to one row below (audited 2026-11-25). Each seam picks its OS adapter in one
place, and callers outside these files carry no `cfg`, with **one exception, the AudioTap**: its macOS-only source
`TapSource::App` is `cfg`-gated in `tap.rs` and in two callers, `lib.rs` (`tap_again`) and `assembly.rs`
(`Tap::AppProcess`).

| Seam | Linux | macOS | macOS status |
|------|-------|-------|--------------|
| Engine (ADR-0001, D51, D52, D55; chosen by the Engine assembly, `assembly.rs`, D77) | Soloist over its WebSocket; or the app / librespot | librespot; Soloist in the Soloist VM; the app (started hidden) | see the Engines table |
| Where Soloist runs: the Soloist runner (`soloist/runner.rs`, D89) | a local process (`Local`, `soloist/runner.rs`) | the OS-free VM runner (`InVm`, `soloist/runner/vm.rs`; its contract tested on Linux against a fake VM, `tests/soloist_vm_runner.rs`) over the Soloist VM (`soloist_vm.rs`, `vm.rs`, `pcm.rs`: vfkit, gvproxy, audio over vsock; dev tool `examples/vm_probe.rs`, which drives `vm::Vm` directly, not the runner). The Daemon tells the Client whether Soloist runs in a VM (`soloist_in_vm`, Q94) | 🔶 #94: CI-built, not yet run on the Mac |
| Engine `auto` and Soloist CPUs (`EngineKind::resolved_on`, `host::soloist_build`, `update::cdn_for`; D54, D73) | `auto` = Soloist on x86_64 and aarch64 (the build for this CPU), librespot on any other CPU | `auto` = librespot | 🔶 tested on Linux for both OSes (the OS and CPU are values) and by CI on macOS; not run on a Mac since it moved (D77, D89) |
| Dying with the Daemon | Soloist is started with `PR_SET_PDEATHSIG` (`soloist/runner.rs`): the kernel stops it when the Daemon dies, even by SIGKILL | no such call: the VM's helpers (vfkit, gvproxy) are the Daemon's children and are stopped when it quits (`kill_on_drop`); after a SIGKILL nothing stops them | 🔶 the SIGKILL case not tried |
| AudioTap (D16; `tap.rs`; the one seam whose `cfg` reaches its callers, see above) | `parec` on the Engine's PipeWire stream (Soloist's, or the app's); librespot in-process | librespot and the Soloist VM in-process; Core Audio process tap on Spotify.app (`process_tap.rs`, 48 → 44.1 kHz; dev tool `examples/tap_probe.rs`) | 🔶 singularity-archive#67 for the app |
| Secrets (D24; `secrets.rs`) | `secret-tool` | `/usr/bin/security` | 🔶 (✅ by 2026-10-20: singularity-archive#64) |
| OS media integration (`os_media.rs`, D89: one module picks the adapter, owns the main-thread handover and the shared policy; `SINGULARITY_NOWPLAYING=off` turns it off on both, `SINGULARITY_DBUS=off` still works, an address in it counts only on Linux, Q99) | MPRIS on D-Bus (`os_media/mpris.rs`; its test `tests/mpris.rs` runs on Linux only) | Now Playing + remote commands (`os_media/now_playing.rs`, MediaPlayer framework, the macOS-only `objc2-*` dependencies in `daemon/Cargo.toml`; the main thread runs the run loop, tokio another thread) | 🔶 singularity-archive#68; the module (#95) not yet run on the Mac |
| Notifications (part of OS media integration) | D-Bus, with the cover | `osascript`, without the cover | 🔶 singularity-archive#68 |
| Autostart, the restart hint (`RESTART`, `lib.rs`), and whether the Daemon runs under it (`run_by_service`, told to Clients in the welcome, #145) | systemd user unit; `systemctl --user restart singularityd`; the Daemon's cgroup is `singularityd.service` | LaunchAgent; `launchctl kickstart -k …`; `XPC_SERVICE_NAME` is the LaunchAgent's label (the Client starts the Daemon without the terminal's) | 🔶 (✅ by 2026-10-20: singularity-archive#69); `run_by_service` not yet run on a Mac |
| Process lookup and stop, CPU and memory use, the uid, the keyring's name: the Host (`config/src/host.rs`, adapters `host/linux.rs` and `host/macos.rs`, D89) | `/proc` (memory in the kernel's page size: 4 K, or 16 K/64 K on ARM kernels) | libproc (`proc_listallpids`, `proc_name`; memory in bytes) | 🔶 #96: the macOS adapter is CI-built, not yet run |
| The computer's name (the Host; how the Spotify app's device is told apart on Connect) | `/etc/hostname`, else `hostname` from `/usr/bin`, `/bin` or `/usr/local/bin` | `scutil --get ComputerName` (`/usr/sbin`) | 🔶 #96, not yet run |
| Opening a URL (the Host) | `xdg-open` from `/usr/bin`, `/bin` or `/usr/local/bin` | `open` from the same | 🔶 #96, not yet run (✅ by 2026-10-20, before the move) |
| The Spotify app: detecting the installed app and starting it (`spotify_app`: `app_engine/linux.rs`, `app_engine/macos.rs`; whether it runs: the Host) | a `spotify` binary in `/usr/bin`, `/bin`, `/usr/local/bin` or `/snap/bin`, started as `spotify --minimized` | `Spotify.app` in `/Applications` or `~/Applications`, its version from its `Info.plist` (`defaults`), started with `open -gj -a Spotify` | 🔶 #96, not yet run (✅ by 2026-10-20, before the move). The health view's **Tools** row names the program that starts the app (`spotify`, `open`) only with `kind = "spotify-app"` |
| Programs the Daemon runs (`programs.rs`; singularity-archive#77, audit S3). Processes are found and stopped through the Host, not `pgrep`/`pkill`; Soloist's Linux builds are unpacked in-process, not by `tar` (#98, Q102) | fixed locations (`/usr/bin`, `/bin`, `/usr/local/bin`; Spotify also `/snap/bin`), never `PATH`; the health check asks for `secret-tool` (while the keyring is on), `pactl`, `parec` | `/usr/bin`, `/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, in that order; the health check asks only for `tar` (the Soloist VM's image) | 🔶 #98, not yet run on the Mac |
| Audio output name (health, `health.rs`) | `pactl` (the Engine's stream → its sink) | cpal's default output device | 🔶 (✅ by 2026-10-20) |
| The Client starting the Daemon | `singularityd` next to the Client (never `PATH`). When a Daemon that the systemd unit ran goes away (replaced, crashed), every Client waits for it to exit and then 5 s for the unit to start the next one before starting one itself (`SERVICE_GRACE`, #145); one the unit didn't run is started again at once | the same, for the LaunchAgent | 🔶 (✅ by 2026-10-20); the wait for launchd not yet run on a Mac |
| The last Home and library (#81, `daemon/src/last_seen.rs`): `home-and-library.json` in the App folders cache dir, owner-only, naming its account, removed on logout | `$XDG_CACHE_HOME/singularity/`, else `~/.cache/singularity/` | the same | 🔶 Linux: tests and a pty run with the fake Catalogue, not yet timed on the real account; not yet run on a Mac |
| Socket dir (App folders, `config/src/app_folders.rs`, D89) | `$XDG_RUNTIME_DIR/singularity/` | `$TMPDIR/singularity-<uid>/singularity/` (per-user `$TMPDIR`; the uid part reads "user", #3) | 🔶 a Mac couldn't make its socket from 2026-10-25 until `b2c8d15` (2026-11-20), which no Mac has run; #97 not yet run there either |

## Not yet run on macOS

The macOS session's checklist (#85). The last run on a Mac was 2026-10-20; everything since (D62–D89) has been built
and tested on Linux, and CI's `macos-latest` job builds it and runs its tests on every PR, but none of it has run on a
real Mac. Each item is something to try there: tick it, or open an issue for what fails.

**Before anyone uses the Mac download:** the published v0.1.0 (2026-11-02) predates `b2c8d15`, so its Daemon can't
make its socket on a Mac. A new release is needed first.

**Start**

- [ ] `tools/check.sh` green on the Mac, then the pty scenarios (`tools/pty/`); `stty size` at full and half screen
  (the D87 sizes, about 180×54 and 90×50).
- [ ] The Daemon starts and makes its socket under `$TMPDIR` (it couldn't on a Mac from 2026-10-25 until `b2c8d15`,
  2026-11-20, which no Mac has run yet); the folder's "user" part (#3).
- [ ] The existing config, listening log and backups are found in the same folders as before (App folders, #97).
  With `HOME` unset, the Client and the Daemon stop and say so; with all four `XDG_*_HOME` set they run (Q93, Q104).

**Engines** (the Engine assembly, D77, and the Soloist session, D76)

- [ ] librespot, the default: play, pause, next, seek, volume, "Restart the player"; after the Mac sleeps and after a
  network change.
- [ ] `auto` still means librespot; the device name in the phone's picker is the one Singularity shows; a bad
  `engine.kind` resets only that key and says so (D77).
- [ ] `kind = "soloist"`, the Soloist VM behind the Soloist runner (#94): first setup, a start in about 6 s after
  that, pairing from the phone, FLAC playing, the audio Widgets moving; the setup steps and the status bar speak of
  the VM (the Daemon now tells the Client, Q94). This is the real check of #94: `examples/vm_probe.rs` drives
  `vm::Vm` directly and doesn't go through the runner.
- [ ] The VM's Soloist session: a Build update at a quiet moment, a restart after the VM is stopped, the volume kept.
- [ ] Quit the Daemon while the VM runs: `vfkit` and `gvproxy` are gone. Then `kill -9` the Daemon: are they still
  running? (macOS has no `PR_SET_PDEATHSIG`; see "Dying with the Daemon".)
- [ ] The VM after the Mac sleeps (#1).
- [ ] `kind = "spotify-app"`: Spotify.app is found (the new `spotify_app` module), its version shows, it starts
  hidden, its Connect device is found by the computer's name (`scutil`, the Host); the health view's Tools row.
- [ ] The process tap on the app and its "System Audio Recording Only" permission on the new macOS
  (singularity-archive#67).

**OS media integration** (#95)

- [ ] The Now Playing entry in Control Centre with librespot and with the VM: title, artist, position; the media keys
  (play/pause, next, previous) and dragging the position (singularity-archive#68).
- [ ] `SINGULARITY_NOWPLAYING=off` turns it off (Q92); `SINGULARITY_DBUS=off` too, and a `SINGULARITY_DBUS` address
  doesn't turn it on (Q99).
- [ ] Track notifications through `osascript`, switched on and off from Settings while the Daemon runs.

**The Host** (#96) **and the health check** (#98)

- [ ] The `top` Widget lists processes with CPU and memory (libproc), the Spotify app and its helpers too; the stream
  Widget's CPU and memory figures for the Engine look right (libproc's numbers, now read through the Host).
- [ ] The stale-Daemon check: reinstall while a Daemon runs; the next Client stops the old Daemon (and only it) and
  starts the new one (singularity-archive#25).
- [ ] "Log in to Spotify" opens the browser (`open`).
- [ ] The health check says "Keychain", names only `tar` when it's missing (`open` only with the app), and its audio
  output row names the Mac's output; a secret round trip in the Keychain (singularity-archive#64).
- [ ] The LaunchAgent (`packaging/launchd/install.sh`) and the restart hint it prints (`launchctl kickstart`).

**Shared code built since 2026-10-21** (no macOS code of its own, never run on a Mac)

- [ ] Jam (D64): start one, the invite link and QR, end it; a guest joining from a second phone.
- [ ] While browsing Home or a playlist, change the track, pause and skip from the phone: Singularity follows each
  change at once, the song you left doesn't come back, and the page you're on doesn't jump (D63, D79, D80, D82).
- [ ] Home without empty slots, a one-Card Shelf as a Spotlight, search on Home, the library on its own, Search keeps
  its query (D68–D72, D74).
- [ ] Lyrics and the Karaoke Widget and Design (D67, D75), big text in the terminal's own font (Ghostty's config where
  macOS keeps it, `termfont.rs`).
- [ ] Free tiling and Arrange mode (D66, D84, D87): mouse, touchpad and keyboard at the Mac's sizes; the Tray, the
  Widget picker, New design…, Delete design (#80); every Widget at the Mac's Pane sizes (#6).
- [ ] The wheel and the touchpad in Ghostty: one notch moves one step (2026-10-23), a touchpad a row a report (D86,
  #32), every list and every popup scrolls under the pointer (popups own the wheel since 2026-11-01; pty `wheel-all`
  shows them all); over a Pane the wheel scrolls a list's view, the pointer highlights a row, a click picks it (D86).
- [ ] The resize pointer appears on a Pane border and goes back to the normal pointer off it, in Ghostty too
  (2026-11-05).
- [ ] Open a song's or a Card's menu and wait while Home or the list refreshes, then pick Play or Add to playlist: it
  acts on the row you opened it on (D81, D85). Open popups one after another (palette, devices, help): the top one
  takes the keys and Esc closes only it (D83). Open a podcast episode and leave its page before it loads: you stay on the page you went to (D88).
- [ ] Smooth Visuals (D62: 30 fps when unfocused) on Metal; the pixel tier in a real Ghostty and kitty; the playing
  Client's CPU on the M2 (#2); the soak and resource numbers (#5).

When it's done: update every row above and the README's "macOS quirks" (#85 step 4).

## Shared, and the same on both

Paths (D50): `~/.config/singularity`, `~/.local/share/singularity`, `~/.local/state/singularity`,
`~/.cache/singularity`, with the `XDG_*` variables respected, named in one place (App folders,
`config/src/app_folders.rs`, D89). With `HOME` unset (and those `XDG_*` unset) the Client and Daemon stop and say so. Terminals: kitty, Ghostty and WezTerm for the pixel tier
on both; the cell tier everywhere else. Visuals: wgpu on Vulkan (Linux) or Metal (macOS), CPU fallback on both.
