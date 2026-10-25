# Codebase audit, second pass — security and trust boundaries (T0024)

Date: 2026-10-25 · Branch: `regent/T0024-audit-security` (off `main` c751a81) · Worker: pi-deepseek
Previous audit: `docs/research/2026-10-audit.md` (2026-10-20…24). Its “Checked, fine” claims are treated as claims
below and were re-read, not assumed. Shared notes for the three audit workers:
`~/.cache/regent/shared/singularity-audit/security.md` (my S1–S10) and `handoffs.md`.

## Scope

Read in full or in the security-relevant parts:

- **Daemon** (`crates/daemon/src`, ~12.2 k lines): `lib.rs`, `engine.rs`, `main.rs`, `soloist.rs`, `update.rs`,
  `app_engine.rs`, `librespot.rs`, `catalogue.rs`, `connect.rs`, `queries.rs`, `library.rs`, `lyrics.rs`,
  `playlists.rs`, `store.rs`, `backup.rs`, `secrets.rs`, `programs.rs`, `health.rs`, `tap.rs`, `mpris.rs`,
  `nowplaying.rs`, `process_tap.rs`, `pcm.rs`, `soloist_vm.rs`, `vm.rs`, plus the integration tests.
- **Protocol and config**: all of `crates/proto/src/lib.rs` and `crates/config/src/lib.rs`.
- **Client** (`crates/client/src`, ~16.5 k lines): `conn.rs`, `main.rs`, `gfx.rs`, `covers.rs`, `cover.rs`,
  `search.rs`, `theme.rs`, `layouts.rs`, `term.rs`, `widgets.rs`, `ui.rs`’s text path, and the parts of `app.rs`
  that consume protocol/network values. The drawing code was only skimmed for raw terminal writes.

Not done: every drawing branch of the client, a run on macOS (the macOS-only modules were read, not compiled here), a
live account or the owner’s files, and dynamic race hunting beyond the tests below. No Spotify request was made.

## Method

1. Follow every value that comes from outside the process: protocol frames, Socket/Connect answers, lyrics, image
   URLs, archive/CDN answers, config files, environment, and the names other local processes can choose.
2. For each `unsafe` block, write down the invariant it relies on and check the call sites.
3. Re-run `cargo audit` and `cargo deny check` once (table below), and re-read the previous audit’s triage.
4. Add bug-proving tests as `#[ignore = "audit S…"]`; removing the ignore shows the failure. No product code changed.

## Findings (most severe first)

### S1 — security — Soloist’s state directory and log are readable by other local users

`crates/daemon/src/soloist.rs:515-521`.

`run_once` creates `…/singularity/soloist-state/data` and `…/cache` with `create_dir_all` (0777 & umask → normally
0755) and `soloist.log` with `tokio::fs::File::create` (0666 & umask → normally 0644); `.log.1` is the previous file
with the same mode. There is no `set_permissions`, unlike `daemon.log` (fixed in the previous audit, finding 1),
`catalogue/` (0700), the secret fallback (0700/0600), `vm/` (0700), the SQLite store (0600) and the recent-searches
file (0600).

Conditions: any other local user who can traverse the user’s home (0755 homes are common) can read the verbose log
and list the state/cache. The log records what plays (the Daemon itself writes Soloist’s output line by line,
`line_rx` → `log.write_all`), and Soloist’s state directory is where the guest/local Soloist keeps its pairing and
cache data — the Daemon cannot promise those files are private. The log is also written **raw**: Soloist’s verbose
lines (which can contain track metadata from Spotify or another person’s playlist) are written without stripping
control characters; `strip_ansi` is only used by the parser. `cat soloist.log` in a terminal is then a terminal-escape
sink (title, OSC 52, hyperlinks) for hostile track names, exactly the class the previous audit closed for the UI.

Evidence: **confirmed by code path**; the exact modes depend on the process umask (the Daemon never sets one). The
owner’s live files were not inspected (working agreement).

Suggested fix: create the state directory 0700 (and `data`/`cache` below it), open the log with
`OpenOptionsExt::mode(0o600)` and `set_permissions` it (and the renamed `.log.1`) to 0600 after opening; strip control
characters before writing log lines (the parser already has `strip_ansi`, broaden it to remove `\x00..\x1f\x7f`).
Effort: **S**.

### S2 — security — `ExportLocalData` follows a symlink planted in the Downloads folder

