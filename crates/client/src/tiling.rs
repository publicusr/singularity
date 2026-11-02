//! Free tiling (D66): edits to the split tree as pure functions, and the drop zones the mouse and Move mode aim at.
//! Every edit returns a new tree (or None); `valid` says whether a tree is fit to show at a terminal size. Nothing
//! here touches the App: the UI offers a drop only when its candidate tree is valid, and commits only that.

use crate::layout::{Axis, Node, Size, MIN_H, MIN_W, WELL_ASPECT, is_hole};
use ratatui::layout::Rect;

/// Panes that can be moved but never closed (D43: every Design keeps them).
pub const CORE: [&str; 4] = ["home", "library", "queue", "lyrics"];

/// A Pane's edge bands: this share of its width and height at each side (the rest is the centre, which swaps).
pub const EDGE_FRAC: f32 = 0.28;
/// The bands across a Pane's short way (top and bottom of a wide Pane, left and right of a tall one) are thinner: a
/// split along its long axis is the likelier wish.
pub const SHORT_EDGE_FRAC: f32 = 0.2;
/// A split is offered only where both halves would be comfortable (owner, 2026-11-01: small Panes kept offering
/// "sandwich" splits when a swap was meant): at least this many columns each side by side, rows each stacked. A Pane
/// too short for a stacked split has no top and bottom bands (that room swaps); too narrow, no left and right ones.
pub const COMFY_W: u16 = 24;
pub const COMFY_H: u16 = 8;
/// Hysteresis at zone borders: the zone under the pointer stays the one it was while the pointer is within this many
/// columns / rows of it, so a pointer resting on a border doesn't flip between two.
pub const STICK_X: u16 = 2;
pub const STICK_Y: u16 = 1;

/// Whether a Pane of this size can be split along `axis` with both halves comfortable.
pub fn split_comfy(r: Rect, axis: Axis) -> bool {
    match axis {
        Axis::Horizontal => r.width >= 2 * COMFY_W,
        Axis::Vertical => r.height >= 2 * COMFY_H,
    }
}

/// Whether Pane `moved` dropped on `zone` gives a comfortable result: a split's two Panes (`moved` and the one it
/// went beside) each at least `COMFY_W` / `COMFY_H` along the split, as the new tree lays them out. Swaps and screen
/// edges always are.
pub fn comfortable(tree: &Node, moved: &str, zone: &Zone, area: Rect, cell_aspect: f64) -> bool {
    let Zone::Beside(target, side) = zone else { return true };
    let slots = tree.compute_shaped(area, cell_aspect).slots;
    let len = |s: &str| slots.iter().find(|(n, _)| n == s).map(|(_, r)| if side.axis() == Axis::Horizontal { r.width } else { r.height });
    let need = if side.axis() == Axis::Horizontal { COMFY_W } else { COMFY_H };
    matches!((len(moved), len(target)), (Some(a), Some(b)) if a >= need && b >= need)
}
/// The outermost column at each side of the layout gives a full-height column; above and below the layout (the top
/// bar, the player) a full-width row. The top row of the layout itself is the top Panes' title bars, where a drag
/// starts: an edge zone there would turn a drag along the titles into a whole row (owner, 2026-11-01).
pub const SCREEN_COLS: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub fn axis(self) -> Axis {
        match self {
            Side::Left | Side::Right => Axis::Horizontal,
            Side::Top | Side::Bottom => Axis::Vertical,
        }
    }
    /// Whether something put on this side comes first in the split (`a`).
    pub fn first(self) -> bool {
        matches!(self, Side::Left | Side::Top)
    }
}

/// Where a dragged Pane would land.
#[derive(Clone, Debug, PartialEq)]
pub enum Zone {
    /// the centre of another Pane: the two trade places
    Swap(String),
    /// an edge band of another Pane: split it, the moved Pane on that side
    Beside(String, Side),
    /// the layout's outer edge: a full-height column / full-width row, taking this share of the screen
    Edge(Side, f32),
}

/// The Pane's share of the screen along `side`'s axis when it becomes a column / row of its own: about the size it
/// had, kept between a fifth and two fifths, and never under its minimum.
pub fn edge_ratio(side: Side, slot: &str, pane: Rect, body: Rect) -> f32 {
    let (len, total, lo, hi) = match side.axis() {
        Axis::Horizontal => (pane.width, body.width, 0.15, 0.35),
        Axis::Vertical => (pane.height, body.height, 0.2, 0.4),
    };
    let total = total.max(1) as f32;
    let floor = (Node::Slot(slot.into()).min_len(side.axis()) + 1) as f32 / total;
    let r = (len as f32 / total).clamp(lo, hi).max(floor).min(0.6);
    (r * 100.0).round() / 100.0
}

