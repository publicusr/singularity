//! The Screen plan (D89, #103): where everything is on screen, worked out from the App's state and the window's size
//! before anything is drawn. The screen's rows, each Pane's rectangle and its title (the text as it fits, and the boxes
//! of its buttons: the Widget's name, ◂ ▸ or ← →, ×, ⊞), the dividers, the Panes too small for their Widget, the Tray's
//! parts in Arrange mode, and a held Widget's highlight, label and chip.
//!
//! Drawing paints it; clicks, the wheel, the Pane menu and Arrange mode look things up in it, so none of them waits for
//! a frame to be drawn (`App::plan` works it out from the state as it is now). What the Widgets draw inside their Panes
//! (rows, cards, buttons) is still registered by their drawing (`App::hit`), under the Panes' own chrome.
//!
//! Everything a frame changes in the App happens before drawing, in `App::frame_begins` (a Widget picker whose Pane
//! went closes, Home without a Pane brings up Arrange mode, a held Widget is let go or found again under the pointer,
//! the label's row and the Tray's first Widget are remembered); `ui::draw` then only paints the plan it is given.

use crate::app::{App, Hit, Modal, Page};
use crate::arrange::TrayLayout;
use crate::arrangement::Arrangement;
use crate::layout::{Divider, Pane, PaneId, Widget};
use ratatui::layout::Rect;

/// The screen's rows: the top bar, the Panes, the player bar, the Tray (Arrange mode only; otherwise no rows) and the
/// hints (Arrange mode: its legend and Done).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rows {
    pub top: Rect,
    pub body: Rect,
    pub player: Rect,
    pub tray: Rect,
    pub hints: Rect,
}

/// A Pane's title: its text as it fits, and where its parts are (each a box a click finds).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Title {
    /// before the title: a space and, in a numbered Theme, the Pane's number (" ¹"); empty for an empty Pane
    pub lead: String,
    /// the title as it fits, cut with "…", and the space after it; empty for an empty Pane (no title at all)
    pub text: String,
    /// drawn lit: the Pane is focused or its Widget is held
    pub lit: bool,
    /// the title bar: a press here moves the Pane's Widget (and Arrange mode's drag starts here)
    pub bar: Rect,
    /// the Widget's name (a click opens the Widget picker), or top this week's switch between artists and tracks
    pub name: Option<(Rect, Hit)>,
    /// the main Pane's ← → (5 cells: " ← → "), while there are pages to go back or forward to
    pub history: Option<Rect>,
    /// ◂ ▸ (5 cells: " ◂ ▸ "): the Widgets this one can give way to
    pub swap: Option<Rect>,
    /// × (3 cells)
    pub close: Option<Rect>,
    /// Arrange mode's ⊞ (3 cells)
    pub split: Option<Rect>,
}

impl Title {
    /// Each part with the Hit a click there is, in the order they win from last to first (a button over the name
    /// over the bar).
    pub fn hits(&self, id: PaneId) -> Vec<(Rect, Hit)> {
        let mut v = vec![(self.bar, Hit::Title(id))];
        v.extend(self.name.clone());
        if let Some(r) = self.history {
            v.push((Rect::new(r.x, r.y, 2, 1), Hit::History(-1)));
            v.push((Rect::new(r.x + 2, r.y, 3, 1), Hit::History(1)));
        }
        if let Some(r) = self.swap {
            v.push((Rect::new(r.x, r.y, 2, 1), Hit::Swap(id, -1)));
            v.push((Rect::new(r.x + 2, r.y, 3, 1), Hit::Swap(id, 1)));
        }
        v.extend(self.close.map(|r| (r, Hit::Close(id))));
        v.extend(self.split.map(|r| (r, Hit::SplitPane(id))));
        v
    }
}

/// One Pane on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct PanePlan {
    pub pane: Pane,
    /// its place among the Panes (its number in a numbered Theme, its border's colour)
    pub index: usize,
    pub rect: Rect,
    pub focused: bool,
    /// its Widget is being moved (by the mouse or picked up with Enter)
    pub held: bool,
    /// smaller than its Widget is usable in: a frame round `needs W × H` (#36); it takes no wheel, keys or clicks for
    /// the Widget it hides
    pub short: bool,
    /// smaller than any Pane: just its frame
    pub bare: bool,
    /// its Widget draws its own frame (the well), instead of the plain one
    pub own_frame: bool,
    pub title: Title,
}

