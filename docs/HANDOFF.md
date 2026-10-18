# Session handoff (2026-10-18, third session)

Read this first in a new session, then `CLAUDE.md`, `docs/SPEC.md` and `GLOSSARY.md`. Decisions and their history are
in `docs/DESIGN-TREE.md` (round log at the bottom: one entry per ticket, with the details); verified facts about
Spotify's private APIs and Connect are in `docs/research/2026-10-library-and-connect.md`.

## Where things stand

- **Phase 0 (planning) ✅, Phase 1 (spikes) ✅.** ADRs 0001–0004.
- **Phase 2/3 (v1 build) ✅ (pending the owner's test round).** Spec issue **#1**, tickets on GitHub
  (`publicusr/singularity`, label `ready-for-agent`). **All v1 tickets are closed (#2–#24).**
- Tests: **179 passing** (`cargo test`), plus ignored benchmarks (`hole_cpu`, `tap_cpu`) and `live_shapes` (needs saved
  live answers). Clippy: only minor style warnings, all in older code (setup.rs, connect.rs, probe example) or
  harmless (`type_complexity` on theme tables, `needless_range_loop` in a test).
- **Owner test round 1 is in** (2026-10-18): 9 points, findings and proposals in the DESIGN-TREE round log ("Owner
  test round 1"). Answered; tickets **#25–#33** written (`ready-for-agent`), built in that order. Genius (opt-in lyrics fallback) is parked
  pending one more answer.
  Their Daemon was a stale 00:08 build that ignores `Shutdown` (see the round log): restart it before trusting tests.
- The owner last tested after #8/#16 (until round 1 above). **Nothing since then has been tested by the owner** (see "Waiting on the owner").

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
| (last) | #23 ✅ | Listening log + library backups (`daemon/src/store.rs`, `backup.rs`), Cockpit's listening Pane |

## What exists in `crates/`

| Crate | What it does now |
|-------|------------------|
| `proto` | Versioned length-prefixed JSON protocol. Events that carry a list must be struct variants (`Devices { devices }`, `Saved { entries }`). `Health` has `engine_build` (the Soloist build date). |
| `config` (`singularity-config`, new) | `config.toml` / `keys.toml` under `~/.config/singularity/`: written with comments on first run, `schema = 1` + migration chain (`CONFIG_MIGRATIONS`, `rename` keeps comments), `set` edits one value keeping the file, `Watched` notices saves (mtime+size, polled ~1 s), bad values → defaults + reported problems. Re-exports `toml_edit` (0.25, TOML 1.1). Sections: `[ui]` design/theme/transparent/layout, `[notifications]` track_change, `[visuals]` gpu/unfocused_fps, `[idle]` enabled/minutes. |
| `daemon` (`singularityd`) | Hub + socket; `transport()` routes player commands (Engine, or Connect while another device plays). `soloist.rs` SoloistEngine (+ `keep_current` updater, `quiet_moment`, `Exit::Update`, resume after restart). `update.rs` CDN check/stage/verify/install. `mpris.rs` MPRIS player + notifications. `catalogue.rs` (+ `SetSaved`/`CheckSaved`, `CatCmd::Resume`; the fake Catalogue keeps a saved set). `connect.rs` (`resume` = generalised transfer). `tap.rs`, `lyrics.rs`, `secrets.rs`, `health.rs`, `library.rs`. `main.rs` follows config.toml live (`follow_config`). `store.rs` LocalStore (listening log in SQLite, `Recorder`, summary, backups on disk, export, delete), `backup.rs` (nightly, paced `CatCmd::Query`, revision reuse); `serve_with(…, Extras { mpris, store })`. |
| `client` (`singularity`) | Everything from before, plus: `saved.rs` (like/follow), `keymap.rs` (standard/vim presets, user bindings), `design.rs` (five Designs), `theme.rs` (built-in Themes + theme files), `looks.rs` (switching, `t`, pickers with live preview, `themes/` watched), `widgets.rs` (Now playing, levels, listening heatmap, rain, scope, top), `layouts.rs` (Slot groups, saved layouts), `visual/idle.rs` (idle takeover). Layout `Size::Last`, subtree minimums. |

Dev tools: `cargo run -p singularityd --example probe -- <op> '<vars>' [--raw file]`, `--example connect_probe`.
**pty harness: `tools/pty/`** (README there; recreate the venv). Scenarios added this session: `like`, `mpris` (real
`playerctl` on a private D-Bus), `config`, `designs`, `layouts`; plus `update_test.py` (real Daemon + stand-in Soloist
`fake_soloist.py` + fake CDN). Every run gets scratch `XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`XDG_DATA_HOME` under its output
folder. Later: `idle-pixels`/`idle-cells` (#17), `listening` (#23, seeds a scratch log).

**Env knobs:** `SINGULARITY_SOLOIST_CDN`, `SINGULARITY_SOLOIST_CHECK_SECS` (updater), `SINGULARITY_TAP=off` (a test
Daemon never listens to the owner's Soloist), `SINGULARITY_DBUS=<address>|off` (MPRIS bus; fake-Engine Daemons stay
off the session bus unless asked), plus the older `SINGULARITY_CPU`, `SINGULARITY_PIXEL`, `SINGULARITY_PIXEL_MOUSE`,
`SINGULARITY_NO_KEYRING`, `SINGULARITY_SOCKET`. #23: `SINGULARITY_DATA=<dir>|off` (LocalStore; fake-Engine Daemons
keep none unless asked), `SINGULARITY_BACKUP_CHECK_SECS`, `SINGULARITY_IGNORE_SHUTDOWN` (#25 tests), and `XDG_DOWNLOAD_DIR` decides where exports go. `SINGULARITY_NOTIFY` is gone (now `notifications.track_change`).

## Then

- Test round 1 tickets, in order: ~~#25 stale Daemon~~ ✅, #26 wheel/highlight, #27 Home by lines, #28 back/forward,
  #29 meters, #30 lyrics wrap, #31 wide terminals, #32 queue drag, #33 Theme tint. Genius parked.
- The owner's screenshots go in `docs/screenshots/` (git-ignored: personal data; never commit or quote them).
- v1.x (spec "Out of Scope"): playlist editing, stats widgets (the listening log is ready for them), Settings screen,
  more Visuals, free-tiling, podcasts, macOS Engine. Turn into tickets with the owner before building.
- Gap left on #11: mouse back/forward buttons (crossterm drops SGR buttons 8/9; needs our own small input layer next
  to the SGR-Pixels code in `client/src/term.rs`).
- Not done on #22: hearts on every track-list row (would need `CheckSaved` for whole playlists).

## Installed binaries

`singularity` and `singularityd` are installed in `~/.cargo/bin` (on the owner's PATH) with
`cargo install --locked --path crates/daemon` and `--path crates/client`. Reinstall both after each closed ticket so the
owner tests the latest; the Client finds `singularityd` next to itself. Last installed: after #17 (reinstalled after
#23 too, see the last commit).

## Local state on the owner's machine (not in git)

Soloist at `~/.local/share/singularity/soloist/` (from #6 on: `soloist.new/` staged, `soloist.old/` previous,
`build.json` per build; the first check adopts the existing install instead of downloading it again), its state in
`…/soloist-state/`. Keyring `service=singularity` (`soloist`, `spotify`). Caches in `~/.cache/singularity/` (covers,
query cache, `notify-cover.jpg`), recent searches in `~/.local/state/singularity/`. From the next start of a new build:
`~/.config/singularity/{config.toml, keys.toml, themes/_example.toml, layouts/}` appear. From #23: the owner's real
listening log `~/.local/share/singularity/listening.db` and `backups/` (personal data: never read it into the repo or
fixtures).
The owner's running Daemon is still an older release build; the Client replaces it after `cargo build --release`.

## Waiting on the owner

- To test (nothing since the flicker fix has been tested): setup/health, queue, lyrics, artist/album pages + history,
  search, devices; and this session's: like/follow (♡ in the player bar, `l`, menus, Follow, Save), media keys / KDE's
  media widget / `playerctl` (KDE widget not verified by me: it needs the real bus), designs (Ctrl+K → Design / Theme,
  `t`), Slots (`v`, ◂ ▸), "Save layout as…", keys.toml / config.toml live edits, vim preset.
- Answered this session: keep the 10 fps unfocused slowdown (now `visuals.unfocused_fps`); keep the black hole on the
  built-in graphics by default, with `visuals.gpu = "high-performance"` for the RX 7900 GRE (done in #20).
- Answered for #23: all five recommendations (Play = 30 s or half, Connect devices included; backups: owned in full,
  followed by name, ~1 req/s; keep 14 + monthly for a year; export to Downloads, delete asks twice; 26-week heatmap).
- My own calls, open to the owner's veto: in Cockpit's short listening Pane two days share a cell (half-blocks), so
  the weeks read as columns of squares; delete is confirmed by running it again within 10 s (as removing a playlist);
  track notifications are off by default; the vim preset moves like to `f`;
  every Design keeps a `home` Pane (Cockpit and Terminal differ a little from the prototype); the auto-written example
  theme is `_example.toml` (files starting with `_` are skipped).
- Soloist's real update path (restart into a new build, resume over Connect) has only run against the stand-in; the
  first real one happens when Spotify publishes a build (Daemon log: "restarting Soloist into the new build").

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
> (GitHub: `publicusr/singularity`, private). Start by reading `docs/HANDOFF.md`, then `CLAUDE.md`, `docs/SPEC.md`,
> `GLOSSARY.md`, the ADRs in `docs/adr/`, and the end of the round log in `docs/DESIGN-TREE.md`. We're building v1 ticket
> by ticket from the GitHub issues (`gh issue list -R publicusr/singularity --label ready-for-agent --state open`),
> following Matt Pocock's flow (spec → tickets → implement), with the project's vocabulary.
>
> All v1 tickets are closed. The owner is testing the installed build (`singularity` in `~/.cargo/bin`); fix the
> bugs they report first, one at a time: reproduce in a test or the pty harness in `tools/pty/` (screenshots), fix,
> `cargo test`, commit, push, reinstall both binaries (`cargo install --locked --path crates/daemon` and
> `--path crates/client`), and keep `docs/` current (DESIGN-TREE round log, ROADMAP, HANDOFF). For new work (v1.x),
> write tickets with the owner first. Never commit secrets or personal listening data (the owner's
> `~/.local/share/singularity/listening.db` and `backups/` included); only kill processes you started; never start a
> second real Daemon against the owner's Soloist; never test MPRIS on the owner's session bus. When a decision is
> genuinely the owner's, ask in the grilling format (numbered questions, a ➡️ recommendation each, worded so "yes"
> accepts). Keep messages to the owner concise; they test
> the builds themselves and give feedback. If the owner reports bugs, fix those first.