`crates/daemon/src/store.rs:279-309` (`export`, `write_private`).

When the user asks to export local data, the Daemon writes
`<Downloads>/singularity-listening-YYYY-MM-DD.tmp` with `OpenOptions::create(true).truncate(true).mode(0600)` and then
renames it. `mode(0600)` only applies when the file is **created**; `open` follows an existing symlink, and the
name/date are predictable. If the Downloads folder is shared with another local user (a group-writable folder, a
`XDG_DOWNLOAD_DIR` pointing at `/tmp` or a shared mount), that user can pre-create that `.tmp` symlink to any file the
Daemon’s user can write: the export truncates and overwrites the target with JSON, then renames the symlink to the
final name. This is a local file-clobber/data-loss primitive, and can be used against shell startup files.

Evidence: **confirmed**; the ignored test
`store::tests::export_does_not_follow_a_planted_temp_symlink` creates the symlink, calls `export`, and currently sees
the victim file replaced with the export JSON (the test fails when un-ignored).

Suggested fix: write a random temp file in the destination with `create_new(true)` plus `O_NOFOLLOW` (or use
`tempfile::NamedTempFile::new_in`), and reject/replace a symlink with `symlink_metadata` before the rename. Effort:
**S**.

### S10 — security — children inherit the secret-bearing environment

`crates/daemon/src/app_engine.rs:143-156`; `client/src/main.rs:178-179`; every `Command::new` (default inheritance).

`SOLOIST_API_KEY` is a supported way to supply the Soloist key (`soloist.rs:106`, named by the health view). The
Daemon’s children inherit the whole environment, including that key:

- AppEngine starts `spotify --minimized` (Linux) or `open -gj -a Spotify` (macOS). On Linux the app and its helper
  processes inherit the key and can read it from their own environment (`/proc/<pid>/environ`, same user); on macOS
  the `open` child inherits it (whether LaunchServices passes it on to Spotify is system-dependent). Any crash
  dump/log that prints the environment repeats it. The app has no use for the key.
- The Client opens the login URL with `xdg-open`/`open`, whose descendants inherit the Client’s environment too.
- `pactl`, `parec`, `secret-tool`, `tar`, `ssh`, `vfkit`, `gvproxy`, `python3`, `dns-sd`, `shasum`, `ssh-keygen`,
  `hdiutil` and `osascript` inherit it as well; these are system tools, but (with S3) a PATH-planted one would get it.

Soloist itself needs the key (it gets it as `--api-key`; inheriting it too is redundant). The documented argv
exposure of the key (`ps`) is a separate, accepted limitation; this is about unrelated children.

Evidence: **confirmed** by `Command`’s default environment inheritance and the spawn sites; no test.

Suggested fix: `.env_remove("SOLOIST_API_KEY")` on every child that does not need it (at least AppEngine’s app/opener
and the client’s opener), or build children with `.env_clear()` plus the few variables they need; for Soloist, pass the
key only via argv (already) and remove it from the child env too. Effort: **S**.

### S3 — security (hardening) — several children still come from `PATH`

`crates/daemon/src/app_engine.rs:131,145,155,180`; `crates/client/src/main.rs:178-179`;
`crates/client/src/conn.rs:180,193`.

#77 moved `pgrep`, `pkill`, `tar`, `secret-tool`, `pactl`, `parec` to fixed locations, but these remain bare names:

- Daemon AppEngine: `open` (macOS, `-gj -a Spotify`), `spotify` (Linux, `--minimized`), and `defaults`, `scutil`,
  `hostname` through the `output(cmd, …)` helper. `installed()` on Linux also picks a `spotify` by scanning `PATH`.
- Client: `xdg-open`/`open` for the Spotify login URL, and the fallback `singularityd` in `start_daemon()` when no
  daemon binary sits next to the client.

Conditions: the Daemon inherits the PATH of whatever started it (a shell, a wrapper, an editor terminal). A
planted `spotify`/`open`/`xdg-open` earlier on PATH is executed with the user’s privileges and (see S10) the
Daemon’s environment. The previous audit’s own rationale for #77 (“PATH belongs to whoever started the Daemon”) still
applies; this is the remainder it missed.

Evidence: **confirmed by code path**; no product test because it requires mutating a live PATH.

