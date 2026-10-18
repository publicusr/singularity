# Session handoff (2026-10-18)

Read this first in a new session, then `CLAUDE.md`, `docs/SPEC.md` and `GLOSSARY.md`. Decisions and their history are
in `docs/DESIGN-TREE.md` (round log at the bottom); verified facts about Spotify's private APIs and Connect are in
`docs/research/2026-10-library-and-connect.md`.

## Where things stand

- **Phase 0 (planning) ✅, Phase 1 (spikes) ✅.** ADRs 0001–0004 (0004 updated: both tracers kept, measurements).
- **Phase 2/3 (v1 build) 🔄.** Spec issue **#1**, tickets on GitHub (`publicusr/singularity`, label `ready-for-agent`).
  **Closed: #2, #3, #4, #5, #7, #8, #9, #10, #11, #12, #13, #14, #15, #16, #24.**
  **In progress: #22** (Daemon side done, Client side not started; see below).
  **Open: #6, #17, #18, #19, #20, #21, #23.**
- The owner tested after #8/#16 and reported two bugs, both fixed (commit `f79a3e2`): the black hole
  flickered (now double-buffered kitty frames at a steady 30 fps, redraws capped at 60/s) and clicking an album played it
  (Cards now open their track list, ▶ Play there). Everything else since then **hasn't been tested by the owner yet**.
- Tests: **110 passing** (`cargo test`), + ignored benchmarks (`hole_cpu`, `tap_cpu`) and `live_shapes` (needs saved
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

## Next: finish #22 (like/follow)

Done (Daemon, committed as WIP): `Command::SetSaved {uri, saved}` (tracks, albums, playlists, artists) →
`addToLibrary`/`removeFromLibrary` with `{libraryItemUris: [uri], interactionId: null}`; `Command::CheckSaved {uris}` →
`areEntitiesInLibrary {uris}` (`data.lookup[].data.saved`, verified live). `Event::Saved { entries }` is the truth to
settle on; on failure it carries the old value (rollback) plus an `Error`. Liked Songs' Daemon cache is dropped after a
track change; the library is refetched after non-track changes. Fake Catalogue: URIs containing "fail" fail.

To do (Client): `saved: HashMap<String, bool>` in App; optimistic toggle → `SetSaved`, settle on `Event::Saved`;
`CheckSaved` for the current item (on track change), the open list's URI, the artist page; a ♥/♡ in the player bar
(click) and key `l`; "Save to / Remove from Liked Songs" in track menus; Follow/Following on artist pages, Save on
album/playlist headers; patch the cached Liked Songs `TrackList` in place (insert the row on like, remove on unlike).
Tests: Seam 2 (optimistic + rollback with a failing URI), Seam 1 (fake Catalogue round trip). Then pty check, commit
`Closes #22`.

## After that (suggested order)

1. **#21** MPRIS + notifications (media keys, KDE widget, playerctl): `zbus` in the Daemon.
2. **#6** Soloist auto-update before the 90-day expiry (lifetime is already parsed; needs a fake-CDN seam test).
3. **#20** config files (TOML, schema version, migrations, hot reload), then **#18** designs/palettes/theme files and
   **#19** slots/drag/saved layouts (they build on #20).
4. **#17** idle takeover (reuse `visual::BlackHole`, the prototype's `lens_cells`), **#23** listening log + backups.
5. Gap left on #11 (commented on the issue): mouse back/forward buttons. crossterm drops SGR buttons 8/9; needs our
   own small input layer (we already do SGR-Pixels in `client/src/term.rs`).

## Waiting on the owner

- To test everything since the flicker fix: setup wizard/health (`Ctrl+K → Health check`), queue, lyrics, artist and
  album pages + history, search (`Ctrl+L`), devices (`d`).
- Two open questions from earlier (unanswered): keep the black hole on the iGPU (low power) rather than the RX 7900 GRE
  (➡️ yes); keep the 10 fps slowdown when the terminal is unfocused (➡️ yes).
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
> First finish **#22** (like/follow): the Daemon side is done; build the Client side as described in HANDOFF ("Next:
> finish #22"). Then work the frontier in this order: **#21** MPRIS + notifications, **#6** Soloist auto-update, **#20**
> config files, **#18** designs/themes, **#19** layouts, **#17** idle takeover, **#23** listening log + backups. For each
> ticket: implement with tests at the seams named in the spec, run `cargo test`, verify the real binaries with the pty
> harness in `tools/pty/` (screenshots), commit with `Closes #N` plus the co-author line, push, and keep `docs/` current
> (DESIGN-TREE round log, ROADMAP, HANDOFF). Never commit secrets or personal listening data; only kill processes you
> started; never start a second real Daemon against the owner's Soloist. When a decision is genuinely the owner's, ask in
> the grilling format (numbered questions, a ➡️ recommendation each, worded so "yes" accepts). Keep messages to the owner
> concise; they test the builds themselves and give feedback. If the owner reports bugs, fix those first.