/// A Widget held by its title (or picked up with Enter): the Pane a release would drop on, with what its label says
/// and where the label goes, and the chip by the pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub target: Option<(PaneId, Rect, String)>,
    pub label: Option<Rect>,
    pub chip: Option<(Rect, String)>,
}

/// Where everything is on screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScreenPlan {
    pub window: Rect,
    /// too small for any layout: just a player (#47), and no Panes
    pub mini: bool,
    pub rows: Rows,
    pub panes: Vec<PanePlan>,
    pub dividers: Vec<Divider>,
    /// Arrange mode's Tray strip, where it is drawn
    pub tray: Option<TrayLayout>,
    pub drag: Option<Drag>,
}

/// What the Panes' part of the plan is worked out from: the App's state (`App::plan`), or a test's plain values.
pub struct Facts<'a> {
    pub window: Rect,
    pub arranging: bool,
    pub layout: &'a Arrangement,
    pub focus: Option<PaneId>,
    /// the Pane whose Widget is being moved
    pub moving: Option<PaneId>,
    /// the built-in Design shown (its Widgets' names)
    pub design: usize,
    /// the Theme numbers its Panes
    pub numbered: bool,
    /// the main Pane's title, after the page it shows (None: its Widget's own)
    pub main_title: Option<String>,
    pub top_week_tracks: bool,
    /// there are pages to go back or forward to (the main Pane's ← →)
    pub history: bool,
}

/// The screen's rows in `area`, arranging or not (the Panes give the Tray its rows, Q39).
pub fn rows(area: Rect, arranging: bool) -> Rows {
    let [top, body, player, tray, hints] = crate::ui::rows(area, arranging);
    Rows { top, body, player, tray, hints }
}

/// Whether the Panes are drawn in `window`: it has been sized, and isn't so small it is just a player.
pub fn has_panes(window: Rect) -> bool {
    window.width > 0 && !crate::mini::wanted(window)
}

/// Whether the Pane holding `widget`, one of `panes` on screen, can be closed (by its ×, the Pane menu's "Close pane",
/// Arrange mode's `x`): not the Required Widget's, and not the only Pane (owner, Q105).
pub fn closable(widget: Option<Widget>, panes: usize) -> bool {
    !crate::arrangement::required(widget) && panes > 1
}

/// Whether a Pane gets a ×: it can be closed (`closable`), and its title has the room. Without the room, the Pane
/// menu still offers "Close pane" (Q105).
pub fn close_button(widget: Option<Widget>, panes: usize, width: u16) -> bool {
    closable(widget, panes) && width >= 14
}

/// The rows, Panes and dividers for `f` (no Tray and no drag: those are Arrange mode's, `App::plan`).
pub fn plan(f: &Facts) -> ScreenPlan {
    let mini = crate::mini::wanted(f.window);
    let rows = rows(f.window, f.arranging);
    let mut plan = ScreenPlan { window: f.window, mini, rows, ..Default::default() };
    if !has_panes(f.window) {
        return plan;
    }
    let computed = f.layout.compute(rows.body);
    let n = computed.panes.len();
    plan.panes = computed.panes.iter().enumerate().map(|(i, (p, r))| pane(f, i, n, *p, *r)).collect();
    plan.dividers = computed.dividers;
    plan
}

