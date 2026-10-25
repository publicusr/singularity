# Codebase audit 2 — correctness, panics, races, state machines (T0025, 2026-10-25)

Second audit (#71's follow-up), read-only: findings only, no product code changed. Three workers split the
angles; this one is **correctness and robustness**. The previous audit (`2026-10-audit.md`) already ran the
tool sweep (clippy, cargo audit/deny, Miri, ASan/LSan/TSan, cargo fuzz, a soak) and fixed its findings 1–19
and issues #73/#74/#75/#77/#80. This one re-reads the code and treats that report's "Checked, fine" as
claims to verify, not facts. Known open issues (#76, #78, #79, #81) are not re-filed.

## Scope

Read, line by line:

- `crates/daemon/src`: `lib.rs` (Hub, sessions, routing), `engine.rs`, `soloist.rs` (supervisor, updater
  handshake, log parser, `SoloistState`), `catalogue.rs` (session, paging, caches, Connect, every command),
  `connect.rs` (cluster, queue edits, remote control), `app_engine.rs`, `librespot.rs`, `tap.rs`, `store.rs`
  (SQLite, recorder, calendar), `backup.rs`, `update.rs`, `queries.rs`, `playlists.rs`, `lyrics.rs`,
  `health.rs`, `mpris.rs`, `main.rs`.
- `crates/proto/src/lib.rs` (framing, every message and event, `position_at`), `crates/config/src/lib.rs`
  (schema, migrations, `set`, `Watched`, `SETTINGS`).
- `crates/client/src`: `main.rs`, `conn.rs`, `app.rs` (state, input, actions, tick, pointer), `queue.rs`,
  `tracks.rs`, `saved.rs`, `playlists.rs`, `settings.rs`, `setup.rs` (wizard matrix), `lyrics.rs`,
  `scrub.rs`, `layout.rs`, `layouts.rs`, `sidebar.rs`.

Not read line by line (judged too far from this angle or already covered): the drawing-heavy client modules
(`ui.rs`, `home.rs`, `widgets.rs`, `ambient.rs`, `visual/*`, `gfx.rs`, `mini.rs`, `looks.rs`, `theme.rs`,
`covers.rs`, `cover.rs`, `search.rs`, `stats.rs`, `artist.rs`, `meters.rs`, `term.rs`, `keymap.rs`); the
macOS-only code (`nowplaying.rs`, `process_tap.rs`, `soloist_vm.rs`, `vm.rs`, `pcm.rs` — Linux here, and
T0025 has three other workers' findings to merge; #81 already covers the VM's clock); the Spotify API
research notes.

## Method

Reading, then targeted checks: every `unwrap`/`expect`/`unreachable!`/index/slice/`as` cast outside tests in
the Daemon and in the Client's event loop and state (`crates/daemon/src` has 106 `unwrap`s, all in tests or
true invariants — the list is below); every `std::sync::Mutex` use for an `.await` inside the guard; every
`tokio::select!` branch for cancel-safety; every spawned task for who notices when it ends; every channel's
capacity and who drops when it is full; the Engine/Catalogue state machines against their status events;
`position_at`/`now_ms`/calendar arithmetic against clock jumps, DST and midnight; the protocol's additive
rules both ways; migrations and the recorder; the queue-edit paths against stale uids; the D63 phone-play
logic. Where a bug could be shown, a test was written (`crates/daemon/tests/audit_correctness.rs`, all
`#[ignore]`d so `main` stays green; removing the ignore line shows the failure — each finding below says
which). Tests and clippy were run through the shared `flock -j2` lock.

## Findings (most severe first)

Severity: crash / data loss / security / resources / correctness / maintainability / polish. *Confirmed* =
shown by a test or an executed path; *suspected* = the code path is exact but the trigger depends on timing
or on another program's behaviour.

### 1. (C2) A dead Engine or Catalogue task is silent: playback control stays dead, Clients are never told — **correctness**, confirmed, effort M

**Fixed (T0027, A1):** a dead Engine reports `Engine(Failed)` and an `Event::Error`, is named in the health view
(`Health.stopped_tasks`) and is built again with a backoff (`Engine::respawn`); a command sent meanwhile gets an
error answer. The Catalogue task runs under a catch-unwind supervisor (same backoff, its command channel survives),
and the other spawned tasks (listening log, backups, tap, MPRIS, Now Playing, config follower, Soloist update
checker) are supervised the same way. Proof un-ignored:
`an_engine_that_dies_is_reported_and_playback_commands_do_not_vanish`; unit test
`a_background_task_that_panics_is_restarted_and_named`. `transport`/`on_engine_event` no longer drop failed sends.

`crates/daemon/src/lib.rs:227-250` (`run_engine`), `:717` (`transport`), `catalogue.rs:167`.

`run_engine` owns the only receiver of `hub.commands`. Its `select!` breaks as soon as the Engine's event
channel closes — i.e. when the Engine task ends, for any reason (a panic, or one of the real Engines' error
paths: `librespot.rs:152` and `app_engine.rs:105` return when `cat.send(CatCmd::Engine(..))` fails). The
receiver is dropped with the task, so afterwards every `hub.commands.send(cmd)` returns `Err`, and every
caller throws it away: `transport()` (`let _ = hub.commands.send(cmd).await`), `on_engine_event`'s Resume
path, `client_session`. Nothing restarts the Engine, sends `Event::Engine(Failed …)` or answers the Client
with an `Event::Error`: the UI keeps the last status and play/pause/seek/volume do nothing, forever. The same
shape applies to the Catalogue task: it is the only reader of `CatCmd`s, and its senders (`lib.rs:683`,
`:726`, `:250`, `:288`, `:459`, `backup.rs:42`) ignore failures, so a dead Catalogue means
every browse action silently does nothing while the UI still says "logged in". And because the two are linked (the app/librespot
Engines quit when the Catalogue's channel breaks), one dead task can take the other down.

Evidence: `crates/daemon/tests/audit_correctness.rs::an_engine_that_dies_is_reported_and_playback_commands_do_not_vanish`
(`--ignored`): an Engine whose task is already gone, then `Play` — the Client gets no `Error`, no `Failed`
status, nothing (2 s timeout).

Fix: treat a closed Engine/Catalogue event channel as a failure: send `EngineStatus::Failed`/`CatalogueStatus`
`Unavailable` (and an `Event::Error` to the Clients), and either restart the task with the existing backoff
patterns or refuse commands loudly (`transport` must not ignore a failed send). Effort M because the
Catalogue has no restart path today; a "report it" fix alone is S.

### 2. (C1) A track description that never comes back wedges those URIs: blank queue rows forever — **correctness**, confirmed, effort S; see also T0024 S8

**Fixed (T0027, A8; also the memory half, T0024 S8):** `describe` answers every URI of a batch (no `take(20)`),
sends `CatEvent::DescribeFailed` for the ones it couldn't look up, and remembers a failure for five minutes so the
same track isn't asked about on every playback update. The Hub clears `describing` on that event and both maps are
bounded (`MAX_DESCRIBED`). Proof un-ignored: `a_description_that_never_arrives_is_asked_for_again`; unit test
`a_describe_batch_covers_every_uri` (51 URIs, cached, recently failed, retried later).

`crates/daemon/src/lib.rs:381-395` (`fill_queue`), `catalogue.rs:1448-1472` (`State::describe`).

`fill_queue` adds every bare track URI to `hub.describing` ("being asked") and only `CatEvent::Described`
clears a URI. But `State::describe` describes at most 20 URIs per call (`uris.into_iter().take(20)`) and
`break`s out of its loop at the first failed `getTrack` **without sending any event**. So:

- with the Spotify app (or librespot) as Engine, the queue from the cluster is up to 50 bare entries
  (`connect.rs:engine_playback`, `QUEUE_SHOWN`) plus the bare current item: the first `Describe` carries 51
  URIs, 31 of them are never described, stay in `hub.describing`, and no later playback state re-asks (they
  are not in `missing` any more) — those rows show "…" for the life of the Daemon;
- one transient `getTrack` failure (rate limit, network) does the same to the rest of its batch.

The memory side (the same set never being trimmed) is T0024's S8; this is the user-visible side and the
test. Evidence: `a_description_that_never_arrives_is_asked_for_again` (`--ignored`): a Catalogue stand-in
answers for one URI of a 25-URI batch; a second identical playback state produces no second `Describe`.

Fix: have `describe` answer with every URI it attempted (a `CatEvent::Described` for the successes and a
`DescribeFailed(Vec<String>)` — or just the attempted list — for the rest) so the Hub clears `describing`;
and either chunk (the Hub could send 20 at a time) or drop the `take(20)` cap in favour of the caller's
batch size. Effort S; the bounded cache for `Hub::described` (T0024 S8) comes along.

### 3. (C14) An LRC timestamp overflows: a debug-build panic inside the Catalogue task, a wrong time in release — **crash (debug) / correctness (release)**, confirmed, effort S

`crates/daemon/src/lyrics.rs:32`: `stamps.push(m * 60_000 + (s * 1000.0).round() as u64)`, with `m` a `u64`
parsed straight out of the text LRCLIB returned. `[18446744073709551615:59.99]` (or a minute field near
`u64::MAX`, and `inf` seconds) overflows: `cargo test`/debug builds panic (`attempt to multiply with
overflow`), the installed release build wraps to an arbitrary time, so the line shows at the wrong moment.
The text is third-party network input. Reached from `CatCmd::Lyrics` on the Catalogue task — where a panic
is exactly finding 1 (the Catalogue dies silently). Taken from T0024's handoff (they saw it while reviewing
parsers); confirmed here.

Evidence: `a_hostile_lrc_timestamp_does_not_overflow` (`--ignored`) panics in `parse_lrc` today; the test
wants saturation (a stamp saturates at `u64::MAX`), which also keeps the `sort_by_key` order stable.

Fix: `m.saturating_mul(60_000).saturating_add(...)`, with the seconds part clamped/saturated too (and skip
non-finite seconds). Effort S.

### 4. (C4) Search handling can deadlock the whole Catalogue — **correctness**, suspected, effort S

**Fixed (T0027, A2):** the commands drained behind a search go into a local queue that runs them next, in their
original order, instead of being re-sent into the channel this task alone reads. Unit test:
`a_search_drains_the_commands_behind_it_in_order`.

`crates/daemon/src/catalogue.rs:262-281`.

```rust
CatCmd::Search(q) => {
    while let Ok(c) = cmds.try_recv() { match c { CatCmd::Search(n) => q = n, other => later.push(other) } }
    st.search(q).await;                    // a network round trip
    for c in later { let _ = self_tx.send(c).await; }   // self_tx → the same bounded (16) channel
}
```

`self_tx` sends into the channel this very task is the only reader of. While `search` awaits, the Hub keeps
sending (each Client's session task blocks in `tx.send(c).await`), so the 16 slots can fill; the first
`self_tx.send(c).await` then waits for capacity that only this task can free → permanent deadlock of the
Catalogue: no browsing, no queue edits, no resume, no backups (and, with finding 1, no report). With several Clients (or the backup's `Query` stream, `Describe`
batches and `CheckSaved`) sending while a search runs, the 16 slots fill; with one Client it needs a
16-command burst during one search, which is why this is *suspected*. A second,
certain effect: commands that arrived **before** the search are re-queued **after** it (e.g. a `PlayFrom`
waits for a slow search).

Fix: don't re-send through the same bounded channel — process `later` in this loop directly (a small local
`VecDeque` of pending commands at the top of `run`), or `try_send` and log what couldn't be re-queued.

### 5. (C6) The Settings page can silently overwrite an edit made in an editor — **correctness (lost setting)**, suspected, effort S

`crates/config/src/lib.rs:410-416` (`set`), `crates/client/src/settings.rs:146` (`settings_write`).

`set()` reads `config.toml`, edits the document in memory and `rename`s a temp file over it. There is no
check that the file is still the one it read; a save from the user's editor that lands between the read and
the rename is gone (and `settings_write` then `apply_config`s its own version). The window is the few
milliseconds between `read_to_string` and `rename`, so this is unlikely — but it is a lost user setting with
an easy guard. The same applies to `toggle_idle` and `toggle_notifications` (they write without touching the
`Watched` stamp). Note the *reverse* direction is fine: a Settings write is atomic, so an editor never reads
half a file.

Fix: before the rename, compare `Watched`'s stamp `(mtime, len)` taken before the read; on a difference,
re-read and re-apply the change once (or refuse and tell the user). Effort S.

### 6. (C5) `[engine] device` is ignored for Soloist on Linux — **correctness (a setting that does nothing)**, confirmed by reading, effort S

`crates/daemon/src/main.rs:134` (`Box::new(SoloistEngine { cache_mb, ..SoloistEngine::from_env() })`),
`soloist.rs:40-50` (`from_env` hard-codes `DEVICE_NAME`), while `config/src/lib.rs:174-176`'s default-file
comment says: "the player's name in device pickers; empty: the default (the app: this computer's name;
librespot: …; Soloist: "Singularity" on Linux, …)". The app (`app_engine.rs:35`), librespot
(`librespot.rs:41`) and the macOS Soloist VM (`soloist.rs:62`) all use it. On Linux the user sets
`device = "Kitchen"` and the device picker still shows "Singularity" — no error, no hint. (The Catalogue's
engine identity is `Identity::default()` on Linux, consistent with the ignored value, so nothing else breaks.)

