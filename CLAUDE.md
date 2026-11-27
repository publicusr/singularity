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
- Specs are built on an **integration branch** (D87: `d87`): helpers branch `d87-<name>` from it, PRs target it, and
  `tools/integrate.sh <branch>` merges a reviewed one in (merge, `run-hidden tools/check.sh`, push). The spec's
  planning docs (DESIGN-TREE row, ADR, glossary terms) land on the integration branch **before** its first build.
  Issues close when the integration branch reaches `main`.
- Helper briefs point at `docs/agents/helper-brief.md` (the rules every build agent follows); review briefs point at
  `CODING_STANDARDS.md` (anything beyond the ticket is a finding for the owner; "nothing else changes" is checked on
  the drawn buffer).
- Owner app tests and re-tests follow `docs/agents/owner-test.md`.
- Spotify's platform rules change often. Re-verify facts in `docs/research/` that are more than ~3 months old before
  building on them.
- Never commit secrets (Soloist API key, tokens).
- **Never write the owner's personal data into the repo** (code, docs, comments, fixtures, pty scenarios or commit
  messages): no real name, device or host names, account names, e-mail addresses, or absolute local paths
  (`/home/…`, `/Users/…`, `~/…`). Write "the owner", "the PC", "the Mac", "a phone",
  `$HOME`, `~/src/singularity`. Fixtures recorded from live answers get fake values, not the owner's. The scrub and
  its history plan are in `docs/research/2026-10-history-scrub.md`.
- Every popup is a `Popup` variant holding its data and a `Modal` variant (`crates/client/src/app.rs`); one is open
  at a time (`App::popup`). `App::layers` is the one order of what's on top (popup, wizard, login code, Arrange mode,
  D83, D87): keys, the mouse, the wheel and drawing follow it. The compiler asks for its arms in `App::modal_scroll` (what
  the wheel does in it) and `ui`'s `layer_rect`/`draw_layer`, and the popup tests open it. Lists scroll with
  `scroll::moved`/`scroll::window`, tall text with a `scroll::TextScroll`.
- The same for everything else the wheel can be over: every Widget is a `pane::Widget` variant
  (`crates/client/src/pane.rs`) and every main-Pane page an `app::Page`; the compiler asks for their arms in
  `App::pane_wheel`/`App::page_wheel` and in the wheel test's cases (`wheel/tests.rs`), which wheels each one, holding
  more than fits in a short Pane, notch by notch. Anything that can hold more than fits scrolls: over a Pane the wheel
  moves a list's view and leaves its highlight (D86, `scroll::Wheeled`; keys move the highlight and the view follows it,
  the pointer over a row of a focused list highlights it); a view that follows the song is a `scroll::Follow`; pty
  `wheel-all` shows them all. Wheel reports become steps in `term::WheelSteps` (a notch's burst is one step, a
  touchpad's report one each; pty `touchpad`).
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
