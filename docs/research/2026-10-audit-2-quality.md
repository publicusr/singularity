# Codebase audit, second pass (2026-10-25): code quality and publish readiness

Angle: **code quality** — does this read like clean, idiomatic, maintainable Rust that a stranger could pick up
after publication? Third of three audits run side by side (T0024 security, T0025 correctness, T0026 quality), all
branched off `main` (`c751a81`). **Findings only**: no product code changed. One bug-proving test was added, marked
`#[ignore]` (Q2).

The previous audit (`docs/research/2026-10-audit.md`, 2026-10-20…24) already ran clippy, cargo audit/deny, Miri,
ASan/LSan/TSan, `cargo fuzz` and a soak, and fixed findings 1–19 and issues #73–#75, #77, #80. This pass does not repeat
that sweep as its method; it reads the code. Where it does run a tool (clippy pedantic, counts, tests) that is a check
on a reading, not the reading itself. Its "Checked, fine" claims were treated as claims to verify.

## Scope

Read in full, line by line: `client/src/app.rs` (2 806), `client/src/ui.rs` (code to line 792; the test module skimmed),
`daemon/src/soloist.rs` (1 026), `daemon/src/engine.rs`, `client/src/layout.rs` (1–320), `client/src/tracks.rs`
(96–466), `client/src/lyrics.rs`, `client/src/conn.rs` (1–100), `client/src/main.rs`, `daemon/src/lib.rs`,
`crates/proto/src/lib.rs`, `crates/config/src/lib.rs` (1–360), `daemon/src/library.rs` (parsers + tests),
`daemon/src/catalogue.rs` (1–1 053 read; 1 053–1 650 by signature and spot reads), `client/src/playlists.rs` (1–181),
`client/src/sidebar.rs`, `client/src/widgets.rs` (1–80), `daemon/src/main.rs`, `crates/*/Cargo.toml`, `README.md`,
`docs/HANDOFF.md`, `docs/PLATFORMS.md`, `docs/SPEC.md` (batch 9), `docs/research/2026-10-audit.md`.

Skimmed or read only by structure, docs and grep (named in "Not read" below): the visual/gfx/gpu modules, the theme
and looks stack, stats/settings/home/search/artist/queue/saved/cover, store/backup/tap/vm/pcm/nowplaying/app_engine/
librespot/update/queries/secrets/health/mpris.

Not read at all beyond signatures and spot checks: `client/src/{design,ambient,meters,scrub,term,layouts,keymap,mini}.rs`,
`daemon/src/{vm,soloist_vm,pcm,process_tap,nowplaying}.rs` (the macOS-only seams), the innards of
`daemon/src/{store,backup,tap}.rs`, `fuzz/`, `tools/pty/`. None of them looked wrong from the outside: docs, module heads, function sizes and
the pedantic pass found nothing there I could turn into a finding I could stand behind.

## Method

1. Read the biggest and most central files first (`app.rs`, `ui.rs`, `catalogue.rs`, `soloist.rs`), then the seams
   (`engine.rs`, `layout.rs`, `proto`, `config`), then followed the call graph out of what I found.
2. Structural measurements over the whole tree (script, not judgement): function lengths by brace matching, file sizes
   with and without test modules, `pub` vs `pub(crate)`, `impl` blocks per type, duplicate patterns (`confirm`,
   `if coarse`, `chars().count() as u16`, `let _ =`, `.ok()`), `#[cfg(target_os)]` placement, test inventory
   (which modules have none), sleeps in tests.
3. Read every finding's site a second time in context before writing it down, and checked the "this is a bug" claims
   with a test or by following the code path (`Q2` has a failing test; `Q3` is marked latent because the two lists
   still agree today — I diffed them programmatically).
4. Ran `cargo test --workspace` (293 pass, 5 ignored) at the start and `cargo clippy --workspace --all-targets --
   -W clippy::pedantic` once, then triaged it (appendix) without applying anything.
5. Publish readiness: checked the tree and `git log --all -p` for secrets and personal data, the crate metadata, the
   README against `Cargo.toml`, and what a public repo would still lack. Facts only; the decisions are the owner's.

## Findings, most severe first

Severities use the task's scale: crash / data loss / security / resources / correctness / maintainability / polish.
"Confirmed" means I reproduced or mechanically proved it; "suspected" means the path is there but I did not run it.

### Q1 · resources · `client/src/ui.rs:61,492`, `client/src/tracks.rs:278,300-440`, `tracks.rs:134-160` · M

The draw loop allocates on every frame, and one of those allocations is proportional to the size of the open list.

- `ui::draw` clones the whole `Theme` (`ui.rs:61`: two `String`s plus `ramp: Vec<Rgb>`, `boxes: Vec<Rgb>`), and
  `player_bar` clones the whole `PlaybackState` (`ui.rs:492`, which carries the queue: a `Vec<QueueEntry>` of `Item`s
  with `Vec<String>` each). At 30 fps that is ~90 clones a second of data the frame only reads.