Fix: `SoloistEngine { device_name: choice.device.trim() or default, ..from_env() }` on Linux, and pass the
same name into the identity (`crates/daemon/src/main.rs:86-92`). Effort S.

### 7. (C9) The D63 queue-answer rule can swallow a real playback state — **correctness**, suspected, effort S

`crates/daemon/src/soloist.rs:877-903`.

D63's three parts are right in the main cases (a track change only counts as ours if our command caused it;
a resume after a restart is dropped when the player came back playing; a `get_queue` answer only fills the
queue). The third one is unconditional: for 5 s (`QUEUE_ANSWER`) after any `track_changed`, the **first**
`playback_state` event is taken as the queue answer and only `upcoming` is applied, whatever else it says —
status, volume, options, position, item. If the answer is slower than the window, or another app's
`playback_state` (a phone pausing) is the first to arrive, that update is lost: the Hub's playback (and with it MPRIS, Now Playing, the listening log and every
Client) keeps the old status until the next event. Concrete case: a phone pauses within 5 s of a track
change; the pause is in a `playback_state`, gets dropped, and the listening log keeps counting the pause as
playing until something else happens. Verified by reading `apply`; not reproduced against a real Soloist.

Fix: mark the request with a nonce/timestamp and only treat an answer that arrives *after* it as the queue
answer, or at least apply everything except `item`/`status` from that event (keep only the queue's track
protection). Effort S.

