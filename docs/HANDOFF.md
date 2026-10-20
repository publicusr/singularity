# Session handoff (2026-10-20, end of the first macOS session)

Read this first in a new session, then `CLAUDE.md`, `docs/SPEC.md` and `GLOSSARY.md`. Decisions and their history are
in `docs/DESIGN-TREE.md` (round log at the bottom: one entry per ticket, with the details); verified facts about
Spotify's private APIs and Connect are in `docs/research/2026-10-library-and-connect.md`.

## Where things stand

- **v1 ✅** (spec issue #1, tickets #2–#24). Owner test rounds 1–3 done and fixed (#25–#33, #38–#45).
- **v1.x in progress** (spec: SPEC.md "v1.x", stories 68–87; decisions DESIGN-TREE D29–D56; spec issue **#34**).
  Batches (D29): ~~1 stats~~ ✅ · ~~2 polish~~ ✅ · ~~3 playlist editing~~ ✅ · ~~4 Settings~~ ✅ · ~~5~~ dropped (D47) ·
  6 podcasts and audiobooks (#59–#61 ✅, **#62 audiobooks built 2026-10-20**, open for the owner's check) · ~~7 free tiling~~ parked as an idea (D59) · **8 macOS (built; owner's
  live checks pending, see below)** · 9 the codebase audit (#71, SPEC "Batch 9", last of all).
- **macOS (batch 8, D48–D56)**: the owner works on an M2 Air (macOS 15.7) for a while. Rule D49: Linux and macOS keep
  the same features; OS code only behind the platform seams; **`docs/PLATFORMS.md` is the exact status**, feature by
  feature, and the README has build/run steps and the macOS quirks for users.
  - ~~#63 builds and tests green~~ ✅ · ~~#64 Keychain~~ ✅ · ~~#66 setup and health per Engine~~ ✅ · ~~#69 LaunchAgent~~ ✅
  - **#65 AppEngine** (Spotify.app over Connect; built, network-change fixes and "Restart the player" await the owner)
  - **#70 librespot Engine** (built, the macOS default since D54; a dead-session watchdog added 2026-10-20 after the
    owner's Daemon lost Spotify while idle; playback, wifi change, media keys await the owner)
  - **#68 Now Playing, media keys, notifications** (built; the owner's media-key / Control Centre check pending)
  - **#72 Soloist in a Linux VM** (`kind = "soloist"` on macOS, D55: built and live-checked with the owner: paired,
    FLAC on the speakers, idle ~1 % CPU; the paired VM sits in the owner's data dir, their config still says `auto`)
  - **#67 audio tap on Spotify.app** (built; needs the owner's terminal allowed under "System Audio Recording Only"
    and the app playing to be checked)
- Engines now: Soloist (Linux; macOS in the VM), librespot (both), the Spotify app (both), the fake one. `auto` =
  Soloist on Linux, librespot on macOS (D56). Costs measured on the M2: librespot ~28 MB / ~1 % playing; the VM
  ~300 MB / ~1 % idle; Spotify.app ~600 MB / 5–25 %.
- Tests: **273 passing** on macOS (`cargo test --workspace`), plus ignored benchmarks (`hole_cpu`, `tap_cpu`),
  `live_shapes` and `secrets_live`. `tools/check-linux.sh` type-checks Linux from the Mac: clean. **Nothing since the
  move to the Mac has run on Linux**: first thing back there, `cargo test --workspace`, the pty scenarios, and a
  session per Engine (Linux builds now need `alsa-lib`).
- **No known open bugs.** (2026-10-20: the #62 live check was meant to be silent but played ~4 s at 40 %: Soloist's
  updater restarted it right then and the volume asked for was lost. Fixed in `1ce954b`; the owner was told.) Fixed this session from owner reports: the AppEngine lost Connect after a wifi change;
  the audio Widgets kept moving while paused; the volume snapped back after a mouse drag (Connect devices report it
  ~0.5 s late: the Client now holds what it set); librespot never noticed a connection closed while idle.
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
| `474ea21` | #47 ✅ | Mini-player below 50×14 (`client/src/mini.rs`) |
| (session 5) | D46 | Waveform seek bar removed (owner) |
| `8a04123` | #51 ✅ | Create/delete own playlists (`daemon/src/playlists.rs`, `client/src/playlists.rs`, `examples/playlist_probe.rs`) |
| `a856151` | #52 ✅ | Edit details… (name + description) on own playlists; descriptions shown in the list header |
| `db1a8a3` | #53 ✅ | Add to playlist… picker (every track menu, Ctrl+K for the playing song); duplicates ask first |
| `790241a` | #54 ✅ | Remove from this playlist (menu, Delete key), by row uid; MPRIS seek test no longer flaky |
| `7d5ae96` | #55 ✅ | Drag rows of your own playlist (preview), Alt+↑↓; moves anchored to a neighbour's uid |
| `9ea047f` | fix | Add to playlist picker: the wheel moves its highlight (owner) |
| `08d3734` | #56 ✅ | Change cover… / Remove cover (`client/src/cover.rs`): drop or type a path, square ≤ 256 KB JPEG preview, upload |
| `25df5a4` | owner | Library sidebar: a miniature cover on every row that has one (`sidebar.rs`) |
| `65b5e50` | owner | The wheel over the queue moves its highlight a song at a time, the list follows |
| `f8251bb` | #57 ✅ | Settings page from config.toml (`client/src/settings.rs`, `singularity_config::{SETTINGS, entries}`) |
| `67aaf11` | #58 ✅ | Settings: a read-only keys tab; batch 4 done |
| `f964997` | #59 ✅ | Show pages and episodes (`library::{episode_row, parse_show_header, parse_episode}`, `MoreTracks`, `FindShow`) |
| `bef0189` | #60 ✅ | Saved shows in the library, Follow on a show page |
| `2a19ad2` | #61 ✅ | Podcast episodes in search (Podcasts chip), Go to show |
| `34f8dc0` | #63 ✅ | macOS builds and tests green; pty harness on macOS (five drifted scenarios fixed); `tools/check-linux.sh`; plan D48–D51 |
| `4bcfce0` | #64 ✅ | Secrets in the Keychain via `/usr/bin/security` (no prompt after rebuilds) |
| `f38a8cf` | #65 🔄 | AppEngine (`daemon/src/app_engine.rs`): Spotify.app over Connect; `[engine]` in config.toml (schema 3) |
| (last) | #66 ✅ | Wizard and health per Engine (`Welcome.engine`: `EngineKind`); `SINGULARITY_ENGINE`, `SINGULARITY_START_APP` |
| `b3e59f1` | #65 | AppEngine survives network changes (re-register on a new dealer connection id, wait for the device, start the app on demand); "Restart the player" |
| `99fa5c4` | #70 | librespot Engine on both OSes (`daemon/src/librespot.rs`, `tap::Feed`); PLATFORMS re-audited; SPEC batch 9 (#71) |
| `92afd72` | D54 | librespot is the macOS default |
| `b2b5fb6` | #68 | Now Playing, media keys, notifications on macOS (`daemon/src/nowplaying.rs`; main thread runs a CFRunLoop) |
| `e703973` | #69 ✅ | LaunchAgent (`packaging/launchd/`), install steps |
| `12a9e47` | owner | Audio Widgets stop when paused (`Analyzer::silence`, `Meters::expire`) |
| `ffc40b5` | #72 | Soloist in a Linux VM (`vm.rs`, `soloist_vm.rs`, `pcm.rs`) |
| `0da0466` | #72, owner | The VM idles at ~1 % (pipe sink over vsock, 1 vCPU); the volume holds after a drag |
| `d101b01` | #72 | Soloist starts at its last volume; `SINGULARITY_SOLOIST=local` for the pty harness |
| `db4c150` | #67, #70 | Process tap on Spotify.app (`process_tap.rs`, `tap::Resampler`); librespot dead-session watchdog |
| `1ce954b` | #62, fix | Live check: Soloist plays chapters, librespot is refused their keys (`examples/chapter_probe`); D57, D58. SoloistEngine keeps the volume asked for across restarts |
| `2d496bf` | #62 | Audiobooks: book pages (`queryBookChapters`, `ListHeader.book`), Audiobooks chip, library rows, librespot says what plays them |

## What exists in `crates/`

| Crate | What it does now |
|-------|------------------|
| `proto` | Versioned length-prefixed JSON protocol. Events that carry a list must be struct variants (`Devices { devices }`, `Saved { entries }`). `Health` has `engine_build` (the Soloist build date). |
| `config` (`singularity-config`, new) | `config.toml` / `keys.toml` under `~/.config/singularity/`: written with comments on first run, `config.toml` at `CONFIG_SCHEMA` 3 (`keys.toml` at `KEYS_SCHEMA` 1) + migration chain (`CONFIG_MIGRATIONS`: 1→2 adds `ui.tint_art`, 2→3 adds `[engine]` kind/device; `rename` keeps comments; `EngineKind::resolved` says what `auto` means per OS), `set` edits one value keeping the file, `Watched` notices saves (mtime+size, polled ~1 s), bad values → defaults + reported problems. Re-exports `toml_edit` (0.25, TOML 1.1). Sections: `[ui]` design/theme/transparent/layout/tint_art, `[notifications]` track_change, `[visuals]` gpu/unfocused_fps, `[idle]` enabled/minutes. `SETTINGS` (key + `Kind`) and `entries(text)` feed the Settings page (#57): a new setting needs a row there too. |
| `daemon` (`singularityd`) | Hub + socket; `transport()` routes player commands (Engine, or Connect while another device plays). `soloist.rs` SoloistEngine (+ `keep_current` updater, `quiet_moment`, `Exit::Update`, resume after restart). `update.rs` CDN check/stage/verify/install. `mpris.rs` MPRIS player + notifications. `catalogue.rs` (+ `SetSaved`/`CheckSaved`, `CatCmd::Resume`; the fake Catalogue keeps a saved set). `connect.rs` (`resume` = generalised transfer). `tap.rs`, `lyrics.rs`, `secrets.rs`, `health.rs`, `library.rs`. `main.rs` follows config.toml live (`follow_config`). `store.rs` LocalStore (listening log in SQLite, `Recorder`, summary, backups on disk, export, delete), `backup.rs` (nightly, paced `CatCmd::Query`, revision reuse); `serve_with(…, Extras { mpris, store })`. Stats: `Store::stats`, log schema 2, `backfill` of old plays' links. `Delivery` (Soloist's download speed + stalls, #45). **Engines** (`engine.rs`: `Engine` trait, `Identity` = the Engine's Connect name/`via_connect`/`player`): `soloist.rs` (+ a VM host on macOS: `soloist_vm.rs` → `vm.rs` (vfkit + gvproxy + Debian, cloud-init setup, SSH, forwards) and `pcm.rs` (vsock PCM → cpal + tap)), `librespot.rs` (Spirc + Player in-process, `Tee` sink → cpal + `tap::Feed`, watchdog), `app_engine.rs` (Spotify.app over Connect). Engines driven over Connect go through `CatCmd::Engine` and get their state from the cluster (`catalogue::app_engine_events`). `tap.rs`: `TapSource` PipeWire(`StreamOf`) / Feed / App (macOS process tap, `process_tap.rs`) / Synthetic; `Analyzer::silence` keeps bars falling while paused. `nowplaying.rs` (macOS Now Playing + remote commands; `TrackNews` shared with MPRIS). `secrets.rs` per OS. `health.rs` audio output per OS. `main.rs` picks the Engine (`SINGULARITY_ENGINE`, config `[engine]`) and on macOS gives the main thread to `nowplaying::main_thread`. |
| `client` (`singularity`) | `app.rs` (state, input, actions, pages + history), `ui.rs` (draw, Pane titles, popups; `opaque_overlays` for kitty), `layout.rs` (split tree, `compute_shaped`: the well ≤ 4:3), `home.rs`, `tracks.rs` (lists, ♥ column), `artist.rs`, `search.rs`, `queue.rs` (drag: `preview`, `target`, `into_queue`), `lyrics.rs` (wrap), `stats.rs` (Stats page + "today"/"top this week" Widgets), `settings.rs` (Settings page: config.toml + keys tab), `ambient.rs` (starfield, marquee, glow), `mini.rs` (mini-player), `saved.rs` (like/follow, `heart`, `want_saved`), `keymap.rs`, `design.rs`, `theme.rs` (Themes, `Tint`, `ramp_at`), `looks.rs`, `widgets.rs`, `layouts.rs` (Slot groups, saved layouts), `setup.rs` (wizard, health, stream Pane), `gfx.rs` (kitty: images under cell backgrounds in kitty), `visual/` (black hole CPU/GPU, well, idle). |

Dev tools: `cargo run -p singularityd --example probe -- <op> '<vars>' [--raw file]`, `--example connect_probe`.
**pty harness: `tools/pty/`** (README lists every scenario; recreate the venv). Runs the real binaries against a fake
Daemon (fake Engine + fake Catalogue), scratch XDG dirs per run, `PTY_COLS`/`PTY_ROWS` for size; its compositor layers
kitty images as kitty does (under/over cell backgrounds, judged by colour). `connect_probe` modes for quiet live checks:
`--swap-context`, `--lift-context` (snapshot → change → read back → restore).

**Env knobs:** `SINGULARITY_SOLOIST_CDN`, `SINGULARITY_SOLOIST_CHECK_SECS` (updater), `SINGULARITY_TAP=off` (a test
Daemon never listens to the owner's Soloist), `SINGULARITY_DBUS=<address>|off` (MPRIS bus; fake-Engine Daemons stay
off the session bus unless asked), plus the older `SINGULARITY_CPU`, `SINGULARITY_PIXEL`, `SINGULARITY_PIXEL_MOUSE`,
`SINGULARITY_NO_KEYRING`, `SINGULARITY_SOCKET`. #23: `SINGULARITY_DATA=<dir>|off` (LocalStore; fake-Engine Daemons
keep none unless asked), `SINGULARITY_BACKUP_CHECK_SECS`, `SINGULARITY_IGNORE_SHUTDOWN` (#25 tests), `SINGULARITY_BELOW_BG=1|0` (#41: kitty's under-background layer), `SINGULARITY_FAKE_STALLS=1` (#45: the fake Engine starves), and `XDG_DOWNLOAD_DIR` decides where exports go. #65/#66: `SINGULARITY_ENGINE=soloist|spotify-app|librespot` (overrides `[engine] kind`), `SINGULARITY_START_APP=0` (the AppEngine never looks for or starts the real app: tests). #68:
`SINGULARITY_NOWPLAYING=on|off` (macOS Now Playing; off by default for a fake Engine). #72: `SINGULARITY_ENGINE=soloist`
on macOS runs the Soloist VM (a scratch `XDG_DATA_HOME` gets its own VM: ~2 min and ~250 MB of downloads the first time);
`SINGULARITY_SOLOIST=local` keeps Soloist local (a stand-in) on any OS: every pty script that pins `soloist` sets it. `SINGULARITY_NOTIFY` is gone (now `notifications.track_change`).

## Then

- **First, if the owner reports anything**: fix it.
- **The owner's live checks (macOS)**, whenever they come back: librespot (wifi change, media keys, Control Centre);
  the VM as their setting (`engine.kind = "soloist"`; measure its CPU while playing FLAC, not measured on the final
  setup yet); the app's audio tap after granting the permission (#67); then close #65, #67, #68, #70, #72.
- **#62**: the owner checks a book on the VM or the app (a chapter resumes where it was left; save a book and see
  its library row: `libraryV3`'s `Audiobook` shape is assumed from search's), then close it. Batch 7 (free tiling) is parked (D59). Now: **batch 9, the audit, started 2026-10-20**: progress and what's left in
  `docs/research/2026-10-audit.md` (macOS tools, security fixes, bounded caches done; Linux, soaks, budgets, Miri, fuzz
  to do)
  (#71; SPEC "Batch 9": bugs, vulnerabilities, resources on both OSes; seeds: `follow_config` polls every second, the
  Catalogue's caches have no bound, `daemon.log` is 0644 and names the account, Soloist's cache is unlimited on Linux,
  the owner's idle Client used ~7 % CPU).
- Batch 3/4/6 items still waiting for the owner's test: playlist covers in the TUI, the queue wheel, the Settings page,
  whether an episode resumes where it was left (#59).
- The owner's screenshots go in `docs/screenshots/` (git-ignored: personal data; never commit or quote them).


## Installed binaries

`singularity` and `singularityd` are installed in `~/.cargo/bin` with `cargo install --locked --path crates/daemon` and
`--path crates/client`. Reinstall both after each change the owner should test; the Client finds `singularityd` next
to itself and replaces an older running Daemon by itself. Linux: last installed after #61. macOS: the repo is at
`~/src/singularity`, Rust from Homebrew's rustup (`/opt/homebrew/opt/rustup/bin` and `~/.cargo/bin` on the
PATH in `~/.zshrc`); last installed after `2d496bf` (2026-10-20). Also on the Mac: zig + cargo-zigbuild + openssl@3
and the target `x86_64-unknown-linux-gnu` (for `tools/check-linux.sh`), and `vfkit` from Homebrew (spike leftover; the
Daemon uses its own pinned copy).


## Local state on the owner's machine (not in git)

Soloist at `~/.local/share/singularity/soloist/` (from #6 on: `soloist.new/` staged, `soloist.old/` previous,
`build.json` per build; the first check adopts the existing install instead of downloading it again), its state in
`…/soloist-state/`. Keyring `service=singularity` (`soloist`, `spotify`). Caches in `~/.cache/singularity/` (covers,
query cache, `notify-cover.jpg`), recent searches in `~/.local/state/singularity/`. From the next start of a new build:
`~/.config/singularity/{config.toml, keys.toml, themes/_example.toml, layouts/}` appear. From #23: the owner's real
listening log `~/.local/share/singularity/listening.db` and `backups/` (personal data: never read it into the repo or
fixtures).
A new Client replaces an outdated Daemon by itself (and, since #25, stops one that ignores `Shutdown`).

On the Mac: no native Soloist. Spotify.app 1.2.94 in `/Applications`, logged in (the AppEngine starts it hidden and
leaves it running). Secrets in the login Keychain (`service=singularity`: `spotify`, and `soloist` since 2026-10-20,
added by the owner with `security add-generic-password -U`). Same `~/.config`/`~/.local` paths as Linux (D50). The
owner's `config.toml` has `[engine] kind = "auto"` (librespot). Devices on the account: "Singularity on the owner’s
MacBook Air" (librespot), "Singularity on the Mac (lossless)" (the VM's Soloist, paired),
"the Mac" (the app), "Singularity" (Linux Soloist, offline). librespot's last volume:
`~/.local/share/singularity/catalogue/librespot-volume`. **The Soloist VM**: `~/.local/share/singularity/vm/` (disk,
EFI vars, SSH key, pinned `bin/vfkit` + `bin/gvproxy`, logs), setup version 2, Soloist paired and logged in inside it
(state in the guest's `/home/singularity/state`); Soloist's volume in `…/soloist-state/volume`; sockets in
`$TMPDIR/sgl-vm/`. Nothing of the VM runs unless `kind = "soloist"`.

## Waiting on the owner

- The live checks listed under "Then".
- Owner calls still standing (open to veto): Settings opens from Ctrl+K only; Cockpit's listening Pane packs two days
  per cell when short; delete asks again within 10 s; track notifications off by default; vim preset moves like to
  `f`; `_example.toml` theme skipped; the rain keeps drifting (calmly) while paused.
- Soloist's real update path ran for the first time on 2026-10-19 12:48 (Linux). Resuming over Connect after a restart
  mid-song has still only run against the stand-in.
- Background noise the owner should know: Stremio's torrent server saturated their uplink once (music stalled);
  #45 now says so in-app. They run Mullvad (local network sharing on).


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
- macOS: test Daemons pin their Engine (`SINGULARITY_ENGINE`, `SINGULARITY_START_APP=0`, `SINGULARITY_SOLOIST=local`
  with a stand-in Soloist, `SINGULARITY_NOWPLAYING=off`, `SINGULARITY_NO_KEYRING=1` unless the test needs the owner's
  login read-only) and use scratch `XDG_*` dirs and a socket in `/tmp/sgl-<uid>/`. A test Daemon with
  `kind = soloist` and the owner's data dir boots the owner's paired VM: only with the owner's go-ahead, and stop it.
  The owner's real Daemon runs whichever Engine their config says; never start a second one on the default socket.
- Things macOS taught us: Esc quits the Client (a pty script must not send it first); a background job in a shell
  script gets `/dev/null` as stdin; macOS's firewall silently withholds LAN connections from untrusted programs;
  terminals can't be given audio-capture permission except by hand.

---

## Prompt for the next session

> You're continuing **Singularity**, a terminal Spotify client in Rust (ratatui) at `~/src/singularity` on
> Linux, `~/src/singularity` on the owner's Mac (GitHub: `publicusr/singularity`, private). Read
> `docs/HANDOFF.md` first, then `CLAUDE.md`, `docs/PLATFORMS.md`, `docs/SPEC.md` (v1, "v1.x" and "Batch 9"),
> `GLOSSARY.md`, the ADRs in `docs/adr/`, and the end of the round log in `docs/DESIGN-TREE.md`.
>
> v1 is done; v1.x is built in batches (D29). Batches 1–4 and 6 (but #62) are done, 5 was dropped. **Batch 8, macOS, is
> built** (D48–D56, tickets #63–#72): Linux and macOS have the same features, OS code only behind the platform seams
> (D49), and `docs/PLATFORMS.md` is the exact status. Several macOS tickets wait only for the owner's live checks
> (HANDOFF "Then"). If you're on Linux, first run everything there: nothing since the Mac work has run on Linux.
> #62 (audiobooks) is built and waits for the owner's check; batch 7 (free tiling) is parked as an idea (D59). Next
> and last: batch 9, the codebase audit (#71).
>
> Build one ticket at a time: code + tests at the spec's seams, `cargo test --workspace` (on a Mac also
> `tools/check-linux.sh`), verify the real binaries in the pty harness (`tools/pty/`, screenshots), commit with
> `Closes #N`, push, reinstall both binaries (`cargo install --locked --path crates/daemon` and `--path crates/client`),
> and keep `docs/` current (DESIGN-TREE round log, HANDOFF, ROADMAP, GLOSSARY, PLATFORMS, README).
>
> If the owner reports bugs, fix those first. Product decisions go to the owner in the grilling format (numbered
> questions, a ➡️ recommendation each, worded so "yes" accepts); facts you look up yourself. Never commit secrets or
> personal listening data (`~/.local/share/singularity/listening.db`, `backups/`, `docs/screenshots/`); only kill
> processes you started; never start a second real Daemon on the owner's default socket; never test MPRIS on the
> owner's session bus; anything audible or state-changing on the owner's account only quietly, minimally, reverted,
> and reported. Keep messages to the owner short; they test the builds themselves.
