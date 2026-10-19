# Singularity

A terminal Spotify client for Linux and macOS that feels like the desktop app, plays lossless audio,
and looks spectacular: a Home feed, Made For You, full library, mouse *and* keyboard, btop-grade themes, and
built-in audio-reactive visuals (yes, including a black hole).

**Status:** building v1 (most of the core works: Home, library, search, artist/album pages, queue, lyrics, devices, visuals). Spec in `docs/SPEC.md`; v1 tickets are GitHub issues (#2–#24).
Product code lives in `crates/`; throwaway code in `prototypes/` and `spikes/`.

## Try it

```sh
cargo build --release
./target/release/singularity        # starts the Daemon (singularityd) automatically
```

Linux and macOS are both supported, with the same features (`docs/PLATFORMS.md` has the exact status, #63–#70).
On Linux the build needs ALSA's development files (Arch: `alsa-lib`; Debian/Ubuntu: `libasound2-dev`). On macOS: Rust
from rustup (`brew install rustup && rustup default stable`). Developing on a Mac, `tools/check-linux.sh` type-checks
the Linux build.

What plays the music is `[engine] kind` in `~/.config/singularity/config.toml` (or the Settings page), read when the
Daemon starts:

| `kind` | What | Lossless | Notes |
|--------|------|----------|-------|
| `auto` (default) | Soloist on Linux, the Spotify app on macOS | yes | |
| `soloist` | Spotify Soloist, headless (Linux only) | yes | needs a Soloist API key |
| `spotify-app` | the official app, started hidden and driven over Connect | yes | ~600 MB while playing |
| `librespot` | built into the Daemon, both OSes | no, 320 kbps | the lightest (~26 MB) |

After a rebuild, the next Client notices the running Daemon is older than `target/release/singularityd` and replaces
it by itself ("Updating singularityd…"; music pauses for a second while Soloist restarts).

- **First run**: a setup wizard asks for your Soloist API key (stored in the keyring), downloads Soloist, and walks you
  through pairing from your phone, the Spotify login and an audio check. Ctrl+K → "Health check" shows what's working.
- **Browsing**: Ctrl+K → "Log in to Spotify" (approve the code at spotify.com/pair). Home, the library sidebar and
  track lists appear once logged in.
- Keys worth knowing: Space play/pause, Shift+←/→ seek, Ctrl+K palette, Ctrl+L search, `d` devices, `v` Disk look,
  Alt+←/→ back/forward, `l` like, `?` help. Everything is clickable too.
- **Settings**: `~/.config/singularity/config.toml` and `keys.toml` (written with comments on first start; saves
  apply at once). `preset = "vim"` in keys.toml adds h/j/k/l movement.
- **Designs**: Ctrl+K → "Design" (Singularity, Event Horizon, Classic+, Cockpit, Terminal) and "Theme", both with a live
  preview; `t` cycles a design's palettes. Your own themes go in `~/.config/singularity/themes/` (see `_example.toml`).
- **Layouts**: drag Pane borders; `v` (or a Pane's ◂ ▸) swaps its Widget; Ctrl+K → "Save layout as…" keeps it, and it
  comes back in Ctrl+K → Design.
- **Idle**: after 3 minutes without input the black hole drifts over the screen; any key wakes it (and does nothing
  else). Shift+I or `[idle]` in config.toml switches it off or changes the delay.
- **Listening log** (private, local): every track you hear for 30 s is logged in `~/.local/share/singularity/`, and
  Liked Songs and your own playlists are backed up there nightly. Cockpit's listening Pane shows it; Ctrl+K → "Export
  listening data" / "Delete listening data". Nothing is uploaded.
- Media keys, KDE's media widget and `playerctl` control the Daemon (MPRIS); Soloist updates itself before its build
  expires.
- `singularityd --fake-engine` runs a pretend player for UI work; `--no-catalogue` skips Spotify browsing;
  `--fake-catalogue crates/client/tests/fixtures/home.json` serves a recorded (anonymised) Home feed.
- Music keeps playing when you quit the Client; `systemctl --user` unit in `packaging/systemd/`.

## Docs

| Doc | What's in it |
|-----|--------------|
| [docs/HANDOFF.md](docs/HANDOFF.md) | Current state, owner feedback, next tickets, prompt for the next session |
| [docs/SPEC.md](docs/SPEC.md) | v1 spec: problem, solution, 67 user stories, implementation and testing decisions |
| [docs/DESIGN-TREE.md](docs/DESIGN-TREE.md) | Every open decision, its dependencies, and the grilling round log |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Phased plan, spikes first |
| [docs/research/2026-10-platform-constraints.md](docs/research/2026-10-platform-constraints.md) | What Spotify does and doesn't allow in 2026, with sources |
| [docs/ideas/visuals-and-widgets.md](docs/ideas/visuals-and-widgets.md) | Brainstorm of visuals, widgets and layout modes |
| [GLOSSARY.md](GLOSSARY.md) | The project's vocabulary (Engine, Catalogue, Shelf, Widget, Pane, …) |
| [docs/adr/](docs/adr/) | Hard-to-reverse decisions: Soloist Engine, private-API Catalogue, Daemon + Client, GPU visuals |
| [docs/research/2026-10-private-api-risk.md](docs/research/2026-10-private-api-risk.md) | How risky the private API is for a one-person client |
| [docs/research/2026-10-macos.md](docs/research/2026-10-macos.md) | How macOS support works (official app as Engine) |
| [docs/PLATFORMS.md](docs/PLATFORMS.md) | Linux/macOS parity: every platform seam and its status |
| [docs/research/2026-10-free-tiling.md](docs/research/2026-10-free-tiling.md) | What fully free tiling would take |
| [spikes/soloist-control/](spikes/soloist-control/) | Spike 1: run, pair, control and tap Soloist (walkthrough inside) |
| [prototypes/design-directions/](prototypes/design-directions/) | Five runnable design directions with a GPU black hole (`cargo run --release`) |

## Process

Planning follows Matt Pocock's [skills](https://github.com/mattpocock/skills):
`grill-with-docs` (interview in rounds over a design tree, write glossary + ADRs as we go), then
`to-spec`, then tickets, then implementation. The grilling skills are vendored in `.claude/skills/`,
so `/grill-with-docs` works in any Claude Code session opened here.
