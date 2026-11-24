//! Arrangement (D87, ADR 0005): the floor plan (where the Panes' outlines are) and which Widget is in which Pane.
//! Everything that changes either goes through its edits, and each edit keeps two promises or is refused, saying why:
//! the floor plan stays valid (every Pane at least the Pane minimum, `MIN_W` × `MIN_H`) and each Widget is on screen at
//! most once. Neither asks what a Pane holds (#48: the gravity well letterboxes its picture in whatever Pane it gets).
//!
//! The two jobs are kept apart: moving contents (swap two Panes' Widgets, fill an empty Pane, replace a Pane's Widget)
//! never touches the floor plan, so every such edit is allowed whatever the sizes (a Widget in a Pane below its own
//! smallest size says so itself); the floor plan changes only by its own edits (move a line, split a Pane, close one).
//! Pane identities stay where they are: contents move, Panes don't.
//!
//! Edits are values: each returns the new Arrangement and leaves this one as it was, so undo can keep the old one.
//! Reading goes through `Deref` to the floor plan (`Node`): where a Pane is, what it holds; writing never does.

use crate::layout::{Divider, Node, PaneId, Widget};
use crate::tiling::Side;
use ratatui::layout::Rect;

/// The Required Widgets: moved, never closed or replaced away (D87: only Home, the main Pane where every page opens).
/// The built-in Designs also have the library, the queue and lyrics (D43), but an edit forks one into a Design of your
/// own, which may let them go: without a Pane, the library and the queue open as popups from the palette.
pub const REQUIRED: [Widget; 1] = [Widget::Home];

/// Whether `w` is a Required Widget.
pub fn required(w: Option<Widget>) -> bool {
    w.is_some_and(|w| REQUIRED.contains(&w))
}

/// The floor plan and its contents. (The next step, #41, adds the Tray: the Widgets set aside.)
#[derive(Clone, Debug, PartialEq)]
pub struct Arrangement {
    plan: Node,
}

/// Why an edit can't happen. `App` words it with the Design's own titles (`why`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// a Pane that isn't in this floor plan
    NoPane,
    /// a Pane onto itself
    SamePane,
    /// two empty Panes: nothing would move
    NothingToMove,
    /// filling a Pane that holds this Widget already
    #[cfg_attr(not(test), allow(dead_code))]
    NotEmpty(Widget),
    /// a Widget already on screen (each is there at most once)
    OnScreen(Widget),
    /// a Required Widget (it can be moved, never removed)
    Required(Widget),
    /// the only Pane left
    LastPane,
    /// the floor plan this would make leaves a Pane under the minimum
    NoRoom,
}

impl Refused {
    /// The reason, in a line for the bar; `name` gives a Widget's title.
    pub fn why(self, name: impl Fn(Widget) -> String) -> String {
        match self {
            Refused::NoPane => "That Pane is gone".into(),
            Refused::SamePane => "That's where it is".into(),
            Refused::NothingToMove => "Both Panes are empty".into(),
            Refused::NotEmpty(w) => format!("{} is there already", name(w)),
            Refused::OnScreen(w) => format!("{} is on screen already", name(w)),
            Refused::Required(w) => format!("{} can be moved, but never removed", name(w)),
            Refused::LastPane => "It's the only Pane".into(),
            Refused::NoRoom => "No room for that".into(),
        }
    }
}

impl From<Node> for Arrangement {
    fn from(plan: Node) -> Arrangement {
        Arrangement { plan }
    }
}

impl std::ops::Deref for Arrangement {
    type Target = Node;
    fn deref(&self) -> &Node {
        &self.plan
    }
}

impl PartialEq<Node> for Arrangement {
    fn eq(&self, other: &Node) -> bool {
        self.plan == *other
    }
}

impl PartialEq<Arrangement> for Node {
    fn eq(&self, other: &Arrangement) -> bool {
        *self == other.plan
    }
}

