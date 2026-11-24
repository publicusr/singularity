//! Arrangement (D87, ADR 0005): the floor plan (where the Panes' outlines are), which Widget is in which Pane, and the
//! Tray (the Widgets set aside, in order). Everything that changes any of them goes through its edits, and each edit
//! keeps its promises or is refused, saying why: the floor plan stays valid (every Pane at least the Pane minimum,
//! `MIN_W` × `MIN_H`), each Widget is in at most one place (a Pane or the Tray), and a Required Widget (Home) is never
//! lost: it can wait in the Tray, but nothing removes it from there. Neither asks what a Pane holds (#48: the gravity well letterboxes its picture in whatever Pane it gets).
//!
//! The two jobs are kept apart: moving contents (swap two Panes' Widgets, fill an empty Pane, replace a Pane's Widget)
//! never touches the floor plan, so every such edit is allowed whatever the sizes (a Widget in a Pane below its own
//! smallest size says so itself); the floor plan changes only by its own edits (move a line, split a Pane, close one).
//! Pane identities stay where they are: contents move, Panes don't.
//!
//! Edits are values: each returns the new Arrangement and leaves this one as it was, so undo can keep the old one.
//! Reading goes through `Deref` to the floor plan (`Node`): where a Pane is, what it holds; writing never does.

use crate::layout::{Divider, Node, PaneId, Widget, is_hole};
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

/// The floor plan, its contents and the Tray.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrangement {
    plan: Node,
    /// the Widgets set aside, in the order they came; none of them is on screen
    tray: Vec<Widget>,
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
    /// an empty Pane: nothing to set aside
    Empty,
    /// a Widget that isn't in the Tray
    NotInTray(Widget),
    /// a second black hole on screen (this one is there already)
    OneHole(Widget),
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
            Refused::Empty => "That Pane is empty".into(),
            Refused::NotInTray(w) => format!("{} isn't in the Tray", name(w)),
            Refused::OneHole(w) => format!("One black hole at a time: {} is on screen", name(w)),
        }
    }
}

/// A floor plan with nothing set aside (a built-in Design, a test's tree). A saved Design brings its Tray along:
/// `Arrangement::new`.
impl From<Node> for Arrangement {
    fn from(plan: Node) -> Arrangement {
        Arrangement { plan, tray: vec![] }
    }
}

impl From<&Node> for Arrangement {
    fn from(plan: &Node) -> Arrangement {
        plan.clone().into()
    }
}

impl From<&Arrangement> for Arrangement {
    fn from(a: &Arrangement) -> Arrangement {
        a.clone()
    }
}

impl std::ops::Deref for Arrangement {
    type Target = Node;
    fn deref(&self) -> &Node {
        &self.plan
    }
}

/// Exactly that floor plan, with nothing in the Tray.
impl PartialEq<Node> for Arrangement {
    fn eq(&self, other: &Node) -> bool {
        self.plan == *other && self.tray.is_empty()
    }
}

impl PartialEq<Arrangement> for Node {
    fn eq(&self, other: &Arrangement) -> bool {
        other == self
    }
}

impl Arrangement {
    /// `plan` with `tray` set aside, as a file said it: a Widget the Tray names twice, or that a Pane shows already,
    /// is kept once (where a Pane shows it).
    pub fn new(plan: Node, tray: Vec<Widget>) -> Arrangement {
        let mut kept: Vec<Widget> = vec![];
        for w in tray {
            if !plan.has(w) && !kept.contains(&w) {
                kept.push(w);
            }
        }
        // a Required Widget the file left out altogether waits in the Tray (Done waits for it, and Home is never lost)
        for r in REQUIRED {
            if !plan.has(r) && !kept.contains(&r) {
                kept.push(r);
            }
        }
        Arrangement { plan, tray: kept }
    }

    /// The Widgets in the Tray, in order.
    pub fn tray(&self) -> &[Widget] {
        &self.tray
    }