/// Superscript digits for numbered Pane titles (Cockpit).
fn superscript(n: usize) -> String {
    n.to_string().chars().map(|c| ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'][c as usize - '0' as usize]).collect()
}

/// Pane `p` at `rect`, the `i`th of `total`.
fn pane(f: &Facts, i: usize, total: usize, p: Pane, rect: Rect) -> PanePlan {
    let w = p.widget;
    // (an empty Pane has no title yet)
    let title = w.map_or("", |w| crate::design::title(f.design, w, crate::widget_catalogue::facts(w).title));
    let title = match w {
        // the main Pane is titled after what it shows
        Some(Widget::Home) => f.main_title.clone().unwrap_or_else(|| title.to_string()),
        // its title is the switch between artists and tracks
        Some(Widget::TopWeek) if f.top_week_tracks => "top tracks · week ⇄".to_string(),
        Some(Widget::TopWeek) => "top artists · week ⇄".to_string(),
        _ => title.to_string(),
    };
    let number = if f.numbered { superscript(i + 1) } else { String::new() };
    let focused = f.focus == Some(p.id);
    let held = f.moving == Some(p.id);
    // a Pane that can close has a × at the right end of its title (Home's, the Required Widget's, doesn't)
    let closable = close_button(w, total, rect.width);
    let tail: u16 = if closable { 3 } else { 0 };
    let lead = format!(" {number}");
    // the main Pane: ← → through the pages seen, while there are any, at the right end of its title like every
    // Pane's arrows (D44); dim when there's nowhere to go that way
    let history = w == Some(Widget::Home) && f.history && rect.width > 20 && !f.arranging;
    // a Pane whose Widget can give way to others always keeps its ◂ ▸ (7 cells at the right): a long title gives way
    let can_swap = w.is_some_and(|w| crate::widget_catalogue::facts(w).group.is_some()) || history;
    // Arrange mode (D87): a split button before the ×, and no ◂ ▸ (what a Pane shows changes by dragging there)
    let split_btn = f.arranging && rect.width >= 14;
    let can_swap = can_swap && !split_btn;
    // the ◂ ▸ keep their place whether or not the Widget shown can close: cycling onto the queue (no ×) moved them
    // three cells, and the next click on ▸ landed on ◂, so the Pane went back and forth between two Widgets
    let tail: u16 = (if can_swap && rect.width >= 14 { 3 } else { tail }) + if split_btn { 3 } else { 0 };
    let used = lead.chars().count();
    let room = (rect.width as usize).saturating_sub(2 + used + if can_swap { 8 } else { 0 } + tail as usize);
    // the cells before the ◂ ▸ (their leading space ends the title): in a Pane too narrow for `room`, the title used
    // to run on under the arrows and its last letter showed between them and the corner ("lis ◂ ▸ g┐")
    let before_arrows = rect.width as i32 - 2 - used as i32 - 6 - tail as i32;
    let title = if title.chars().count() < room {
        title
    } else if room > 1 {
        title.chars().take(room - 2).collect::<String>() + "…"
    } else if can_swap {
        let free = before_arrows.max(0) as usize;
        if title.chars().count() <= free { title } else { title.chars().take(free.saturating_sub(1)).collect::<String>() + if free > 0 { "…" } else { "" } }
    } else {
        title
    };
    let (lead, text) = match w {
        Some(_) => (lead, format!("{title} ")),
        // an empty Pane has no title at all (not even the spaces round one)
        None => (String::new(), String::new()),
    };
    // smaller than its Widget is usable in: a plain frame round one dim line, `needs W × H` (#36)
    let short = w.is_some_and(|w| crate::needs::too_small(w, rect));
    // its title bar: drag it to move the Pane (it wins over a divider's grip on the row below a horizontal divider;
    // the buttons win over it in turn)
    let bar = Rect::new(rect.x + 1, rect.y, rect.width.saturating_sub(2), 1);
    let name = if w == Some(Widget::TopWeek) && !short {
        let tw = (lead.chars().count() + text.chars().count()) as u16;
        Some((Rect::new(rect.x + 1, rect.y, tw.min(rect.width.saturating_sub(2)), 1), Hit::TopWeekToggle))
    } else if w.is_some() && !crate::arrangement::required(w) {
        // the Widget's name: a click opens the Widget picker (top this week's title is its switch, and Home's is the
        // page's)
        let x = rect.x + 1 + lead.chars().count() as u16;
        let nw = (text.chars().count() as u16).saturating_sub(1).min(rect.right().saturating_sub(x + 1));
        Some((Rect::new(x, rect.y, nw, 1), Hit::TitleName(p.id)))
    } else {
        None
    };
    let bare = rect.width < crate::layout::MIN_W || rect.height < crate::layout::MIN_H;
    let own_frame = w.is_some_and(|w| crate::widget_catalogue::facts(w).own_frame.is_some()) && !short;
    let mut t = Title { lead, text, lit: focused || held, bar, name, ..Default::default() };
    // smaller than any Pane: just its frame, no buttons
    if !bare {
        // (the well draws its own frame: no arrows on it)
        if !own_frame {
            if history {
                t.history = Some(Rect::new(rect.right().saturating_sub(7), rect.y, 5, 1));
            } else if can_swap && rect.width >= 12 {
                // a Pane whose Widget can give way to others says so: ◂ ▸ in its title swap them (so does v)
                t.swap = Some(Rect::new(rect.right().saturating_sub(7 + tail), rect.y, 5, 1));
            }
        }
        t.close = closable.then(|| Rect::new(rect.right().saturating_sub(5), rect.y, 3, 1));
        t.split = split_btn.then(|| Rect::new(rect.right().saturating_sub(if closable { 8 } else { 5 }), rect.y, 3, 1));
    }
    PanePlan { pane: p, index: i, rect, focused, held, short, bare, own_frame, title: t }
}

fn inside(r: &Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
}

impl ScreenPlan {
    /// Every Pane's title parts, in the order they win from last to first.
    pub fn chrome(&self) -> Vec<(Rect, Hit)> {
        self.panes.iter().flat_map(|p| p.title.hits(p.pane.id)).collect()
    }

    /// The title part (of any Pane) at (x, y).
    pub fn title_hit(&self, x: u16, y: u16) -> Option<Hit> {
        self.chrome().into_iter().rev().find(|(r, _)| inside(r, x, y)).map(|(_, h)| h)
    }

    /// The divider whose grip is at (x, y).
    pub fn divider_at(&self, x: u16, y: u16) -> Option<usize> {
        self.dividers.iter().rposition(|d| inside(&d.grip, x, y))
    }

    /// The Pane at (x, y).
    pub fn pane_at(&self, x: u16, y: u16) -> Option<PaneId> {
        self.panes.iter().rev().find(|p| inside(&p.rect, x, y)).map(|p| p.pane.id)
    }

    /// Pane `id`, if it is on screen.
    pub fn pane(&self, id: PaneId) -> Option<&PanePlan> {
        self.panes.iter().find(|p| p.pane.id == id)
    }

    /// Whether Pane `id` is on screen too small for its Widget.
    pub fn short(&self, id: PaneId) -> bool {
        self.pane(id).is_some_and(|p| p.short)
    }

    /// Where `h` is, if it is one of the plan's own (a Pane, a title part, a divider's grip).
    pub fn rect_of(&self, h: &Hit) -> Option<Rect> {
        match h {
            Hit::Focus(p) => self.pane(*p).map(|p| p.rect),
            Hit::Divider(i) => self.dividers.get(*i).map(|d| d.grip),
            _ => self.chrome().into_iter().find(|(_, x)| x == h).map(|(r, _)| r),
        }
    }
}

impl App {
    /// What the Panes' plan is worked out from, as the App is now.
    fn plan_facts(&self) -> Facts<'_> {
        let main_title = match &self.page {
            Page::Search => Some("search".to_string()),
            Page::Stats(_) => Some("stats".to_string()),
            Page::Settings(_) => Some("settings".to_string()),
            Page::Artist(a) => Some(a.name.clone()),
            Page::List { .. } => self.list_view().map(|v| v.title.clone()),
            Page::Home => None,
        };
        Facts {
            window: self.window,
            arranging: self.arrange.is_on(),
            layout: &self.layout,
            focus: self.focus,
            moving: self.moving(),
            design: self.shown.design(),
            numbered: self.theme.numbered,
            main_title,
            top_week_tracks: self.top_week_tracks,
            history: !self.back.is_empty() || !self.forward.is_empty(),
        }
    }

    /// Where everything is on screen, as the App is now (before the first frame or a resize, a window of no size: no
    /// Panes).
    pub fn plan(&self) -> ScreenPlan {
        let mut plan = self.panes_plan();
        if !has_panes(self.window) {
            return plan;
        }
        let r = plan.rows.tray;
        if self.arrange.is_on() && r.height >= 3 && r.width >= 20 {
            let design = self.shown.design();
            let first = self.arrange.tray_first().unwrap_or(0);
            plan.tray = Some(crate::arrange::tray_layout(self.layout.tray(), design, crate::arrange::Arrange::tray_reserve(&self.gesture, design), r, first));
        }
        // a held Widget: the Pane under the pointer is highlighted with a label, a chip follows the pointer. Neither
        // changes how anything under it is drawn (no step down a tier: that flickered): images are placed around them
        if let Some((target, chip_text)) = self.drag_marks() {
            let target = target.and_then(|(id, label)| Some((id, plan.pane(id)?.rect, label)));
            let chip = self.chip_rect(plan.window, &chip_text).map(|r| (r, chip_text));
            let label = target.as_ref().and_then(|(id, r, l)| crate::arrange::label_rect(*r, l, chip.as_ref().map(|c| c.0), crate::arrange::Arrange::label_row(&self.gesture, *id)));
            plan.drag = Some(Drag { target, label, chip });
        }
        plan
    }

    /// The plan's rows, Panes and dividers, without the Tray and the drag (which Arrange mode works out from the
    /// dividers: `App::plan` would ask itself).
    fn panes_plan(&self) -> ScreenPlan {
        plan(&self.plan_facts())
    }

    /// The dividers between the Panes, where the plan has them now (Arrange mode's Scene, the pointer's shape).
    pub fn dividers(&self) -> Vec<Divider> {
        self.panes_plan().dividers
    }

    /// Whether Pane `p` is on screen too small for its Widget (the plan's): it takes no wheel, keys or clicks for it.
    pub fn pane_is_short(&self, p: PaneId) -> bool {
        self.panes_plan().short(p)
    }

    /// A frame begins in `area`: everything a frame changes happens here, before drawing, and the plan it draws is
    /// worked out. A Widget picker whose Pane went closes; Home without a Pane brings up Arrange mode; the meters
    /// let go of old peaks; the window and the Panes' area follow `area` (a held Widget finds the Pane under the
    /// pointer again); a popup or the mini player lets a held Widget go, and a Widget picked up with Enter is put back
    /// once something else edited the layout; the label's row and the Tray's first Widget are remembered.
    pub fn frame_begins(&mut self, area: Rect) -> ScreenPlan {
        self.picker_lost_its_pane();
        self.arrange_if_homeless();
        self.meters.expire();
        self.begin_frame();
        self.window = area;
        // covers and the hole follow the Theme's tint (a Theme switch, a theme file saved, `ui.tint_art` edited)
        let tint = self.theme.art_tint(self.config.ui.tint_art);
        self.hole.set_tint(tint.as_ref().map(|t| t.stops4()));
        self.art_tint = tint;
        // too small for any layout: just a player (#47)
        self.mini = crate::mini::wanted(area);
        if self.mini {
            self.overlays.clear();
            self.tile_cancel();
            if let Some(k) = self.kitty.as_mut() {
                k.holes.clear();
            }
            return self.plan();
        }
        // something drawn on top now (the login code, the wizard): a held Widget is let go (its chip would cover it),
        // a held divider let go where it is, as when a popup opens (Q103)
        if self.modal().is_some_and(|m| m != Modal::Arrange) {
            self.gesture_interrupted();
        }
        // a Widget picked up with Enter, and the layout edited by something else since: it is put back
        self.pick_up_check();
        let body = rows(area, self.arrange.is_on()).body;
        if self.body != body {
            self.body = body;
            // a new window size moves every Pane: the one under a held Widget is found again
            self.tile_resized();
        }
        let plan = self.plan();
        // popups are known up front, so pixel images under them can step down to cells this frame; what's over the
        // Panes, the top first (D83)
        let mut overlays: Vec<Rect> = self.layers().iter().filter_map(|&m| crate::ui::layer_rect(area, self, m)).collect();
        overlays.extend(self.toast.as_ref().map(|(msg, _)| crate::ui::toast_rect(area, msg)));
        self.overlays = overlays;
        if let Some(d) = &plan.drag {
            // the label stays on its row next frame while the chip leaves that free
            if let Some((id, ..)) = d.target {
                crate::arrange::Arrange::label_placed(&mut self.gesture, id, d.label);
            }
            if let Some(k) = self.kitty.as_mut() {
                k.holes = d.label.into_iter().chain(d.chip.as_ref().map(|(r, _)| *r)).collect();
            }
        } else if let Some(k) = self.kitty.as_mut() {
            k.holes.clear();
        }
        if let Some(t) = &plan.tray {
            self.arrange.tray_drawn_from(t.first);
        }
        // a hole too small for its Widget isn't drawn: the idle takeover doesn't start from it
        if plan.panes.iter().any(|p| p.short && crate::layout::is_hole(p.pane.widget)) {
            self.hole_origin = None;
        }
        plan
    }
}

#[cfg(test)]
mod tests;