/// The drop zone under (x, y) while `dragged` is being moved, from the layout as it stands (not the preview, so the
/// zone under the pointer never changes because of what the preview did). None over the Pane itself and outside.
/// Above the layout (the top bar) and below it (the player) are the screen's top and bottom edges.
pub fn zone_at(slots: &[(String, Rect)], body: Rect, dragged: &str, x: u16, y: u16) -> Option<Zone> {
    let inside = |r: Rect| x >= r.x && x < r.right() && y >= r.y && y < r.bottom();
    let me = slots.iter().find(|(s, _)| s == dragged)?.1;
    if x < body.x || x >= body.right() || inside(me) {
        return None;
    }
    let screen = if y < body.y {
        Some(Side::Top)
    } else if y >= body.bottom() {
        Some(Side::Bottom)
    } else if x < body.x + SCREEN_COLS {
        Some(Side::Left)
    } else if x + SCREEN_COLS >= body.right() {
        Some(Side::Right)
    } else {
        None
    };
    if let Some(s) = screen {
        return Some(Zone::Edge(s, edge_ratio(s, dragged, me, body)));
    }
    let (name, r) = slots.iter().find(|(_, r)| inside(*r))?;
    // the long axis (a cell is about twice as tall as wide): its bands are the full share, the short axis' thinner;
    // an axis that can't be split comfortably has no bands at all (its room is the centre's: a swap)
    let wide = r.width >= 2 * r.height;
    let band = |len: u16, frac: f32, ok: bool| if ok { ((len as f32 * frac).round() as u16).min(len.saturating_sub(1) / 2).max(1) } else { 0 };
    let bx = band(r.width, if wide { EDGE_FRAC } else { SHORT_EDGE_FRAC }, split_comfy(*r, Axis::Horizontal));
    let by = band(r.height, if wide { SHORT_EDGE_FRAC } else { EDGE_FRAC }, split_comfy(*r, Axis::Vertical));
    // how far into each band the pointer is (0 = at its inner edge, 1 = on the border); the deepest wins in a corner,
    // the long axis' on a tie
    let depth = |d: u16, b: u16| (d < b).then(|| 1.0 - d as f32 / b as f32);
    let long = |s: Side| if (s.axis() == Axis::Horizontal) == wide { 0.01 } else { 0.0 };
    let sides = [
        (Side::Left, depth(x - r.x, bx)),
        (Side::Right, depth(r.right() - 1 - x, bx)),
        (Side::Top, depth(y - r.y, by)),
        (Side::Bottom, depth(r.bottom() - 1 - y, by)),
    ];
    let best = sides.iter().filter_map(|(s, d)| d.map(|d| (*s, d + long(*s)))).max_by(|a, b| a.1.total_cmp(&b.1));
    Some(match best {
        Some((s, _)) => Zone::Beside(name.clone(), s),
        None => Zone::Swap(name.clone()),
    })
}

/// `zone_at` with hysteresis: while the pointer is within `STICK_X` columns / `STICK_Y` rows of where `prev` was,
/// `prev` stays (a pointer resting on the border between two zones doesn't flicker between them).
pub fn zone_near(slots: &[(String, Rect)], body: Rect, dragged: &str, x: u16, y: u16, prev: Option<&Zone>) -> Option<Zone> {
    let here = zone_at(slots, body, dragged, x, y);
    let Some(p) = prev.filter(|p| here.as_ref() != Some(*p)) else { return here };
    let (sx, sy) = (STICK_X as i32, STICK_Y as i32);
    let near = (-sy..=sy).flat_map(|dy| (-sx..=sx).map(move |dx| (dx, dy))).filter(|&d| d != (0, 0));
    let held = near
        .filter_map(|(dx, dy)| Some(((x as i32 + dx).try_into().ok()?, (y as i32 + dy).try_into().ok()?)))
        .any(|(nx, ny): (u16, u16)| zone_at(slots, body, dragged, nx, ny).as_ref() == Some(p));
    if held { Some(p.clone()) } else { here }
}

/// The Pane next to `name` in direction `dir`: the one sharing the longest stretch of its edge.
pub fn neighbour(slots: &[(String, Rect)], name: &str, dir: Side) -> Option<String> {
    let me = slots.iter().find(|(s, _)| s == name)?.1;
    let span = |a0: u16, a1: u16, b0: u16, b1: u16| a1.min(b1).saturating_sub(a0.max(b0));
    slots
        .iter()
        .filter(|(s, _)| s != name)
        .filter_map(|(s, o)| {
            let (touches, overlap) = match dir {
                Side::Left => (o.right() == me.x, span(o.y, o.bottom(), me.y, me.bottom())),
                Side::Right => (o.x == me.right(), span(o.y, o.bottom(), me.y, me.bottom())),
                Side::Top => (o.bottom() == me.y, span(o.x, o.right(), me.x, me.right())),
                Side::Bottom => (o.y == me.bottom(), span(o.x, o.right(), me.x, me.right())),
            };
            (touches && overlap > 0).then(|| (s.clone(), overlap))
        })
        .max_by_key(|(_, overlap)| *overlap)
        .map(|(s, _)| s)
}

impl Node {
    /// Whether the tree could be shown at `area` as it is: unique names, sane sizes, every Pane at least its
    /// minimum (the well and the hole taller than the rest) and the well no wider than its shape allows.
    pub fn valid(&self, area: Rect, cell_aspect: f64) -> bool {
        let names = self.slot_names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        if names.is_empty() || sorted.len() != names.len() || names.iter().any(|n| n.is_empty()) || !self.sizes_sane() {
            return false;
        }
        if self.min_len(Axis::Horizontal) > area.width || self.min_len(Axis::Vertical) > area.height {
            return false;
        }
        let out = self.compute_shaped(area, cell_aspect);
        out.slots.len() == names.len()
            && out.slots.iter().all(|(s, r)| {
                let min_h = if is_hole(s) { crate::layout::WELL_MIN_H } else { MIN_H };
                let shape = (r.width.saturating_sub(2)) as f64 / ((r.height.saturating_sub(2)).max(1) as f64 * cell_aspect);
                r.width >= MIN_W && r.height >= min_h && (s != "well" || shape <= WELL_ASPECT * 1.05 + 0.12)
            })
    }

    /// Ratios in range and fixed sizes that mean something.
    fn sizes_sane(&self) -> bool {
        match self {
            Node::Slot(_) => true,
            Node::Split { first, a, b, .. } => {
                (match first {
                    Size::Ratio(r) => r.is_finite() && (0.05..=0.95).contains(r),
                    Size::Cells(c) | Size::Last(c) => (1..=2000).contains(c),
                }) && a.sizes_sane()
                    && b.sizes_sane()
            }
        }
    }

    /// Whether `slot` is on screen.
    pub fn has(&self, slot: &str) -> bool {
        self.path_of(slot).is_some()
    }

