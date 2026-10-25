# Personal data in git history: what is there, and how to scrub it

**Status: plan only.** Nothing in this document has been run on the real repository. T0030 scrubbed the
*current tree* (`docs/HANDOFF.md`, the audit report, two spike READMEs, one test fixture) and tested the
commands below on a throwaway mirror clone in `$TMPDIR`, then deleted the clone. **Never force-push or
rewrite the real history without the owner's explicit go-ahead**; this changes every commit id.

Companion: the current-tree scrub and the publishing checklist are in `CLAUDE.md`; the audit finding is
`docs/research/2026-10-audit-2-quality.md` Q16.

## What is in history

Scanned with `git log -p --all`, `git rev-list --all | xargs git grep`, and `git log --format='%B'`
(author/committer metadata included). Counts are "commits whose tree contains it" unless stated.

| Category | Commits | Files | Since |
|----------|---------|-------|-------|
| The owner's first name (and surname in commit metadata) | 49 | `docs/HANDOFF.md`, `docs/research/2026-10-audit-2-quality.md` | `16ed79c`, 2026-10-20 |
| The Mac's Connect device names, built from the owner's name | 49 | `docs/HANDOFF.md`, `docs/research/2026-10-audit-2-quality.md` | `16ed79c`, 2026-10-20 |
| The PC's host name (the librespot Connect name) | 32 | `docs/HANDOFF.md` | `e3ebbdb`, 2026-10-21 |
| The personal Linux project path (`~/…`) | 24 | `docs/HANDOFF.md`, `docs/research/2026-10-audit-2-quality.md`, `spikes/*/README.md` | `d7d39bc`, 2026-10-23 |
| The personal macOS project path (`~/…`, old repo folder name) | 65 | `docs/HANDOFF.md`, `docs/research/2026-10-audit-2-quality.md` | `f38a8cf`, 2026-10-19 |
| Author/committer identity: GitHub handle + e-mail | 119 | commit metadata | start |
| Author/committer identity: real name + host-derived e-mail | 33 | commit metadata | `16ed79c`-era, 2026-10-19/20 |
| Absolute worktree paths in commit messages | 7 | commit messages (`a5be871`, six `tuios checkpoint` commits) | `d7d39bc`, 2026-10-23 |

Scope: 158 commits in all (`main` is 152; six local `refs/tuios/checkpoints/*` agent checkpoints are not
on `origin`), 5 local branches, all at `dca94c4` before this ticket landed.

Notes:

- The device names are not a separate string; they contain the owner's name (`"Singularity on <name>'s
  MacBook Air"`), so the name replacement covers them. The test fixture `crates/daemon/tests/app_engine.rs`
  says `"Owner’s Mac"` — neutral, no real name; it stays.
- The macOS project path also contains an old repository folder name; replacing the whole path (as below) is
  enough, the folder name alone never appears without it.
- `git log -S` (added/removed) finds far fewer commits than the "commits whose tree contains it" count above:
  a string introduced once stays in every later commit's tree. The rewrite must touch all of them, which
  `git filter-repo --replace-text` does.
- **Checked, nothing found:** no Spotify username or account id, no playlist/track/episode name from the owner's
  own listening (the fixtures use invented artists and `spotify:track:…fixture`), no IP address, serial or key.
  `/home/u/…` (`crates/client/src/cover.rs` tests) and `/home/singularity/…` (the VM guest) are not the owner.
- **Not personal, left alone:** the GitHub repo slug `publicusr/singularity` (in `Cargo.toml`, `README.md`,
  the launchd label `io.github.publicusr.singularityd`, and the lyrics user agent) is the project's public
  identity, not a name; changing it breaks installed LaunchAgents and every link, so it is the owner's call.
  Also kept: distro names (Nobara/Fedora) and the hardware model (`M2 Air`) — a model, not a name. Agent
  scratch paths (`~/.cache/regent/…` in two audit reports) are local-only but identify no person; clean them
  up separately if they should not ship.

## The replace-text file (outside the repo)

The exact strings live in a file that must never be committed. A ready file sits in the T0030 worktree at
`.scratch/personal-data.txt` (git-excluded via `.git/info/exclude`; `.regent-report.md` says the same). It is
plain text, one `literal:<old>==><new>` pair per line, longest/most specific first:

```
literal:<owner's full name>==>Singularity contributor
literal:<owner's name-derived e-mail>==>contributor@example.invalid
literal:<owner's Mac host name>.local==>example.invalid
literal:<owner's first name>'s MacBook Air==>the Mac
literal:<owner's first name>'s==>the owner's
literal:<owner's first name>==>the owner
literal:<GitHub handle>==>contributor
literal:<GitHub e-mail>==>contributor@example.invalid
literal:<PC host name>==>host
literal:<absolute worktree path prefix>==>~/worktrees/singularity/
literal:<Linux project path (absolute, ~ form and both parent folders)>==>~/src/singularity
literal:<macOS project path (absolute and ~ form, plus the parent folder)>==>~/src/singularity
literal:<home directory>==>~
```

(`>`/`<` are not special; the separator is exactly `==>`, and an old value must not contain it. The ready
file lists the Linux and macOS paths in every form seen in history, including the parent folders that appear
without the project name in one commit subject.)

