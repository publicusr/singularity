# Helper brief: the rules for every build agent

A helper builds one ticket in its own git worktree. The brief it gets names the ticket, the branch and anything
special; everything below applies always. Read `CLAUDE.md`, `GLOSSARY.md`, the ticket, its parent spec and the ADRs
it names first.

## Scope

- **Build the ticket's acceptance criteria and nothing else.** No new keys, menu items, words, hints or behaviour
  beyond the ticket; if something seems missing, ask in the PR body (a question for the owner) instead of adding it.
  Reviewers check this (`CODING_STANDARDS.md`).
- Keep every established look the ticket doesn't mark. Verify with before/after pty shots of the cells you touched
  and of one cell you didn't.
- If a ticket spans two subsystems (say, the Daemon's protocol and the Client's layout), build it as two commits or
  say in the PR body that it should be split.

## Branch and PR

- Branch `d87-<name>` (a `d87/…` prefix can't be pushed next to `d87`), started from `origin/<integration branch>`.
- PR base is the integration branch (now `d87`), never `main`. Draft until `run-hidden tools/check.sh` is green.
- Commit messages say what changed for the user; end them with the attribution line from the session. No `Closes #N`
  on an integration branch (the issue closes when it reaches `main`); write `Part of #N`.
- **Keep the PR body current**: done, left to do, judgement calls, questions for the owner, before/after images
  (paths under `$TMPDIR`, never in the repo). Update it after every commit, so another agent can pick it up.
- If your context runs low, stop at a green, coherent point, commit, push, and write in the PR body what's left.

## Running things

- Every build, test and pty run goes through `run-hidden` (nothing may appear on the screen or make a sound).
- Set `TMPDIR=$(realpath "$TMPDIR")` before tests (some compare canonical paths). Pty runs inside `run-hidden` need
  `XDG_RUNTIME_DIR=$(mktemp -d /tmp/claude-1000/sgl-XXXX)` (socket paths must be short).
- `run-hidden tools/check.sh` is what CI runs; it must be green before you say done.
- `conn::tests::only_a_singularityd_is_stopped` always fails inside `run-hidden`; that one failure is known.
- Post on the machine board: `claude-board post -k heavy --eta <time> "<what>"` before a test batch or long run,
  `-k done` after, `-k fault` at once on any crash, GPU error, OOM or hang.
- **Never run `cargo fmt`**: the repo isn't rustfmt-clean. Match the surrounding style; `git add` only your files.
- Kill only processes you started.

## Data

- No personal data anywhere in the repo or commit messages: no real names, device or host names, account names,
  e-mail addresses, absolute local paths. Write "the owner", "the PC", `$HOME`.
- Never commit secrets. Fixtures get fake values.