    /// Without Slot `name`: its sibling takes the room its parent split had. None if `name` isn't here or is the
    /// only Pane.
    pub fn without(&self, name: &str) -> Option<Node> {
        match self {
            Node::Slot(_) => None,
            Node::Split { axis, first, a, b } => {
                let is = |n: &Node| matches!(n, Node::Slot(s) if s == name);
                if is(a) {
                    return Some((**b).clone());
                }
                if is(b) {
                    return Some((**a).clone());
                }
                if let Some(na) = a.without(name) {
                    return Some(Node::split(*axis, *first, na, (**b).clone()));
                }
                b.without(name).map(|nb| Node::split(*axis, *first, (**a).clone(), nb))
            }
        }
    }

    /// A new Pane `new` on `side` of Pane `target`, which gives it half its room. None if `target` isn't here or
    /// `new` already is.
    pub fn beside(&self, target: &str, new: &str, side: Side) -> Option<Node> {
        if self.has(new) {
            return None;
        }
        fn go(n: &Node, target: &str, new: &str, side: Side) -> Option<Node> {
            match n {
                Node::Slot(s) if s == target => {
                    let (old, fresh) = (Node::slot(s), Node::slot(new));
                    let (a, b) = if side.first() { (fresh, old) } else { (old, fresh) };
                    Some(Node::split(side.axis(), Size::Ratio(0.5), a, b))
                }
                Node::Slot(_) => None,
                Node::Split { axis, first, a, b } => match go(a, target, new, side) {
                    Some(na) => Some(Node::split(*axis, *first, na, (**b).clone())),
                    None => go(b, target, new, side).map(|nb| Node::split(*axis, *first, (**a).clone(), nb)),
                },
            }
        }
        go(self, target, new, side)
    }

    /// A new Pane `new` as a full-height column / full-width row on `side` of the whole layout, taking `ratio` of it.
    pub fn at_edge(&self, new: &str, side: Side, ratio: f32) -> Option<Node> {
        if self.has(new) {
            return None;
        }
        let ratio = ((if side.first() { ratio } else { 1.0 - ratio }) * 100.0).round() / 100.0;
        let (fresh, rest) = (Node::slot(new), self.clone());
        let (a, b) = if side.first() { (fresh, rest) } else { (rest, fresh) };
        Some(Node::split(side.axis(), Size::Ratio(ratio), a, b))
    }

    /// Panes `x` and `y` trading places.
    pub fn swapped(&self, x: &str, y: &str) -> Option<Node> {
        if x == y || !self.has(x) || !self.has(y) {
            return None;
        }
        fn go(n: &Node, x: &str, y: &str) -> Node {
            match n {
                Node::Slot(s) if s == x => Node::slot(y),
                Node::Slot(s) if s == y => Node::slot(x),
                Node::Slot(s) => Node::slot(s),
                Node::Split { axis, first, a, b } => Node::split(*axis, *first, go(a, x, y), go(b, x, y)),
            }
        }
        Some(go(self, x, y))
    }

    /// Pane `name` dropped on `to`. None if it can't go there, or if the drop would only change sizes: the Panes in
    /// the same rows and columns, in the same order (dropped on the near edge of its own neighbour, or on the screen
    /// edge it already spans). Such a drop looked like a bug: one Pane just grew or shrank (owner, 2026-11-01).
    pub fn moved(&self, name: &str, to: &Zone) -> Option<Node> {
        self.moved_any(name, to).filter(|n| !n.same_arrangement(self))
    }

    /// `moved`, size-only results included.
    pub fn moved_any(&self, name: &str, to: &Zone) -> Option<Node> {
        match to {
            Zone::Swap(t) => self.swapped(name, t),
            Zone::Beside(t, side) if t != name => self.without(name)?.beside(t, name, *side),
            Zone::Beside(..) => None,
            Zone::Edge(side, ratio) => self.without(name)?.at_edge(name, *side, *ratio),
        }
    }