- `tracks::draw` calls `view.order(&list.rows)` every frame (`tracks.rs:278`), which builds a fresh `Vec<usize>` over
  **the whole list** — `MAX_TRACKS` is 10 000 in the daemon (`catalogue.rs:147`) — and, when a filter is typed, does a
  `format!` + `to_lowercase()` per row (`tracks.rs:135-145`). On a 10 000-song Liked Songs that is an 80 KB allocation
  plus up to 10 000 string allocations, 30 times a second, while the user types.
- Per row per frame: `" ".repeat(w as usize)` (`tracks.rs:402`), `clip()` returns a fresh `String` for title, artists
  and album (`tracks.rs:406,412,429` — `Buffer::set_stringn` already truncates), plus `format!` for the row number and
  duration (`tracks.rs:403,439`); the same `" ".repeat(width)` idiom is in `ui.rs:235,717`, `sidebar.rs:73`,
  `queue.rs:204`, `stats.rs:388`.

Evidence: **confirmed by reading**, and consistent with the measured numbers in the previous audit — 20 000
allocations/s in the pty `designs` profile, where "a third of them [was] the lyrics Pane re-wrapping every line every
frame" and "~10 % covers' tint keys", both fixed the same way (`docs/research/2026-10-audit.md`, "Linux, measured").
Counting `.clone()`/`format!` outside test modules in the ten draw modules: 60 and ~100.

Suggested fix, in value order: (1) cache the view order in `TrackView` and invalidate on filter/sort/rows change — it
is needed by eight call sites anyway (`view_order()` recomputes it per keypress too); (2) pass `&Theme` and
`&PlaybackState` into the draw functions instead of cloning (the borrow only has to be arranged so `app.hit(...)` can
still run — collect hits in a local `Vec` first, as `tracks.rs` already does); (3) write padding and clipped strings
straight into the `Buffer` (`for x in .. { cell.set_char(' ') }` and a `truncate_to_width` that takes `&mut Buffer`).
This is also the lever the old audit named for the Mac's frame cost (#79): "a frame that changed nothing could still
skip the image upload".

### Q2 · correctness · `client/src/app.rs:931-938` · S

When the Client's command channel is full, `App::send` drops the command and tells the user **"Not connected to
singularityd"**, which is false and sends them off to restart a Daemon that is running fine.

`send` does `tx.try_send(c).is_ok()` and, on either `Full` or `Closed`, shows the "not connected" toast. The channel
is `mpsc::channel(64)` (`conn.rs`), drained by one task that also reads the socket, so a burst (many clicks, a
long-press drag that sends a seek per percent until the nudge coalescer catches up, a stalled daemon) can fill it.
The command is silently lost either way; only the message is wrong.

Evidence: **confirmed**, by the test added in this audit:

```
crates/client/src/app.rs (tests)
#[ignore = "audit Q2: a full command queue is reported as \"Not connected to singularityd\""]
fn a_busy_command_queue_is_not_reported_as_a_missing_daemon()
```

With the `#[ignore]` removed it fails: `a full queue isn't a missing Daemon, but the toast was "Not connected to
singularityd"` (run with `--ignored` to see the failure).

**See also T0025 C11**: the correctness audit filed the same bug independently (as "suspected"); the test above is
the proof, their finding is the write-up. Prefer theirs if the two ever differ.

Suggested fix: distinguish the two: `Closed` → "Not connected to singularityd"; `Full` → "singularityd is busy;
that didn't go through" (or await the send with a short timeout on the connection task instead of `try_send`).

### Q3 · correctness (latent) · `daemon/src/lib.rs:648-702` · S

`client_session` routes commands to the Catalogue with **two hand-maintained lists of the same 25 commands**: one big
or-pattern in the match guard (`Command::Login | Command::Logout | …`) and one match arm each inside, ending in
`_ => CatCmd::Home`. A command added to the guard but forgotten in the inner match compiles fine and silently answers
by fetching Home. (`Command::GetHome` relies on that fallback today, so it also hides the intent.)

Evidence: **suspected, not a bug today** — I diffed the guard (25 commands) against the inner match (24) with a
script: only `GetHome` falls through, as intended. The trap is one added command away.

Suggested fix: one `fn catalogue_cmd(c: Command) -> Option<CatCmd>` with an exhaustive `match` (no `_` arm), called
from the session; the guard becomes `if let Some(c) = catalogue_cmd(cmd) { … }`. Removes ~25 duplicated lines and the
fallback.

### Q4 · maintainability · `daemon/src/catalogue.rs:281-774` · M

The fake Catalogue lives inside the real one. `spawn_fake` (212 lines) plus `fake_devices`, `fake_search`,
`fake_audiobooks`, `fake_podcasts`, `fake_artist`, `fake_lyrics`, `fake_library`, `fake_query`, `fake_saved`,
`fake_tracks`, `FAKE_BOOK`, `fake_book`, `fake_show` are ~490 lines of test double compiled into `singularityd` and
reached with `--fake-catalogue` (`main.rs:96`). Three *real* functions (`remote_cluster`, `emit_cluster`,
`app_engine_events`, lines 494–538) are sandwiched in the middle of them, so the Connect glue is 500 lines away from
the Connect code it belongs with.

