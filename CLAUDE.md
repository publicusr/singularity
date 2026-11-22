# Working agreements

- **New session? Read `docs/HANDOFF.md` first** if you have one: a local, git-ignored file with the current state,
  feedback and the next tickets.

- Planning runs on the grilling method (the `mattpocock-skills` plugin's `grilling`): rounds over the design tree in
  `docs/DESIGN-TREE.md`, numbered questions with a ➡️ recommendation each, worded so "yes" accepts.
  Look facts up yourself; put decisions to the owner and wait.
- Planning is done; implementation is authorised ticket by ticket (owner: "keep building, follow the plan").
  New product decisions still go to the owner in grilling format.
- After each round: update `docs/DESIGN-TREE.md` (status, outcome, round log), write resolved terms
  to `GLOSSARY.md` immediately, and offer an ADR only for decisions that are hard to reverse,
  surprising, and a real trade-off.
- `docs/SPEC.md` is the v1 spec; tickets are GitHub issues (`ready-for-agent`). Close them with `Closes #N` in commits.
- Spotify's platform rules change often. Re-verify facts in `docs/research/` that are more than ~3 months old before
  building on them.
- Never commit secrets (Soloist API key, tokens).
- **Never write the owner's personal data into the repo** (code, docs, comments, fixtures, pty scenarios or commit
  messages): no real name, device or host names, account names, e-mail addresses, or absolute local paths
  (`/home/…`, `/Users/…`, `~/…`). Write "the owner", "the PC", "the Mac", "a phone",
  `$HOME`, `~/src/singularity`. Fixtures recorded from live answers get fake values, not the owner's. The scrub and
  its history plan are in `docs/research/2026-10-history-scrub.md`.
- Every popup is a `Modal` variant (`crates/client/src/app.rs`): that gives it the mouse, and the compiler then asks
  for its arm in `App::modal_scroll` (what the wheel does in it) and the popup wheel test opens it. Lists scroll with
  `scroll::moved`/`scroll::window`, tall text with a `scroll::TextScroll`.
- The same for everything else the wheel can be over: every Widget is a `wheel::Widget` variant
  (`crates/client/src/wheel.rs`) and every main-Pane page an `app::Page`; the compiler asks for their arms in
  `App::pane_wheel`/`App::page_wheel` and in the wheel test's cases (`wheel/tests.rs`), which wheels each one, holding
  more than fits in a short Pane, notch by notch. Anything that can hold more than fits scrolls (a list's window follows
  its highlight; a view that follows the song is a `scroll::Follow`); pty `wheel-all` shows them all.
- Verify user-visible changes in a pseudo-terminal (pyte + PIL screenshots) before closing a ticket.
- Only kill processes you started; test sockets go in `$XDG_RUNTIME_DIR/sgl-*`, on macOS
  `/tmp/sgl-<uid>/` (Unix socket paths must be short).
- Linux and macOS have the same features (D49): OS-specific code only behind the platform seams in `docs/PLATFORMS.md`.
  Before pushing, `tools/check.sh` runs what CI runs. On a Mac, `tools/check-linux.sh` type-checks the Linux build;
  run it before committing. `docs/PLATFORMS.md` is the
  exact parity status: update its rows whenever something changes on either OS. The README's "Build and run" and
  "macOS quirks" sections are the user-facing version: keep them true too.
- Test Daemons pin their Engine and stay in scratch dirs (`SINGULARITY_ENGINE`, `SINGULARITY_SOLOIST=local`,
  `SINGULARITY_START_APP=0`, `SINGULARITY_NOWPLAYING=off`, `SINGULARITY_NO_KEYRING=1`, scratch `XDG_*`; `tools/pty/shot.py` sets them all).

## Agent skills

### Issue tracker

GitHub Issues on this repo, through `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default roles, each label named after its role (`needs-triage` … `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the root. See `docs/agents/domain.md`.