Suggested fix: extend `programs::Tool` with `Open`, `Spotify` (Linux: a short allowlist such as `/usr/bin`,
`/usr/local/bin`, and perhaps the user’s known install dirs — but not arbitrary PATH), `Defaults`, `Scutil`,
`Hostname`, `XdgOpen`; use absolute paths for the client opener; if the client cannot find `singularityd` next to
itself, report it instead of falling back to PATH. Effort: **M**.

### S4 — security (hardening) — the Soloist build archive is trusted after extraction

`crates/daemon/src/update.rs:139-160,208`; `crates/daemon/src/vm.rs:250-260,331-346`.

`download_with_cap` streams the tarball with a 256 MB compressed cap, then runs `tar -xzf` into `soloist.new/`,
executes `soloist.new/soloist --version`, and writes `build.json`. Nothing checks the extracted tree:

- `write_build(&dir, …)` uses `std::fs::write(dir.join("build.json"))`; a **symlinked** `build.json` in the archive
  is followed, so the write lands on the link’s target outside `soloist.new/`. Proved by the ignored test
  `an_archive_cannot_write_outside_its_directory_through_a_symlink` (currently the victim file is replaced with the
  build JSON).
- `version_of(&dir.join("soloist"))` executes whatever is at that name; a symlink to an existing executable (or a
  script inside the archive) is run with `--version` before install.
- Hard links, setuid bits and member permissions are not inspected; GNU tar’s own traversal defenses are the only
  ones. The 256 MB cap is on the **compressed** stream, so a gzip bomb within the cap can expand to hundreds of GB
  and fill the disk during extraction. The VM’s guest script (`sgl-soloist`) has the same `curl -sf | tar -xz`
  pattern with no cap at all (inside the guest).

Conditions: a malicious or compromised CDN answer (the update is fetched over TLS from Spotify’s CDN), or a broken
archive. The CDN is already trusted to supply the binary, so this is defense in depth; it still converts a
write-outside bug into arbitrary file overwrite and makes the staging step run an attacker-chosen target.

Suggested fix: extract with a Rust `tar` reader (or verify the tree afterwards) refusing symlinks/hard links,
`..`/absolute members, non-regular entries and files owned by anyone but the user; cap the **unpacked** byte count and
entry count; require `soloist` to be a regular executable file and `build.json` not to exist (or be a regular file)
before writing; do the same with `curl --max-filesize`/a checked `tar` in the guest script. Effort: **M**.

### S5 — security (low) / correctness — the socket directory check only looks at the last component

`crates/daemon/src/lib.rs:92-113`.

`bind` calls `create_dir_all(dir)`, checks `symlink_metadata(dir)` (final component), then `set_permissions(dir,
0700)`. Consequences:

1. An **intermediate** symlink is followed: `bind(<tmp>/link/inside/d.sock)` where `link` is a symlink to a directory
   owned by the user succeeds and puts the socket under the link target. The ignored test
   `the_socket_never_goes_through_an_intermediate_symlink` proves it (currently `bind` returns `Ok`). In the
   `/tmp/singularity-<uid>` fallback an attacker can create that symlink; the final directory still has to be user-
   owned, so the socket lands in a user-owned place, but the attacker controls the path resolution and can retarget it
   later (denial of service; the Client’s `peer_cred` check still refuses a socket owned by another uid).
2. `bind` chmods **whatever** directory contains `SINGULARITY_SOCKET` to 0700 — e.g. `$HOME` if the user sets
   `SINGULARITY_SOCKET=$HOME/daemon.sock`. That is a surprising mode change on an arbitrary pre-existing directory.
3. The directory is created with the process umask and only then chmod’ed; with a permissive umask (000/002) another
   user has a window to create `daemon.sock` or another entry first. That is a DoS (the real Daemon exits when it sees
   the path in use); no code execution, because clients check the peer uid.

Evidence: **confirmed** for (1), (2) by code and the test; (3) by code path.

Suggested fix: create the socket directory with `DirBuilder::recursive(true).mode(0o700)` (or set the umask around
creation) and verify every component with `symlink_metadata` (no symlinks, owned by the user), or use
`openat(O_NOFOLLOW)`; do not chmod a pre-existing arbitrary parent — require the dedicated `singularity` directory
under the runtime dir. Effort: **S/M**.

### S6 — security (low) — the MPRIS notification cover fetch accepts any URL

`crates/daemon/src/mpris.rs:399-410`.