Evidence: confirmed by reading; line numbers from `grep -n '^fn\|^pub fn'`. The file is 1 783 lines, of which the
real Catalogue is ~1 250.

Suggested fix: `daemon/src/catalogue/fake.rs` (module `#[cfg(any(test, feature = "fake"))]` or just a `pub mod
fake`, depending on how far `--fake-catalogue` should survive publication); `remote_cluster`/`emit_cluster`/
`app_engine_events` move next to `ConnectRemote` in `connect.rs`. Mechanical, no behaviour change; it makes the next
two findings half the work.

### Q5 · maintainability · `daemon/src/catalogue.rs:803-1647` · M

`impl State` is **845 lines, 33 methods**, holding six concerns: login/session (`login`, `connect`, `logout`,
`session`, `query`), browsing (`home`, `library`, `library_items`, `library_level`, `track_list`, `more_tracks`,
`fetch_pages`, `artist`, `search`, `find_show`, `lyrics`, `describe`), playback (`play_from`, `queue`, `resume`,
`remote_command`, `engine_command`, `engine_device`, `devices`, `transfer`, `remote`), playlist editing
(`create_playlist`, `add_to_playlist`, `remove_from_playlist`, `move_in_playlist`, `edit_playlist`,
`delete_playlist`, `set_cover`), saved state (`set_saved`, `check_saved`) and raw queries. Every edit to one concern
opens a file that another concern is also editing; there is no seam that says "this is request building" vs "this is
response parsing".

Evidence: confirmed by reading; method list from the `impl` block.

Suggested fix, in this order: (a) the request/parse split — most methods do `self.query(op, vars)` → `library::parse_*`
→ `self.ev.send(…)`; hoisting the `op`+`vars` construction per operation into small `fn …_request(...) -> (&str,
Value)` functions makes them testable against the fixtures `library.rs` already uses (see Q13); (b) then move the
playlist-editing methods into `catalogue/edit.rs` and the playback ones into `catalogue/play.rs`, leaving
`catalogue.rs` with login, browsing and the command loop. Effort is M only because of the mechanical move; do it after
Q4 so the file is a sane size to start from.

### Q6 · maintainability · `client/src/app.rs:574-2659` + `cover.rs:106`, `looks.rs:56`, `playlists.rs:82,331,423,470`, `saved.rs:82`, `settings.rs:125` · L

