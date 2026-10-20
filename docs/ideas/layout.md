# Layout: idea bank

Ideas, not commitments. One gets built only after the owner picks it up again.

## Free tiling (parked 2026-10-20, D59)

A window-manager-style edit mode on the existing layout tree: split any Pane horizontally or vertically, close
Panes, drag one onto another to swap or move it, then save the result as a layout. Planned as batch 7, a trial (D37),
and parked before it was built: the owner finds the built-in layouts, swappable Slots, drag-to-resize and saved
layouts (#19) "very satisfactory".

- What it would take: `research/2026-10-free-tiling.md` (moderate effort; the split tree in `client/src/layout.rs`
  already holds user layouts, so the edit mode builds on it rather than replacing it).
- Spec story 85 (SPEC.md, "Layout"), marked parked.
- Still true when it comes back: every built-in Design keeps Home, library, queue and lyrics (D43), and the well keeps
  its shape (`compute_shaped`). An edit mode would need the same guard for user-made trees.
