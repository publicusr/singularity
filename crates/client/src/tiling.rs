//! The split tree's edits as pure functions, under `Arrangement` (which is what the App edits through): every edit
//! returns a new tree (or None); `valid` says whether a tree can be shown at a terminal size (every Pane at least the
//! Pane minimum, whatever it holds). Edits name Panes by their identity (`PaneId`), which stays where it is. (D66's
//! drop zones, live reflow and the dwell went with D87: a drag only moves contents now.)

use crate::layout::{Axis, MIN_H, MIN_W, Node, Pane, PaneId, Size, Widget};
use ratatui::layout::Rect;

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

/// The Pane next to Pane `id` in direction `dir`: the one sharing the longest stretch of its edge.
pub fn neighbour(panes: &[(Pane, Rect)], id: PaneId, dir: Side) -> Option<PaneId> {
    let me = panes.iter().find(|(p, _)| p.id == id)?.1;
    let span = |a0: u16, a1: u16, b0: u16, b1: u16| a1.min(b1).saturating_sub(a0.max(b0));
    panes
        .iter()
        .filter(|(p, _)| p.id != id)
        .filter_map(|(p, o)| {
            let (touches, overlap) = match dir {
                Side::Left => (o.right() == me.x, span(o.y, o.bottom(), me.y, me.bottom())),
                Side::Right => (o.x == me.right(), span(o.y, o.bottom(), me.y, me.bottom())),
                Side::Top => (o.bottom() == me.y, span(o.x, o.right(), me.x, me.right())),
                Side::Bottom => (o.y == me.bottom(), span(o.x, o.right(), me.x, me.right())),
            };
            (touches && overlap > 0).then_some((p.id, overlap))
        })
        .max_by_key(|(_, overlap)| *overlap)
        .map(|(p, _)| p)
}

impl Node {
    /// Whether the tree could be shown at `area` as it is: unique Panes and Widgets, sane sizes and every Pane at least
    /// the Pane minimum, whatever it holds (D87, #48: a Widget in too small a Pane says so itself).
    pub fn valid(&self, area: Rect) -> bool {
        let panes = self.panes();
        let mut ids: Vec<PaneId> = panes.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        let widgets = self.widgets();
        let mut unique = widgets.clone();
        unique.sort();
        unique.dedup();
        if panes.is_empty() || ids.len() != panes.len() || unique.len() != widgets.len() || !self.sizes_sane() {
            return false;
        }
        if self.min_len(Axis::Horizontal) > area.width || self.min_len(Axis::Vertical) > area.height {
            return false;
        }
        let out = self.compute(area);
        out.panes.len() == panes.len() && out.panes.iter().all(|(_, r)| r.width >= MIN_W && r.height >= MIN_H)
    }

    /// Ratios in range and fixed sizes that mean something.
    pub(crate) fn sizes_sane(&self) -> bool {
        match self {
            Node::Pane(_) => true,
            Node::Split { first, a, b, .. } => {
                (match first {
                    Size::Ratio(r) => r.is_finite() && (0.05..=0.95).contains(r),
                    Size::Cells(c) | Size::Last(c) => (1..=2000).contains(c),
                }) && a.sizes_sane()
                    && b.sizes_sane()
            }
        }
    }

    /// Whether `w` is on screen.
    pub fn has(&self, w: Widget) -> bool {
        self.pane_of(w).is_some()
    }

    /// Whether Pane `id` is in this tree.
    pub fn contains(&self, id: PaneId) -> bool {
        self.path_of(id).is_some()
    }

    /// Without Pane `id`: its sibling takes the room its parent split had. None if it isn't here or is the only Pane.
    pub fn without(&self, id: PaneId) -> Option<Node> {
        match self {
            Node::Pane(_) => None,
            Node::Split { axis, first, a, b } => {
                let is = |n: &Node| matches!(n, Node::Pane(p) if p.id == id);
                if is(a) {
                    return Some((**b).clone());
                }
                if is(b) {
                    return Some((**a).clone());
                }
                if let Some(na) = a.without(id) {
                    return Some(Node::split(*axis, *first, na, (**b).clone()));
                }
                b.without(id).map(|nb| Node::split(*axis, *first, (**a).clone(), nb))
            }
        }
    }