impl Arrangement {
    /// The floor plan, as saved.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn plan(&self) -> &Node {
        &self.plan
    }

    /// Whether the floor plan is valid at `area`: Panes with identities of their own, each Widget at most once, sizes
    /// that mean something, and every Pane at least the Pane minimum where it is drawn, whatever it holds.
    pub fn fits(&self, area: Rect) -> bool {
        self.plan.valid(area)
    }

    // ── contents: never the floor plan ────────────────────────────────

    /// Panes `a` and `b` trade Widgets (an empty one takes the other's and leaves it empty). The Panes stay where they
    /// are. Allowed whatever their sizes.
    pub fn swap(&self, a: PaneId, b: PaneId) -> Result<Arrangement, Refused> {
        let (Some(pa), Some(pb)) = (self.plan.get(a), self.plan.get(b)) else { return Err(Refused::NoPane) };
        if a == b {
            return Err(Refused::SamePane);
        }
        if pa.widget.is_none() && pb.widget.is_none() {
            return Err(Refused::NothingToMove);
        }
        let mut plan = self.plan.clone();
        plan.put(a, pb.widget);
        plan.put(b, pa.widget);
        Ok(Arrangement { plan })
    }

    /// Widget `w` (not on screen) into empty Pane `pane`. (The Widget picker and the Tray bring Widgets in: #37, #41.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn fill(&self, pane: PaneId, w: Widget) -> Result<Arrangement, Refused> {
        match self.plan.get(pane) {
            None => Err(Refused::NoPane),
            Some(p) if p.widget.is_some() => Err(Refused::NotEmpty(p.widget.unwrap_or(w))),
            Some(_) => self.put_new(pane, w),
        }
    }

    /// Pane `pane` shows Widget `w` (not on screen) instead of what it showed, which leaves the screen. A Required
    /// Widget (Home) is never replaced; an empty Pane is filled.
    pub fn replace(&self, pane: PaneId, w: Widget) -> Result<Arrangement, Refused> {
        match self.plan.get(pane).map(|p| p.widget) {
            None => Err(Refused::NoPane),
            Some(Some(r)) if required(Some(r)) => Err(Refused::Required(r)),
            Some(_) => self.put_new(pane, w),
        }
    }

    fn put_new(&self, pane: PaneId, w: Widget) -> Result<Arrangement, Refused> {
        if self.plan.has(w) {
            return Err(Refused::OnScreen(w));
        }
        let mut plan = self.plan.clone();
        plan.put(pane, Some(w));
        Ok(Arrangement { plan })
    }

    // ── the floor plan ────────────────────────────────────────────────

    /// Line `d` moved to `pos`, or as far toward it as it goes: the furthest cell where the floor plan is still valid,
    /// so a fast drag past the limit stops at the limit, not short of it (#21). Never refused: at worst the line stays.
    pub fn move_line(&self, d: &Divider, pos: u16, area: Rect) -> Arrangement {
        let ok = |a: &Arrangement| a.fits(area);
        let to = |p: u16| {
            let mut plan = self.plan.clone();
            plan.drag(d, p);
            Arrangement { plan }
        };
        let t = to(pos);
        if ok(&t) || !self.fits(area) {
            return t;
        }
        // from where the line is now (it fits), the furthest cell toward `pos` that still does (found by halving)
        let Some(mut good) = self.plan.boundary(d).filter(|&b| ok(&to(b))) else { return self.clone() };
        let mut bad = pos;
        while good.abs_diff(bad) > 1 {
            let mid = ((good as u32 + bad as u32) / 2) as u16;
            if ok(&to(mid)) { good = mid } else { bad = mid }
        }
        to(good)
    }

    /// Pane `pane`'s line a step further out (`grow`) or in: 2 columns or 1 row, the nearest line around it that can
    /// move and still leaves the floor plan valid.
    pub fn step_line(&self, pane: PaneId, grow: bool, area: Rect) -> Result<Arrangement, Refused> {
        if !self.plan.contains(pane) {
            return Err(Refused::NoPane);
        }
        let plan = self.plan.resized(pane, grow, area).ok_or(Refused::NoRoom)?;
        self.checked(plan, area)
    }

    /// A new Pane on `side` of Pane `pane`, taking half its room and nothing else (every other Pane stays exactly where it
    /// is), holding `w` (one not on screen) or nothing.
    pub fn split(&self, pane: PaneId, side: Side, w: Option<Widget>, area: Rect) -> Result<Arrangement, Refused> {
        if !self.plan.contains(pane) {
            return Err(Refused::NoPane);
        }
        if let Some(w) = w.filter(|w| self.plan.has(*w)) {
            return Err(Refused::OnScreen(w));
        }
        let plan = self.plan.split_beside(pane, w, side).ok_or(Refused::NoPane)?;
        let a = self.checked(plan, area)?;
        // only the Pane split is divided: a split that could only happen by squeezing (or growing) another Pane is
        // refused, as one without room (D87: the floor plan holds still)
        let after = a.plan.compute(area).panes;
        let moved = self.plan.compute(area).panes.iter().filter(|(p, _)| p.id != pane).any(|(p, r)| !after.iter().any(|(q, s)| q.id == p.id && s == r));
        if moved { Err(Refused::NoRoom) } else { Ok(a) }
    }

    /// Without Pane `pane` (its neighbour takes the room); refused for a Required Widget (Home).
    pub fn close(&self, pane: PaneId, area: Rect) -> Result<Arrangement, Refused> {
        match self.plan.get(pane) {
            None => return Err(Refused::NoPane),
            Some(p) => {
                if let Some(w) = p.widget.filter(|w| required(Some(*w))) {
                    return Err(Refused::Required(w));
                }
            }
        }
        let plan = self.plan.without(pane).ok_or(Refused::LastPane)?;
        self.checked(plan, area)
    }

    /// A new floor plan, if it keeps the promise: valid at `area`.
    fn checked(&self, plan: Node, area: Rect) -> Result<Arrangement, Refused> {
        let a = Arrangement { plan };
        if a.fits(area) { Ok(a) } else { Err(Refused::NoRoom) }
    }
}

/// The floor plan alone, without what's in it: what a contents edit must leave exactly as it was.
#[cfg_attr(not(test), allow(dead_code))]
pub fn outline(n: &Node) -> Node {
    match n {
        Node::Pane(p) => Node::Pane(crate::layout::Pane { id: p.id, widget: None }),
        Node::Split { axis, first, a, b } => Node::Split { axis: *axis, first: *first, a: Box::new(outline(a)), b: Box::new(outline(b)) },
    }
}

#[cfg(test)]
mod tests;
