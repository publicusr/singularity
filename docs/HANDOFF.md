# Session handoff (2026-10-17)

Read this first in a new session, then `CLAUDE.md`, `docs/SPEC.md` and `GLOSSARY.md`.

## Where things stand

- **Phase 0 (planning) ✅**: four grilling rounds; decisions in `docs/DESIGN-TREE.md`; ADRs 0001–0004.
- **Phase 1 (spikes) ✅**: Soloist streams lossless FLAC once the Connect device's quality is set to Lossless
  (per device, from the phone); the private API returns the owner's real Home feed. See `spikes/*/FINDINGS.md`.
- **Phase 2 (walking skeleton) 🔄**: spec issue **#1** with v1 tickets **#2–#24** on GitHub (`publicusr/singularity`,
  label `ready-for-agent`, native "blocked by" links). **Closed: #2, #3, #4, #7, #24, #8, #9.** The owner tested the build: "everything
  feels good".

## What exists in `crates/`

| Crate | What it does now |
|-------|------------------|
| `proto` | Versioned length-prefixed JSON protocol: Hello/Welcome/Incompatible, Command, Event (Playback, Engine, Catalogue, Home, Error), unknown-variant tolerant |
| `daemon` (`singularityd`) | Hub + Unix socket; `Engine` trait (task + channels) with `SoloistEngine` (download, keyring key, WebSocket, restart/backoff, expiry, PDEATHSIG), `FakeEngine` (`--fake-engine`), `NoEngine`; `catalogue` (device-code login, librespot fork session, Home → Shelves/Cards); `queries` (QueryRegistry: hashes from web-player chunks, 3-day cache, refresh on 412) |
| `client` (`singularity`) | Autostarts/reconnects to the Daemon; split-tree layout (Singularity preset, draggable dividers); theme tokens (OLED); hit-testing; command palette; help; player bar with transport/seek/volume/shuffle/repeat; login overlay; Home as a text preview |

Tests: 60 passing (`cargo test`), covering protocol seam tests with FakeEngine, a Soloist transcript contract test, layout
invariants, UI snapshots, the Home parser, QueryRegistry fixtures and the seek/volume scrub math.

Local state on the owner's machine (not in git): Soloist at `~/.local/share/singularity/soloist/`, its pairing in
`…/soloist-state/`, Spotify credentials in `…/catalogue/credentials.json` (0600, copied from spike 2; move to the
keyring in #5), query cache in `~/.cache/singularity/pathfinder-queries.json`.

## Owner feedback to act on

- **Seek precision (#24) — done, awaiting the owner's test:** SGR-Pixels pointer on kitty/Ghostty/WezTerm, 1 s-per-cell
  fine drag elsewhere, tooltip, seek on release, wheel/Shift+arrow ±1 s (`crates/client/src/scrub.rs`, `term.rs`).
  If kitty's pixel origin turns out off by a cell, check `MouseUnits::to_cells`.
- **Home (#8) — done, awaiting the owner's test:** D28 settled (arrows browse the focused Pane; seek = Shift+←/→ and `,`/`.`;
  snap scrolling). Pixel tier lives in `client/src/gfx.rs` (`Kitty`: transmit once, place, evict) — reuse it for #16.
  Covers: `client/src/covers.rs`. Not verified in a real kitty window by me (pty only, APC stream decoded); owner to confirm.
- **Library (#9) — done, awaiting the owner's test:** ConnectRemote (`daemon/src/connect.rs`) starts contexts at track N
  (verified live on the owner's Soloist; facts in `docs/research/2026-10-library-and-connect.md`). Dev probes:
  `cargo run -p singularityd --example probe -- <op> '<vars>'` (shape only) and `--example connect_probe` (device list).
  `/` filters a list (type-to-filter needs a key because letters are bound); a folder's live shape is unverified
  (the owner has none), the parser accepts any `*Folder*` typename. Follow-up for #5: Soloist's API key is visible in `ps`.
- Owner preferences established across the rounds (also in SPEC § Design principles): OLED black everywhere, calm
  visuals, no white peaks, full resolution first, function before spectacle, terminal-driven aesthetics, never cover
  or move content (only the idle takeover may).

## Next tickets (frontier, in suggested order)

1. **#15** AudioTap → spectrum stream; **#16** GPU black hole + gravity well (port from the prototype: `blackhole.wgsl`,
   `gpu.rs`, `bend_borders`, `warped_frame`)
2. Then #5 (setup wizard + keyring for the Spotify credentials), #6 (auto-update), #12, #13, #14, #18, #19, #20, #21, #22, #23, #17, #10, #11

## Working rules (from CLAUDE.md, repeated because they matter)

- One ticket at a time: code + tests, run `cargo test`, verify in a pty (`/tmp/…/scratchpad/shot.py` pattern: pyte + PIL in a venv, real binaries,
  own daemon on `$XDG_RUNTIME_DIR/sgl-<pid>.sock`, pty winsize with pixel dims so pixel-mouse paths can be exercised; recreate it if the scratchpad is gone), commit with `Closes #N` and the co-author line, push.
- Never commit secrets or personal listening data. Fixtures are anonymised.
- Kill only processes you started (match by exact name and by the socket/state path; never `pkill -f` patterns that match your own shell).
- Unix socket paths must stay short (< 108 bytes): use `$XDG_RUNTIME_DIR/sgl-…` for test sockets.
- Spotify's private API: metadata only, no crawling, back off, never retry in a loop.

---

## Prompt for the next session

> You're continuing **Singularity**, a terminal Spotify client in Rust (ratatui) at `~/src/singularity`
> (GitHub: `publicusr/singularity`, private). Start by reading `docs/HANDOFF.md`, then `CLAUDE.md`, `docs/SPEC.md`,
> `GLOSSARY.md` and the ADRs in `docs/adr/`. Planning and spikes are done; we're building v1 ticket by ticket from the
> GitHub issues (`gh issue list -R publicusr/singularity --label ready-for-agent --state open`), following Matt Pocock's
> flow (spec → tickets → implement), with the project's vocabulary.
>
> Work the frontier in this order: **#24** (seek bar precise to 1 s when dragging, owner feedback: currently it jumps 3–4 s),
> then **#8** (Home with cover-art cards, pixel tier via kitty + half-block fallback), then **#9**, **#15**, **#16**. For each
> ticket: implement with tests at the seams named in the spec, run `cargo test`, verify the real binaries in a pseudo-terminal
> (screenshots), commit with `Closes #N` plus the co-author line, push, and keep `docs/` current (DESIGN-TREE round log,
> ROADMAP, HANDOFF). Reuse the prototype in `prototypes/design-directions` for rendering and visuals, rewritten to production
> quality. Don't commit secrets or personal listening data; only kill processes you started. When a decision is genuinely
> the owner's, ask in the grilling format (numbered questions, a ➡️ recommendation each, worded so "yes" accepts). Keep
> messages to the owner concise; they test the builds themselves and give feedback.
