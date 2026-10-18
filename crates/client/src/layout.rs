//! Split-tree layout engine (D18). A layout is a binary tree of splits whose leaves are Slots.
//! Built-in designs are preset trees; user layouts are saved trees (#19); free tiling edits the tree (v1.x).

use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// children side by side (divider is vertical)
    Horizontal,
    /// children stacked (divider is horizontal)
    Vertical,
}

/// How a split divides its area: the *first* child's size, or the *last* child's (`Last`: a fixed right/bottom column).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    Cells(u16),
    Ratio(#[serde(serialize_with = "exact")] f32),
    Last(u16),
}

/// A ratio written as its shortest decimal (`0.58`, not `0.5799999833`), which reads back to the same f32.
fn exact<S: serde::Serializer>(r: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(format!("{r}").parse().unwrap_or(*r as f64))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Node {
    Split { axis: Axis, first: Size, a: Box<Node>, b: Box<Node> },
    Slot(String),
}

/// Smallest Pane we will ever produce, per axis (borders + one line of content).
pub const MIN_W: u16 = 12;
pub const MIN_H: u16 = 3;

/// The gravity well's widest shape, width over height in pixels (4:3, owner round 2; was 16:10); a little wider is let
/// be. Dragging the divider can still make it narrower.
pub const WELL_ASPECT: f64 = 4.0 / 3.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    /// path of child indices from the root to the split node (0 = a, 1 = b)
    pub path: Vec<u8>,
    pub axis: Axis,
    /// the split node's whole area (needed to convert a mouse position back into a size)
    pub area: Rect,
    /// the cells a mouse can grab
    pub grip: Rect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Computed {
    pub slots: Vec<(String, Rect)>,
    pub dividers: Vec<Divider>,
}

impl Node {
    fn split(axis: Axis, first: Size, a: Node, b: Node) -> Node {
        Node::Split { axis, first, a: Box::new(a), b: Box::new(b) }
    }
    fn slot(name: &str) -> Node {
        Node::Slot(name.into())
    }

    /// The Singularity design (default): library | home+queue | gravity well + lyrics + instruments.
    pub fn singularity() -> Node {
        use Axis::*;
        use Size::*;
        let left = Node::split(Vertical, Ratio(0.68), Node::slot("library"), Node::slot("stream"));
        let mid = Node::split(Vertical, Ratio(0.52), Node::slot("home"), Node::slot("queue"));
        let instruments = Node::split(Horizontal, Ratio(0.5), Node::slot("loudness"), Node::slot("fft"));
        let right = Node::split(Vertical, Ratio(0.44), Node::slot("well"), Node::split(Vertical, Ratio(0.57), Node::slot("lyrics"), instruments));
        Node::split(Horizontal, Cells(30), left, Node::split(Horizontal, Ratio(0.58), mid, right))
    }

    /// Event Horizon: library | home + up next | the black hole (no well: it owns its quadrant) + lyrics.
    pub fn event_horizon() -> Node {
        use Axis::*;
        use Size::*;
        let mid = Node::split(Vertical, Ratio(0.62), Node::slot("home"), Node::slot("queue"));
        let right = Node::split(Vertical, Ratio(0.64), Node::slot("hole"), Node::slot("lyrics"));
        Node::split(Horizontal, Cells(28), Node::slot("library"), Node::split(Horizontal, Ratio(0.54), mid, right))
    }

    /// Classic+: the desktop app's shape. Library | Home over the queue | Now playing over lyrics.
    pub fn classic() -> Node {
        use Axis::*;
        use Size::*;
        let mid = Node::split(Vertical, Ratio(0.66), Node::slot("home"), Node::slot("queue"));
        let right = Node::split(Vertical, Ratio(0.55), Node::slot("nowplaying"), Node::slot("lyrics"));
        Node::split(Horizontal, Cells(30), Node::slot("library"), Node::split(Horizontal, Last(40), mid, right))
    }

    /// Cockpit: instruments on top (spectrum, stream; loudness, levels, listening), browsing below (Home | queue over
    /// lyrics | library).
    pub fn cockpit() -> Node {
        use Axis::*;
        use Size::*;
        let r1 = Node::split(Horizontal, Ratio(0.66), Node::slot("fft"), Node::slot("stream"));
        let r2 = Node::split(Horizontal, Ratio(0.3), Node::slot("loudness"), Node::split(Horizontal, Ratio(0.36), Node::slot("levels"), Node::slot("listening")));
        let r3 = Node::split(Horizontal, Ratio(0.52), Node::slot("home"), Node::split(Horizontal, Ratio(0.55), Node::split(Vertical, Ratio(0.5), Node::slot("queue"), Node::slot("lyrics")), Node::slot("library")));
        Node::split(Vertical, Ratio(0.36), r1, Node::split(Vertical, Ratio(0.3), r2, r3))
    }

    /// Terminal: tree | home over rain + lyrics log | scope, the queue, fft (`top` is a swap away).
    pub fn terminal() -> Node {
        use Axis::*;
        use Size::*;
        let mid = Node::split(Vertical, Ratio(0.58), Node::slot("home"), Node::split(Horizontal, Ratio(0.5), Node::slot("rain"), Node::slot("lyrics")));
        let right = Node::split(Vertical, Ratio(0.35), Node::slot("scope"), Node::split(Vertical, Ratio(0.45), Node::slot("queue"), Node::slot("fft")));
        Node::split(Horizontal, Cells(30), Node::slot("library"), Node::split(Horizontal, Last(36), mid, right))
    }

    /// The least room this subtree needs along `axis`: a Slot's minimum, summed along a split's own axis.
    fn min_len(&self, axis: Axis) -> u16 {
        match self {
            Node::Slot(_) => match axis {
                Axis::Horizontal => MIN_W,
                Axis::Vertical => MIN_H,
            },
            Node::Split { axis: own, a, b, .. } if *own == axis => a.min_len(axis) + b.min_len(axis),
            Node::Split { a, b, .. } => a.min_len(axis).max(b.min_len(axis)),
        }
    }

    /// The Slots' names, in order.
    pub fn slot_names(&self) -> Vec<String> {
        match self {
            Node::Slot(s) => vec![s.clone()],
            Node::Split { a, b, .. } => {
                let mut v = a.slot_names();
                v.extend(b.slot_names());
                v
            }
        }
    }

    /// Put Widget `to` in the Slot that shows `from`.
    pub fn swap(&mut self, from: &str, to: &str) -> bool {
        match self {
            Node::Slot(s) if s == from => {
                *s = to.into();
                true
            }
            Node::Slot(_) => false,
            Node::Split { a, b, .. } => a.swap(from, to) || b.swap(from, to),
        }
    }

    pub fn compute(&self, area: Rect) -> Computed {
        let mut out = Computed::default();
        self.compute_into(area, &mut Vec::new(), None, &mut out);
        out
    }

    /// `compute`, but a gravity well wider than `WELL_ASPECT` (cells are `cell_aspect` times taller than wide) gives
    /// its extra width to its neighbour across the nearest side-by-side split (in Singularity, the middle column).
    pub fn compute_shaped(&self, area: Rect, cell_aspect: f64) -> Computed {
        let out = self.compute(area);
        let Some(well) = out.slots.iter().find(|(s, _)| s == "well").map(|(_, r)| *r) else { return out };
        // the inner picture, inside the frame
        let want = ((well.height.saturating_sub(2)) as f64 * cell_aspect * WELL_ASPECT).round() as u16 + 2;
        if well.width <= want + want / 20 {
            return out;
        }
        let Some(path) = self.path_of("well") else { return out };
        let split = (0..path.len()).rev().find(|&k| matches!(self.at_path(&path[..k]), Some(Node::Split { axis: Axis::Horizontal, .. })));
        let Some(k) = split else { return out };
        let extra = (well.width - want) as i32;
        // the well on the right: its split's first side grows; on the left: shrinks
        let delta = if path[k] == 1 { extra } else { -extra };
        let mut shaped = Computed::default();
        self.compute_into(area, &mut Vec::new(), Some((&path[..k], delta)), &mut shaped);
        shaped
    }

    /// Where Slot `name` is: child indices from the root (0 = a, 1 = b).
    fn path_of(&self, name: &str) -> Option<Vec<u8>> {
        match self {
            Node::Slot(s) => (s == name).then(Vec::new),
            Node::Split { a, b, .. } => [a, b].into_iter().enumerate().find_map(|(i, c)| {
                let mut p = c.path_of(name)?;
                p.insert(0, i as u8);
                Some(p)
            }),
        }
    }

    fn at_path(&self, path: &[u8]) -> Option<&Node> {
        match (path.split_first(), self) {
            (None, n) => Some(n),
            (Some((0, rest)), Node::Split { a, .. }) => a.at_path(rest),
            (Some((_, rest)), Node::Split { b, .. }) => b.at_path(rest),
            _ => None,
        }
    }

    /// `nudge`: the split at this path moves its boundary by that many cells (minimums still hold).
    fn compute_into(&self, area: Rect, path: &mut Vec<u8>, nudge: Option<(&[u8], i32)>, out: &mut Computed) {
        match self {
            Node::Slot(name) => out.slots.push((name.clone(), area)),
            Node::Split { axis, first, a, b } => {
                let total = match axis {
                    Axis::Horizontal => area.width,
                    Axis::Vertical => area.height,
                };
                let (min_a, min_b) = (a.min_len(*axis), b.min_len(*axis));
                let want = match first {
                    Size::Cells(c) => *c,
                    Size::Ratio(r) => (total as f32 * r.clamp(0.05, 0.95)).round() as u16,
                    Size::Last(c) => total.saturating_sub(*c),
                };
                // keep both sides at least as big as their own Panes need when there is room; otherwise share the
                // room in proportion to those needs
                let want = match nudge {
                    Some((at, by)) if at == path.as_slice() => (want as i32 + by).clamp(0, total as i32) as u16,
                    _ => want,
                };
                let n = if total >= min_a + min_b { want.clamp(min_a, total - min_b) } else { (total as u32 * min_a as u32 / (min_a + min_b).max(1) as u32) as u16 };
                let (ra, rb, grip) = match axis {
                    Axis::Horizontal => (
                        Rect::new(area.x, area.y, n, area.height),
                        Rect::new(area.x + n, area.y, total - n, area.height),
                        Rect::new((area.x + n).saturating_sub(1), area.y, 2, area.height),
                    ),
                    Axis::Vertical => (
                        Rect::new(area.x, area.y, area.width, n),
                        Rect::new(area.x, area.y + n, area.width, total - n),
                        Rect::new(area.x, (area.y + n).saturating_sub(1), area.width, 2),
                    ),
                };
                out.dividers.push(Divider { path: path.clone(), axis: *axis, area, grip });
                path.push(0);
                a.compute_into(ra, path, nudge, out);
                path.pop();
                path.push(1);
                b.compute_into(rb, path, nudge, out);
                path.pop();
            }
        }
    }

    fn at_path_mut(&mut self, path: &[u8]) -> Option<&mut Node> {
        match (path.split_first(), self) {
            (None, n) => Some(n),
            (Some((0, rest)), Node::Split { a, .. }) => a.at_path_mut(rest),
            (Some((_, rest)), Node::Split { b, .. }) => b.at_path_mut(rest),
            _ => None,
        }
    }

    /// Move a divider so the boundary sits at mouse column/row `pos`. Keeps the size's kind
    /// (fixed cells stay fixed, ratios stay ratios) so window resizes behave as the preset intended.
    pub fn drag(&mut self, d: &Divider, pos: u16) {
        let (start, total) = match d.axis {
            Axis::Horizontal => (d.area.x, d.area.width),
            Axis::Vertical => (d.area.y, d.area.height),
        };
        let Some(Node::Split { a, b, .. }) = self.at_path_mut(&d.path) else { return };
        let (min_a, min_b) = (a.min_len(d.axis), b.min_len(d.axis));
        if total < min_a + min_b {
            return;
        }
        let n = pos.saturating_sub(start).clamp(min_a, total - min_b);
        if let Some(Node::Split { first, .. }) = self.at_path_mut(&d.path) {
            *first = match first {
                Size::Cells(_) => Size::Cells(n),
                Size::Ratio(_) => Size::Ratio(n as f32 / total as f32),
                Size::Last(_) => Size::Last(total - n),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_tile_the_area_exactly() {
        let designs = [Node::singularity(), Node::event_horizon(), Node::classic(), Node::cockpit(), Node::terminal()];
        for ((w, h), d) in [(80, 24), (120, 40), (300, 80), (40, 10)].into_iter().flat_map(|s| designs.iter().map(move |d| (s, d))) {
            let area = Rect::new(0, 0, w, h);
            let c = d.compute(area);
            let cells: u32 = c.slots.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
            assert_eq!(cells, w as u32 * h as u32, "{w}x{h}");
            for (i, (_, a)) in c.slots.iter().enumerate() {
                assert!(a.width > 0 && a.height > 0, "zero-size slot at {w}x{h}");
                for (_, b) in c.slots.iter().skip(i + 1) {
                    assert!(a.intersection(*b).is_empty(), "overlap at {w}x{h}");
                }
            }
        }
    }

    #[test]
    fn dragging_any_divider_anywhere_never_overlaps_or_vanishes() {
        for preset in [Node::singularity(), Node::event_horizon(), Node::classic(), Node::cockpit(), Node::terminal()] {
            for (w, h) in [(80, 24), (160, 48), (300, 90)] {
                let area = Rect::new(0, 0, w, h);
                for pos in [0u16, 1, 5, w / 3, w / 2, w - 3, w, u16::MAX] {
                    let mut n = preset.clone();
                    for i in 0..n.compute(area).dividers.len() {
                        let d = n.compute(area).dividers[i].clone();
                        let p = if d.axis == Axis::Vertical { pos.min(h + 5) } else { pos };
                        n.drag(&d, p);
                    }
                    let c = n.compute(area);
                    let cells: u32 = c.slots.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
                    assert_eq!(cells, w as u32 * h as u32, "{w}x{h} after dragging to {pos}");
                    for (i, (sa, a)) in c.slots.iter().enumerate() {
                        assert!(a.width >= MIN_W.min(w / 4) && a.height >= MIN_H.min(h / 6), "{sa} shrank to {a:?} at {w}x{h} (pos {pos})");
                        for (_, b) in c.slots.iter().skip(i + 1) {
                            assert!(a.intersection(*b).is_empty(), "overlap at {w}x{h}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_design_has_home_the_library_the_queue_and_lyrics() {
        // owner rule (D43): the rest is each Design's own
        for (name, d) in [("singularity", Node::singularity()), ("event-horizon", Node::event_horizon()), ("classic", Node::classic()), ("cockpit", Node::cockpit()), ("terminal", Node::terminal())] {
            let slots = d.slot_names();
            for must in ["home", "library", "queue", "lyrics"] {
                assert!(slots.iter().any(|s| s == must), "{name} has no {must}: {slots:?}");
            }
        }
    }

    #[test]
    fn a_wide_well_keeps_its_shape_and_the_middle_gets_the_width() {
        let n = Node::singularity();
        for (w, h, aspect) in [(256u16, 68u16, 2.0), (256, 68, 2.2), (400, 60, 2.0), (140, 40, 2.0)] {
            let area = Rect::new(0, 0, w, h);
            let (plain, shaped) = (n.compute(area), n.compute_shaped(area, aspect));
            let get = |c: &Computed, s: &str| c.slots.iter().find(|(n, _)| n == s).unwrap().1;
            let well = get(&shaped, "well");
            let shape = (well.width - 2) as f64 / ((well.height - 2) as f64 * aspect);
            assert!(shape <= WELL_ASPECT * 1.05 + 0.05, "{w}x{h}: {well:?} is {shape:.2}");
            let freed = get(&plain, "well").width - well.width;
            assert_eq!(get(&shaped, "home").width, get(&plain, "home").width + freed, "{w}x{h}: the middle takes it");
            assert_eq!(get(&shaped, "library").width, get(&plain, "library").width);
            let cells: u32 = shaped.slots.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
            assert_eq!(cells, w as u32 * h as u32);
        }
        // already well-shaped: nothing moves
        let area = Rect::new(0, 0, 120, 40);
        assert_eq!(n.compute_shaped(area, 2.0), n.compute(area));
        // no well, no change
        assert_eq!(Node::classic().compute_shaped(Rect::new(0, 0, 300, 60), 2.0), Node::classic().compute(Rect::new(0, 0, 300, 60)));
    }

    #[test]
    fn a_tree_survives_toml_exactly() {
        let mut n = Node::singularity();
        let area = Rect::new(0, 0, 173, 51);
        let d = n.compute(area).dividers[1].clone();
        n.drag(&d, 97);
        let doc = singularity_config::toml_edit::ser::to_document(&n).unwrap();
        let text = doc.to_string();
        assert!(!text.contains("99999"), "ratios are written short:\n{text}");
        let back: Node = singularity_config::toml_edit::de::from_str(&text).unwrap();
        assert_eq!(back, n);
    }

    #[test]
    fn dragging_respects_minimums_and_kind() {
        let mut n = Node::singularity();
        let area = Rect::new(0, 0, 160, 50);
        let c = n.compute(area);
        let root = c.dividers[0].clone();
        n.drag(&root, 2); // way too far left
        let c2 = n.compute(area);
        let lib = c2.slots.iter().find(|(s, _)| s == "library").unwrap().1;
        assert_eq!(lib.width, MIN_W);
        assert!(matches!(n, Node::Split { first: Size::Cells(_), .. }));
    }
}
