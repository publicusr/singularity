# Singularity

A Spotify TUI client for Linux and macOS that keeps nearly everything the desktop app does, plays lossless audio
and puts the visuals first. Keyboard and mouse driven, with built-in themes and audio-reactive visuals (yes, including
a black hole), modular and customisable.

![Singularity in the Rosé theme: the black hole, an oscilloscope, the title marquee, a spectrum and the cover's glow over the Home feed, the library and synced lyrics](docs/media/rose.png)

| Ember | An album | Queue and lyrics |
|---|---|---|
| ![The Ember theme: the marquee, an oscilloscope and the black hole over Home, lyrics and the library](docs/media/ember.png) | ![An album open beside the queue, synced lyrics and the library, with a starfield, digital rain and Now playing above](docs/media/album.png) | ![Home with Made For You and your top mixes, the queue and synced lyrics](docs/media/queue.png) |

The Jam picker: the invite link, its QR code and a guest (from the test harness, made-up account).

![The device picker with a Jam running: its invite link, QR code and a guest](docs/media/jam.png)

**Status:** `v0.1.0`. **Linux** is the supported platform; **macOS is a beta**: everything is there, but some of it
hasn't been checked by hand on a Mac yet (see the issues). Spec in `docs/SPEC.md`.

Linux and macOS have the same features (the rule is D49); `docs/PLATFORMS.md` lists every feature's exact status on
each, and the few real differences. **Linux on ARM** (aarch64, e.g. a Raspberry Pi 5 or an ARM laptop) builds the
same way and runs Soloist's arm64 build, but hasn't been tried on ARM hardware yet.

## Not affiliated with Spotify

Singularity is an independent project. It is **not affiliated with, endorsed by or connected to Spotify**. It needs
a **Spotify Premium** account, and it talks to Spotify through **private, undocumented APIs**, the ones Spotify's own
apps use. That is against Spotify's Terms of Use, and Spotify may change those APIs, and break Singularity, at any
time. Spotify could also restrict or close an account that uses them. **Use it with your own account, at your own
risk.** Singularity never sees your password (you log in on spotify.com) and plays only what your account may play.

## Build and run

Two programs: `singularityd`, the Daemon (it plays the music and keeps playing when you close the terminal), and
`singularity`, the Client (the TUI; open as many as you like). The Client starts the Daemon by itself.

### Linux

Needs: Rust 1.89 or newer (stable, via rustup), a C toolchain, `pkg-config`, the OpenSSL and ALSA development files, PipeWire (or
PulseAudio) with `pactl`/`parec`, and a Secret Service keyring (`secret-tool`: GNOME Keyring, KWallet, KeePassXC).
Arch: `pacman -S rustup base-devel pkgconf openssl alsa-lib libpulse libsecret`; Debian/Ubuntu:
`apt install build-essential pkg-config libssl-dev libasound2-dev pulseaudio-utils libsecret-tools`.

```sh
cargo install --locked --path crates/daemon
cargo install --locked --path crates/client
singularity                         # ~/.cargo/bin on your PATH
```

The same on x86_64 and on ARM (aarch64): the Daemon downloads the Soloist build for your CPU.

(For a quick try without installing: `cargo build --release && ./target/release/singularity`.)

First run: the setup wizard asks for a **Soloist API key** (free, from https://developer.spotify.com/dashboard/soloist
with your Premium account; it goes to the keyring), downloads Soloist from Spotify, and asks you to pick
"Singularity" once in your phone's Spotify device picker (same network). Then "Log in to Spotify" (approve a code in
the browser) for Home, the library and search, and an audio check. Lossless: on the phone, device picker →
Singularity → Change quality settings → Lossless (once).

On a PC shared with other people: Soloist takes its key only on its command line, so other local users can see it
with `ps`. Mounting `/proc` with `hidepid=invisible` hides other users' processes (the health check warns about it too).

Start at login (optional): `cp packaging/systemd/singularityd.service ~/.config/systemd/user/ && systemctl --user
enable --now singularityd`.

### macOS (beta)

Needs macOS 14.2 or later (15 tested, on an M2 Air), Apple's Command Line Tools (`xcode-select --install`), and Rust:
`brew install rustup && rustup default stable` (Homebrew's rustup is keg-only: put
`/opt/homebrew/opt/rustup/bin` and `~/.cargo/bin` on your PATH, e.g. in `~/.zshrc`).

```sh
cargo install --locked --path crates/daemon
cargo install --locked --path crates/client
singularity
```

First run: "Log in to Spotify" (approve a code in the browser). That's all for the default player, librespot: it
plays as your account from inside the Daemon. For lossless, see "Choosing the player" below.

Start at login (optional): `packaging/launchd/install.sh` (a LaunchAgent; `--uninstall` removes it).

### Choosing the player