### 8. (C7) Dragging a queue row while the queue changes: underflow (debug panic) — **crash (debug) / correctness**, suspected, effort S

`crates/client/src/app.rs:1843-1856` (`drop_queued`).

```rust
let Some((start, _)) = crate::queue::section(self, d.from) else { return };
let e = &self.playback.queue[d.from];
...
let to = if d.into_queue { d.to } else { d.to - start };
```

`d.from`/`d.to` were computed against the queue as it was when the drag started. A Playback event during the
drag replaces `playback.queue` (a track change, a phone editing the queue): `d.from` can now name another
entry, and the entry's section can start *after* `d.to` (e.g. dragging a context song to the first row of
its section, then five queued songs appear above it) → `d.to - start` underflows: a panic in debug builds
(`attempt to subtract with overflow`), a huge `usize` in release (sent as `MoveInQueue.to`; the Daemon clamps
it, so the song quietly lands at the end). `&queue[d.from]` also reads whatever entry now has that index, so
the move can name a different song. The indexes are only range-checked (`section()` returns None when
`from` is past the end), not re-validated for identity.

Fix: carry the dragged entry's URI/uid in `QueueDrag` (the Client already uses uids everywhere else) and
recompute `from`/`to` from it at drop time; use `saturating_sub` for the section offset. Effort S.

