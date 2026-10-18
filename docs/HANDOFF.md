# Session handoff (2026-10-18)

Read this first in a new session, then `CLAUDE.md`, `docs/SPEC.md` and `GLOSSARY.md`. Decisions and their history are
in `docs/DESIGN-TREE.md` (round log at the bottom); verified facts about Spotify's private APIs and Connect are in
`docs/research/2026-10-library-and-connect.md`.

## Where things stand

- **Phase 0 (planning) ✅, Phase 1 (spikes) ✅.** ADRs 0001–0004 (0004 updated: both tracers kept, measurements).
- **Phase 2/3 (v1 build) 🔄.** Spec issue **#1**, tickets on GitHub (`publicusr/singularity`, label `ready-for-agent`).
  **Closed: #2, #3, #4, #5, #6, #7, #8, #9, #10, #11, #12, #13, #14, #15, #16, #18, #19, #20, #21, #22, #24.**
  **Open: #17, #23.**
- The owner tested after #8/#16 and reported two bugs, both fixed (commit `f79a3e2`): the black hole
  flickered (now double-buffered kitty frames at a steady 30 fps, redraws capped at 60/s) and clicking an album played it
  (Cards now open their track list, ▶ Play there). Everything else since then **hasn't been tested by the owner yet**.
- Tests: **162 passing** (`cargo test`), + ignored benchmarks (`hole_cpu`, `tap_cpu`) and `live_shapes` (needs saved
  live answers). Clippy: only minor pre-existing style warnings.

## What exists in `crates/`

| Crate | What it does now |
|-------|------------------|
| `proto` | Versioned length-prefixed JSON protocol (Hello/Welcome, Command, Event; unknown-variant tolerant). Events that carry a list must be struct variants (`Devices { devices }`, `Saved { entries }`): the tagged encoding can't hold a bare sequence (a test guards it). |
| `daemon` (`singularityd`) | Hub + socket. `soloist.rs`: SoloistEngine (download, key from env/keyring, Soloist's output piped and parsed by `LogParser`: stream format, lifetime, bad key, locked data dir; log rotated to `.1`; refuses to touch a running Soloist's files). `catalogue.rs`: login, session, Home, library, track lists, search, artist pages, lyrics, queue edits, devices, saved state, fake Catalogue (`--fake-catalogue <home.json>`). `library.rs`: all Pathfinder parsers. `connect.rs`: ConnectRemote (hidden controller, play-at-offset, `set_queue`, transfer-by-play, remote transport, cluster → devices/remote playback). `tap.rs`: AudioTap. `lyrics.rs`: Spotify + LRCLIB. `secrets.rs`: keyring via `secret-tool` + owner-only file fallback (`SINGULARITY_NO_KEYRING=1` for tests). `health.rs`. Replaced by a newer Client via `Command::Shutdown` (exits 75). |
| `client` (`singularity`) | Layout (Singularity preset), theme tokens, hit-testing, palette (16 rows, scrolls), help, player bar (pixel-precise scrub, links, ⧉ device label), pages with back/forward history: Home (`home.rs`, `draw_feed` is reused), track lists (`tracks.rs`), artist pages (`artist.rs`), search (`search.rs`); sidebar, queue, lyrics, meters (spectrum/loudness/VU), stream stats + setup wizard + health check (`setup.rs`), device picker, context menus (`Target`), covers (`covers.rs`), kitty pixel tier (`gfx.rs`), black hole + gravity well (`visual/`). Replaces an outdated Daemon by itself after a rebuild. |

Dev tools: `cargo run -p singularityd --example probe -- <op> '<vars>' [--raw file]` (prints a Pathfinder answer's
*shape*; `--lyrics <uri>` for lyrics), `--example connect_probe` (devices, `--play-next`, `--remove-queued`).
**pty harness: `tools/pty/`** (see its README; recreate the venv there).

## Local state on the owner's machine (not in git)