`[engine] kind` in `~/.config/singularity/config.toml` (or Ctrl+K → Settings → `engine.kind`); restart Singularity
(quit with `q`, start again) after changing it.

| `kind` | Linux | macOS | Lossless | Cost while playing |
|--------|-------|-------|----------|--------------------|
| `auto` (default) | = `soloist` | = `librespot` | Linux yes, macOS no | |
| `soloist` | Spotify's Soloist, headless | Soloist in a small Linux VM that Singularity runs for you | yes | macOS: ~300 MB, idle ~1 % CPU |
| `librespot` | built into the Daemon | built into the Daemon | no (320 kbps at most) | ~28 MB, ~1 % CPU |
| `spotify-app` | the official app, hidden, driven over Connect | the same | yes (the app's own setting) | ~600 MB, 5–25 % CPU |

All of them show up in Spotify's device pickers and are driven the same way; the audio Widgets work with every one
(with the app on macOS, after granting a permission: see below).

### macOS quirks (what's different from Linux, and why)

- **The default plays at most 320 kbps.** Soloist, Spotify's own headless lossless player, is Linux-only, so on a Mac
  the default is librespot (light, no setup), and lossless means `kind = "soloist"` (the VM) or `"spotify-app"`.
- **`kind = "soloist"` runs a Linux VM** (Apple's Virtualization framework, through `vfkit`; networking through
  `gvproxy`). The first start downloads ~250 MB (the two helpers, pinned by checksum, and Debian's arm64 cloud
  image) and sets the VM up in about two minutes; after that it starts in ~6 s. It lives in
  `~/.local/share/singularity/vm/` (~1.5–2 GB on disk) and uses ~300 MB of memory while it runs. It needs a Soloist
  API key (the wizard asks; or `security add-generic-password -U -s singularity -a soloist -w`) and **one pairing
  from your phone on the same Wi-Fi**: pick "Singularity on <your Mac> (lossless)" in the device picker, and set it
  to Lossless there. A quality change applies from the next track.
  - With a VPN on (Mullvad and similar), allow local network sharing, or the phone can't reach the Mac to pair.
  - macOS's firewall silently blocks programs it doesn't trust from accepting connections, Singularity included, so
    while pairing (only then) a tiny relay runs in Apple's own `python3`.
  - Pairing can't be done without the phone: Soloist only accepts a login minted for it by Spotify's apps.
- **`kind = "spotify-app"`** needs Spotify.app installed and logged in to the same account; Singularity starts it
  hidden and leaves it running. For the audio Widgets macOS must let your terminal record other apps' audio: System
  Settings → Privacy & Security → Screen & System Audio Recording → **System Audio Recording Only** → add your
  terminal app. Without it the Widgets stay flat and Singularity says so once.
- **Audiobooks don't play on the default.** Spotify won't give librespot the keys to audiobook chapters, so books
  play with `kind = "soloist"` (the VM) or `"spotify-app"`; with librespot their pages, library rows and search
  still work, and pressing play says so. (Same on Linux with librespot; Linux's default, Soloist, plays them.)
- **Media keys and Control Centre**: with librespot or the VM, Singularity has its own Now Playing entry (play,
  pause, next, previous, drag the position); with the app, the app's own entry. There's no MPRIS (that's D-Bus,
  Linux's).
- **Track notifications** (`[notifications] track_change`) appear through `osascript`, so macOS shows them as Script
  Editor's, without the cover.
- **Secrets** go to the login Keychain through `/usr/bin/security` (so rebuilt binaries read them without a prompt).
- The Daemon's socket is under `$TMPDIR` (no `$XDG_RUNTIME_DIR` on macOS); config and data use the same
  `~/.config/singularity` and `~/.local/share/singularity` paths as Linux.
- The stream Pane's download speed and stall count come from Soloist's log on Linux; the other players don't report
  them.

## Using it

- Ctrl+K → "Health check" shows what's working (player, login, network, audio output, stream quality).
- **Browsing**: Ctrl+K → "Log in to Spotify" (approve the code at spotify.com/pair). Home, the library sidebar and
  track lists appear once logged in.
- **Jam**: `d` opens the device picker; its last row starts a Jam on this device (`j` there too). Friends scan the QR
  or open the link to add songs and skip; Enter on the row twice ends it, and the music keeps playing. Hosting needs
  Premium.
- Keys worth knowing: Space play/pause, Shift+←/→ seek, Ctrl+K palette, Ctrl+L search, `d` devices, `v` swaps the focused Pane's Widget (on the black hole: its Disk look), `w` picks any Widget for it,
  Alt+←/→ back/forward, `l` like, `?` help. Everything is clickable too. `q` (or Esc) quits the Client; the music
  keeps playing.
- **Settings**: Ctrl+K → "Settings", or `~/.config/singularity/config.toml` and `keys.toml` (written with comments on
  first start; saves apply at once). `preset = "vim"` in keys.toml adds h/j/k/l movement.
- **Designs**: Ctrl+K → "Pick a design" (Singularity, Event Horizon, Classic+, Cockpit, Terminal, Karaoke: the line being sung big, a ball bouncing word to word) and "Pick a theme", both with a live
  preview; `t` cycles a design's palettes. Your own themes go in `~/.config/singularity/themes/` (see `_example.toml`).
  If the lyrics or karaoke light up early or late all through a song (Bluetooth headphones lag), Settings →
  `lyrics.delay_ms` shifts them (positive: later).
- **Layouts**: drag the border between two Panes to resize them (it lights up under the pointer, and kitty or
  Ghostty show a resize pointer); `v` (or a Pane's ◂ ▸) swaps its Widget; Ctrl+K → "Save design as…" keeps it, and it
  comes back, colours included, in Ctrl+K → Pick a design.
- **Tiling**: drag a Pane by its title bar onto another Pane and the two Widgets swap places (onto an empty Pane, it
  fills it). While you drag nothing moves: the Pane under the pointer lights up at once and says what letting go does
  ("swap with queue"), and a small chip with the Widget's name follows the pointer. Letting go on its own Pane, off
  the Panes or Esc cancels. Every drop is allowed, whatever the sizes; Panes keep their outlines, and only dragging a
  border, a split or a close changes those. While the chip moves the Client draws at `visuals.motion_fps` (Settings;
  30–240, default 120: set it to your monitor's refresh rate). Your terminal caps it too: Ghostty follows the
  monitor; in kitty lower `repaint_delay` (and keep `sync_to_monitor`); otherwise see your terminal's repaint
  settings. Dragging a title starts **Arrange mode** (so does `m`, Ctrl+K → Arrange, or Arrange in a Pane's menu): the
  **Tray** shows under the player bar (which keeps working, so the music goes on; the Panes give up the Tray's rows
  until Done), each Pane gets ` ⊞ ` (split) and `×` (close), and the bottom line
  shows the keys and **Done** (or Esc). Drop a Widget on the Tray to set it aside (its Pane stays, empty); drag it from
  the Tray onto a Pane to put it there (that Pane's Widget waits in the Tray instead). Closing a Pane sends its Widget
  to the Tray too. `×` on a Widget in the Tray, or Clear Tray, removes them (Home stays); **+ Widget** on the Tray
  opens the Widget picker and puts the pick in the Tray. Done waits until Home has a
  Pane. Each Design keeps its Tray, across restarts; the Tray only shows in Arrange mode. `×` in a
  title closes a Pane. Right-click a Pane's title (or its body, where that has no menu of its own) for its menu:
  Replace widget… opens the Widget picker (type to filter; Widgets not on screen first, those on screen dimmed below a
  line, and picking one swaps the two Panes' Widgets; one too big for the Pane says what it needs), then
  `Split  ←  →  ↑  ↓` (click an arrow; the word Split goes the lit way, the Pane's longer side at first, and ← → move
  it), Close pane, Arrange. A split makes a new empty Pane on that side and opens the picker on it: pick what it
  shows, or Esc to leave it empty (it says so) and arrange; a split and its pick are one undo step. A click on the Widget's
  name in a title, `w` on the focused Pane, or Ctrl+K → Replace widget… open the picker too (Home's Pane keeps Home).
  Ctrl+K also has Split left / right / up / down, Close pane, Arrange and Reset design. In Arrange mode the keyboard
  does the same: arrows move between Panes, Enter picks up the focused Widget (the same lit Pane and chip as a drag),
  arrows aim and Enter drops it there (Esc or `m` puts it back); `t` sends the focused Widget to the Tray, `s` then an
  arrow splits its Pane that way, `x` closes it, `+` (or `=`) and `-` move its border left or right of it, `]` and `[`
  the one above or below it, `w` opens the **+ Widget** picker, and Esc or `m` is Done. ↓ from a bottom Pane goes
  onto the Tray: ← → move along it, `x` (or Delete, Backspace) removes the Widget there (as its `×`; Home stays),
  Enter picks a Widget up, ↑ aims back into the Panes and Enter puts it there (a
  Widget picked up from a Pane and aimed down at the Tray waits there, shown in the Tray's row where it will go);
  after its last Widget (or alone, when it's empty) comes **+ Widget**, where Enter opens the picker. In keys.toml, `move_pane` is another name for
  `arrange`. Ctrl+Z undoes a layout change and
  Ctrl+Y (or Ctrl+Shift+Z) redoes it; the bar offers "undo" for a few seconds after each one. Home can move but never close (the built-in
  Designs also have the library, the queue and lyrics; your own may let them go: Ctrl+K → Your Library / Queue then
  opens them as a popup, and focuses their Pane where they have one), and no border drag, split or close can leave a Pane too small. An edit never interrupts you and never
  changes a built-in Design: it is kept at once as "Singularity (edited)" (outside Arrange mode the bar says so), and
  Done asks what to call it if you changed something while arranging (Esc keeps it unnamed). Ctrl+K → Save design as…
  names one any time, Reset design goes back to the built-in one, and a Design of your own saves itself. Ctrl+K →
  **New design…** starts from a blank slate: Arrange mode with one empty Pane filling the screen and Home in the Tray;
  split it, fill it from the Tray, and Done asks for a name and saves it with the current Theme (Esc before anything is
  placed, Ctrl+D on the name prompt or Ctrl+K → Discard new design goes back to the Design you had, untouched; Esc on
  the prompt keeps building).
- **Idle**: after 3 minutes without input the black hole drifts over the screen; any key wakes it (and does nothing
  else). Shift+I or `[idle]` in config.toml switches it off or changes the delay.
- **Listening log** (private, local): every track you hear for 30 s is logged in `~/.local/share/singularity/`, and
  Liked Songs and your own playlists are backed up there nightly. Cockpit's listening Pane and the Stats page show it;
  Ctrl+K → "Export listening data" / "Delete listening data". Nothing is uploaded.
- After a rebuild, the next Client notices the running Daemon is older than the installed `singularityd` and replaces
  it by itself ("Updating singularityd…"; the music pauses for a moment).
- Terminals: kitty, Ghostty and WezTerm get the pixel tier (covers, the GPU black hole); every other terminal the cell
  tier.

### For development

- `singularityd --fake-engine` runs a pretend player for UI work; `--no-catalogue` skips Spotify browsing;
  `--fake-catalogue crates/client/tests/fixtures/home.json` serves a recorded (anonymised) Home feed (a build with
  `cargo build -p singularityd --features fake`).
- `tools/check.sh` runs what CI runs (tests, clippy, `cargo deny`); on a Mac, `tools/check-linux.sh` type-checks the Linux build (zig, cargo-zigbuild,
  openssl@3: see the script). The pty harness in `tools/pty/` screenshots the real binaries (its README).
- `git config core.hooksPath tools/hooks` turns on the pre-commit hook, which runs clippy (`SKIP_CLIPPY_HOOK=1` skips it).
- `CLAUDE.md` has the working agreements.

## Docs

| Doc | What's in it |
|-----|--------------|
| [docs/SPEC.md](docs/SPEC.md) | The spec: v1 (stories 1–67), v1.x (68–87), implementation and testing decisions, the final audit |
| [docs/DESIGN-TREE.md](docs/DESIGN-TREE.md) | Every open decision, its dependencies, and the grilling round log |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Phased plan, spikes first |
| [docs/research/2026-10-platform-constraints.md](docs/research/2026-10-platform-constraints.md) | What Spotify does and doesn't allow in 2026, with sources |
| [docs/ideas/visuals-and-widgets.md](docs/ideas/visuals-and-widgets.md) | Brainstorm of visuals, widgets and layout modes |
| [docs/ideas/layout.md](docs/ideas/layout.md) | Free tiling: the idea bank it started from (built, D66) |
| [GLOSSARY.md](GLOSSARY.md) | The project's vocabulary (Engine, Catalogue, Shelf, Widget, Pane, …) |
| [docs/adr/](docs/adr/) | Hard-to-reverse decisions: Soloist Engine, private-API Catalogue, Daemon + Client, GPU visuals |
| [docs/research/2026-10-private-api-risk.md](docs/research/2026-10-private-api-risk.md) | How risky the private API is for a one-person client |
| [docs/PLATFORMS.md](docs/PLATFORMS.md) | Linux/macOS parity: every feature and platform seam, and its exact status |
| [docs/research/2026-10-macos.md](docs/research/2026-10-macos.md) | How macOS support came about (the Engine options, the audio tap) |
| [docs/research/2026-10-soloist-vm-spike.md](docs/research/2026-10-soloist-vm-spike.md) | Soloist in a Linux VM on macOS: measurements, pairing, the firewall |
| [docs/research/2026-10-free-tiling.md](docs/research/2026-10-free-tiling.md) | What fully free tiling would take |

The early spikes (`spikes/`) and design prototypes (`prototypes/`) are in the git history, before the first public
release.

## Process

Planning follows Matt Pocock's [skills](https://github.com/mattpocock/skills):
`grill-with-docs` (interview in rounds over a design tree, write glossary + ADRs as we go), then
`to-spec`, then tickets, then implementation. Install them with
`claude plugin install mattpocock-skills@claude-plugins-official`; `docs/agents/` says how they're set up here.

## Licence

MIT: see [LICENSE](LICENSE).
