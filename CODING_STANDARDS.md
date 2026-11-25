# Coding standards for reviews

What a reviewer checks on every PR, besides "does it work". A finding is reported even when the code is good, if it
breaks one of these. Review briefs point here instead of repeating it.

## 1. Nothing beyond the ticket

The owner decides what the app does. Helpers fill gaps with their own product choices; those go back to the owner.

- Every **new key, menu item, palette word, user-facing text, hint, or behaviour** that is not in the ticket's
  acceptance criteria or a recorded owner decision (a D-row in `docs/DESIGN-TREE.md`, a ticket comment, an ADR) is a
  finding.
- List them in the review under a separate heading **"Beyond the ticket"**, one line each: what it is, where
  (`file:line`), and whether it can simply be removed. The owner answers each one; until then it doesn't merge.
- This includes changes to an existing look: a Widget's layout, spacing, alignment or font at a size the ticket didn't
  mark. Established looks stay unless the ticket names them.

## 2. "Nothing else changes" is checked on the drawn buffer

When a PR claims a change leaves something alone ("other sizes unchanged", "a Replace never moves a Pane", "medium
looks the same"), check it on what is **drawn**, not on rects or layout values.

- Render the before and after into a `ratatui` buffer (or a pty shot) and diff the cells. A rect can be equal while
  the text inside it moved, was centred, or lost a row.
- Report which cells were compared; a claim without a frame diff is a finding.

## 3. The repo's own rules

- `CLAUDE.md` working agreements (Popup/Modal arms, `wheel::Widget`/`app::Page` arms, platform seams, no personal
  data, test Daemons pinned and in scratch dirs).
- Tests go through the App (keys, mouse, wheel) where the ticket says so, not only through inner functions.
- No `cargo fmt`; the diff touches only the lines the change needs.
- Docs that the change makes untrue (`README`, `docs/SPEC.md`, `docs/PLATFORMS.md`, `GLOSSARY.md`) are updated in the
  same PR.