    /// The first Required Widget without a Pane (waiting in the Tray), if any: Done waits for it.
    pub fn unplaced(&self) -> Option<Widget> {
        REQUIRED.into_iter().find(|w| !self.plan.has(*w))
    }

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
        Ok(Arrangement { plan, tray: self.tray.clone() })
    }

    /// Widget `w` (not on screen; from the Tray, if it waits there) into empty Pane `pane`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn fill(&self, pane: PaneId, w: Widget) -> Result<Arrangement, Refused> {
        match self.plan.get(pane) {
            None => Err(Refused::NoPane),
            Some(p) if p.widget.is_some() => Err(Refused::NotEmpty(p.widget.unwrap_or(w))),
            Some(_) => self.put_new(pane, w),
        }
    }

    /// Pane `pane` shows Widget `w` (not on screen; from the Tray, if it waits there) instead of what it showed, which
    /// leaves the screen (not into the Tray: the Widget picker still offers it). A Required Widget (Home) is never
    /// replaced; an empty Pane is filled.
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
        Ok(Arrangement { plan, tray: self.without_tray(w) })
    }

    /// The Tray without `w`.
    fn without_tray(&self, w: Widget) -> Vec<Widget> {
        self.tray.iter().copied().filter(|t| *t != w).collect()
    }

    // ── the Tray ──────────────────────────────────────────────────────

    /// Pane `pane`'s Widget into the Tray (at its end); the Pane stays, empty. Home too: it waits there until it has a
    /// Pane again (Done waits for it).
    pub fn to_tray(&self, pane: PaneId) -> Result<Arrangement, Refused> {
        let w = self.plan.get(pane).ok_or(Refused::NoPane)?.widget.ok_or(Refused::Empty)?;
        let mut plan = self.plan.clone();
        plan.put(pane, None);
        let mut tray = self.tray.clone();
        tray.push(w);
        Ok(Arrangement { plan, tray })
    }

    /// Widget `w` from the Tray into Pane `pane`: an empty Pane is filled; an occupied one's Widget goes into the Tray
    /// in `w`'s place (nothing is lost in a swap).
    pub fn out_of_tray(&self, w: Widget, pane: PaneId) -> Result<Arrangement, Refused> {
        let here = self.plan.get(pane).ok_or(Refused::NoPane)?.widget;
        let at = self.tray.iter().position(|t| *t == w).ok_or(Refused::NotInTray(w))?;
        // one black hole at a time (they share the hole's state): only onto the Pane holding the other
        if let Some(other) = self.plan.widgets().into_iter().find(|o| is_hole(Some(*o)) && is_hole(Some(w)) && here != Some(*o)) {
            return Err(Refused::OneHole(other));
        }
        let mut plan = self.plan.clone();
        plan.put(pane, Some(w));
        let mut tray = self.tray.clone();
        match here {
            Some(h) => tray[at] = h,
            None => {
                tray.remove(at);
            }
        }
        Ok(Arrangement { plan, tray })
    }

    /// Without `w` in the Tray (its `×`): it is gone, until the Widget picker brings it back. Home is never removed.
    pub fn remove_from_tray(&self, w: Widget) -> Result<Arrangement, Refused> {
        if !self.tray.contains(&w) {
            return Err(Refused::NotInTray(w));
        }
        if required(Some(w)) {
            return Err(Refused::Required(w));
        }
        Ok(Arrangement { plan: self.plan.clone(), tray: self.without_tray(w) })
    }

    /// Clear Tray: everything in it is removed but the Required Widgets (Home stays).
    pub fn clear_tray(&self) -> Arrangement {
        Arrangement { plan: self.plan.clone(), tray: self.tray.iter().copied().filter(|w| required(Some(*w))).collect() }
    }

    // ── the floor plan ────────────────────────────────────────────────

    /// Line `d` moved to `pos`, or as far toward it as it goes: the furthest cell where the floor plan is still valid,
    /// so a fast drag past the limit stops at the limit, not short of it (#21). Never refused: at worst the line stays.
    pub fn move_line(&self, d: &Divider, pos: u16, area: Rect) -> Arrangement {
        let ok = |a: &Arrangement| a.fits(area);
        let to = |p: u16| {
            let mut plan = self.plan.clone();
            plan.drag(d, p);
            Arrangement { plan, tray: self.tray.clone() }
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
    /// is), holding `w` (one not on screen; from the Tray, if it waits there) or nothing.
    pub fn split(&self, pane: PaneId, side: Side, w: Option<Widget>, area: Rect) -> Result<Arrangement, Refused> {
        if !self.plan.contains(pane) {
            return Err(Refused::NoPane);
        }
        if let Some(w) = w.filter(|w| self.plan.has(*w)) {
            return Err(Refused::OnScreen(w));
        }
        let plan = self.plan.split_beside(pane, w, side).ok_or(Refused::NoPane)?;
        let mut a = self.checked(plan, area)?;
        if let Some(w) = w {
            a.tray = self.without_tray(w);
        }
        // only the Pane split is divided: a split that could only happen by squeezing (or growing) another Pane is
        // refused, as one without room (D87: the floor plan holds still)
        let after = a.plan.compute(area).panes;
        let moved = self.plan.compute(area).panes.iter().filter(|(p, _)| p.id != pane).any(|(p, r)| !after.iter().any(|(q, s)| q.id == p.id && s == r));
        if moved { Err(Refused::NoRoom) } else { Ok(a) }
    }

    /// Without Pane `pane` (its neighbour takes the room); its Widget goes into the Tray (closing never destroys
    /// anything). Refused for a Required Widget's Pane (Home's).
    pub fn close(&self, pane: PaneId, area: Rect) -> Result<Arrangement, Refused> {
        let w = match self.plan.get(pane) {
            None => return Err(Refused::NoPane),
            Some(p) => {
                if let Some(w) = p.widget.filter(|w| required(Some(*w))) {
                    return Err(Refused::Required(w));
                }
                p.widget
            }
        };
        let plan = self.plan.without(pane).ok_or(Refused::LastPane)?;
        let mut a = self.checked(plan, area)?;
        a.tray.extend(w);
        Ok(a)
    }

    /// A new floor plan (the Tray as it is), if it keeps the promise: valid at `area`.
    fn checked(&self, plan: Node, area: Rect) -> Result<Arrangement, Refused> {
        let a = Arrangement { plan, tray: self.tray.clone() };
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