`cached_cover` calls `client.get(url)` without checking the scheme, unlike the Client’s cover path
(`covers.rs` requires `https://`). Playback metadata (`Item.cover_url`) comes from Soloist/Spotify/Connect; a broken
or hostile answer can make the Daemon issue a GET to `http://127.0.0.1:<port>/…` or another plain-HTTP/local address
(blind SSRF/CSRF-style request) and cache the body under `notify-cover.jpg`. The response is not sent back anywhere,
so the impact is limited, but the two cover paths should agree.

Evidence: **confirmed by code path** (exploitability depends on influencing metadata: suspected).

Suggested fix: require `https://` (and optionally Spotify’s image hosts) before `client.get`, as the Client does.
Effort: **S**.

### S7 — resources — three downloads have no size cap (or timeout)

`crates/daemon/src/vm.rs:250-260`; `crates/daemon/src/queries.rs:101-103`; the guest script at `vm.rs:331-346`.

- `vm::download` (vfkit, gvproxy, the Debian image) uses `reqwest::get` with no timeout and writes every chunk with no
  cap; the checksum is compared only after the whole body is on disk, so a huge/broken answer fills the disk first.
- `QueryRegistry::get` reads the web-player HTML/JS with `.text()` and no cap or timeout beyond the client’s 20 s;
  `refresh` fetches ~160 chunks, so a huge answer is held in memory whole.
- The guest’s `sgl-soloist` uses `curl -sf … | tar -xz` with no `--max-filesize`/unpacked cap.

The Soloist update itself is capped (#74); these are the paths the #74 fix did not cover. A related correctness issue
(a refresh blocks the Catalogue task for minutes) is T0025 C8; **see also C8**, not repeated.

Evidence: **confirmed by code path**.

Suggested fix: reuse `save_archive`-style counting for `vm::download`, add a `reqwest` timeout, cap `get` bodies
(web-player chunks are small) and the guest `curl`/`tar`; delete partial files on failure. Effort: **S**.

### S8 — resources — both Hub description maps are unbounded

`crates/daemon/src/lib.rs:381-395,405-417`; `crates/daemon/src/catalogue.rs:1448-1478`.

`Hub::described` receives every `CatEvent::Described` entry and is never trimmed (the Catalogue’s own `described`
cache is trimmed with `MAX_DESCRIBED`; the Hub’s is not). `Hub::describing` records every bare URI asked about and
only removes it when a `Described` answer arrives; `State::describe` takes 20 at a time and `break`s on the first
error, so failed/truncated lookups stay in `describing` forever — those tracks are never looked up again, and unique
URIs accumulate for the Daemon’s life. This is the one cache the #71 bounded-cache fix missed (T0025 C1/C3 found the
same wedge from the correctness side; **see also C1, C3, C2**).

Evidence: **confirmed by code path**; T0025 has the failing test for the “asked once, never again” half.

Suggested fix: trim `Hub::described` with the same bound as the Catalogue (`MAX_DESCRIBED`), and remove URIs from
`describing` when a `Describe` fails or when it is dropped (send a failure event, or cap/clear the set); do not
`break` on the first failure without telling the Hub. Effort: **S**.

### S9 — security (low) — private writers do not tighten an existing file’s mode

`crates/daemon/src/secrets.rs:137-150`; `crates/daemon/src/store.rs:304-309`.

`write_private` (secrets) and the store’s `write_private` open with `OpenOptionsExt::mode(0o600)`, which only applies
when creating; an existing file with 0644 keeps that mode (the SQLite store explicitly `set_permissions` its DB, so
the pattern is known). A secret fallback file or an exported/backup file copied with loose permissions stays
readable. The directories are 0700 on creation, so this needs a pre-existing loose file (an older build, a copied
backup, a restore); it is a hardening gap, not a live exploit.

Evidence: **confirmed** by `OpenOptions` semantics and code (`mode(…)` only on create; no `set_permissions` after
open).

Suggested fix: `set_permissions(path, 0600)` after opening (or `fchmod` the handle) in both helpers, as
`Store::open` already does. Effort: **S**.

## Checked, fine

- **Socket ownership and peer credentials.** `bind` refuses a final directory that is a symlink/not ours/whose parent
  is not ours or root (S5 covers the intermediate-component gap); the Daemon drops connections whose `peer_cred` uid
  is not ours (`lib.rs:123`); the Client verifies the peer uid before the handshake (`client/conn.rs:52`). A live
  Daemon is detected by connecting, a stale path is removed. The socket directory is 0700 after `bind`.
