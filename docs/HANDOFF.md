# Session handoff (2026-10-18, end of the fourth session)

Read this first in a new session, then `CLAUDE.md`, `docs/SPEC.md` and `GLOSSARY.md`. Decisions and their history are
in `docs/DESIGN-TREE.md` (round log at the bottom: one entry per ticket, with the details); verified facts about
Spotify's private APIs and Connect are in `docs/research/2026-10-library-and-connect.md`.

## Where things stand

- **v1 ✅** (spec issue #1, tickets #2–#24). Owner test rounds 1–3 done and fixed (#25–#33, #38–#45).
- **v1.x in progress** (spec: SPEC.md "v1.x", stories 68–86; decisions DESIGN-TREE D29–D45; spec issue **#34**).
  Batches (D29): ~~1 stats~~ ✅ (#35–#37) · ~~2 unbuilt v1 items + polish~~ ✅ (#46 waveform, later removed (D46), #47 mini-player, #49 ♥ rows,
  #50 album song into the queue; #48 mouse buttons dropped, D45) · **3 playlist editing (next)** · 4 Settings screen ·
  5 album-adaptive theme + more Visuals · 6 podcasts and audiobooks · 7 free tiling (trial) · 8 macOS.
  Visual demos judged: starfield, marquee, glow kept and finished (#44, `ambient.rs`).
- Tests: **217 passing** (`cargo test --workspace`), plus ignored benchmarks (`hole_cpu`, `tap_cpu`) and `live_shapes`.
- **No open bugs.** The waveform seek bar (#46) was removed again at the owner's call (D46).
- Rule since round 2 (D43): every built-in Design keeps Home, library, queue and lyrics (a test enforces it).

### Recent commits (all pushed to `main`)

| Commit | Ticket | What |
|--------|--------|------|
| `c019a29` | #22 ✅ | Like/follow, Client side: `client/src/saved.rs` (optimistic, rollback, Liked Songs patched in place) |
| `eb1b487` | #21 ✅ | MPRIS + track notifications in the Daemon (`daemon/src/mpris.rs`, zbus 5.19) |
| `1e44e71` | #6 ✅ | Soloist auto-update (`daemon/src/update.rs`, quiet-moment restart, Connect resume) |
| `395c0ce` | #20 ✅ | Config files and keymap (`crates/config`, `client/src/keymap.rs`) |
| `e470db1` | #18 ✅ | Five designs, palette variants, theme files (`design.rs`, `theme.rs`, `looks.rs`, `widgets.rs`) |
| `043f960` | #19 ✅ | Slots, resize limits, saved layouts (`layouts.rs`) |
| `ea77ac1` | #17 ✅ | Idle takeover (`visual/idle.rs`), App tests, pty `idle-pixels`/`idle-cells` |
| `0cabec8` | #23 ✅ | Listening log + library backups (`daemon/src/store.rs`, `backup.rs`), Cockpit's listening Pane |
| `f02e909` | #25 ✅ | A Daemon that ignores `Shutdown` is stopped (`SO_PEERCRED` pid, SIGTERM/SIGKILL) |
| `502b637` | #26 ✅ | The wheel moves the highlight in track lists and the library |
| `736e385` | #27 ✅ | Home scrolls by lines, part-visible rows (cropped kitty placements) |
| `137497b` | #28 ✅ | ◂ ▸ back/forward in the main Pane's title |
| `db94e22` | #29 ✅ | Loudness and levels follow the real output level (`out_level`, `out_vu`, `out_bands`) |
| `6a55da1` | #30 ✅ | Lyrics wrap; `Lyrics.missing` says why there are none; lookups logged at info |
| `fd62c6a` | #31 ✅ | Wide terminals: the well keeps ~16:10 (`compute_shaped`), track-list columns capped |
| `48915b7` | #32 🔄 | Drag to reorder your queued songs (`MoveInQueue`, `QueueEdit::Move`, `CatEvent::Resync`); issue open until the live check |
| `672a646` | #33 ✅ | Covers and the black hole in the Theme's colours (`theme::Tint`, `ui.tint_art`, config schema 2) |
| `fc63fc0` | #32 ✅ | Drag "Next from" songs too (within their section); queue edits check Singularity is the active Connect device |
| `237dc8f` | #35 ✅ | Stats in the Daemon: `GetStats` → `Event::Stats`, log schema 2 with album/artist links |
| `a4783d9` | #36 ✅ | Stats page (`client/src/stats.rs`, `Page::Stats`); old plays get links via `backfill` |
| `0e7b89e` | #37 ✅ | Widgets "today" and "top this week" (side Slot group); batch 1 done |
| `9d70991` | D36 | Visual demos (`client/src/demos.rs`): starfield, up next, marquee, glow |
| `c07559c` | #38–#41 ✅ | Artist/search wheel, calmer rain, well at 4:3, kitty images under cell backgrounds (popups don't pixelate) |
| `a8a8f95` | #42 ✅ | Every Design keeps Home, library, queue, lyrics (D43) |
| `08ba7a2` | #43 ✅ | All arrows at the right of a Pane's title; back/forward as ← → (D44) |
| `e2232c4` | #44 ✅ | Ambient Widgets (`client/src/ambient.rs`): starfield, marquee, glow; up next dropped; round 2 done |
| `522c2ba` | #41 follow-up | Popups opaque over under-background images in kitty (#010101) |
| `dc59c0c` | #45 ✅ | Stream Pane shows download speed and stalls; one toast when the connection is too slow |
| `79016bd` | #50 ✅ | Drag an album song up into "Next in queue" (preview, `into_queue`) |
| `42591fd` | #49 ✅ | ♥ on every track row, asked about in batches of 50 |
| `b552d97` | #46 ✅ | Waveform seek bar (`client/src/waveform.rs`, cached per track) |
| (last) | #47 ✅ | Mini-player below 50×14 (`client/src/mini.rs`) |

## What exists in `crates/`

| Crate | What it does now |
|-------|------------------|
| `proto` | Versioned length-prefixed JSON protocol. Events that carry a list must be struct variants (`Devices { devices }`, `Saved { entries }`). `Health` has `engine_build` (the Soloist build date). |
| `config` (`singularity-config`, new) | `config.toml` / `keys.toml` under `~/.config/singularity/`: written with comments on first run, `config.toml` at `CONFIG_SCHEMA` 2 (`keys.toml` at `KEYS_SCHEMA` 1) + migration chain (`CONFIG_MIGRATIONS`: 1→2 adds `ui.tint_art`; `rename` keeps comments), `set` edits one value keeping the file, `Watched` notices saves (mtime+size, polled ~1 s), bad values → defaults + reported problems. Re-exports `toml_edit` (0.25, TOML 1.1). Sections: `[ui]` design/theme/transparent/layout/tint_art, `[notifications]` track_change, `[visuals]` gpu/unfocused_fps, `[idle]` enabled/minutes. |
| `daemon` (`singularityd`) | Hub + socket; `transport()` routes player commands (Engine, or Connect while another device plays). `soloist.rs` SoloistEngine (+ `keep_current` updater, `quiet_moment`, `Exit::Update`, resume after restart). `update.rs` CDN check/stage/verify/install. `mpris.rs` MPRIS player + notifications. `catalogue.rs` (+ `SetSaved`/`CheckSaved`, `CatCmd::Resume`; the fake Catalogue keeps a saved set). `connect.rs` (`resume` = generalised transfer). `tap.rs`, `lyrics.rs`, `secrets.rs`, `health.rs`, `library.rs`. `main.rs` follows config.toml live (`follow_config`). `store.rs` LocalStore (listening log in SQLite, `Recorder`, summary, backups on disk, export, delete), `backup.rs` (nightly, paced `CatCmd::Query`, revision reuse); `serve_with(…, Extras { mpris, store })`. Stats: `Store::stats`, log schema 2, `backfill` of old plays' links. `Delivery` (Soloist's download speed + stalls, #45). |
| `client` (`singularity`) | `app.rs` (state, input, actions, pages + history), `ui.rs` (draw, Pane titles, popups; `opaque_overlays` for kitty), `layout.rs` (split tree, `compute_shaped`: the well ≤ 4:3), `home.rs`, `tracks.rs` (lists, ♥ column), `artist.rs`, `search.rs`, `queue.rs` (drag: `preview`, `target`, `into_queue`), `lyrics.rs` (wrap), `stats.rs` (Stats page + "today"/"top this week" Widgets), `ambient.rs` (starfield, marquee, glow), `mini.rs` (mini-player), `saved.rs` (like/follow, `heart`, `want_saved`), `keymap.rs`, `design.rs`, `theme.rs` (Themes, `Tint`, `ramp_at`), `looks.rs`, `widgets.rs`, `layouts.rs` (Slot groups, saved layouts), `setup.rs` (wizard, health, stream Pane), `gfx.rs` (kitty: images under cell backgrounds in kitty), `visual/` (black hole CPU/GPU, well, idle). |

Dev tools: `cargo run -p singularityd --example probe -- <op> '<vars>' [--raw file]`, `--example connect_probe`.
**pty harness: `tools/pty/`** (README lists every scenario; recreate the venv). Runs the real binaries against a fake
Daemon (fake Engine + fake Catalogue), scratch XDG dirs per run, `PTY_COLS`/`PTY_ROWS` for size; its compositor layers
kitty images as kitty does (under/over cell backgrounds, judged by colour). `connect_probe` modes for quiet live checks:
`--swap-context`, `--lift-context` (snapshot → change → read back → restore).

**Env knobs:** `SINGULARITY_SOLOIST_CDN`, `SINGULARITY_SOLOIST_CHECK_SECS` (updater), `SINGULARITY_TAP=off` (a test
Daemon never listens to the owner's Soloist), `SINGULARITY_DBUS=<address>|off` (MPRIS bus; fake-Engine Daemons stay
off the session bus unless asked), plus the older `SINGULARITY_CPU`, `SINGULARITY_PIXEL`, `SINGULARITY_PIXEL_MOUSE`,
`SINGULARITY_NO_KEYRING`, `SINGULARITY_SOCKET`. #23: `SINGULARITY_DATA=<dir>|off` (LocalStore; fake-Engine Daemons
keep none unless asked), `SINGULARITY_BACKUP_CHECK_SECS`, `SINGULARITY_IGNORE_SHUTDOWN` (#25 tests), `SINGULARITY_BELOW_BG=1|0` (#41: kitty's under-background layer), `SINGULARITY_FAKE_STALLS=1` (#45: the fake Engine starves), and `XDG_DOWNLOAD_DIR` decides where exports go. `SINGULARITY_NOTIFY` is gone (now `notifications.track_change`).

## Then

- **Batch 3: playlist editing** (stories 76–79, D32, D40). Write its tickets first (as GitHub issues under #34, label
  `ready-for-agent` + `v1.x`), then build one at a time. Scope (owner): own playlists only; create, rename and
  description, delete (asks twice), "Add to playlist…" from any track menu with a searchable picker, remove tracks,
  drag to reorder (as the queue), **custom covers** (a path or an image dropped on the terminal, previewed, cropped
  square, ≤ 256 KB JPEG). No folder management. The private routes are in `docs/research/2026-10-playlists-and-podcasts.md`
  (Pathfinder `addToPlaylist` / `removeFromPlaylist` / `moveItemsInPlaylist` / `editablePlaylists`; spclient
  `POST /playlist`, `/playlist/{id}/changes`, `/user/{username}/rootlist/changes`; covers via
  `image-upload.spotify.com/v4/playlist` + `register-image`). Re-verify against the current web-player bundle (the
  QueryRegistry caches its operation names in `~/.cache/singularity/pathfinder-queries.json`), and give each write
  path one quiet, reverted live check on a scratch playlist the session creates (and deletes) itself.
- Then batches 4–8 in order; each gets tickets just before it's built.
- The owner's screenshots go in `docs/screenshots/` (git-ignored: personal data; never commit or quote them).

## Installed binaries

`singularity` and `singularityd` are installed in `~/.cargo/bin` (on the owner's PATH) with
`cargo install --locked --path crates/daemon` and `--path crates/client`. Reinstall both after each closed ticket so the
owner tests the latest; the Client finds `singularityd` next to itself. Last installed: after #47.

## Local state on the owner's machine (not in git)

Soloist at `~/.local/share/singularity/soloist/` (from #6 on: `soloist.new/` staged, `soloist.old/` previous,
`build.json` per build; the first check adopts the existing install instead of downloading it again), its state in
`…/soloist-state/`. Keyring `service=singularity` (`soloist`, `spotify`). Caches in `~/.cache/singularity/` (covers,
query cache, `notify-cover.jpg`), recent searches in `~/.local/state/singularity/`. From the next start of a new build:
`~/.config/singularity/{config.toml, keys.toml, themes/_example.toml, layouts/}` appear. From #23: the owner's real
listening log `~/.local/share/singularity/listening.db` and `backups/` (personal data: never read it into the repo or
fixtures).
A new Client replaces an outdated Daemon by itself (and, since #25, stops one that ignores `Shutdown`).

## Waiting on the owner

- Owner calls still standing (open to veto): Cockpit's listening Pane packs two days per cell when short; delete asks
  again within 10 s; track notifications off by default; vim preset moves like to `f`; `_example.toml` theme skipped.
- Soloist's real update path (restart into a new build, resume over Connect) has only run against the stand-in; the
  first real one happens when Spotify publishes a build (Daemon log: "restarting Soloist into the new build").
- Background noise the owner should know: Stremio's torrent server saturated their uplink once (music stalled);
  #45 now says so in-app.

## Working rules (from CLAUDE.md, repeated because they matter)

- One ticket at a time: code + tests at the spec's seams, `cargo test`, verify the real binaries in the pty harness,
  commit with `Closes #N` and the co-author line, push, keep `docs/` current (DESIGN-TREE round log, ROADMAP, HANDOFF).
- Never commit secrets or personal listening data. Fixtures are anonymised; raw live answers stay in the scratchpad.
- Kill only processes you started. **A Client autostarts a real Daemon** if nothing answers on its socket: in tests,
  always start a fake Daemon first, and never run a second real Daemon against the owner's Soloist state.
- Never test MPRIS on the owner's session bus (private `dbus-daemon`, no activation, as the tests and harness do).
- Live tests that touch the owner's account or player: read-only by default; anything audible or state-changing only
  quietly, minimally, reverted, and reported.
- Unix socket paths < 108 bytes: `$XDG_RUNTIME_DIR/sgl-…` (also for private D-Bus sockets). Private API: metadata only,
  no crawling, no retry loops. Prefer dependency versions that are a few weeks old (zbus 6.0 was skipped for 5.19).
- The repo isn't rustfmt-formatted (long lines); don't run `cargo fmt` on whole files.

---

## Prompt for the next session

> You're continuing **Singularity**, a terminal Spotify client in Rust (ratatui) at `~/src/singularity`
> (GitHub: `publicusr/singularity`, private). Read `docs/HANDOFF.md` first, then `CLAUDE.md`, `docs/SPEC.md` (v1 and
> the "v1.x" part), `GLOSSARY.md`, the ADRs in `docs/adr/`, and the end of the round log in `docs/DESIGN-TREE.md`.
>
> v1 is done; v1.x is being built in batches (D29). Batches 1 (stats) and 2 (waveform, mini-player, ♥ rows, album song
> into the queue) are done and owner-tested. **Next is batch 3, playlist editing**: first write its tickets under spec
> issue #34 (scope and private API routes in HANDOFF "Then"), re-verify the routes against the current web-player
> bundle, then build one ticket at a time: code + tests at the spec's seams, `cargo test --workspace`, verify the real
> binaries in the pty harness (`tools/pty/`, screenshots), commit with `Closes #N`, push, reinstall both binaries
> (`cargo install --locked --path crates/daemon` and `--path crates/client`), and keep `docs/` current (DESIGN-TREE
> round log, HANDOFF, ROADMAP, GLOSSARY). Each write to Spotify gets one quiet, reverted live check on a scratch
> playlist you create and delete yourself.
>
> If the
> owner reports bugs, fix those first. Product decisions go to the owner in the grilling format (numbered questions,
> a ➡️ recommendation each, worded so "yes" accepts); facts you look up yourself. Never commit secrets or personal
> listening data (`~/.local/share/singularity/listening.db`, `backups/`, `docs/screenshots/`); only kill processes you
> started; never start a second real Daemon against the owner's Soloist; never test MPRIS on the owner's session bus.
> Keep messages to the owner short; they test the builds themselves.