    /// A new Pane holding `w` (or nothing) on `side` of Pane `target`, which gives it half its room.
    pub fn split_beside(&self, target: PaneId, w: Option<Widget>, side: Side) -> Option<Node> {
        self.put_beside(target, Pane { id: self.next_id(), widget: w }, side)
    }

    /// Pane `id` holds `w` (or nothing) from now on; nothing else changes. False if there is no such Pane.
    pub(crate) fn put(&mut self, id: PaneId, w: Option<Widget>) -> bool {
        match self {
            Node::Pane(p) if p.id == id => {
                p.widget = w;
                true
            }
            Node::Pane(_) => false,
            Node::Split { a, b, .. } => a.put(id, w) || b.put(id, w),
        }
    }

    /// Pane `new` (one not in this tree) on `side` of Pane `target`, which gives it half its room.
    fn put_beside(&self, target: PaneId, new: Pane, side: Side) -> Option<Node> {
        if self.contains(new.id) || new.widget.is_some_and(|w| self.has(w)) {
            return None;
        }
        fn go(n: &Node, target: PaneId, new: Pane, side: Side) -> Option<Node> {
            match n {
                Node::Pane(p) if p.id == target => {
                    let (old, fresh) = (Node::Pane(*p), Node::Pane(new));
                    let (a, b) = if side.first() { (fresh, old) } else { (old, fresh) };
                    Some(Node::split(side.axis(), Size::Ratio(0.5), a, b))
                }
                Node::Pane(_) => None,
                Node::Split { axis, first, a, b } => match go(a, target, new, side) {
                    Some(na) => Some(Node::split(*axis, *first, na, (**b).clone())),
                    None => go(b, target, new, side).map(|nb| Node::split(*axis, *first, (**a).clone(), nb)),
                },
            }
        }
        go(self, target, new, side)
    }