- **Protocol framing.** 4-byte big-endian length, `MAX_FRAME = 16 MiB` checked before the body allocation on read and
  after serialization on write; `serde_json` has its own recursion limit; unknown variants decode as `Unknown`.
  (A same-uid process can still open many connections; it can already ask the Daemon to `Shutdown`, so this is not a
  new boundary. Not filed.)
- **Commands do not take filesystem paths or start processes from sent values.** `SetPlaylistCover` carries base64
  JPEG bytes (decoded with a bounded, overflow-checked decoder), not a path; playlist changes validate the id as
  base62 (`playlists.rs::playlist_path`); `ExportLocalData`/`DeleteLocalData` use the store’s fixed paths (S2 is the
  temp-file issue); `SetSoloistKey` stores to the keyring/fixed fallback; no command names a config path or a program.
  `Shutdown` only ends the Daemon for a same-uid client (the socket dir is 0700 and peer uid is checked).
- **Secrets in logs and errors.** Grepped every `tracing`/`eprintln`/`println` near secret handling: `api_key` is
  never formatted; `store_key` logs only where the key went; `secrets::set/get/clear` log nothing; `health::location`
  reports `environment (SOLOIST_API_KEY)`/`keyring`/`file`/`missing`, never the value; `BadKey` says Spotify rejected
  it without echoing it. The one file-permission gap is S1; S9 is the pre-existing-mode gap.
- **Keyring use.** Linux `secret-tool store` gets the value on stdin; macOS `security -i` gets a hex-encoded line on
  stdin, never on a command line; the account name is checked alphanumeric for the `-i` path. `SINGULARITY_NO_KEYRING`
  skips the keyring for tests/deliberate file-only setups.
- **Subprocess argument building.** Every `Command` passes values as separate argv entries (no shell): `pkill -x`
  exact names, notification title/body as `osascript` argv, `gvproxy`/`vfkit` numeric/constant arguments, VM SSH
  commands are constant scripts with the key on stdin, `dns-sd` name/port as argv. The issues are PATH (S3) and
  environment inheritance (S10), not quoting/injection.
- **Downloads and updates (apart from S4/S7).** reqwest uses rustls with default certificate validation (no
  `danger_accept_invalid_certs`, no HTTP-only fallback in the real CDN URL); the Soloist archive is streamed under a
  256 MB compressed cap, unpacked into a fresh directory, removed on any failure, and `--version` is run before
  install. The older build is kept. No expiry check is a signature check; the TLS trust in Spotify’s CDN is the
  documented model.
- **Network parsers.** Read `library.rs`, `lyrics.rs`, `playlists.rs`, `catalogue.rs`’s Pathfinder/Home parsing,
  `client/covers.rs` and `client/cover.rs`. Values are defensively defaulted; no parser recurses on nested JSON beyond
  `serde_json`’s limit; string slicing positions come from `find`/fixed ASCII; the previous audit’s `hostile_input`
  test (20 000 random JSON/LRC/frames) plus `cargo fuzz` corpora still apply. Download bodies are capped for Client
  covers (8 MB, decode 4096²/256 MB) and MPRIS covers (8 MB); image URLs from Home/library are fetched only when drawn
  and only if `https://` (Client); S6 is the MPRIS exception. **Fixed (T0029):** a picture dropped or typed for a
  playlist cover (`client/cover.rs` `prepare`) decoded without those limits; it now goes through `covers::decode`, and
  a test checks that a 12000×12000 header is refused before anything is allocated. URLs taken from answers are used in
  JSON bodies or fixed hosts (`spclient.wg.spotify.com`, `image-upload.spotify.com`, `lrclib.net`, `open.spotify.com`), not
  `file://`/local paths.
- **Terminal output.** All Client text goes through `Buffer::set_stringn`/ratatui, which filters `char::is_control`
  (checked in `ratatui-core`’s `buffer.rs`); the raw writes are kitty graphics built from numeric ids/crops and
  base64, and an OSC 52 clipboard payload that is base64 of a `https://open.spotify.com/...` link. Hostile track,
  playlist and artist names cannot inject escapes into the UI. (S1 is the log file, not the UI.)
