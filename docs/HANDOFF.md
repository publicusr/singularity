# Session handoff (2026-10-18, second session)

Read this first in a new session, then `CLAUDE.md`, `docs/SPEC.md` and `GLOSSARY.md`. Decisions and their history are
in `docs/DESIGN-TREE.md` (round log at the bottom: one entry per ticket, with the details); verified facts about
Spotify's private APIs and Connect are in `docs/research/2026-10-library-and-connect.md`.

## Where things stand

- **Phase 0 (planning) ✅, Phase 1 (spikes) ✅.** ADRs 0001–0004.
- **Phase 2/3 (v1 build) 🔄.** Spec issue **#1**, tickets on GitHub (`publicusr/singularity`, label `ready-for-agent`).
  **Closed: #2–#16, #18, #19, #20, #21, #22, #24.**
  **In progress: #17** idle takeover (WIP commit, see below). **Open: #23** listening log + backups.
- Tests: **164 passing** (`cargo test`), plus ignored benchmarks (`hole_cpu`, `tap_cpu`) and `live_shapes` (needs saved
  live answers). Clippy: only minor style warnings, all in older code (setup.rs, connect.rs, probe example) or
  harmless (`type_complexity` on theme tables, `needless_range_loop` in a test).
- The owner last tested after #8/#16. **Nothing since then has been tested by the owner** (see "Waiting on the owner").

### This session's commits (all pushed to `main`)

| Commit | Ticket | What |
|--------|--------|------|
| `c019a29` | #22 ✅ | Like/follow, Client side: `client/src/saved.rs` (optimistic, rollback, Liked Songs patched in place) |
| `eb1b487` | #21 ✅ | MPRIS + track notifications in the Daemon (`daemon/src/mpris.rs`, zbus 5.19) |
| `1e44e71` | #6 ✅ | Soloist auto-update (`daemon/src/update.rs`, quiet-moment restart, Connect resume) |
| `395c0ce` | #20 ✅ | Config files and keymap (`crates/config`, `client/src/keymap.rs`) |
| `e470db1` | #18 ✅ | Five designs, palette variants, theme files (`design.rs`, `theme.rs`, `looks.rs`, `widgets.rs`) |
| `043f960` | #19 ✅ | Slots, resize limits, saved layouts (`layouts.rs`) |
| (last) | #17 🔄 | Idle takeover, WIP: code in, App-level tests and pty check still to do |

## What exists in `crates/`

| Crate | What it does now |
|-------|------------------|
| `proto` | Versioned length-prefixed JSON protocol. Events that carry a list must be struct variants (`Devices { devices }`, `Saved { entries }`). `Health` has `engine_build` (the Soloist build date). |
| `config` (`singularity-config`, new) | `config.toml` / `keys.toml` under `~/.config/singularity/`: written with comments on first run, `schema = 1` + migration chain (`CONFIG_MIGRATIONS`, `rename` keeps comments), `set` edits one value keeping the file, `Watched` notices saves (mtime+size, polled ~1 s), bad values → defaults + reported problems. Re-exports `toml_edit` (0.25, TOML 1.1). Sections: `[ui]` design/theme/transparent/layout, `[notifications]` track_change, `[visuals]` gpu/unfocused_fps, `[idle]` enabled/minutes. |
| `daemon` (`singularityd`) | Hub + socket; `transport()` routes player commands (Engine, or Connect while another device plays). `soloist.rs` SoloistEngine (+ `keep_current` updater, `quiet_moment`, `Exit::Update`, resume after restart). `update.rs` CDN check/stage/verify/install. `mpris.rs` MPRIS player + notifications. `catalogue.rs` (+ `SetSaved`/`CheckSaved`, `CatCmd::Resume`; the fake Catalogue keeps a saved set). `connect.rs` (`resume` = generalised transfer). `tap.rs`, `lyrics.rs`, `secrets.rs`, `health.rs`, `library.rs`. `main.rs` follows config.toml live (`follow_config`). |
| `client` (`singularity`) | Everything from before, plus: `saved.rs` (like/follow), `keymap.rs` (standard/vim presets, user bindings), `design.rs` (five Designs), `theme.rs` (built-in Themes + theme files), `looks.rs` (switching, `t`, pickers with live preview, `themes/` watched), `widgets.rs` (Now playing, levels, listening placeholder, rain, scope, top), `layouts.rs` (Slot groups, saved layouts), `visual/idle.rs` (#17 WIP). Layout `Size::Last`, subtree minimums. |

Dev tools: `cargo run -p singularityd --example probe -- <op> '<vars>' [--raw file]`, `--example connect_probe`.
**pty harness: `tools/pty/`** (README there; recreate the venv). Scenarios added this session: `like`, `mpris` (real
`playerctl` on a private D-Bus), `config`, `designs`, `layouts`; plus `update_test.py` (real Daemon + stand-in Soloist
`fake_soloist.py` + fake CDN). Every run now gets scratch `XDG_CONFIG_HOME`/`XDG_STATE_HOME` under its output folder.

**Env knobs:** `SINGULARITY_SOLOIST_CDN`, `SINGULARITY_SOLOIST_CHECK_SECS` (updater), `SINGULARITY_TAP=off` (a test
Daemon never listens to the owner's Soloist), `SINGULARITY_DBUS=<address>|off` (MPRIS bus; fake-Engine Daemons stay
off the session bus unless asked), plus the older `SINGULARITY_CPU`, `SINGULARITY_PIXEL`, `SINGULARITY_PIXEL_MOUSE`,
`SINGULARITY_NO_KEYRING`, `SINGULARITY_SOCKET`. `SINGULARITY_NOTIFY` is gone (now `notifications.track_change`).

## #17 idle takeover: where it stopped

Done (committed as WIP, builds, all tests pass):
- `visual/idle.rs`: `place` (break out from the hole's Pane over 4 s, then a Lissajous drift; no hole on screen → from
  the top-right corner), `draw` (dims the body, `lens_cells` ported from the prototype, then the hole: kitty image
  id 2 in the pixel tier, half-blocks where the disk is lit otherwise), `empty_well`. Unit tests for both.
- `BlackHole::free_frame` (any centre, given shadow radius, no stars); `Frame.center` in the CPU tracer and GPU params
  (the WGSL already had it); `blackhole::reach`.
- App: `last_input`, `idle_since`, `idle_preview`, `hole_origin` (set by the well and the plain hole each frame);
  `tick` starts the takeover after `idle.minutes`; `wake()` at the top of `key`, `pointer` and `paste` swallows the
  waking input; `Action::ToggleIdle` (Shift+I, writes `idle.enabled`), `Action::IdlePreview` (palette). The well shows
  "· the well is empty ·" while its hole is out. Help lists Shift+I. Config `[idle] enabled = true, minutes = 3`.

To do:
1. App-level tests (in `app.rs`'s test module, which can set `last_input` back): the waking key/click is not acted on
   (e.g. `n` sends no `Next`, then a second `n` does); Shift+I writes `idle.enabled = false` and a restarted App
   doesn't idle; the well reads "the well is empty" while idle.
2. pty check: a scenario `idle` (palette → "Idle takeover: show it now", screenshots at 0.5 s / 5 s in the pixel tier
   and the cell tier, then a key wakes it). Check the hole image sits under the lensed text (z=-1) and is removed on wake.
3. README line, DESIGN-TREE entry → done, ROADMAP, then commit `Closes #17`.

## Then

- **#23** local listening log + nightly library backups (SQLite under the XDG data dir, owner-only, export/delete; spec
  "LocalStore"). Fill Cockpit's `listening` Pane (`widgets::listening`) with the heatmap from it.
- Gap left on #11: mouse back/forward buttons (crossterm drops SGR buttons 8/9; needs our own small input layer next
  to the SGR-Pixels code in `client/src/term.rs`).
