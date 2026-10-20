# Working agreements

- **New session? Read `docs/HANDOFF.md` first.** It holds the current state, owner feedback and the next tickets.

- Planning runs on the grilling method (`.claude/skills/grilling`): rounds over the design tree in
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
- Verify user-visible changes in a pseudo-terminal (pyte + PIL screenshots) before closing a ticket.
- Only kill processes you started; test sockets go in `$XDG_RUNTIME_DIR/sgl-*`, on macOS
  `/tmp/sgl-<uid>/` (Unix socket paths must be short).
- Linux and macOS have the same features (D49): OS-specific code only behind the platform seams in `docs/PLATFORMS.md`.
  On a Mac, `tools/check-linux.sh` type-checks the Linux build; run it before committing. `docs/PLATFORMS.md` is the
  exact parity status: update its rows whenever something changes on either OS. The README's "Build and run" and
  "macOS quirks" sections are the user-facing version: keep them true too.
- Test Daemons pin their Engine and stay in scratch dirs (`SINGULARITY_ENGINE`, `SINGULARITY_SOLOIST=local`,
  `SINGULARITY_START_APP=0`, `SINGULARITY_NOWPLAYING=off`, `SINGULARITY_NO_KEYRING=1`, scratch `XDG_*`): see HANDOFF.