Soloist at `~/.local/share/singularity/soloist/`, its state in `…/soloist-state/` (`soloist.log`, `soloist.log.1`).
Soloist key and Spotify login in the keyring (`service=singularity`, accounts `soloist` / `spotify`; the old
`…/catalogue/credentials.json` migrates on the next connect). Covers `~/.cache/singularity/covers`, query cache
`~/.cache/singularity/pathfinder-queries.json`, recent searches `~/.local/state/singularity/recent-searches.json` (0600).
Soloist only accepts its key as `--api-key`, so `ps` shows it (no other way in; noted in the health check).

**Incidents in this session (told the owner):** a test Daemon truncated the running Soloist's log and deleted its
`ws.port` file (both bugs fixed: rotation instead of truncation, pid-lock check before touching anything). Playback
wasn't affected; the port file comes back when Soloist next starts. Live tests on the owner's player were done quietly
(volume 10, paused afterwards, volume restored to 63 last time): play-at-offset, queue add/remove, transfer both ways.

## #22 like/follow (done 2026-10-18)

`client/src/saved.rs` holds the store: optimistic toggles (`SetSaved`), settled by `Event::Saved` (stale answers ignored
while a newer toggle is out), Liked Songs patched in place with a stash for rollbacks. `CheckSaved` goes out for the
playing track (on change), every opened list/artist page, and a menu's target when unknown. The fake Catalogue keeps a
saved set (its fake tracks start liked; "fail" URIs fail). pty scenario: `like`. Not done: hearts on every track-list
row (would need `CheckSaved` for whole playlists; only Liked Songs is known for free).

## #21 MPRIS (done 2026-10-18)

`daemon/src/mpris.rs`; see the DESIGN-TREE round log. Tests: `daemon/tests/mpris.rs` starts its own `dbus-daemon`
(no service activation) and plays playerctl's part; pty scenario `mpris` runs real `playerctl` on a private bus.
**Never test MPRIS on the owner's session bus**: a fake Daemon would show up in their media widget. Notifications
are `notifications.track_change` in config.toml (off by default; palette: "Track-change notifications on / off").
Not verified by me: KDE's media widget itself (needs the real bus); the owner checks it after rebuilding.

## #6 Soloist auto-update (done 2026-10-18)