- Not done on #22: hearts on every track-list row (would need `CheckSaved` for whole playlists).

## Local state on the owner's machine (not in git)

Soloist at `~/.local/share/singularity/soloist/` (from #6 on: `soloist.new/` staged, `soloist.old/` previous,
`build.json` per build; the first check adopts the existing install instead of downloading it again), its state in
`…/soloist-state/`. Keyring `service=singularity` (`soloist`, `spotify`). Caches in `~/.cache/singularity/` (covers,
query cache, `notify-cover.jpg`), recent searches in `~/.local/state/singularity/`. From the next start of a new build:
`~/.config/singularity/{config.toml, keys.toml, themes/_example.toml, layouts/}` appear.
The owner's running Daemon is still an older release build; the Client replaces it after `cargo build --release`.

## Waiting on the owner

- To test (nothing since the flicker fix has been tested): setup/health, queue, lyrics, artist/album pages + history,
  search, devices; and this session's: like/follow (♡ in the player bar, `l`, menus, Follow, Save), media keys / KDE's
  media widget / `playerctl` (KDE widget not verified by me: it needs the real bus), designs (Ctrl+K → Design / Theme,
  `t`), Slots (`v`, ◂ ▸), "Save layout as…", keys.toml / config.toml live edits, vim preset.
- Answered this session: keep the 10 fps unfocused slowdown (now `visuals.unfocused_fps`); keep the black hole on the
  built-in graphics by default, with `visuals.gpu = "high-performance"` for the RX 7900 GRE (done in #20).
- My own calls, open to the owner's veto: track notifications are off by default; the vim preset moves like to `f`;
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
> First finish **#17** (idle takeover): the code is in as a WIP commit; add the App-level tests and the pty check listed
> in HANDOFF ("#17 idle takeover: where it stopped"), then close it. Then **#23** (local listening log + library backups,
> and fill Cockpit's listening Pane). For each ticket: implement with tests at the seams named in the spec, run
> `cargo test`, verify the real binaries with the pty harness in `tools/pty/` (screenshots), commit with `Closes #N` plus
> the co-author line, push, and keep `docs/` current (DESIGN-TREE round log, ROADMAP, HANDOFF). Never commit secrets or
> personal listening data; only kill processes you started; never start a second real Daemon against the owner's Soloist;
> never test MPRIS on the owner's session bus. When a decision is genuinely the owner's, ask in the grilling format
> (numbered questions, a ➡️ recommendation each, worded so "yes" accepts). Keep messages to the owner concise; they test
> the builds themselves and give feedback. If the owner reports bugs, fix those first.