`App` is a god object: **98 fields (77 of them `pub`) and 231 inherent methods across six files**. Modules reach into
each other's state freely (`app.playlists.press`, `app.saved.checked_now`, `app.catalogue`, …), and the invariants
between fields ("exactly one page is open", "a filter is only set while a list is open", "`volume_hold` shadows
`playback.volume`") are enforced by hand in whichever method remembers to.

Evidence: confirmed by counting (script over `pub struct App { … }` and `impl App` blocks). The symptoms that make it
costly are Q7 (one 402-line method), Q9 (four confirmations in four shapes) and the page state in Q10's neighbour:
`show()` (`app.rs:1426`) clears `artist`/`stats_view`/`settings_view` and takes `search`, and `current_page()`,
`active_feed()`, `feed_view()`, `panes()`'s title match and `ui::draw`'s dispatch (`ui.rs:429-458`) each re-derive the
same "which page is open" from five `Option` fields.

Suggested fix, staged so every step compiles and the tests keep passing:
1. `enum Main { Home, List(TrackView), Artist(ArtistView), Search(SearchView), Stats(StatsView), Settings(SettingsView) }`
   replacing `view`/`artist`/`search`/`search_kept`/`stats_view`/`settings_view`. `Page` already models this; `show()`
   becomes the only place that swaps it, and `current_page()`/`active_feed()`/`ui::draw`'s five `if slot == "home" &&
   …` arms become one `match`. (M)
2. Split `app.rs` along the section comments it already has: `app/mod.rs` (state + `new`/`load_config`), `app/pages.rs`
   (678–940: config, idle, frame bookkeeping, navigation), `app/actions.rs` (941–1070), `app/keys.rs` (1070–1318),
   `app/mouse.rs` (2210–2620), `app/lists.rs` (1319–2075), `app/transport.rs` (2075–2210). The file is already written
   in these blocks; nothing but `use` lines changes. (M)
3. Only then decide whether `App` should still expose 77 `pub` fields or getters. (S, optional)

### Q7 · maintainability · `client/src/app.rs:2219-2620` · S/M

`App::pointer` is a single **402-line** `match` with ~70 arms, three levels deep in places (event kind → overlay
state → hit), and the wheel arm repeats the same "page or one step" computation eight times:

```
2526:  let step = if coarse { v.visible.max(1) as i64 } else { 1 };
2564:  let step = if coarse { v.visible.max(1) as i64 } else { 1 };
2572:  let step = if coarse { self.sidebar.visible.max(1) as i64 } else { 1 };
2584:  let step = if coarse { self.queue_view.visible.max(1) as i64 } else { 1 };
…
```

with four more `clamp(0, len.saturating_sub(1))` copies in the same arm, and the key handlers repeating it again
(`tracks_key`, `side_key`, `queue_key`, `stats_key`).

Evidence: confirmed by reading (`fnlen` brace count: 402 lines) and grep for `if coarse {` (8, all inside `pointer`).

Suggested fix: `fn pointer(&mut self, …)` → dispatch on `m.kind` into `press`, `drag`, `wheel`; `wheel` splits into
one method per pane (`wheel_home`, `wheel_library`, `wheel_queue`, `wheel_stats`, `wheel_settings`, `wheel_lyrics`)
which removes the repeated `if pane.as_deref() == Some("…")` chain; a `fn step(&self, coarse: bool, visible: usize) ->
i64` for the eight copies. No behaviour change, and each resulting method fits on a screen.

### Q8 · maintainability · `client/src/app.rs:87-258` · S

Every `Action` is written out by hand in **three places** and looked up in a fourth: `ALL: [Action; 52]` (52
elements), `label()` (52 arms), `id()` (52 arms, the `keys.toml` names) and `from_id()` (a linear scan over `ALL`).
Adding an action is four edits, and a typo in `id()` is silent: `keys.toml` then accepts a name that no action has
(`from_id` returns `None` and the key does nothing).

Evidence: confirmed by reading; `keys_toml_applies_live_and_reports_conflicts` covers the keymap's own conflict
reporting, not this table.

Suggested fix: one table `const ACTIONS: &[(Action, &str, &str)] = &[(Action::TogglePlay, "play_pause", "Play /
pause"), …]`, with `ALL`, `label`, `id`, `from_id` derived from it; `ALL` becomes `ACTIONS.iter().map(|a| a.0)`.
A duplicate id then needs a test (`assert_eq!(ids.len(), ids.dedup().count())`), which is two lines.

### Q9 · maintainability · `app.rs:1016,1892,1208,2306,2362,2404,2419`, `playlists.rs:62,72,144,393` · S

"Second time within N seconds" is hand-rolled six times, in four shapes and with three windows: `confirm_delete:
Option<Instant>` (10 s), `confirm_remove: Option<(String, Instant)>` (3 s), `playlists.confirm: Option<(String,
Instant)>` (10 s), `playlists.dup: Option<(String, Vec<String>, Instant)>` (10 s), plus double-click detection
`last_click: Option<(Instant, usize)>` where the "what" is smuggled into the index with sentinels
`usize::MAX - i` (top tracks), `usize::MAX / 2 - i` (search), `usize::MAX / 3 - i` (stats), `usize::MAX / 4 - i` (top
this week) and a bare `i` (track rows).

Evidence: confirmed by grep (`elapsed() < CONFIRM`, `elapsed() < Duration`). The sentinel encoding is the risky half:
row index 0 of the top-tracks list and `usize::MAX - 0` are told apart only because the lists are short.

Suggested fix:

```rust
struct SecondTime<T>(Option<(T, Instant)>);
impl<T: PartialEq> SecondTime<T> { fn armed(&mut self, what: T, window: Duration) -> bool { /* take, compare, re-arm */ } }
```
and `enum Click { Row(usize), TopTrack(usize), Search(usize), Stats(usize), TopWeek(usize) }` for `last_click`. Six
call sites become one line each; the three windows stay as named constants (`CONFIRM`, `REMOVE_CONFIRM`,
`DOUBLE_CLICK`).

### Q10 · maintainability · `daemon/src/soloist.rs:37,51,59,76,81,88,268,439,525,527,530,590,595,598,600,652,691` · M

`docs/PLATFORMS.md` says "OS-specific code lives only behind the platform seams below"; `soloist.rs` is the exception:
the macOS Soloist-in-a-VM host is woven through the shared supervisor with 14 `#[cfg(target_os = "macos")]` blocks
plus 3 `#[cfg(not(target_os = "macos"))]` ones: 17 OS-gated sites in one file, where the tree has 35 `cfg(target_os`
occurrences in all — including inside the 296-line `run_once`, and to keep one function body for both OSes it uses a type-level trick:

```rust
// soloist.rs:81-84
#[cfg(not(target_os = "macos"))]
fn vm(&self) -> Option<&std::convert::Infallible> { None }
…
// soloist.rs:527-528
#[cfg(not(target_os = "macos"))]
let in_vm: Option<std::convert::Infallible> = None;
…
Some(_) => unreachable!(),
```

with `#[cfg_attr(not(target_os = "macos"), allow(unused_assignments, unused_variables))]` on the function to pay for
it. A reader on Linux cannot see what the function does; a reader on macOS sees two versions interleaved.

Evidence: confirmed by reading and by `grep -c 'cfg(target_os'` per file (soloist.rs 14 + 3 negated, lib.rs 4,
tap.rs 3, nowplaying.rs 3, main.rs 3, app_engine.rs 3; everything else ≤2).

Suggested fix: a small host seam, mirroring the ones that already work (`secrets.rs`, `health.rs`, `nowplaying.rs`):

```rust
enum Host { Local, Vm(Arc<VmHost>) }
impl Host {
    async fn launch(&self, …) -> anyhow::Result<(Child, Option<ChildStdin>)>;
    fn ws_addr(&self, state: &Path) -> …;
    async fn announce(&self, port: u16, name: &str);
    fn alive(&self) -> bool;
}
```
`run_once` keeps one body with no `cfg`; the two implementations live in `soloist/local.rs` and `soloist_vm.rs`. That
also deletes the `Infallible` hack and the `allow(unused_*)`.

### Q11 · maintainability · `daemon/src/catalogue.rs` (14×), `r#12` elsewhere; `daemon/src/mpris.rs:402-420` · S

Errors are dropped without a trace in places where the failure matters:

- 178 `let _ =` sites in `crates/*/src`, ~40 of them channel sends. Fourteen are `let _ = self.ev.send(CatEvent::Error(
  format!("…")))` — i.e. **when an error event cannot be delivered, the error is lost entirely**: no log line, no UI
  message. `run_catalogue`'s `let _ = hub.events.send(Event::Error { … })` (`lib.rs:511-513`) has the same hole one layer up.
- `mpris.rs:402-420` (the Now Playing cover) is a `.ok()?` chain — build client, GET, status, chunks, create dir,
  write, rename — so *any* failure is silently a missing cover. Compare `catalogue.rs`'s `fetch_pages`, which sends a
  `CatEvent::Error` with the reason.

Evidence: confirmed by grep and by reading both sites. Not a crash; it is the difference between "the cover didn't
load" and "nothing happens, ever, and no log says why".

**See also T0025 C2/C13** — a task that dies plus a swallowed send is the user-visible end of the same hole (the
feature is gone and nothing reports it); **T0024 S6** is the URL check missing on the same cover fetch.

Suggested fix: one helper `fn send_ev(&self, ev: CatEvent)` that `tracing::warn!`s when the send fails (same for
`hub.events`), and `tracing::debug!` at the `.ok()?` sites in `mpris.rs` (a `let x = … .inspect_err(|e|
tracing::debug!("cover: {e}"))?` chain, or an explicit `match`).

### Q12 · maintainability · `crates/proto/src/lib.rs:313,380,391,405,494` · S/M

The protocol's "kind" fields are strings whose allowed values live in doc comments and are compared by hand
throughout both crates: `Shelf.kind` (`shortcuts`/`shelf`/`feed`), `Card.kind` (`album`, `playlist`, `artist`,
`episode`, `show`, `other`), `LibraryItem.kind` (+`liked`, `folder`), `Device.kind` (`computer`, `smartphone`, …) and
`QueueEntry.source` (`context`/`queue`/`autoplay`). 11 `kind == "…"` comparisons, 8 `match …knid.as_str()`, 13
`source == "queue"`. An unknown value falls into a `_ =>` and quietly does nothing: `App::open`'s `_ => return`
(`app.rs:1356`) means a Card of a kind Spotify adds tomorrow is clickable and dead, with no message.

Evidence: confirmed by grep. The codebase already shows the better pattern: `PlayStatus`, `RepeatMode`, `StatsPeriod`,
`EngineKind` are enums with `#[serde(other)]`-style fallbacks.

Suggested fix: `enum CardKind { Album, Playlist, Artist, Episode, Show, Other }` etc. with `#[serde(rename_all =
"snake_case")]` and `#[serde(other)] Other` (so a new Spotify kind still decodes), and `QueueSource` for
`QueueEntry.source`. Wire-compatible (the JSON stays the same), and both `match`es become exhaustive — the compiler
then points at every place that must be taught the new kind. Do `QueueEntry.source` and `Card.kind` first; they have
the most call sites.

### Q13 · maintainability (tests) · `daemon/tests/protocol.rs:52,71,148,214`, `daemon/tests/mpris.rs:85`, `client/src/ui.rs:995,1009,1021`, `client/src/search.rs:282,287`; `daemon/src/catalogue.rs` · S/M

Two test-quality problems:

1. **Timing-dependent tests.** `a_client_that_stops_reading_is_let_go` sleeps 7 s and then allows 5 s for the
   connection to drain — it measures a 5 s `WRITE_DEADLINE` against a wall clock. The spectrum test requires 30 frames
   (nominal 1 s at 30 Hz) within 2 s and then sleeps 2 500 ms to see the bars clear. `ui.rs`'s nudge tests sleep
   `NUDGE_SETTLE + 20 ms`, `search.rs`'s debounce tests sleep `DEBOUNCE + 10 ms`, `mpris.rs` sleeps 30 ms. These are
   the tests that flake on a loaded machine — and this machine ran three audit workers side by side today.
2. **The real Catalogue has no tests; the tests have the fake.** `catalogue.rs`'s only two tests cover `trim_timed`/
   `trim_any` and `parse_home`. `impl State` (Q5: 845 lines, 33 methods — login, paging, playlist add/remove/move/cover
   with its rollback paths) is exercised only through `spawn_fake`, whose answers are invented (`fake_query`,
   `fake_tracks`), so a change in the real request shape or the real answer shape is invisible until it meets Spotify.
   `library.rs` shows the pattern that would fix it: pure parsers + anonymised fixtures (11 tests) +
   `tests/hostile_input.rs`.

Evidence: confirmed by the test inventory (`grep -c '#\[test\]'` per module) and by reading the tests named above.

Suggested fix: for (1) use `tokio::time::pause()`/`advance()` where the code is in-process (`WRITE_DEADLINE`,
`DEBOUNCE`, `NUDGE_SETTLE` are all `tokio::time::sleep` or `Instant`-based inside `#[tokio::test]`-able code), and
widen the frame-rate bound to "eventually, within 10 s" for the spectrum test; for (2) do Q5(a) first (the pure
request/parse split), then add fixture tests per operation the way `library.rs` already does. Also: the four
`#[ignore]`s without a reason in the attribute (`conn.rs:236`, `library.rs:642`, `tap.rs:639`, `visual/mod.rs:208`,
`tests/secrets_live.rs:7`) do have their reasons in the doc comment above — clippy's `ignore_without_reason` wants
them in the attribute; one line each.

### Q14 · polish · `client/src/app.rs:2111-2115`, `client/src/ui.rs:15-30,470-475`, `client/src/app.rs:1954` · S

Stale text and a dead branch that a stranger would trip over:

- `App::frame_rate` carries **two stacked doc comments**, the first describing behaviour that was removed:
  "Frames a second (D60): 30 focused, `visuals.unfocused_fps` unfocused, **and at most 5 while nothing plays and nobody
  has touched anything for 2 s**" (`app.rs:2111-2113`), then "Frames a second: 30, or … (D62: no slower rate while
  nothing plays; the owner found 5 fps choppy.)" (`2114-2115`). D62 undid D60; only the second is true.
- `slot_info` still promises features that shipped months ago — `"library" => ("~/library", "library sidebar arrives
  with #9")`, `"well" => ("gravity well", "the black hole arrives with #16")`, `"loudness"/"fft" => "arrives with
  #15"` (`ui.rs:15-25`) — and the fallback that would draw them (`ui.rs:470-475`) is **unreachable**: every slot in
  `slot_info` is handled by an earlier arm, so the strings are dead. A test even asserts they don't appear
  (`ui.rs:1425`).
- `MenuAct::GoTo`'s fallback says `"{kind} pages arrive with #11"` (`app.rs:1954`), a v1 ticket.

Evidence: confirmed by reading; the unreachable fallback by enumerating `slot_info`'s slots against `panes()`'s arms.

Suggested fix: delete the first `frame_rate` paragraph; drop the `todo` tuple element and the fallback arm (make
`slot_info` return just the title, or keep a `debug_assert!` for unknown slots); replace `#11` with a plain sentence
("it has no page of its own").

### Q15 · polish · `app.rs:913,920,1039,2141,2379`; `app.rs:1208,2362,2404,2419` · S

Magic numbers doing structural work:

- `Rect::new(0, 0, 200, 60)` is "some area big enough for the layout", passed to `layout.compute` in five places to
  ask *which slots exist*. It also recomputes the whole layout each time (five sites, one of them per frame in
  `tick`). A named `const PROBE: Rect = Rect::new(0, 0, 200, 60)` (or better, `fn slots(&self) -> Vec<String>` cached on
  layout change) says what it means.
- The `usize::MAX / 2 - i` family (Q9) is the same problem in index form.

Evidence: confirmed by grep.

### Q16 · polish (publish readiness — facts, no decisions) · repo root · S–L depending on what the owner wants

What a public repo would still be missing or exposing. Lists only; these are the owner's calls.

- **No licence file.** `Cargo.toml` says `license = "MIT"` for the workspace, but there is no `LICENSE` (or
  `LICENSE-MIT`) in the tree and no per-file headers. `cargo deny`'s config (`deny.toml`) allow-lists dependency
  licences — it does not supply one for this code.
- **No CI.** No `.github/` at all: the checks the previous audits ran by hand (test, clippy, deny, fmt-free style) are
  not run by anything on push. (`tools/check-linux.sh` exists for the macOS side; nothing runs it automatically.)
- **Crate metadata gaps:** no `rust-version` (MSRV) anywhere, though the code uses edition 2024 and recent APIs
  (`is_multiple_of`, let-chains in `if` guards); no `authors` field; all four crates `publish = false` (fine for a binary
  app — a fact, not a finding); `description` present for all four.
- **Personal data in the tree:** `docs/HANDOFF.md:279-281` names the owner and their machines ("Singularity on
  the Mac", the app's device name, the Linux hostname); `docs/HANDOFF.md:331` and `README.md` mention
  the private GitHub repo; local paths (`~/src/singularity`, `~/src/singularity`) appear
  throughout the docs. `crates/client/src/cover.rs`'s tests use `/home/u/…` (fine). No secrets found (see "Checked,
  fine").
- **Non-product trees committed:** `prototypes/`, `spikes/`, `.claude/` (agent instructions), `.project.yml`, and
  `CLAUDE.md`'s working agreements — all fine to keep, but they are what a visitor sees beside `crates/`.
- **Build steps:** `README.md`'s Linux section is accurate against `crates/*/Cargo.toml` (needs `alsa-lib`,
  `pkg-config`, OpenSSL; SQLite is bundled via `rusqlite`; no `protoc` needed). The macOS section matches PLATFORMS.
  What is missing is a note that macOS builds need the same `cargo install` twice, and that `--locked` needs the
  committed `Cargo.lock` (it is committed).
- **Docs drift:** `docs/HANDOFF.md:217` says `config.toml` is at `CONFIG_SCHEMA` **4** with migrations through 3→4;
  the code is at **5** with a 4→5 migration (`smooth_unfocused`, `config/src/lib.rs:17,264-280`). Same paragraph lists
  the settings sections but not `engine.cache_gb`'s 4→5 companion. A stranger's first stop is this table.

Suggested fix: the licence file and CI are the two that block "publish" in any normal sense; the rest is a choice.

## Checked, fine

Verified, not assumed:

- **Protocol framing and versioning** (`proto/src/lib.rs`, all of it): `MAX_FRAME` checked on write *and* read, length
  as `u32` big-endian, `Ok(None)` on clean EOF, `#[serde(other)] Unknown` on both message enums, `#[serde(default)]` on
  every field added after schema 1 (`artist_uris`, `album_uri`, `uid`, `progress`, `header`, `book`, `engine_build`,
  `missing_tools`, `out_*`, `plays_today`, …). The doc comment's versioning rule ("bump on breaking, additive is free")
  matches what the code does; I found no additive change that forgot its default.
- **Config** (`config/src/lib.rs` 1–360 + the 13 tests): schema number, migration chain with comment-preserving
  edits (`add_*` reuses the comment text from `DEFAULT_CONFIG`), `migrate` refusing a newer file with a message,
  `parse` falling back to defaults + problems, atomic writes via temp + rename, `Watched` mtime+size. A bad value never
  stops the app, which is what the module doc claims.
- **Layout** (`client/src/layout.rs`): `compute`/`compute_shaped`/`drag` under minimums; the two tests tile every
  preset at four sizes exactly, assert no overlap and non-zero slots, and drag every divider to `0`, `1`, `w/2`,
  `w-3`, `w`, `u16::MAX` — that is the "spaghetti wall" I would have expected to be fragile, and it is tested.
- **Parsers** (`daemon/src/library.rs`, `catalogue.rs::parse_home`): pure `Value → struct` functions, 11 + 1 fixture
  tests, unknown shapes skipped rather than panicked, plus `tests/hostile_input.rs` (20 000 random JSON trees through
  every parser, from the previous audit — I read the test, it does what it says).
- **The Soloist log parser** (`soloist.rs::LogParser`): pure, small, and the doc comment records when each line format
  was verified against a real Soloist (`2026-10-17`, `2026-10-18`) — the house style at its best.
- **Engine seam** (`daemon/src/engine.rs`): a 16-line trait with a documented reason for `Box<Self>`, `NoEngine` and
  `FakeEngine` behind it; the fake is documented as such and only reachable with `--fake-engine`. (It is ~130 lines in
  the production module — same smell as Q4, smaller.)
- **Connection** (`client/src/conn.rs` 1–100): uid check against another user's socket (`peer_cred`), handshake with
  protocol version, one-time replacement of an older Daemon, no `unwrap` in the retry path, all sleeps bounded and
  named. The reconnection loop is 3 s / 300 ms, as documented.
- **No `unwrap`/panic outside tests where it matters.** 50 `unwrap`/`expect` outside test modules (463 with the tests), and the ones I followed
  are invariants with a reason: `expect("cluster lock")`/`expect("store lock")` (Q11's neighbour; a poisoned std
  `Mutex` after another panic), `rows.last().unwrap()` in `lyrics::wrap` (seeded with one row, `lyrics.rs:43`),
  `credits.last().unwrap()` (a 1–3 element `vec![]`, `lyrics.rs:151`), `expect("row exists")` in `sidebar.rs:66`
  guarded by `len(app)` in the same function. The panic-shaped bugs are T0025's angle; I found none in the draw paths
  I read beyond these.
- **Dependencies:** every declared dependency is used in `src` (checked each client dep by name; the daemon's by the
  same grep). No `allow(dead_code)` beyond one `#[cfg_attr(not(test), allow(dead_code))] pub fn mouse` (a test
  helper), and all eight `#[allow(clippy::…)]` sites carry their reason next to them (4 × `too_many_arguments`, 3 ×
  `large_enum_variant` with a paragraph, 1 × `excessive_precision`).
- **Comments and docs, house style:** plain, short, why-not-what, ticket/decision numbers where a choice was made
  (`#45`, `D55`, `ADR-0003`). I read ~2 000 comments and found the style held; the exceptions are Q14.
- **Tests as a suite:** 293 pass, 5 ignored (benchmarks + live checks with their reasons in doc comments), none touch
  the network, MPRIS runs on a private bus. The flaky-by-construction ones are named in Q13.
- **Secrets in the tree or history:** `git log --all -p` over `*.toml`, `*.json`, `*.md` for
  `(api_key|token|password|secret)\s*[:=]\s*<value>` and `git log --all --name-only` for
  `secret|credential|token|.env|screenshot|listening|.key`: no value, no file. The client id in
  `catalogue.rs:22` is Spotify's public desktop-client id, as librespot/spotatui use (the security audit owns
  whether that is acceptable).
- **`unsafe`**: out of my angle (T0024 read it). The two blocks in `soloist.rs` (`prctl`, `getuid` in `conn.rs`) carry
  their `SAFETY` comment.

## Appendices

### Clippy pedantic, triaged (`cargo clippy --workspace --all-targets -- -W clippy::pedantic`)

**1 701 warnings**: client 1 151, daemon 505, proto 22, config 23. No errors; `-D warnings` with the normal set is
still clean (that is the #80 state). 59 % of them are the cast family, which is geometry arithmetic on terminal
cells (`usize → u16`, `u16 → f32`) where the values are bounded by the screen; the daemon's are mostly protocol
numbers.

| Category | Lints | Count | Verdict |
|---|---|---|---|
| Casts | `cast_possible_truncation` 363, `cast_sign_loss` 168, `cast_precision_loss` 104, `cast_possible_wrap` 91 | 726 | **Don't** enable workspace-wide (bounded cell math). Enable for `proto` + `daemon` only, where the numbers come off the wire. |
| Casts, mechanical | `cast_lossless` | 278 | Enable; `From`-based, `cargo clippy --fix` applies 279 of them; makes the bounded casts stand out. |
| Literals | `unreadable_literal` | 108 | Enable. Timestamps and bitrates (`1_643_334`, `44_100`) read better with separators. |
| Must-use | `must_use_candidate` | 99 | Enable in `proto`/`config` (pure getters); optional in the client (draw functions). |
| Names | `similar_names` | 68 | Allow with a reason: `x/y/w/h/r` are the geometry idioms here. |
| Docs | `doc_markdown` 44, `missing_errors_doc` 41, `missing_panics_doc` 7 | 92 | `doc_markdown` yes (backticks, matches the house style); `missing_errors_doc` no (these are internal fns whose callers are 3 lines away). |
| Options | `map_unwrap_or` | 68 | Enable; `map_or` is shorter and `--fix`-able. |
| Structure | `items_after_statements` 35, `too_many_lines` 19, `unused_self` 16 | 70 | `items_after_statements` + `unused_self` yes (the latter points at exactly the free functions Q6 wants); `too_many_lines` no (own limit is ~140; the 19 over 100 are listed in Q5/Q6/Q7 and get fixed by the splits). |
| Small | `default_trait_access` 30, `redundant_closure_for_method_calls` 14, `enum_glob_use` 13, `assigning_clones` 12, `single_match_else` 12, `match_same_arms` 11, `ignore_without_reason` 3, `float_cmp` 6, `unnested_or_patterns` 5, … | ~130 | Enable the whole group; all mechanical, all `--fix`-able or a one-line reason. |

What I would put in `Cargo.toml` (`[workspace.lints.clippy]`) after the owner's yes: `cast_lossless`,
`unreadable_literal`, `map_unwrap_or`, `doc_markdown`, `items_after_statements`, `unused_self`,
`redundant_closure_for_method_calls`, `match_same_arms`, `assigning_clones`, `ignore_without_reason`, plus
`must_use_candidate` for `proto`/`config`; and `similar_names`/`cast_*` allowed with the reason written down. That is
~430 warnings, essentially all mechanical, and the 726 geometry casts stay quiet instead of drowning them.

### `unused_crate_dependencies`, `missing_docs`

`unused_crate_dependencies`: nothing unused (matches the previous audit's `cargo machete`). `missing_docs` on public
items: the four crates are `publish = false` and 755 `pub` items vs 13 `pub(crate)` — i.e. there is no public API
surface to document; `daemon/src/lib.rs`'s 23 `pub mod`s exist so `tests/*.rs` can reach internals. Enabling
`missing_docs` would mean documenting test seams. Recommendation: leave it off, and note in `lib.rs` *why* everything
is `pub` (one paragraph), so the next reader does not take it for an API promise.

### Cross-worker notes

The shared folder was empty when I started (16:15) and by the time I wrote this up `correctness.md` (17:45) and
`security.md` (17:30) existed; I read both fully and cross-referenced rather than repeated:

- **Q2 = T0025 C11** (the "Not connected" toast). Same bug, filed independently; my ignored test is the proof, their
  entry is the write-up.
- **Q11** is the same hole as **T0025 C2/C13** one layer up (a swallowed send + an unsupervised task = a feature
  silently gone); the cover fetch in Q11 is **T0024 S6**'s site.
- Nothing else of mine is in either file, and none of their findings touches my structure/API/test items (Q1, Q3–Q10,
  Q12–Q16). Their `S3` (tools from PATH) and my Q16 (publish readiness) do not overlap.
- My two handoff notes for T0025 — `MIN_H = 3` leaving a 1-row Pane body (are all `bottom() - 1` draws safe?), and
  `App::open`'s silent `_ => return` for an unknown card kind — are not addressed in `correctness.md`, so they stand
  as open questions for phase 2.
