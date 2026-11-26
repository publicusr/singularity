#!/bin/sh
# Merge a reviewed branch into the integration branch, check it, push it.
#   tools/integrate.sh <branch> [into]      (into defaults to d87)
# Steps: fresh worktree of origin/<into> -> merge --no-ff origin/<branch> -> board heavy -> run-hidden tools/check.sh
# -> push HEAD:<into> -> board done -> remove the worktree. Stops on a conflict or a red check, prints what failed,
# and leaves the worktree for fixing (its path is printed).
set -eu
branch=${1:?usage: tools/integrate.sh <branch> [into]}
into=${2:-d87}
repo=$(git rev-parse --show-toplevel)
tmp=$(realpath "${TMPDIR:-/tmp}")
wt="$tmp/integrate-$into-$(printf %s "$branch" | tr / -)"

git -C "$repo" fetch -q origin "$into" "$branch"
if [ -e "$wt" ]; then echo "integrate: $wt exists; remove it (git worktree remove --force $wt) or finish there" >&2; exit 1; fi
git -C "$repo" worktree add -q --detach "$wt" "origin/$into"
cd "$wt"

title=$(gh pr list --head "$branch" --base "$into" --state all --json number,title -q '.[0] // empty | "#\(.number): \(.title)"' 2>/dev/null || true)
msg="Merge ${title:-$branch}"
if ! git merge --no-ff --no-verify -q -m "$msg" "origin/$branch"; then
    echo "integrate: conflict merging $branch into $into; files:" >&2
    git diff --name-only --diff-filter=U >&2
    echo "integrate: worktree left at $wt" >&2
    exit 1
fi

board() { command -v claude-board >/dev/null 2>&1 && claude-board post "$@" || true; }
board -k heavy --eta 15m "singularity: check.sh for $branch into $into"
export TMPDIR="$tmp"
if command -v run-hidden >/dev/null 2>&1 && [ -z "${HIDDEN_SANDBOXED:-}" ]; then run=run-hidden; else run=; fi
if ! $run tools/check.sh; then
    board -k done "singularity: check.sh RED for $branch into $into"
    echo "integrate: check.sh failed; worktree left at $wt" >&2
    exit 1
fi
board -k done "singularity: $branch merged into $into, checks green"

git push -q origin "HEAD:$into"
cd "$repo"
git worktree remove --force "$wt"
echo "integrate: $branch is in $into ($(git rev-parse --short "origin/$into" 2>/dev/null || true))"