### 9. (C10) Repeat-one (and replaying a track) logs a single play — **correctness (stats)**, confirmed, effort S

`crates/daemon/src/store.rs:348-382` (`Recorder::observe`, `Current::finish`).

`Recorder.observe` decides "same play" from the track URI and "not idle" only. A track that starts over —
repeat one, or the user replaying it — is therefore still the same `Current`; `played_ms` keeps accumulating
until the track finally changes and is then clamped to `item.duration_ms`. Consequences: "played N times"
(`StatsRow.all_time_plays`), the per-track plays in stats, the listening minutes and the play counts in
`ListeningSummary` undercount every looped track (a repeat-one track played ten times logs one play of one
duration), and a long repeat-one session never logs a play at all until the track changes. The `PlaybackState` carries
enough to see it (`position_ms`/`timestamp_ms` going back to the start).

Evidence: `a_repeated_track_is_logged_as_a_second_play` (`--ignored`): the same track at position 0, 200 s
later, logs nothing today.

Fix: in `observe`, when the same URI arrives with a position that went backwards (by more than a couple of
seconds) and a timestamp later than the last one, finish the current play and start a new one. Effort S.

### 10. (C8) A query-registry refresh blocks all browsing for minutes — **correctness (availability)**, suspected, effort S. Size cap: see T0024 S7