    /// Whether the two trees put the same Panes in the same rows and columns, in the same order, whatever the sizes
    /// and however the splits nest (`a | (b | c)` is `(a | b) | c`).
    pub fn same_arrangement(&self, other: &Node) -> bool {
        fn flat<'a>(n: &'a Node, axis: Axis, out: &mut Vec<&'a Node>) {
            match n {
                Node::Split { axis: own, a, b, .. } if *own == axis => {
                    flat(a, axis, out);
                    flat(b, axis, out);
                }
                _ => out.push(n),
            }
        }
        match (self, other) {
            (Node::Slot(x), Node::Slot(y)) => x == y,
            (Node::Split { axis: x, .. }, Node::Split { axis: y, .. }) if x == y => {
                let (mut p, mut q) = (vec![], vec![]);
                flat(self, *x, &mut p);
                flat(other, *y, &mut q);
                p.len() == q.len() && p.iter().zip(&q).all(|(m, n)| m.same_arrangement(n))
            }
            _ => false,
        }
    }

    /// Without Pane `name`, unless it is one of the four every Design keeps.
    pub fn closed(&self, name: &str) -> Option<Node> {
        if CORE.contains(&name) {
            return None;
        }
        self.without(name)
    }

    /// Pane `name` a step bigger or smaller (2 columns, or 1 row): the nearest split around it that can move its
    /// boundary does. None if none can.
    pub fn resized(&self, name: &str, grow: bool, area: Rect) -> Option<Node> {
        let path = self.path_of(name)?;
        let now = self.compute(area);
        for k in (0..path.len()).rev() {
            let Some(d) = now.dividers.iter().find(|d| d.path == path[..k]) else { continue };
            let (boundary, step) = match d.axis {
                Axis::Horizontal => (d.grip.x + 1, 2),
                Axis::Vertical => (d.grip.y + 1, 1),
            };
            // growing the first side moves the boundary away from the origin; growing the second, toward it
            let out = if grow == (path[k] == 0) { boundary as i32 + step } else { boundary as i32 - step };
            let mut n = self.clone();
            n.drag(d, out.max(0) as u16);
            if n.compute(area) != now {
                return Some(n);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Side::*;

    fn rect(n: &Node, area: Rect, s: &str) -> Rect {
        n.compute(area).slots.iter().find(|(x, _)| x == s).unwrap().1
    }

    const AREA: Rect = Rect { x: 0, y: 1, width: 160, height: 43 };

    #[test]
    fn removing_a_pane_gives_its_room_to_the_sibling() {
        let n = Node::event_horizon();
        let m = n.without("lyrics").unwrap();
        assert!(!m.has("lyrics"));
        // the hole is alone in its column now: it takes the lyrics' rows too
        assert_eq!(rect(&m, AREA, "hole").height, AREA.height);
        assert_eq!(rect(&m, AREA, "hole").width, rect(&n, AREA, "hole").width);
        assert!(n.without("nope").is_none());
        assert!(Node::slot("only").without("only").is_none(), "the last Pane stays");
    }

    #[test]
    fn a_pane_goes_beside_another_on_the_side_asked() {
        let n = Node::classic();
        for (side, left_of) in [(Left, true), (Right, false)] {
            let m = n.beside("queue", "fft", side).unwrap();
            let (f, q) = (rect(&m, AREA, "fft"), rect(&m, AREA, "queue"));
            assert_eq!(f.y, q.y);
            assert_eq!(f.x < q.x, left_of);
            assert!((f.width as i32 - q.width as i32).abs() <= 1, "half each: {f:?} {q:?}");
        }
        for (side, above) in [(Top, true), (Bottom, false)] {
            let m = n.beside("queue", "fft", side).unwrap();
            let (f, q) = (rect(&m, AREA, "fft"), rect(&m, AREA, "queue"));
            assert_eq!(f.x, q.x);
            assert_eq!(f.y < q.y, above);
        }
        assert!(n.beside("queue", "home", Left).is_none(), "no Widget twice");
        assert!(n.beside("nope", "fft", Left).is_none());
    }

    #[test]
    fn a_screen_edge_gives_a_whole_column_or_row() {
        let n = Node::classic();
        let m = n.at_edge("fft", Left, 0.2).unwrap();
        let f = rect(&m, AREA, "fft");
        assert_eq!((f.x, f.y, f.height), (AREA.x, AREA.y, AREA.height));
        assert!((f.width as i32 - 32).abs() <= 1);
        let m = n.at_edge("fft", Bottom, 0.25).unwrap();
        let f = rect(&m, AREA, "fft");
        assert_eq!((f.x, f.width, f.bottom()), (0, AREA.width, AREA.bottom()));
        assert!((f.height as i32 - 11).abs() <= 1, "{f:?}");
        let m = n.at_edge("fft", Right, 0.25).unwrap();
        assert_eq!(rect(&m, AREA, "fft").right(), AREA.right());
        assert!(n.at_edge("home", Left, 0.2).is_none());
    }

    #[test]
    fn swapping_trades_places_and_moving_collapses_the_old_spot() {
        let n = Node::classic();
        let m = n.swapped("home", "lyrics").unwrap();
        assert_eq!(rect(&m, AREA, "home"), rect(&n, AREA, "lyrics"));
        assert_eq!(rect(&m, AREA, "lyrics"), rect(&n, AREA, "home"));
        assert!(n.swapped("home", "home").is_none() && n.swapped("home", "nope").is_none());
        let m = n.moved("queue", &Zone::Beside("lyrics".into(), Right)).unwrap();
        assert_eq!(rect(&m, AREA, "home").height, AREA.height, "home takes the queue's room");
        assert_eq!(m.slot_names().len(), n.slot_names().len());
        assert!(n.moved("queue", &Zone::Beside("queue".into(), Left)).is_none());
        let m = n.moved("queue", &Zone::Swap("lyrics".into())).unwrap();
        assert_eq!(m, n.swapped("queue", "lyrics").unwrap());
    }

    #[test]
    fn the_core_four_cannot_be_closed_the_rest_can() {
        let n = Node::cockpit();
        for c in CORE {
            assert!(n.closed(c).is_none(), "{c}");
        }
        let m = n.closed("fft").unwrap();
        assert!(!m.has("fft") && m.valid(AREA, 2.0));
    }

    #[test]
    fn a_step_resizes_the_nearest_split_that_can_move() {
        let n = Node::classic();
        let before = rect(&n, AREA, "library").width;
        let m = n.resized("library", true, AREA).unwrap();
        assert_eq!(rect(&m, AREA, "library").width, before + 2);
        let m = n.resized("library", false, AREA).unwrap();
        assert_eq!(rect(&m, AREA, "library").width, before - 2);
        // home is in a stack: it grows taller first
        let h = rect(&n, AREA, "home").height;
        assert_eq!(rect(&n.resized("home", true, AREA).unwrap(), AREA, "home").height, h + 1);
        // squeezed to its minimum, it can't shrink more: the next split out is tried, else None
        let mut m = n.clone();
        for _ in 0..80 {
            m = m.resized("library", false, AREA).unwrap_or(m);
        }
        assert_eq!(rect(&m, AREA, "library").width, MIN_W);
        assert!(m.valid(AREA, 2.0));
    }

    #[test]
    fn validity_catches_small_trees_dupes_and_bad_sizes() {
        let n = Node::singularity();
        assert!(n.valid(AREA, 2.0));
        assert!(!n.valid(Rect::new(0, 0, 20, 8), 2.0), "no room for this many Panes");
        let dup = Node::split(Axis::Horizontal, Size::Ratio(0.5), Node::slot("home"), Node::slot("home"));
        assert!(!dup.valid(AREA, 2.0));
        let bad = Node::split(Axis::Horizontal, Size::Ratio(1.5), Node::slot("home"), Node::slot("queue"));
        assert!(!bad.valid(AREA, 2.0));
        let nan = Node::split(Axis::Horizontal, Size::Ratio(f32::NAN), Node::slot("home"), Node::slot("queue"));
        assert!(!nan.valid(AREA, 2.0));
        let zero = Node::split(Axis::Horizontal, Size::Cells(0), Node::slot("home"), Node::slot("queue"));
        assert!(!zero.valid(AREA, 2.0));
        // the well wants more height than a plain Pane
        let col = |h| Node::split(Axis::Vertical, Size::Cells(h), Node::slot("well"), Node::slot("queue"));
        let flat = Node::split(Axis::Horizontal, Size::Ratio(0.3), col(4), Node::slot("home"));
        assert!(!flat.valid(AREA, 2.0));
        // too wide for its shape, but a split beside it can take the width
        let ok = Node::split(Axis::Horizontal, Size::Ratio(0.3), col(8), Node::slot("home"));
        assert!(ok.valid(AREA, 2.0));
        // a well across the whole width has no neighbour to give it to
        let wide = Node::split(Axis::Vertical, Size::Cells(8), Node::slot("well"), Node::slot("home"));
        assert!(!wide.valid(AREA, 2.0));
    }

    fn slots_of(n: &Node) -> Vec<(String, Rect)> {
        n.compute(AREA).slots
    }

    #[test]
    fn the_zones_of_a_pane_are_its_edges_and_its_centre() {
        // two big Panes side by side (80x43 each): every band is there
        let n = Node::split(Axis::Horizontal, Size::Ratio(0.5), Node::slot("fft"), Node::slot("queue"));
        let slots = slots_of(&n);
        let q = rect(&n, AREA, "queue");
        let (cx, cy) = (q.x + q.width / 2, q.y + q.height / 2);
        assert_eq!(zone_at(&slots, AREA, "fft_or_whatever", cx, cy), None, "an unknown Pane has no zones");
        let zone = |x, y| zone_at(&slots, AREA, "fft", x, y);
        assert_eq!(zone(cx, cy), Some(Zone::Swap("queue".into())));
        assert_eq!(zone(q.x, cy), Some(Zone::Beside("queue".into(), Left)));
        assert_eq!(zone(q.right() - 2, cy), Some(Zone::Beside("queue".into(), Right)), "(the last column is the screen edge)");
        assert_eq!(zone(cx, q.y), Some(Zone::Beside("queue".into(), Top)));
        assert_eq!(zone(cx, q.bottom() - 1), Some(Zone::Beside("queue".into(), Bottom)));
        // a tall Pane: its top and bottom bands are the outer 28%, the left and right ones (its short way) 20%; just
        // inside a band is the centre
        let (bx, by) = ((q.width as f32 * SHORT_EDGE_FRAC).round() as u16, (q.height as f32 * EDGE_FRAC).round() as u16);
        assert_eq!(zone(q.x + bx - 1, cy), Some(Zone::Beside("queue".into(), Left)));
        assert_eq!(zone(q.x + bx, cy), Some(Zone::Swap("queue".into())));
        assert_eq!(zone(cx, q.y + by - 1), Some(Zone::Beside("queue".into(), Top)));
        assert_eq!(zone(cx, q.y + by), Some(Zone::Swap("queue".into())));
        // over the Pane itself: nothing
        let l = rect(&n, AREA, "fft");
        assert_eq!(zone(l.x + 3, l.y + 2), None);
        // beside the layout: nothing
        assert_eq!(zone_at(&slots, Rect::new(10, 1, 140, 43), "fft", 5, cy), None);
    }

    /// The owner's "sandwich" (2026-11-01): small Panes offered a top/bottom split where a swap was meant. A split is
    /// offered only where both halves would be comfortable; that room is the centre's (a swap) otherwise.
    #[test]
    fn a_split_is_offered_only_where_both_halves_are_comfortable() {
        use Axis::*;
        let s = |x| Node::slot(x);
        // a short wide Pane (160x10) under a big one: no top or bottom band, its left and right ones stay
        let n = Node::split(Vertical, Size::Cells(33), s("home"), s("loudness"));
        let slots = slots_of(&n);
        let l = rect(&n, AREA, "loudness");
        assert!(l.width >= 2 * COMFY_W && l.height < 2 * COMFY_H, "{l:?}");
        let cx = l.x + l.width / 2;
        for y in l.y..l.bottom() {
            assert_eq!(zone_at(&slots, AREA, "home", cx, y), Some(Zone::Swap("loudness".into())), "row {y}");
        }
        assert_eq!(zone_at(&slots, AREA, "home", l.x + 1, l.y + 4), Some(Zone::Beside("loudness".into(), Left)));
        assert_eq!(zone_at(&slots, AREA, "home", l.x + 2, l.y), Some(Zone::Beside("loudness".into(), Left)), "a corner is the side band's");
        // a tall narrow one (30x43): no left or right band, top and bottom stay
        let n = Node::split(Horizontal, Size::Cells(30), s("fft"), s("home"));
        let slots = slots_of(&n);
        let f = rect(&n, AREA, "fft");
        for x in f.x..f.right() {
            assert!(!matches!(zone_at(&slots, AREA, "home", x, f.y + f.height / 2), Some(Zone::Beside(_, Left | Right))), "column {x}");
        }
        assert_eq!(zone_at(&slots, AREA, "home", f.x + 5, f.y), Some(Zone::Beside("fft".into(), Top)));
        // a small instrument (40x12): a swap wherever the pointer is
        let n = Node::split(Horizontal, Size::Cells(40), Node::split(Vertical, Size::Cells(12), s("fft"), s("lyrics")), s("home"));
        let slots = slots_of(&n);
        let f = rect(&n, AREA, "fft");
        for (x, y) in [(f.x + 1, f.y), (f.x + 1, f.y + 5), (f.right() - 1, f.bottom() - 1), (f.x + 20, f.y), (f.x + 20, f.bottom() - 1)] {
            assert_eq!(zone_at(&slots, AREA, "home", x, y), Some(Zone::Swap("fft".into())), "({x}, {y}) of {f:?}");
        }
        // the rule as a function: whole halves, along the split
        assert!(split_comfy(Rect::new(0, 0, 48, 16), Horizontal) && split_comfy(Rect::new(0, 0, 48, 16), Vertical));
        assert!(!split_comfy(Rect::new(0, 0, 47, 15), Horizontal) && !split_comfy(Rect::new(0, 0, 47, 15), Vertical));
        // judged on the result too: the queue split in beside a 40x43 column would leave two 20-wide halves
        let wide = Node::split(Horizontal, Size::Cells(40), s("lyrics"), Node::split(Horizontal, Size::Ratio(0.5), s("home"), s("queue")));
        let z = Zone::Beside("lyrics".into(), Right);
        let t = wide.moved("queue", &z).unwrap();
        assert!(!comfortable(&t, "queue", &z, AREA, 2.0), "20 columns each");
        let z = Zone::Beside("lyrics".into(), Bottom);
        let t = wide.moved("queue", &z).unwrap();
        assert!(comfortable(&t, "queue", &z, AREA, 2.0), "21 rows each");
        assert!(comfortable(&wide, "queue", &Zone::Swap("home".into()), AREA, 2.0), "a swap always is");
    }

    #[test]
    fn a_pointer_resting_on_a_border_keeps_its_zone() {
        let n = Node::split(Axis::Horizontal, Size::Ratio(0.5), Node::slot("fft"), Node::slot("queue"));
        let slots = slots_of(&n);
        let q = rect(&n, AREA, "queue");
        let cy = q.y + q.height / 2;
        let bx = (q.width as f32 * SHORT_EDGE_FRAC).round() as u16;
        let band = Zone::Beside("queue".into(), Left);
        let centre = Zone::Swap("queue".into());
        // just inside the centre: from the band, still the band (two columns of slack), from nowhere the centre
        assert_eq!(zone_at(&slots, AREA, "fft", q.x + bx, cy), Some(centre.clone()));
        assert_eq!(zone_near(&slots, AREA, "fft", q.x + bx, cy, Some(&band)), Some(band.clone()));
        assert_eq!(zone_near(&slots, AREA, "fft", q.x + bx + 1, cy, Some(&band)), Some(band.clone()));
        assert_eq!(zone_near(&slots, AREA, "fft", q.x + bx + 2, cy, Some(&band)), Some(centre.clone()), "past the slack");
        // and back the other way
        assert_eq!(zone_near(&slots, AREA, "fft", q.x + bx - 1, cy, Some(&centre)), Some(centre.clone()));
        assert_eq!(zone_near(&slots, AREA, "fft", q.x + bx - 3, cy, Some(&centre)), Some(band.clone()));
        // a row of slack up and down
        let by = (q.height as f32 * EDGE_FRAC).round() as u16;
        let top = Zone::Beside("queue".into(), Top);
        let cx = q.x + q.width / 2;
        assert_eq!(zone_near(&slots, AREA, "fft", cx, q.y + by, Some(&top)), Some(top.clone()));
        assert_eq!(zone_near(&slots, AREA, "fft", cx, q.y + by + 1, Some(&top)), Some(centre.clone()));
        // nothing before: no slack
        assert_eq!(zone_near(&slots, AREA, "fft", q.x + bx, cy, None), Some(centre));
    }

    #[test]
    fn the_screen_edges_are_the_outer_columns_and_past_the_top_and_bottom() {
        let n = Node::classic();
        let slots = slots_of(&n);
        let zone = |x, y| zone_at(&slots, AREA, "queue", x, y);
        assert!(matches!(zone(0, 20), Some(Zone::Edge(Left, _))));
        // (the library is 30 wide: too narrow for a split beside it, so its edge is its centre)
        assert!(matches!(zone(1, 20), Some(Zone::Swap(ref l)) if l == "library"), "one column, not two");
        assert!(matches!(zone(AREA.right() - 1, 10), Some(Zone::Edge(Right, _))));
        assert!(matches!(zone(AREA.right() - 2, 10), Some(Zone::Swap(ref n) | Zone::Beside(ref n, _)) if n == "nowplaying"));
        // the top row is the top Panes' titles: dragging along them is not the screen's edge; the top bar above is
        assert!(matches!(zone(60, AREA.y), Some(Zone::Beside(ref h, Top)) if h == "home"));
        assert!(matches!(zone(60, AREA.y - 1), Some(Zone::Edge(Top, _))));
        // the last row is a bottom border; the player below it is the bottom edge
        assert!(matches!(zone(10, AREA.bottom() - 1), Some(Zone::Beside(ref l, Bottom)) if l == "library"));
        assert!(matches!(zone(10, AREA.bottom()), Some(Zone::Edge(Bottom, _))));
        assert!(matches!(zone(10, AREA.bottom() + 3), Some(Zone::Edge(Bottom, _))));
        let Some(Zone::Edge(_, r)) = zone(0, 20) else { panic!() };
        assert!((0.15..=0.35).contains(&r), "{r}");
    }

    /// The owner's "near the edge it just extends one of the panes like 15%" (2026-11-01): Classic's queue held a
    /// little way up into Home's bottom band, or a Pane pushed against the screen edge it already spans. Both made a
    /// tree with the same Panes in the same places and only the sizes changed (Home 66% to 50%), shown and committed
    /// as if it were a move. Neither is a drop now.
    #[test]
    fn a_drop_that_only_changes_sizes_is_not_a_drop() {
        let n = Node::classic();
        let slots = slots_of(&n);
        let (h, q) = (rect(&n, AREA, "home"), rect(&n, AREA, "queue"));
        // just over the border, in Home's bottom band: the zone is there...
        let z = zone_at(&slots, AREA, "queue", h.x + 20, h.bottom() - 2).unwrap();
        assert_eq!(z, Zone::Beside("home".into(), Bottom));
        // ...and before, dropping there re-split home/queue 50/50: home lost 16% of its height
        let any = n.moved_any("queue", &z).unwrap();
        assert!(any.same_arrangement(&n) && rect(&any, AREA, "home").height < h.height - 5, "{:?} vs {h:?}", rect(&any, AREA, "home"));
        assert_eq!(n.moved("queue", &z), None, "not a move");
        // Home's top band puts the queue above it, which is a move
        assert!(n.moved("queue", &Zone::Beside("home".into(), Top)).is_some());
        assert!(q.y > h.y);
        // the library already spans the left edge: Edge(Left) would only change its width (Cells(30) to a ratio)
        assert_eq!(n.moved("library", &Zone::Edge(Left, 0.2)), None);
        assert!(n.moved_any("library", &Zone::Edge(Left, 0.2)).is_some());
        // a nesting that differs but shows the same row of three is the same arrangement
        use crate::layout::Size::Ratio;
        let s = |x| Node::slot(x);
        let l = Node::split(Axis::Horizontal, Ratio(0.3), s("a"), Node::split(Axis::Horizontal, Ratio(0.5), s("b"), s("c")));
        let r = Node::split(Axis::Horizontal, Ratio(0.6), Node::split(Axis::Horizontal, Ratio(0.5), s("a"), s("b")), s("c"));
        assert!(l.same_arrangement(&r));
        assert!(!l.same_arrangement(&l.swapped("a", "b").unwrap()));
        let v = Node::split(Axis::Vertical, Ratio(0.3), s("a"), Node::split(Axis::Horizontal, Ratio(0.5), s("b"), s("c")));
        assert!(!l.same_arrangement(&v));
    }

    #[test]
    fn neighbours_are_the_panes_sharing_an_edge() {
        let n = Node::classic();
        let slots = slots_of(&n);
        assert_eq!(neighbour(&slots, "library", Right).as_deref(), Some("home"));
        assert_eq!(neighbour(&slots, "home", Left).as_deref(), Some("library"));
        assert_eq!(neighbour(&slots, "home", Bottom).as_deref(), Some("queue"));
        assert_eq!(neighbour(&slots, "queue", Top).as_deref(), Some("home"));
        assert_eq!(neighbour(&slots, "home", Right).as_deref(), Some("nowplaying"));
        assert_eq!(neighbour(&slots, "library", Left), None);
        assert_eq!(neighbour(&slots, "library", Top), None);
        assert_eq!(neighbour(&slots, "nope", Top), None);
    }

    #[test]
    fn an_edited_tree_survives_toml_exactly() {
        let n = Node::singularity();
        let m = n.moved("fft", &Zone::Edge(Bottom, 0.27)).unwrap().moved("lyrics", &Zone::Beside("home".into(), Right)).unwrap();
        let doc = singularity_config::toml_edit::ser::to_document(&m).unwrap();
        let back: Node = singularity_config::toml_edit::de::from_str(&doc.to_string()).unwrap();
        assert_eq!(back, m);
    }

    /// A small seeded generator (xorshift64*), so a failing sequence can be replayed from its seed.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    const POOL: [&str; 16] = ["fft", "loudness", "levels", "scope", "rain", "starfield", "stream", "top", "listening", "today", "marquee", "glow", "vinyl", "waterfall", "aurora", "ripples"];

    /// What a committed tree must always satisfy, whatever got it there.
    fn assert_sound(n: &Node, body: Rect, aspect: f64, file: bool, ctx: &str) {
        assert!(n.valid(body, aspect), "{ctx}: not valid at {body:?}\n{n:?}");
        let names = n.slot_names();
        let mut u = names.clone();
        u.sort();
        u.dedup();
        assert_eq!(u.len(), names.len(), "{ctx}: a name twice: {names:?}");
        for c in CORE {
            assert!(names.iter().any(|s| s == c), "{ctx}: lost {c}");
        }
        assert!(!(names.contains(&"well".into()) && names.contains(&"hole".into())), "{ctx}: two holes");
        let c = n.compute_shaped(body, aspect);
        let cells: u32 = c.slots.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
        assert_eq!(cells, body.width as u32 * body.height as u32, "{ctx}: panes don't tile the screen");
        for (i, (s, r)) in c.slots.iter().enumerate() {
            assert!(r.width >= MIN_W && r.height >= if is_hole(s) { crate::layout::WELL_MIN_H } else { MIN_H }, "{ctx}: {s} is {r:?}");
            assert!(c.slots.iter().skip(i + 1).all(|(_, o)| r.intersection(*o).is_empty()), "{ctx}: overlap");
        }
        if let Some((_, w)) = c.slots.iter().find(|(s, _)| s == "well") {
            let shape = (w.width - 2) as f64 / ((w.height - 2) as f64 * aspect);
            assert!(shape <= WELL_ASPECT * 1.05 + 0.12, "{ctx}: the well is {w:?}, {shape:.2} wide");
        }
        for d in &c.dividers {
            assert!(n.at_path(&d.path).is_some_and(|x| matches!(x, Node::Split { .. })), "{ctx}: divider off the tree");
        }
        let doc = singularity_config::toml_edit::ser::to_document(n).unwrap();
        let back: Node = singularity_config::toml_edit::de::from_str(&doc.to_string()).unwrap();
        assert_eq!(&back, n, "{ctx}: TOML changed it");
        // saved the way layouts.rs writes it, it reads back identically too
        if file {
            let dir = tempfile::tempdir().unwrap();
            let saved = crate::layouts::save(dir.path(), "p", "singularity", "singularity", n).unwrap();
            assert_eq!(crate::layouts::load_dir(dir.path()).0, vec![saved], "{ctx}: a saved file changed it");
        }
    }

    /// Thousands of random edits (moves to random drop zones, swaps, splits, closes, resizes, divider drags, undo and
    /// redo) on every built-in Design at terminal sizes from 60x20 to 300x90. Every edit is offered only when its tree
    /// is valid, as the UI does; every tree that gets committed must then be sound; a drop always changes the
    /// arrangement, never just sizes; undo and redo walk back and forth through exactly the trees committed.
    #[test]
    fn random_edits_never_break_a_layout() {
        let designs = [("singularity", Node::singularity()), ("event-horizon", Node::event_horizon()), ("classic", Node::classic()), ("cockpit", Node::cockpit()), ("terminal", Node::terminal())];
        let sizes = [(60u16, 20u16), (80, 24), (100, 30), (120, 40), (160, 48), (200, 60), (300, 90)];
        let (mut sequences, mut committed, mut refused, mut skipped, mut undone) = (0, 0, 0, 0, 0);
        for (name, base) in &designs {
            for &(w, h) in &sizes {
                let body = Rect::new(0, 1, w, h - 5);
                for seed in 1..=40u64 {
                    let aspect = [1.8, 2.0, 2.2][seed as usize % 3];
                    if !base.valid(body, aspect) {
                        skipped += 1;
                        continue;
                    }
                    sequences += 1;
                    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((w as u64) << 32 | h as u64));
                    let mut tree = base.clone();
                    // the undo history, and a plain model of what it must hold
                    let mut hist: crate::arrange::history::History<Node> = Default::default();
                    let (mut back, mut fwd): (Vec<Node>, Vec<Node>) = (vec![], vec![]);
                    for step in 0..60 {
                        let ctx = format!("{name} {w}x{h} seed {seed} step {step}");
                        let slots = tree.compute_shaped(body, aspect).slots;
                        let pick = |rng: &mut Rng| slots[rng.below(slots.len())].0.clone();
                        let roll = rng.below(9);
                        if roll >= 7 {
                            let undo = roll == 7;
                            let ok = |t: &Node| t.valid(body, aspect);
                            let got = if undo { hist.undo(tree.clone(), ok) } else { hist.redo(tree.clone(), ok) };
                            let want = if undo { back.pop() } else { fwd.pop() };
                            assert_eq!(got, want, "{ctx}: {}", if undo { "undo" } else { "redo" });
                            if let Some(t) = got {
                                if undo { fwd.push(tree.clone()) } else { back.push(tree.clone()) }
                                assert_sound(&t, body, aspect, false, &ctx);
                                tree = t;
                                undone += 1;
                            }
                            continue;
                        }
                        let cand: Option<Node> = match roll {
                            // drag a Pane to a random place on screen (the bars above and below too), as the mouse would
                            0..=2 => {
                                let who = pick(&mut rng);
                                let (x, y) = (rng.below(w as usize) as u16, rng.below(h as usize) as u16);
                                let z = zone_at(&slots, body, &who, x, y);
                                // a split is only offered on a Pane with room for two comfortable halves
                                if let Some(Zone::Beside(t, side)) = &z {
                                    let r = slots.iter().find(|(s, _)| s == t).unwrap().1;
                                    assert!(split_comfy(r, side.axis()), "{ctx}: {side:?} of {t} {r:?} offered");
                                }
                                let m = z.as_ref().and_then(|z| tree.moved(&who, z).filter(|m| comfortable(m, &who, z, body, aspect)));
                                assert!(m.as_ref().is_none_or(|m| !m.same_arrangement(&tree)), "{ctx}: a drop that only changes sizes");
                                m
                            }
                            3 => {
                                let (a, b) = (pick(&mut rng), pick(&mut rng));
                                tree.swapped(&a, &b)
                            }
                            4 => {
                                let free: Vec<&str> = POOL.iter().copied().filter(|p| !tree.has(p)).collect();
                                let side = [Left, Right, Top, Bottom][rng.below(4)];
                                match free.is_empty() {
                                    true => None,
                                    false => tree.beside(&pick(&mut rng), free[rng.below(free.len())], side),
                                }
                            }
                            5 => tree.closed(&pick(&mut rng)),
                            _ => {
                                let who = pick(&mut rng);
                                match rng.below(3) {
                                    0 | 1 => tree.resized(&who, rng.below(2) == 0, body),
                                    _ => {
                                        let mut t = tree.clone();
                                        let ds = t.compute(body).dividers;
                                        let d = ds[rng.below(ds.len())].clone();
                                        t.drag(&d, rng.below(w as usize + 20) as u16);
                                        Some(t)
                                    }
                                }
                            }
                        };
                        match cand {
                            Some(c) if c != tree && c.valid(body, aspect) => {
                                assert_sound(&c, body, aspect, committed % 6 == 0, &ctx);
                                hist.push(tree.clone());
                                back.push(tree.clone());
                                if back.len() > crate::arrange::history::CAP {
                                    back.remove(0);
                                }
                                fwd.clear();
                                tree = c;
                                committed += 1;
                            }
                            _ => refused += 1,
                        }
                    }
                    // and it still lays out, without a panic or an overlap, at any other size
                    for (w2, h2) in [(40, 10), (90, 30), (250, 80)] {
                        let c = tree.compute_shaped(Rect::new(0, 1, w2, h2), aspect);
                        let cells: u32 = c.slots.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
                        assert_eq!(cells, w2 as u32 * h2 as u32, "{name} {w}x{h} seed {seed} shown at {w2}x{h2}");
                    }
                }
            }
        }
        eprintln!("{sequences} sequences ({skipped} skipped: the Design itself doesn't fit), {committed} edits committed, {refused} refused, {undone} undos and redos");
        assert!(sequences >= 1000 && committed >= 15_000 && undone >= 5_000, "{sequences} {committed} {undone}");
    }
}