    /// Pane `id` a step bigger or smaller (2 columns, or 1 row): the nearest split around it that can move its
    /// boundary does. None if none can.
    pub fn resized(&self, id: PaneId, grow: bool, area: Rect) -> Option<Node> {
        let path = self.path_of(id)?;
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
    use crate::layout::Widget as W;
    use Side::*;

    fn rect(n: &Node, area: Rect, w: W) -> Rect {
        n.compute(area).rect_of(w).unwrap()
    }

    /// The Pane showing `w`.
    fn id(n: &Node, w: W) -> PaneId {
        n.pane_of(w).unwrap_or_else(|| panic!("no Pane shows {w}"))
    }

    /// An identity no Pane has.
    const NOPE: PaneId = PaneId(999);

    const AREA: Rect = Rect { x: 0, y: 1, width: 160, height: 43 };

    #[test]
    fn removing_a_pane_gives_its_room_to_the_sibling() {
        let n = Node::event_horizon();
        let m = n.without(id(&n, W::Lyrics)).unwrap();
        assert!(!m.has(W::Lyrics));
        // the hole is alone in its column now: it takes the lyrics' rows too
        assert_eq!(rect(&m, AREA, W::Hole).height, AREA.height);
        assert_eq!(rect(&m, AREA, W::Hole).width, rect(&n, AREA, W::Hole).width);
        assert!(n.without(NOPE).is_none());
        assert!(Node::pane(W::Fft).without(PaneId(1)).is_none(), "the last Pane stays");
    }

    #[test]
    fn a_pane_goes_beside_another_on_the_side_asked() {
        let n = Node::classic();
        for (side, left_of) in [(Left, true), (Right, false)] {
            let m = n.split_beside(id(&n, W::Queue), Some(W::Fft), side).unwrap();
            let (f, q) = (rect(&m, AREA, W::Fft), rect(&m, AREA, W::Queue));
            assert_eq!(f.y, q.y);
            assert_eq!(f.x < q.x, left_of);
            assert!((f.width as i32 - q.width as i32).abs() <= 1, "half each: {f:?} {q:?}");
        }
        for (side, above) in [(Top, true), (Bottom, false)] {
            let m = n.split_beside(id(&n, W::Queue), Some(W::Fft), side).unwrap();
            let (f, q) = (rect(&m, AREA, W::Fft), rect(&m, AREA, W::Queue));
            assert_eq!(f.x, q.x);
            assert_eq!(f.y < q.y, above);
        }
        assert!(n.split_beside(id(&n, W::Queue), Some(W::Home), Left).is_none(), "no Widget twice");
        assert!(n.split_beside(NOPE, Some(W::Fft), Left).is_none());
    }

    #[test]
    fn a_step_resizes_the_nearest_split_that_can_move() {
        let n = Node::classic();
        let before = rect(&n, AREA, W::Library).width;
        let m = n.resized(id(&n, W::Library), true, AREA).unwrap();
        assert_eq!(rect(&m, AREA, W::Library).width, before + 2);
        let m = n.resized(id(&n, W::Library), false, AREA).unwrap();
        assert_eq!(rect(&m, AREA, W::Library).width, before - 2);
        // home is in a stack: it grows taller first
        let h = rect(&n, AREA, W::Home).height;
        assert_eq!(rect(&n.resized(id(&n, W::Home), true, AREA).unwrap(), AREA, W::Home).height, h + 1);
        // squeezed to its minimum, it can't shrink more: the next split out is tried, else None
        let mut m = n.clone();
        for _ in 0..80 {
            m = m.resized(id(&m, W::Library), false, AREA).unwrap_or(m);
        }
        assert_eq!(rect(&m, AREA, W::Library).width, MIN_W);
        assert!(m.valid(AREA));
    }

    #[test]
    fn validity_catches_small_trees_dupes_and_bad_sizes() {
        let n = Node::singularity();
        assert!(n.valid(AREA));
        assert!(!n.valid(Rect::new(0, 0, 20, 8)), "no room for this many Panes");
        let dup = Node::split(Axis::Horizontal, Size::Ratio(0.5), Node::pane(W::Home), Node::pane(W::Home));
        assert!(!dup.valid(AREA));
        let bad = Node::split(Axis::Horizontal, Size::Ratio(1.5), Node::pane(W::Home), Node::pane(W::Queue));
        assert!(!bad.valid(AREA));
        let nan = Node::split(Axis::Horizontal, Size::Ratio(f32::NAN), Node::pane(W::Home), Node::pane(W::Queue));
        assert!(!nan.valid(AREA));
        let zero = Node::split(Axis::Horizontal, Size::Cells(0), Node::pane(W::Home), Node::pane(W::Queue));
        assert!(!zero.valid(AREA));
        // whatever a Pane holds: a flat gravity well, or one across the whole width, is a valid floor plan (it says
        // `needs W × H`, or letterboxes its picture)
        let col = |h| Node::split(Axis::Vertical, Size::Cells(h), Node::pane(W::Well), Node::pane(W::Queue));
        assert!(Node::split(Axis::Horizontal, Size::Ratio(0.3), col(MIN_H), Node::pane(W::Home)).valid(AREA));
        assert!(Node::split(Axis::Vertical, Size::Cells(8), Node::pane(W::Well), Node::pane(W::Home)).valid(AREA));
    }

    #[test]
    fn neighbours_are_the_panes_sharing_an_edge() {
        let n = Node::classic();
        let slots = n.compute(AREA).panes;
        assert_eq!(neighbour(&slots, id(&n, W::Library), Right), Some(id(&n, W::Home)));
        assert_eq!(neighbour(&slots, id(&n, W::Home), Left), Some(id(&n, W::Library)));
        assert_eq!(neighbour(&slots, id(&n, W::Home), Bottom), Some(id(&n, W::Queue)));
        assert_eq!(neighbour(&slots, id(&n, W::Queue), Top), Some(id(&n, W::Home)));
        assert_eq!(neighbour(&slots, id(&n, W::Home), Right), Some(id(&n, W::NowPlaying)));
        assert_eq!(neighbour(&slots, id(&n, W::Library), Left), None);
        assert_eq!(neighbour(&slots, id(&n, W::Library), Top), None);
        assert_eq!(neighbour(&slots, NOPE, Top), None);
    }

    #[test]
    fn an_edited_tree_survives_toml_exactly() {
        let n = Node::singularity();
        let m = n.without(id(&n, W::Fft)).unwrap().split_beside(id(&n, W::Home), None, Right).unwrap();
        let mut m = m.split_beside(id(&m, W::Lyrics), Some(W::Fft), Bottom).unwrap();
        m.put(id(&m, W::Queue), Some(W::Rain));
        let doc = singularity_config::toml_edit::ser::to_document(&m).unwrap();
        let back: Node = singularity_config::toml_edit::de::from_str(&doc.to_string()).unwrap();
        assert_eq!(back, m);
    }
}