`crates/daemon/src/queries.rs:78-113`.

`QueryRegistry::refresh` runs on the Catalogue task: the web player's HTML, its main bundle, then ~160 chunk
URLs in 20 sequential batches of 8, each request with a 20 s timeout. Nothing else in the Catalogue runs
meanwhile (single task, one command at a time), so a slow CDN or a stalled chunk freezes every browse action
for up to 20 batches × 20 s = several minutes, with no message; the Hub's `CatCmd` channel (16) fills and
each Client's session task blocks sending. The size cap on those downloads is T0024's S7; this is the
"single task, no timeout budget" half. Trigger: a registry refresh (stale cache, or any retired hash → 412 →
`refresh`) on a bad connection.

Fix: do the crawl off the command task (a separate task with its own timeout budget) and keep serving from
the old hashes until it finishes, or cap the whole refresh (e.g. 60 s) and fail the one query. Effort M.

### 11. (C11) Commands can be dropped with a misleading "Not connected" message — **polish / correctness**, suspected, effort S

`crates/client/src/app.rs:931-941` (`send`).

```rust
match (&self.link, &self.commands) {
    (Link::Connected { .. }, Some(tx)) => tx.try_send(c).is_ok(),
    _ => false,
}
if !ok { self.say("Not connected to singularityd"); }
```

The channel holds 64 commands; the connection task drains it promptly, but a burst (a fast volume drag while
the Daemon is busy, or a reconnect) can fill it, and then a Play/Pause/Seek is thrown away with a message
that says the opposite of the truth (the Client *is* connected). The user sees one toast, assumes a hiccup,
and the command never happened. Fix: `try_send` with a short await (e.g. `timeout(250 ms, send)`) and a
message that matches ("singularityd is busy — try again"), or coalesce volume commands. Effort S.

### 12. (C12) The initial snapshot can miss a change made while it is being sent — **correctness (narrow)**, confirmed by reading, effort S

