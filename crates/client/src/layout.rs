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
    Ratio(f32),
    Last(u16),
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

    /// Classic+: the desktop app's shape. Library | Home | Now playing.
    pub fn classic() -> Node {
        use Axis::*;
        use Size::*;
        Node::split(Horizontal, Cells(30), Node::slot("library"), Node::split(Horizontal, Last(40), Node::slot("home"), Node::slot("nowplaying")))
    }

    /// Cockpit: instruments on top (spectrum, stream; loudness, levels, listening), browsing below.
    pub fn cockpit() -> Node {
        use Axis::*;
        use Size::*;
        let r1 = Node::split(Horizontal, Ratio(0.66), Node::slot("fft"), Node::slot("stream"));
        let r2 = Node::split(Horizontal, Ratio(0.3), Node::slot("loudness"), Node::split(Horizontal, Ratio(0.36), Node::slot("levels"), Node::slot("listening")));
        let r3 = Node::split(Horizontal, Ratio(0.62), Node::slot("home"), Node::split(Vertical, Ratio(0.5), Node::slot("queue"), Node::slot("library")));
        Node::split(Vertical, Ratio(0.36), r1, Node::split(Vertical, Ratio(0.3), r2, r3))
    }

    /// Terminal: tree | home over rain + lyrics log | scope, top, fft.
    pub fn terminal() -> Node {
        use Axis::*;
        use Size::*;
        let mid = Node::split(Vertical, Ratio(0.58), Node::slot("home"), Node::split(Horizontal, Ratio(0.5), Node::slot("rain"), Node::slot("lyrics")));
        let right = Node::split(Vertical, Ratio(0.35), Node::slot("scope"), Node::split(Vertical, Ratio(0.45), Node::slot("top"), Node::slot("fft")));
        Node::split(Horizontal, Cells(30), Node::slot("library"), Node::split(Horizontal, Last(36), mid, right))
    }

    pub fn compute(&self, area: Rect) -> Computed {
        let mut out = Computed::default();
        self.compute_into(area, &mut Vec::new(), &mut out);
        out
    }

    fn compute_into(&self, area: Rect, path: &mut Vec<u8>, out: &mut Computed) {
        match self {
            Node::Slot(name) => out.slots.push((name.clone(), area)),
            Node::Split { axis, first, a, b } => {
                let total = match axis {
                    Axis::Horizontal => area.width,
                    Axis::Vertical => area.height,
                };
                let min = match axis {
                    Axis::Horizontal => MIN_W,
                    Axis::Vertical => MIN_H,
                };
                let want = match first {
                    Size::Cells(c) => *c,
                    Size::Ratio(r) => (total as f32 * r.clamp(0.05, 0.95)).round() as u16,
                    Size::Last(c) => total.saturating_sub(*c),
                };
                // keep both children at least `min` when there is room; otherwise split evenly
                let n = if total >= min * 2 { want.clamp(min, total - min) } else { total / 2 };
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
                a.compute_into(ra, path, out);
                path.pop();
                path.push(1);
                b.compute_into(rb, path, out);
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
        let (start, total, min) = match d.axis {
            Axis::Horizontal => (d.area.x, d.area.width, MIN_W),
            Axis::Vertical => (d.area.y, d.area.height, MIN_H),
        };
        if total < min * 2 {
            return;
        }
        let n = pos.saturating_sub(start).clamp(min, total - min);
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
