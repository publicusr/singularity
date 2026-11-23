# Arranging swaps contents on a fixed floor plan

Status: accepted (2026-11-23, D87; supersedes D66's live reflow, edge drop zones and "no edit mode")

D66 let one title drag both move a Widget and reshape the layout: a Pane's edges split it, the screen's edges made a
whole column or row, and the layout reflowed live to show the hovered drop. Used on a small display it failed the way
iOS home screens do ("apps teleport around"): the whole app changed shape under the pointer, a plain swap was the
smallest target, and dropping a Pane where you pictured it felt almost impossible. We split the two jobs. A drag only
moves **contents**: onto another Pane (the two Widgets swap, or it fills an empty one) or onto the **Tray**, where it
waits. The **floor plan** changes only by its own explicit edits: dragging the line between two Panes, splitting,
closing. While you drag, nothing moves but a highlight and a small chip. This lives in an **Arrange mode** (Tray, empty
Panes, split and close buttons, Done), which a title drag enters, so a quick swap is still one gesture.

## Considered options

- **Keep D66 and smooth it** (#31: drop = preview, dwell tuning, path-replay tests). Rejected: the reshaping itself is
  the problem; #31's replayer and stuck-drag fixes are kept as the baseline.
- **Drag-and-drop removed entirely, menus only.** Rejected: the owner prefers the mouse, and a swap by drag is natural
  once nothing else moves.

## Consequences

- Putting a Widget into a new column takes two steps (split, then fill) instead of one edge drop.
- A Pane can be empty, and each Design saves its Tray: the saved Design format changes.
- Every drop is allowed, so a Widget may sit in a Pane below its smallest size and must say so cleanly; every Widget
  has to scale well to any Pane size, which is why a size contact sheet comes first.