**Fixed (T0027, B6):** the subscription is made before the snapshot is built, and a playback event older than the
snapshot's anchor isn't forwarded (it's already in the snapshot). Unit test:
`playback_older_than_the_snapshot_is_superseded`. The change in the window can't be triggered deterministically from
the protocol (it's microseconds between reading the Hub and subscribing); the unit test pins the ordering rule.

`crates/daemon/src/lib.rs:554-570`: the snapshot is read from the Hub's mutexes and sent, and only *then*
`hub.events.subscribe()`. A Playback/Engine/Catalogue event in that window is in neither the snapshot nor the
subscription; with a paused/idle Daemon no later event corrects it, so the Client shows the old track/status
for as long as nothing changes (minutes). The Client has `Command::GetState`, but it does not send it on
connect (the snapshot is push-only, `conn.rs:78-90` + `main.rs:157`).

Fix: subscribe before building the snapshot, and skip the events the snapshot already contains (or send
`GetState` after `Connected` from the Client — one line in `App::on_connected`). Effort S.

### 13. (C15) `library_level` has no page cap — **correctness (low)**, suspected, effort S

**Fixed (T0027, B10):** the walk stops at the same item cap as the track lists (`library_page_continues`). Unit
test: `the_library_walk_has_a_page_cap`.

`crates/daemon/src/catalogue.rs:932-948`: `loop { … offset += LIBRARY_PAGE; if n == 0 || offset >= page.total { return } }`.
Unlike the track lists (`MAX_TRACKS`) and the backup (`MAX_TRACKS`), the library walk has no cap: a broken
`totalCount` near `u32::MAX` with non-empty pages means up to ~85 M sequential requests (and an unbounded
item list) before the `u32` wraps. Requires a hostile/broken Spotify answer, so it is low. (T0024's handoff
also flagged `fetch_pages`; that one is already bounded by `until` = `MAX_TRACKS`/`SHOW_BATCH`, so no
finding there.) Fix: cap the pages as `fetch_pages` does. Effort S.

### 14. (C13) Every background task but the Engine's is unsupervised, and they die quietly — **polish / correctness (feature silently gone)**, confirmed by reading, effort M

**Fixed (T0027, A1):** `lib::supervise` runs each one, names a stop in the log, in `Event::Error` and in
`Health.stopped_tasks` (a row goes after the task has run 60 s again), and starts it again with a growing backoff;
the Soloist update checker gets the same inside `soloist.rs` (`EngineEvent::TaskStopped`/`TaskStarted`). Unit test:
`a_background_task_that_panics_is_restarted_and_named`.

`crates/daemon/src/lib.rs:185-201` (the `tokio::spawn`s whose `JoinHandle`s are dropped), `main.rs:166`,
`soloist.rs:286,339`, `mpris.rs:28-32`.

The Daemon spawns the play logger, the backup runner, the tap, the catalogue pump, MPRIS/Now Playing, the
config follower and the Soloist update checker, and keeps none of the handles. If any of them ends or panics
(the tap has no error path at all, the others log at most one line), the feature is silently gone for the
daemon's life: media keys and the desktop widget (MPRIS), frozen audio meters (the tap), no nightly library
backups, `notifications.track_change` no longer followed, no Soloist updates. Only the Engine and the
Catalogue pumps matter more (finding 1); the health view names none of them. Fix: report an ended task in
`tracing` and an `Event::Error`, with the existing backoff restart where it makes sense (backup, MPRIS, the
tap), and add rows to the health view. Effort M.

## Checked, fine (and how)

- **Panics outside tests.** All 106 `unwrap`s in `crates/daemon/src` (and the Client's few) were checked
  one by one; those outside tests are `expect`s on invariants the code just established
  (`app_engine.rs:74` after the `is_none` check; `store.rs:356` inside `same`; `catalogue.rs:1080` after
  `remote.is_none()` was handled; `queries.rs:34` on a capture group of a matched regex; the `LazyLock`
  static regexes; `mpris.rs:125` on plain values; the mutex `expect`s below) and the static-regex/`unreachable!`
  cases that are indeed unreachable (`soloist.rs:537` after the `#[cfg]` split, `tap.rs:404` after the `App`
  source is rewritten). `player[..1]` in `catalogue.rs:1392-1393` would panic on an empty `Identity::player`, but
  every construction site passes a non-empty literal (`engine.rs:48`, `app_engine.rs:37`, `librespot.rs:42`).
  The client indexes that could go stale were checked and re-bound at the point of use
  (`sidebar.rs:66` — the library cannot change inside `draw`; `stats_entry` — one snapshot for both the entry
  and its row; `settings.rs:322` — an index found by position). `lyrics.rs:151`'s `credits.last()` is never
  empty. The LRC arithmetic in `parse_lrc` is finding 3.
- **Locks across `.await`.** Every `std::sync::Mutex` (the SQLite `Store` in `lib.rs:316/353/403`, `backup.rs:132/138`,
  `soloist_vm.rs:26`, the Connect cluster in `catalogue.rs`) is locked inside a block that ends before the
  next `.await`; the client's `covers`/`saved` have none. `tokio::sync::Mutex` guards are never held across a
  channel send that could block the same task. No lock-order cycle: `described` then `describing` everywhere
  (`fill_queue`, `CatEvent::Described`); `playback`/`remote` are taken one at a time. Poisoned stores would
  panic in `log_plays`/`backup` (a task ending silently — finding 1), not corrupt data.
- **`select!` cancel-safety.** `client_session`'s `reading`/`writer`, `run_engine`, `run_catalogue`, `tap::run`,
  `mpris::serve`, `librespot::run` (its `&mut task` and the watchdog's `timeout(.., &mut task)` share the
  JoinHandle but never select two branches on it at once) and `soloist::run_once` (`child.wait`, `ws_rx.next`,
  `sleep_until`, `cmd_rx.recv`, `news_rx.recv`, and `Notify::notified` — tokio forwards a `notify_one` permit
  when a notified future is dropped, so a select branch that loses cannot lose the update notification; a
  missed notification would only delay an update, which is checked again at the next track change). The
  `read_msg` in a cancelled branch would drop a half-read frame, but every such cancellation closes the
  session anyway.
- **Channels.** `hub.events` (256): a lagging subscriber skips events (documented) — for `log_plays` that can
  at worst mis-attribute time between two observed events, never panic; MPRIS/Now Playing only redraw state.
  `spectrum` (8): frames are disposable and skipped by design. Per-Client `out_tx` (64) has the 5 s
  write deadline (#71). `commands` (64) and `CatCmd` (16) are the two that drop silently — findings 1 and 11.
  `Feed` (64) drops packets when the tap falls behind, by design, and the tap's listener count is balanced on
  every exit path (including a Client that disconnects while subscribed).
- **Protocol skew, both ways.** Only additive changes since v1: new event variants decode as `Event::Unknown`
  (and commands as `Command::Unknown`) on an old peer; new *fields* are ignored by an old reader (serde), and
  every field added to an existing struct has `#[serde(default)]` for the old writer (`Welcome.engine`,
  `SpectrumFrame.out_*`, `Item.artist_uris`/`album_uri`, `QueueEntry.uid`, `TrackRow.uid`/`progress`/…,
  `Health.engine_build`/`missing_tools`, `ListeningSummary.plays_today`, `Lyrics.missing`). A truncated or
  oversized frame is an `Err` on both sides → reconnect, never a panic (`MAX_FRAME`, `read_exact`). The
  handshake refuses a mismatched `protocol` with a message naming `RESTART`.
- **Time.** `position_at` saturates (a clock that steps back freezes, never wraps) and clamps to the
  duration; a NaN speed clamps to 0 rather than wrapping (`as u64` saturates). Connect positions are
  re-anchored on the local clock at arrival (`connect.rs:playback`, `transfer`), and a cluster timestamp in
  the future yields elapsed 0. All intervals use `Instant` (`duration_since` saturates). The calendar code is
  integer (`local_day` via `localtime_r` with the offset at that instant, `days_from_civil` round-trips,
  DST/midnight have `tests/stats_tz.rs`); backup retention and stats use signed day numbers.
- **SQLite and backups.** `migrate` runs inside a transaction and is idempotent per `user_version`;
  `Store::open` on a fresh file creates the v1 table then migrates to 2 exactly as an old file does (tested);
  every write goes through the one `std::sync::Mutex`, so "concurrent writes" are serialised; `write_backup`
  replaces today's file atomically and retention keeps 14 days + 12 monthly firsts (tested); `delete_all`
  vacuums and removes the backups dir.
- **The tap's bounds.** `Analyzer::frame`'s band edges are within `1..N/2` for the fixed 35–16 kHz range
  (verified numerically: the top band's `i0` is ~677 < 1024), `as_chunks::<2>()` ignores an odd tail sample,
  the PCM buffer in `capture` consumes whole 8-byte frames and carries the remainder, and the quiet path
  sends one clearing frame then nothing.
- **Engine state machines.** Soloist: every exit path stops the child (`kill_on_drop`, `PDEATHSIG`, the VM
  session's stdin); the backoff restarts after a healthy run (#14 in the last audit's terms); a staged build
  is only acted on while `update_waiting()`; the resume point survives a failed restart; a bad key waits and
  a new key cuts the wait short; the queue answer no longer replaces the track (D63, apart from finding 7).
  librespot: an asked restart is immediate, a dropped session restarts from the saved state after 1 s, the 5 s
  watchdog catches a closed idle connection, and `Unavailable` is only reported, never retried in a loop.
  AppEngine: the `pgrep` check is cached for 2 s (#15 last time); `RestartEngine` is a real quit+start.
  Bookkeeping that would notice their death is missing — finding 1.
- **Queue edits.** Connect's `edit_queue` removes/moves by uid first and only falls back to the URI (and
  refuses context/autoplay entries), against the queue revision it fetched in the same call; the Client sends
  a uid whenever it has one and the Daemon resyncs (`CatEvent::Resync`) when Spotify refuses, so a stale uid
  cannot silently corrupt the queue. (The one gap is the dra**g** race, finding 8.)
- **`setup::step`'s matrix** (Soloist / app / librespot × every `EngineStatus`/`CatalogueStatus` ×
  `heard`/`engine_was_ready`/`dismissed`) was walked by hand against the tests in `setup.rs`; every
  combination maps to an actionable step and `wanted()` never nags a user who has dismissed it. `Step::Done`
  for "no status yet" is intentional (the Daemon answers within a frame).
- **The Client's stale state.** `playback_event` replaces the whole state, keeps a just-set volume for 2.5 s
  (the Connect volume-lag fix), and `lyrics` are filtered by URI at draw time, so a late answer for the
  previous track cannot show; list pages are keyed by URI and filled by offset (replays and gaps are safe);
  `saved` settles on `Event::Saved` with a pending counter per URI (an answer that arrives while a newer
  toggle is still out is ignored, so the last answer received stands) and rolls back optimistically-patched
  Liked Songs rows; playlist edits keep the pre-edit values in
  `was` and roll back in order; the sidebar/queue selections are clamped at draw time.
- **The D63 phone-play logic** was re-read end to end (finding 7 is its one gap): `change_is_ours` needs our
  own command for a playing track's change, `quiet_moment` lets a paused/idle player be updated at once, the
  restart resumes only if the new player came back with no track, and a remote change is followed rather
  than interrupted.
- **Migrations** (config 1→5, `keys.toml` 1, log 1→2): each step is in place, keeps comments and unknown
  keys, a newer file is left alone with a problem reported, and a single migration is exercised per schema
  (their tests).

## What is not done here

- No live Spotify session was used (no phone, no account): the D63 logic, the Connect queue edits and the
  app-engine queue were verified by reading and by stand-ins only. Findings 7 and 8 are the ones a real phone
  session would settle first.
- macOS-only code (Now Playing, the process tap, the VM) was not read; #81 covers the VM clock and T0026 owns
  the platform seams.
- Findings from the other two workers are cross-referenced in the shared notes
  (`~/.cache/regent/shared/singularity-audit/correctness.md`): T0024's S8 (the same `describing` set, memory
  side), S7 (download caps — finding 10's sibling), and their handoff's LRC overflow (finding 3).

## The added tests

`crates/daemon/tests/audit_correctness.rs`, all `#[ignore]`d so `main` stays green; each names its finding and
fails against today's code (removing the `#[ignore]` line shows it):

```
cargo test -p singularityd --test audit_correctness -- --ignored   # 4 fail: C1, C2, C10, C14
cargo test -p singularityd --test audit_correctness                # 0 run: main stays green
```