- **`unsafe` blocks re-checked.** `main.rs` `fstat`/`fchmod` on fd 2 (regular file only); `soloist.rs` `prctl` in
  `pre_exec` (async-signal-safe call, then `Ok`); `store.rs` `localtime_r` on a zeroed `tm` with the null check;
  `gfx.rs` `shm_open(O_CREAT|O_EXCL, 0600)`/`ftruncate`/`mmap`/`copy_nonoverlapping`/`munmap`/`unlink` with every
  failure path unlinking, names unique per upload and swept after 2 s; `conn.rs` `proc_name` into a 64-byte buffer,
  `kill` only after checking the pid’s executable name starts with `singularityd`; `widgets.rs`/`visual/mod.rs`
  libproc/`getrusage` buffers sized before the call and counts bounds-checked; `process_tap.rs` Core Audio pointers
  from locals/boxed `Ctx` that outlive the callback, `mBuffers[0]` after checking `mNumberBuffers > 0` and `mData`
  non-null, byte size / 4, panic caught around the sink; `nowplaying.rs` Objective-C objects are Apple constants or
  `cast_unchecked` to `AnyObject`, handlers live for the process; `pcm.rs` (no `unsafe`) parses fixed 8-byte sample
  chunks and bounds the jitter buffer. No new memory-safety issue found; Miri cannot run these FFI paths.
- **Files at rest.** `daemon.log` 0600 (Client open + Daemon `fchmod`); SQLite `listening.db` 0600; backups dir 0700,
  files 0600; secret fallback dir 0700/file 0600 on creation; catalogue credentials dir 0700 (librespot cache below
  it); VM dir 0700, SSH key 0600; recent searches 0600; config `config.toml`/`keys.toml` 0644 but carry no secrets
  (only settings/device names). Gaps: S1, S2, S9.
- **Environment/ownership of the fallback dirs.** The socket directory checks ownership (S5 for the intermediate
  symlink); the other data/config dirs fall back to `temp_dir()` only when `HOME`/`XDG_*` are unset — uncommon for a
  desktop login, and the files themselves are 0600/0700 on creation. Not filed separately.
- **`cargo audit` / `cargo deny`.** See the table; the previous triage is still the whole set.

## Tools

| Tool | Result |
|------|--------|
| `cargo audit` (586 crates, advisory DB fetched 2026-10-25) | 3 advisories, unchanged from the previous audit: `quick-xml` 0.38.4 RUSTSEC-2026-0194/0195 (both high, quadratic attribute check / unbounded namespace allocation) pulled in by the librespot fork, which parses only Spotify’s authenticated answers; `rsa` 0.9.10 RUSTSEC-2023-0071 (Marvin timing on decryption, not used: librespot only verifies Spotify’s signature). All three are accepted with reasons in `deny.toml`; no new advisory. `cargo audit` therefore exits 1 — expected for this tree. |
| `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` (the same three reasoned ignores; no unmaintained zbus 5 helpers after #75). |
| `cargo test --workspace -j2` | **293 passed, 0 failed, 8 ignored** (the previous 5 plus this audit’s 3 ignored bug proofs); green. The 3 new tests are `#[ignore]` with `audit S…` reasons. |
| `cargo clippy --workspace --all-targets -j2` | clean (Linux, 2026-10-25). |
| Bug-proving tests (all ignored) | `store::tests::export_does_not_follow_a_planted_temp_symlink` (S2), `tests/update.rs::an_archive_cannot_write_outside_its_directory_through_a_symlink` (S4), `tests/protocol.rs::the_socket_never_goes_through_an_intermediate_symlink` (S5). Removing the ignore shows the failure. |

## Changes in this branch

- 3 ignored tests (above), no product code.
- This report; one line in `docs/HANDOFF.md` pointing here; shared `security.md`/`handoffs.md` entries.

## Not filed / left for the other angles

- T0025 already has C1/C3 (the description wedge), C14 (LRC overflow from my handoff) and C15 (the page-counter
  wrap); C8 (refresh blocks browsing) is the correctness side of S7. My S8 adds the Hub-side unbounded maps.
- T0026 has the PATH/`Tool` cleanup (S3) and the `tar`-reader suggestion (S4) in its handoffs.
- Residual, not filed: a same-uid process can ask the Daemon to `Shutdown` or open many protocol connections; both are
  within that user’s authority and the socket dir is 0700. The 16 MiB frame allocation is reached only after the peer
  uid check.