Alongside it are a short **mailmap** for the two identities (`Singularity contributor
<contributor@example.invalid> <old name and e-mail>`), at `.scratch/mailmap`, and a **message callback**
(`.scratch/message_callback.py`) because `git filter-repo` has no `--replace-message` flag — commit messages
are rewritten with `--message-callback`. The callback reads the same replace-text file:

```python
import os
path = os.path.join(os.environ.get("SCRUB", ".scratch"), "personal-data.txt")
for line in open(path, encoding="utf-8"):
    line = line.strip()
    if not line or line.startswith("#"):
        continue
    old, sep, new = line.partition("==>")
    if not sep:
        continue
    for prefix in ("literal:", "glob:", "regex:"):
        if old.startswith(prefix):
            old = old[len(prefix):]
            break
    message = message.replace(old.encode(), new.encode())
return message
```

If `.scratch/` is gone, rebuild the three files from the categories in the table (the owner's own name,
host names and paths) and the skeleton above. Do not add them to git.

## The rewrite (tested on a throwaway mirror)

Run from the worktree that holds `.scratch/`; `$TMPDIR` is on disk, never `/tmp` (RAM on this machine).

```sh
# 0. Everyone stops: no worker is in a worktree, nothing is half-committed.
#    Land every open branch you want to keep first (they are all remapped below).

export SCRUB="$(pwd)/.scratch"               # holds personal-data.txt, mailmap, message_callback.py
git clone --no-local --mirror "$(pwd)" "$TMPDIR/singularity-scrub/repo.git"
cd "$TMPDIR/singularity-scrub/repo.git"

# 1. Rewrite blobs, authors/committers and commit messages in one pass.
git filter-repo \
  --replace-text "$SCRUB/personal-data.txt" \
  --mailmap "$SCRUB/mailmap" \
  --message-callback "$(cat "$SCRUB/message_callback.py")"

# 2. Verify (see below). Only then take the rewritten mirror back to GitHub.

# 3. Push every branch and tag, with force. filter-repo removed `origin`.
git remote add origin git@github.com:publicusr/singularity.git
git push --force origin --all
git push --force origin --tags
```

`--no-local` matters: a plain local `git clone --mirror` hardlinks the objects and filter-repo then refuses
("this does not look like a fresh clone"). With `--no-local` the commands above run without `--force`.
Do **not** push with `--mirror`: it would also publish the local `refs/tuios/checkpoints/*` (agent
checkpoints, absolute worktree paths in their messages). They are rewritten by the run, but they are not
part of the project.

## Verification (do all of it in the clone before pushing)

```sh
cd "$TMPDIR/singularity-scrub/repo.git"

# 1. One neutral identity (plus the tool's `tuios <>` checkpoint commits), no old e-mail.
git log --all --format='%an <%ae>' | sort -u

# 2. No personal string in any tree, in any commit. Prints local strings; that is fine here.
while IFS= read -r line; do
  case "$line" in ''|'#'*) continue;; esac
  old=${line%%==>*}; old=${old#literal:}
  n=$(git grep -l -F -- "$old" $(git rev-list --all) 2>/dev/null | wc -l)
  printf '%3d commit:file hits: %s\n' "$n" "$old"
done < "$SCRUB/personal-data.txt"      # every line must be 0

# 3. No personal string in any commit message.
while IFS= read -r line; do
  case "$line" in ''|'#'*) continue;; esac
  old=${line%%==>*}; old=${old#literal:}
  n=$(git log --all --format='%B' | grep -F -c -- "$old" || true)
  printf '%3d message hits: %s\n' "$n" "$old"
done < "$SCRUB/personal-data.txt"      # every line must be 0

# 4. Same number of commits; every SHA different from the originals.
git rev-list --all --count             # take the number before the rewrite; it must match after
```

T0030's test run (before its own commits landed): 158 commits before, 158 after, **0 of 158 SHAs
unchanged** (author/committer rewriting alone changes every one), 0 hits in trees and messages, one neutral
identity plus `tuios <>`. The final check with this ticket's commits in place: 160/160 changed, 0 hits.

Build check after the rewrite (the replacements only touch docs and one test string, but prove it):

```sh
git clone "$TMPDIR/singularity-scrub/repo.git" "$TMPDIR/singularity-scrub/check"
cd "$TMPDIR/singularity-scrub/check" && git checkout main && cargo test --workspace
```

## Consequences, to tell the owner first

- **Every SHA changes** (all 160 here; 158 before this ticket's commits), including `main`'s tip. Every
  clone, worktree and CI checkout must be re-cloned (or `git fetch && git reset --hard origin/main`);
  branches that were pushed need `--force-with-lease` after being re-created from the rewritten refs.
- Every open branch must be landed *before* the rewrite or rebased onto the new `main`; rewriting rewrites
  the refs, not the commits sitting in someone else's clone.
- The push is a force push; GitHub keeps the old objects reachable by SHA for a while. For a repo that
  becomes public, the only reliable purge is to **delete and recreate the repository** (or ask GitHub
  Support to run garbage collection); a force push alone is not enough. If the old repo has forks, ask
  the owners to delete them too.
- The rewrite is permanent only after the force push; until then it is just a scratch clone. Delete
  `$TMPDIR/singularity-scrub` afterwards.
- Not covered here: local agent checkpoints (`refs/tuios/checkpoints/*`) in other worktrees, reflogs,
  notes, and `.git/config` remotes — they are local, but a `--mirror` push would publish the refs; then
  delete them.