`daemon/src/update.rs` + the SoloistEngine (`keep_current`, `quiet_moment`, `Exit::Update`, `EngineEvent::Resume` →
`CatCmd::Resume` → `ConnectRemote::resume`). Seam test `daemon/tests/update.rs` (fake CDN over HTTP, stand-in builds);
pty: `tools/pty/update_test.py` (real Daemon + stand-in Soloist + fake CDN, scratch dirs). On the owner's machine the
first check adopts the existing install (unpacked after the CDN's Last-Modified) instead of downloading it again.
The restart-into-the-new-build path is proven only against the stand-in: the real one first happens on the owner's
machine when Spotify publishes the next build (watch `soloist.log` / the Daemon log for "restarting Soloist").

Test knobs (env): `SINGULARITY_SOLOIST_CDN`, `SINGULARITY_SOLOIST_CHECK_SECS`, `SINGULARITY_TAP=off` (a test Daemon
never listens to the owner's Soloist), `SINGULARITY_DBUS=<address>|off`.

## #20 config files (done 2026-10-18)

`crates/config` (load/parse/migrate/set/Watched, DEFAULT_CONFIG/DEFAULT_KEYS), `client/src/keymap.rs` (Chord,
presets, overrides), App: `load_config`, `poll_config` (in `tick`), `typing()`; Daemon: `follow_config` in main.rs.
New settings go into `Config` + `DEFAULT_CONFIG` (a new key with a serde default needs no migration; a rename or a
changed meaning needs a migration in `CONFIG_MIGRATIONS` and a schema bump). The owner's real files appear on their
next start of the new build.

## #18 designs and themes (done 2026-10-18)

See the DESIGN-TREE round log. Picking: Ctrl+K → "Design" / "Theme" (live preview), `t` next palette. pty scenario
`designs`. The listening Pane in Cockpit waits for #23's log. #19 builds on `design::DESIGNS[..].layout` (built-in
layouts) and `Node` (serde-ready) for saved ones in `layouts/`.

## #19 slots and saved layouts (done 2026-10-18)

`layouts.rs` (groups, `next`, save/load), `Node::{slot_names, swap, min_len}`, App: `swap_widget`, `save_layout`,
`use_layout`, the name prompt (`naming`). pty scenario `layouts` (swap, drag, save, a second Client finds it).

## Next (suggested order)

1. **#17** idle takeover (reuse `visual::BlackHole`, the prototype's `lens_cells`), **#23** listening log + backups.
2. Gap left on #11 (commented on the issue): mouse back/forward buttons. crossterm drops SGR buttons 8/9; needs our
   own small input layer (we already do SGR-Pixels in `client/src/term.rs`).

## Waiting on the owner

- To test everything since the flicker fix: setup wizard/health (`Ctrl+K → Health check`), queue, lyrics, artist and
  album pages + history, search (`Ctrl+L`), devices (`d`), like/follow (♡ in the player bar, `l`, menus, Follow), media keys / KDE media widget / playerctl
  (after `cargo build --release`, the Client replaces the running Daemon).
- Answered 2026-10-18: keep the 10 fps slowdown when the terminal is unfocused; keep the black hole on the iGPU by
  default and add a `gpu = "low-power" | "high-performance"` setting in #20 (commented on the issue).
- D17 answered "phone only for now" (Soloist has a remote-quality feature flag, no setter found).

## Working rules (from CLAUDE.md, repeated because they matter)

- One ticket at a time: code + tests at the spec's seams, `cargo test`, verify the real binaries in the pty harness,
  commit with `Closes #N` and the co-author line, push, keep `docs/` current (DESIGN-TREE round log, ROADMAP, HANDOFF).
- Never commit secrets or personal listening data (counts, names, ids from the owner's account included). Fixtures are
  anonymised; raw live answers stay in the session scratchpad.
- Kill only processes you started. **A Client autostarts a real Daemon** if nothing answers on its socket: in tests,
  always start a fake Daemon first, and never run a second real Daemon against the owner's Soloist state.
- Live tests that touch the owner's account or player: read-only by default; anything audible or state-changing only
  quietly, minimally, reverted, and reported.
- Unix socket paths < 108 bytes: `$XDG_RUNTIME_DIR/sgl-…`. Private API: metadata only, no crawling, no retry loops.
- The repo isn't rustfmt-formatted (long lines); don't run `cargo fmt` on whole files.

---

## Prompt for the next session

> You're continuing **Singularity**, a terminal Spotify client in Rust (ratatui) at `~/src/singularity`
> (GitHub: `publicusr/singularity`, private). Start by reading `docs/HANDOFF.md`, then `CLAUDE.md`, `docs/SPEC.md`,
> `GLOSSARY.md`, the ADRs in `docs/adr/`, and `docs/research/2026-10-library-and-connect.md`. We're building v1 ticket by
> ticket from the GitHub issues (`gh issue list -R publicusr/singularity --label ready-for-agent --state open`), following
> Matt Pocock's flow (spec → tickets → implement), with the project's vocabulary.
>
> Work the frontier in this order: **#20**
> config files, **#18** designs/themes, **#19** layouts, **#17** idle takeover, **#23** listening log + backups. For each
> ticket: implement with tests at the seams named in the spec, run `cargo test`, verify the real binaries with the pty
> harness in `tools/pty/` (screenshots), commit with `Closes #N` plus the co-author line, push, and keep `docs/` current
> (DESIGN-TREE round log, ROADMAP, HANDOFF). Never commit secrets or personal listening data; only kill processes you
> started; never start a second real Daemon against the owner's Soloist. When a decision is genuinely the owner's, ask in
> the grilling format (numbered questions, a ➡️ recommendation each, worded so "yes" accepts). Keep messages to the owner
> concise; they test the builds themselves and give feedback. If the owner reports bugs, fix those first.
