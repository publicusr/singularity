# How hard is fully free tiling? (researched 2026-10-17)

Owner (Round 4, Q2): "probably b), though c sounds really interesting, could be too complex. Look into how
difficult it would be."

## What (c) means

Window-manager-style layout: split any pane horizontally or vertically, close panes, drag a pane onto
another to move or swap it, drag dividers to resize, focus by direction, and save the result.

## Prior art (ratatui ecosystem)

- **[ratatui-hypertile](https://docs.rs/ratatui-hypertile)**: a binary-space-partition layout engine for
  ratatui (splits, focus navigation, resizing, moving panes). It computes rectangles only, so rendering
  and input stay ours. A companion crate, *ratatui-hypertile-extras*, adds keymaps, a command palette,
  workspaces and move animations.
- **[panes](https://docs.rs/crate/panes/0.19.0)**: a renderer-agnostic layout engine with a ratatui
  adapter. It supports resizing, focus, and snapshot/restore of layouts (a good fit for `layouts/*.toml`).
- **[ratkit](https://docs.rs/crate/ratkit/0.2.2)**: a two-pane draggable splitter only.
- Non-ratatui references for interaction details: the [Jarvis tiling system](https://www.mintlify.com/dyoburon/jarvis/architecture/tiling-system)
  (split ratios clamped to 0.1–0.9, a ~6 px hit zone for divider drags) and iced's `pane_grid`.

## Difficulty

| Part | Effort | Notes |
|------|--------|-------|
| Split-tree data model + serialise to TOML | Small | Off-the-shelf (hypertile/panes) or ~300 lines of our own |
| Render panes from the tree | Small | Same as (b): presets are just fixed trees |
| Drag dividers to resize | Small–medium | Needed for (b) anyway |
| Split / close / swap via palette + keys | Medium | Mostly UX: sensible defaults for what a new pane contains |
| Drag-and-drop panes with drop zones | Medium | Preview overlay; kitty images must follow their panes (already handled per-frame) |
| Minimum sizes, collapse at small terminal sizes | Medium | The real complexity: every Widget needs a min size and a compact form |
| Pixel-tier Visuals in arbitrary rects | None | Already resolution-independent |

**Verdict:** (c) is not a rabbit hole if the layout engine is a split tree from day one. Then (b) is
"tree with fixed shape, swappable leaves, draggable dividers", and (c) is "plus edit operations on the
tree". The extra cost of (c) over (b) is roughly the drag-and-drop and split/close UX: weeks, not months.

**Recommendation:** build the layout engine as a split tree in v1 (choose hypertile vs panes vs our own
in the walking-skeleton phase), ship (b) in v1, and add a *layout edit mode* (c) in v1.x.
