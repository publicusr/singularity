//! Arranging in the running Client (D87, ADR 0005): Arrange mode and the Tray, a title drag that moves a Widget onto
//! another Pane or the Tray, Move mode on the keyboard, close and split (a split makes an empty Pane on any of the
//! four sides and opens the Widget picker on it, #39), and what an edit does to the Design on screen. The edits
//! themselves are `Arrangement`'s; layout undo is `history`.
//!
//! Arrange mode is a layer (`App::layers`): the player bar stays (its buttons, seek bar and keys work, so the music
//! goes on through a rearrange), the Tray strip goes under it and the legend and Done take the hints' row. The Panes'
//! area is the Tray's height shorter meanwhile, every Pane scaled by the floor plan's own rule (its proportions, and
//! the saved floor plan, never change for it), and Done gives the room back (Q39). Each Pane's title gets a split and
//! a close button. A title drag starts it (so a quick swap is one gesture: the mode stays on after the drop), as do
//! the palette's and the Pane menu's "Arrange", and Esc in a split's Widget picker. Done (or Esc) ends it, once Home
//! has a Pane again.
//!
//! A drag only moves contents. When a drag starts Arrange mode the Panes make room for the Tray once, as it starts
//! (Q39); after that, while a Widget is held, nothing on screen moves but a highlight on the Pane (or the Tray) under
//! the pointer (at once, no dwell, saying what a release would do) and a chip with the Widget's name by
//! the pointer. The release drops there: on a Pane the two Widgets swap (or an empty Pane is filled); on the Tray the
//! Widget waits there and its Pane is left empty; from the Tray onto a Pane it takes that Pane, whose Widget goes into
//! the Tray in its place. On its own Pane, off the Panes, on Esc, or when the release was never heard, the drag ends
//! and nothing changes. Every drop is allowed: no drop changes the floor plan.
//!
//! "New design…" (#42) starts Arrange mode from one empty Pane filling the screen, with Home waiting in the Tray, whose
//! "+ Widget" opens the Widget picker to put any Widget there. Nothing is written while it is built: Done asks for a
//! name and saves it as a Design of the user's own with the current Theme; Esc before anything is placed goes back to
//! the Design that was on screen, untouched.
//!
//! The Design files (owner, Q33): the name prompt never interrupts an edit. An edit to a built-in Design is kept at
//! once as its one unnamed "<Design> (edited)" (the built-in itself never changes; Reset design goes back): outside
//! Arrange mode the bar says so ("Kept as …"), and in Arrange mode Done asks for a name, if anything changed. A Design
//! of the user's own saves itself. Save design as… names one any time.
//!
//! Undo (owner, 2026-11-01): every committed edit (a drop, close, split, Tray edit, Move mode step, size step, divider
//! release, Reset design) can be undone, back to 50 edits, for the session. Undo restores the Arrangement and which
//! Design it was, and the files follow: a Design of the user's gets the restored tree written to its file (and one
//! that a Reset dropped comes back). Undoing the first edit of a built-in Design returns to the built-in one: the
//! unnamed "<Design> (edited)" that edit made is removed again (no orphan), while a Design the user named stays in the
//! picker as saved (it was a deliberate save, like after Reset design). With Done's name prompt up, undo closes it. A
//! state whose floor plan no longer fits the window is refused with a notice.

pub mod history;

use crate::app::{Action, App, Hit, Menu, MenuAct, Modal, NameFor, Naming, Target};
use crate::design::DESIGNS;
use crate::arrangement::{Arrangement, Refused};
use crate::layout::{Divider, Node, Pane, PaneId, Widget, is_hole};
use crate::theme::Theme;
use crate::tiling::{Side, neighbour};
use crate::ui::{blank, text};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use std::time::{Duration, Instant};

/// How long the bar offers "undo" after an edit.
pub const OFFER: Duration = Duration::from_secs(4);

/// The chip by the pointer is at most this wide, and this far right of it.
pub const CHIP_MAX: u16 = 20;
const CHIP_GAP: u16 = 2;

/// A Widget held by its Pane's title (or picked up from the Tray): where the press was, where the pointer is, and
/// (once it has moved far enough to be a drag rather than a click) what is under the pointer: another Pane, or the
/// Tray: what a release drops on.
pub struct TileDrag {
    /// the Pane picked up (its Widget is what moves); from the Tray, no Pane of the layout's (`PaneId::UNSET`)
    pub pane: Pane,
    from: (u16, u16),
    pub at: (u16, u16),
    pub live: bool,
    /// the Pane under the pointer, other than its own: highlighted, and where a release drops
    pub over: Option<PaneId>,
    /// the pointer is on the Tray (and the Widget comes from a Pane): highlighted, and a release puts it there
    pub tray: bool,
    /// picked up from the Tray
    pub from_tray: bool,
}

/// The Tray strip's height: the rows the Panes give up while arranging (Q39).
pub const TRAY_H: u16 = 3;

/// Arrange mode, while it is on: the Arrangement when it began and whether an edit was made since, so Done can
/// tell whether anything changed, and the first Widget the Tray strip shows (when it holds more than fits).
pub struct Arranging {
    since: Arrangement,
    /// an edit was made in it (previewing other Designs in the picker isn't one)
    edited: bool,
    pub tray_first: usize,
    /// a New design… being built (#42)
    fresh: Option<Box<Fresh>>,
}

/// A New design… being built (#42): what was on screen before it (Esc before anything is placed puts it back,
/// untouched) and the undo history from before it (the build's own steps don't reach into it). Nothing is written
/// until Done names it.
struct Fresh {
    before: Snapshot,
    history: history::History<Snapshot>,
}

/// What undo goes back to: the layout, the Design it was (None: the built-in one) and what its file said about it,
/// and the focus.
#[derive(Clone, Debug)]
pub struct Snapshot {
    tree: Arrangement,
    name: Option<String>,
    /// (name, design, edited) from the Design's file
    file: Option<(String, String, bool)>,
    focus: Option<PaneId>,
}

/// The OSC 22 bytes that give the pointer `shape`; "" puts the ordinary arrow back. Ghostty (1.3) ignores an empty
/// name ("unknown cursor shape") and would keep the resize arrow for good, and `text` gives an I-beam (the owner,
/// 2026-11-05: "can it just be normal"); `default` is the plain arrow in Ghostty, kitty and foot alike.
pub fn pointer_shape_seq(shape: &str) -> String {
    format!("\x1b]22;{}\x1b\\", if shape.is_empty() { "default" } else { shape })
}

/// A border that can be grabbed, lit in the accent colour (only its line glyphs: a title on it keeps its colours).
pub fn light_grip(buf: &mut Buffer, grip: Rect, th: &Theme) {
    for y in grip.top()..grip.bottom() {
        for x in grip.left()..grip.right() {
            if let Some(c) = buf.cell_mut((x, y))
                && c.symbol().chars().next().is_some_and(|ch| ('\u{2500}'..='\u{257f}').contains(&ch))
            {
                c.set_fg(th.accent.color());
            }
        }
    }
}

/// The Pane menu's Split row: each arrow splits that way (`SPLIT_SIDES`, in this order).
pub const SPLIT_ROW: &str = "Split  ←  →  ↑  ↓";
pub const SPLIT_SIDES: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

/// The side after (`right`) or before `side` on the Split row, round at the ends.
pub fn next_side(side: Side, right: bool) -> Side {
    let i = SPLIT_SIDES.iter().position(|s| *s == side).unwrap_or(0);
    SPLIT_SIDES[(i + if right { 1 } else { 3 }) % 4]
}

impl App {
    pub fn cell_aspect(&self) -> f64 {
        self.cell_px.map(|(w, h)| h / w).unwrap_or(2.0)
    }

    /// Where the Panes are drawn (the last frame's), or a roomy guess before the first.
    pub fn pane_area(&self) -> Rect {
        if self.body.width > 0 { self.body } else { Rect::new(0, 1, 200, 55) }
    }

    /// Why an edit was refused, in the bar's words.
    pub(crate) fn refused(&mut self, e: Refused) {
        let design = self.design;
        self.say(e.why(|w| crate::ui::pane_name(design, w)));
    }

    /// The Pane to act on: the focused one, or Home's if the focus is on none of this layout's.
    pub fn focused_pane(&self) -> Option<PaneId> {
        self.focus.filter(|p| self.layout.contains(*p)).or_else(|| self.layout.pane_of(Widget::Home))
    }

    /// What the focused Pane shows (None: nothing is focused, or its Pane is empty).
    pub fn focused(&self) -> Option<Widget> {
        self.focus.and_then(|p| self.layout.widget_in(p))
    }

    /// Whether the focused Pane shows `w`.
    pub fn is_focused(&self, w: Widget) -> bool {
        self.focused() == Some(w)
    }

    /// Focus the Pane showing `w` (no Pane, if none does).
    pub fn focus_on(&mut self, w: Widget) {
        self.focus = self.layout.pane_of(w);
    }

    /// Show `tree` in place of the whole layout (a Design, an undo): the Panes of another tree are other Panes, so the
    /// focus stays on the Widget it was on.
    pub fn set_layout(&mut self, tree: impl Into<Arrangement>) {
        let was = self.focused();
        self.layout = tree.into();
        self.focus = was.and_then(|w| self.layout.pane_of(w));
    }

    /// The Pane whose Widget is being moved (held by the mouse, or in Move mode): drawn highlighted.
    pub fn moving(&self) -> Option<PaneId> {
        self.tile.as_ref().filter(|t| t.live).map(|t| t.pane.id).or(self.move_mode)
    }

    // ── the mouse ─────────────────────────────────────────────────────

    /// A press on a Pane's title bar: a click unless the pointer moves a few cells before the release.
    pub fn tile_down(&mut self, id: PaneId, x: u16, y: u16) {
        self.focus = Some(id);
        let pane = self.layout.get(id).unwrap_or(Pane { id, widget: None });
        self.tile = Some(TileDrag { pane, from: (x, y), at: (x, y), live: false, over: None, tray: false, from_tray: false });
    }

    /// A press on Widget `w` in the Tray: a drag takes it onto a Pane.
    fn tray_down(&mut self, w: Widget, x: u16, y: u16) {
        if !self.layout.tray().contains(&w) {
            return;
        }
        let pane = Pane { id: PaneId::UNSET, widget: Some(w) };
        self.tile = Some(TileDrag { pane, from: (x, y), at: (x, y), live: false, over: None, tray: false, from_tray: true });
    }

    /// Where the Tray strip is in Arrange mode: under the player bar, which is under the Panes (Q39).
    pub fn tray_rect(&self) -> Rect {
        self.rows(true)[3]
    }

    /// Where the player bar is: under the Panes.
    pub fn player_rect(&self) -> Rect {
        self.rows(self.arranging.is_some())[2]
    }

    /// The screen's rows (`ui::rows`) in this window, arranging or not; before the first frame, around a roomy guess.
    fn rows(&self, arranging: bool) -> [Rect; 5] {
        let w = if self.window.width > 0 { self.window } else { Rect::new(0, 0, 200, 60) };
        crate::ui::rows(w, arranging)
    }

    /// The Panes' area again, for the window's size and whether Arrange mode is on (it takes the Tray's rows), at once:
    /// a release in the same batch of input finds the Pane under it where the next frame draws it.
    fn relayout(&mut self) {
        if self.window.width > 0 {
            self.body = crate::ui::rows(self.window, self.arranging.is_some())[1];
        }
        self.tile_resized();
    }

    /// What a release at (x, y) would drop on, for a Widget held from Pane `own` (or the Tray): the Pane under it
    /// other than its own, or (for a Pane's Widget) the Tray.
    fn aim(&self, x: u16, y: u16, own: PaneId, from_tray: bool) -> (Option<PaneId>, bool) {
        let t = self.tray_rect();
        if self.arranging.is_some() && x >= t.x && x < t.right() && y >= t.y && y < t.bottom() {
            return (None, !from_tray);
        }
        (self.pane_under(x, y).filter(|p| *p != own), false)
    }

    /// The Pane under (x, y), in the layout as it is drawn.
    pub fn pane_under(&self, x: u16, y: u16) -> Option<PaneId> {
        let area = self.pane_area();
        self.layout.compute(area).panes.into_iter().find(|(_, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()).map(|(p, _)| p.id)
    }

    /// Esc while a Widget is held: nothing changes.
    pub fn tile_cancel(&mut self) {
        self.tile = None;
    }

    /// Mouse events while a Widget is held: only the left button's (another button pressed meanwhile doesn't drop it).
    pub fn tile_pointer(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::Drag(MouseButton::Left) => self.tile_move(m.column, m.row),
            MouseEventKind::Up(MouseButton::Left) => self.tile_up(m.column, m.row),
            _ => {}
        }
    }

    /// A bare move or a fresh press while a Widget or a divider is held: its release went somewhere else (another
    /// window, after the focus moved) and was never heard (#21: the Pane stayed held and swallowed every click). A held
    /// Widget stays where it was; a divider is let go where it is.
    pub fn release_unheard(&mut self, kind: MouseEventKind) {
        if !matches!(kind, MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)) {
            return;
        }
        self.tile_cancel();
        if matches!(self.drag, Some(Hit::Divider(_))) {
            self.drag = None;
            self.divider_released();
        }
    }

    /// The pointer at (x, y) with the button down: past a few cells it is a drag (an empty Pane has nothing to carry),
    /// which turns Arrange mode on, and what is under it is the one highlighted, at once.
    fn tile_move(&mut self, x: u16, y: u16) {
        let Some(t) = self.tile.as_ref() else { return };
        if !t.live && (x.abs_diff(t.from.0) < 3 && y.abs_diff(t.from.1) < 2 || t.pane.widget.is_none()) {
            return;
        }
        let (own, from_tray) = (t.pane.id, t.from_tray);
        // on while the Widget is held, and after the drop: a quick swap stays one gesture (D87)
        self.enter_arrange();
        let (over, tray) = self.aim(x, y, own, from_tray);
        if let Some(t) = self.tile.as_mut() {
            t.live = true;
            t.at = (x, y);
            t.over = over;
            t.tray = tray;
        }
    }

    /// The terminal took a new size (`w` × `h`): the Panes' area is known at once, so a release in the same batch of
    /// input finds the Pane under it in the new window, not the old (#40 review).
    pub fn window_resized(&mut self, w: u16, h: u16) {
        self.window = Rect::new(0, 0, w, h);
        self.mini = crate::mini::wanted(self.window);
        self.relayout();
    }

    /// The window changed size while a Widget is held: the Panes moved under the pointer, so the one under it is found
    /// again (#21: a release dropped on a Pane of the old window).
    pub fn tile_resized(&mut self) {
        let Some((own, from_tray, (x, y))) = self.tile.as_ref().filter(|t| t.live).map(|t| (t.pane.id, t.from_tray, t.at)) else { return };
        let (over, tray) = self.aim(x, y, own, from_tray);
        if let Some(t) = self.tile.as_mut() {
            t.over = over;
            t.tray = tray;
        }
    }

    /// Let go at (x, y): on another Pane the two Widgets swap (an empty one is filled); on the Tray the Widget waits
    /// there; from the Tray onto a Pane it takes the Pane (its Widget goes into the Tray). Anywhere else nothing changes.
    fn tile_up(&mut self, x: u16, y: u16) {
        let Some(t) = self.tile.as_ref() else { return };
        if !t.live {
            let (id, from, w, from_tray) = (t.pane.id, t.from, t.pane.widget, t.from_tray);
            self.tile = None;
            // a click on a Widget in the Tray does nothing
            if from_tray {
                return;
            }
            // a click on the Widget's name: the Widget picker (D87)
            if self.hit_at(from.0, from.1) == Some(Hit::TitleName(id)) {
                return self.open_widget_picker(id);
            }
            // a click elsewhere on the title: the focus moved at the press; the hole's Pane also changes its look
            if is_hole(w) {
                self.run(Action::NextLook);
            }
            return;
        }
        // the release's own cell is the last word (a finger slips as it lifts; moves can come combined)
        self.tile_move(x, y);
        let Some(t) = self.tile.take() else { return };
        let edit = match (t.from_tray, t.pane.widget, t.over) {
            (true, Some(w), Some(to)) => self.layout.out_of_tray(w, to).map(|a| (a, Some(to))),
            (false, _, _) if t.tray => self.layout.to_tray(t.pane.id).map(|a| (a, None)),
            (false, _, Some(to)) => self.layout.swap(t.pane.id, to).map(|a| (a, Some(to))),
            _ => return,
        };
        match edit {
            Ok((a, to)) => {
                if to.is_some() {
                    self.focus = to;
                }
                self.commit_layout(a);
            }
            Err(e) => self.refused(e),
        }
    }

    /// While a Widget is held over another Pane: that Pane, what its label says ("swap with queue", "put here"), and
    /// the chip's text ("⠿ stream"). Over its own Pane or none: the chip alone.
    pub fn drag_marks(&self) -> Option<(Option<(PaneId, String)>, String)> {
        let t = self.tile.as_ref().filter(|t| t.live)?;
        let w = t.pane.widget?;
        let design = self.design;
        let target = t.over.map(|o| {
            let label = match self.layout.widget_in(o) {
                Some(x) => format!("swap with {}", crate::ui::pane_name(design, x)),
                None => "put here".to_string(),
            };
            (o, label)
        });
        Some((target, format!("⠿ {}", crate::ui::pane_name(design, w))))
    }

    /// The chip by the pointer for `text`: one row, at most `CHIP_MAX` wide, right of the pointer (left of it where
    /// the screen ends), inside `area`.
    pub fn chip_rect(&self, area: Rect, text: &str) -> Option<Rect> {
        let t = self.tile.as_ref().filter(|t| t.live)?;
        let w = (text.chars().count() as u16 + 2).min(CHIP_MAX).min(area.width);
        let (x, y) = t.at;
        let right = x.saturating_add(CHIP_GAP);
        let x = if right + w <= area.right() { right } else { x.saturating_sub(CHIP_GAP + w).max(area.x) };
        Some(Rect::new(x, y.clamp(area.y, area.bottom().saturating_sub(1)), w, 1))
    }

    /// The divider to light up: the one being dragged, else the one under the pointer.
    pub fn grip_lit(&self) -> Option<usize> {
        match &self.drag {
            Some(Hit::Divider(i)) => Some(*i),
            _ if self.tile.is_none() && matches!(self.modal(), None | Some(Modal::Arrange)) => self.grip_hover,
            _ => None,
        }
    }

    /// The pointer left the window or the layout changed under it (focus lost, a resize): no border is under it
    /// until the next mouse event says so.
    pub fn pointer_gone(&mut self) {
        self.grip_hover = None;
    }

    /// The mouse pointer's shape for the terminal (OSC 22: kitty, Ghostty, foot…; others ignore it): a resize arrow
    /// over a border that can be dragged, else the terminal's own ("").
    pub fn pointer_shape(&self) -> &'static str {
        match self.grip_lit().and_then(|i| self.dividers.get(i)) {
            Some(d) if d.axis == crate::layout::Axis::Horizontal => "ew-resize",
            Some(_) => "ns-resize",
            None => "",
        }
    }

    /// A divider dragged to `pos`, or as far toward it as it goes without leaving a Pane too small (or the well out of
    /// shape): a fast drag past that stops at the limit, not short of it (#21).
    pub fn drag_divider(&mut self, d: &Divider, pos: u16) {
        let area = self.pane_area();
        self.layout = self.layout.move_line(d, pos, area);
    }

    /// A divider let go: a layout that changed is an edit.
    pub fn divider_released(&mut self) {
        if let Some(before) = self.divider_from.take().filter(|b| *b != self.layout) {
            let mut was = self.snapshot();
            was.tree = before;
            self.remember(was);
            self.edited();
        }
    }

    // ── edits and what they do to the Design ──────────────────────────

    /// Show `tree` as the layout and treat it as an edit (one undo step).
    /// `tree` replaces the layout as the rest of the last edit (a split's pick): one undo takes both back.
    pub(crate) fn amend_layout(&mut self, tree: Arrangement) {
        self.layout = tree;
        self.undo_offer = Some((Instant::now(), Action::UndoLayout));
        self.want_widget_data();
        self.edited();
    }

    pub(crate) fn commit_layout(&mut self, tree: Arrangement) {
        let was = self.snapshot();
        self.layout = tree;
        self.remember(was);
        self.want_widget_data();
        self.edited();
    }

    fn snapshot(&self) -> Snapshot {
        let file = self.layout_name.as_ref().and_then(|id| self.saved_layouts.iter().find(|s| s.id == *id)).map(|s| (s.name.clone(), s.design.clone(), s.edited));
        Snapshot { tree: self.layout.clone(), name: self.layout_name.clone(), file, focus: self.focus }
    }

    /// An edit replaced `was`: undo can go back to it, and the bar offers that for a few seconds.
    fn remember(&mut self, was: Snapshot) {
        self.history.push(was);
        self.undo_offer = Some((Instant::now(), Action::UndoLayout));
    }

    // ── undo ──────────────────────────────────────────────────────────

    pub fn undo_layout(&mut self) {
        self.step_history(true);
    }

    pub fn redo_layout(&mut self) {
        self.step_history(false);
    }

    fn step_history(&mut self, back: bool) {
        // a held Widget is let go first; the keyboard and the bar never reach here mid-drag, but be sure
        self.tile = None;
        let area = self.pane_area();
        let now = self.snapshot();
        let fits = |s: &Snapshot| s.tree.fits(area);
        let got = if back { self.history.undo(now, fits) } else { self.history.redo(now, fits) };
        let (any, what) = if back { (self.history.can_undo(), "undo") } else { (self.history.can_redo(), "redo") };
        match got {
            Some(s) => {
                self.toast = None;
                self.restore(s);
                self.undo_offer = Some((Instant::now(), if back { Action::RedoLayout } else { Action::UndoLayout }));
                if self.toast.is_none() {
                    let key = self.keymap.label(if back { Action::RedoLayout } else { Action::UndoLayout });
                    let again = if key.is_empty() { String::new() } else { format!(" · {key} {}", if back { "redoes" } else { "undoes" }) };
                    self.say(format!("{}{again}", if back { "Undone" } else { "Redone" }));
                }
            }
            None if any => self.say(format!("Can't {what}: that layout doesn't fit this window")),
            None => self.say(format!("Nothing to {what}")),
        }
    }

    /// Put `s` back on screen, and its Design's file with it.
    fn restore(&mut self, s: Snapshot) {
        let was = self.layout_name.clone().and_then(|id| self.saved_layouts.iter().find(|x| x.id == id).cloned());
        // Done's name prompt: the changes it would name are going
        if self.naming().is_some_and(|n| n.purpose == NameFor::Fork) {
            self.close_if_open(crate::app::Modal::Naming);
        }
        self.set_layout(s.tree);
        self.focus = s.focus.filter(|p| self.layout.contains(*p)).or_else(|| self.layout.pane_of(Widget::Home));
        if self.move_mode.is_some_and(|m| !self.layout.contains(m)) {
            self.move_mode = None;
        }
        self.want_widget_data();
        match s.name {
            Some(id) => {
                let (name, design, edited) = s.file.unwrap_or_else(|| (id.clone(), DESIGNS[self.design].id.to_string(), false));
                self.write_layout(&id, &name, &design, edited);
            }
            None => {
                self.layout_name = None;
                let base = DESIGNS[self.design].name;
                match was {
                    // the unnamed edit of a built-in Design goes with the edit that made it
                    Some(w) if w.edited => {
                        if let Some(dir) = &self.layouts_dir {
                            let _ = std::fs::remove_file(dir.join(format!("{}.toml", w.id)));
                        }
                        self.reload_layouts();
                        self.save_ui();
                        self.say(format!("Back to {base}"));
                    }
                    Some(w) => {
                        self.save_ui();
                        self.say(format!("Back to {base} · \"{}\" is still in the Design picker", w.name));
                    }
                    None => {}
                }
            }
        }
    }

    /// The layout was edited. A Design of the user's own (or the unnamed edit of a built-in one) follows in its file;
    /// an edit of a built-in Design is kept at once as its "<Design> (edited)", never asking (Q33): outside Arrange
    /// mode the bar says so, and in Arrange mode Done asks for a name.
    pub fn edited(&mut self) {
        if let Some(a) = self.arranging.as_mut() {
            a.edited = true;
            // a New design… is written when Done names it
            if a.fresh.is_some() {
                return;
            }
        }
        let own = self.layout_name.clone().and_then(|id| self.saved_layouts.iter().find(|s| s.id == id).cloned());
        match own {
            Some(s) => self.write_layout(&s.id, &s.name, &s.design, s.edited),
            None => {
                self.layout_name = None;
                if self.layouts_dir.is_some() {
                    self.keep_edited();
                    if self.arranging.is_none() && self.layout_name.is_some() {
                        self.say(format!("Kept as {} (edited)", DESIGNS[self.design].name));
                    }
                }
            }
        }
    }

    /// Whether the Design on screen is the unnamed "<Design> (edited)" of a built-in one.
    fn on_edited_copy(&self) -> bool {
        self.layout_name.as_ref().is_some_and(|id| self.saved_layouts.iter().any(|s| s.id == *id && s.edited))
    }

    /// The unnamed "<Design> (edited)" on screen got a name (Done's prompt): its file goes, the named one has it all.
    pub fn named_edited_copy(&mut self, was: Option<String>) {
        let Some(id) = was.filter(|id| self.layout_name.as_ref() != Some(id) && self.saved_layouts.iter().any(|s| s.id == *id && s.edited)) else { return };
        if let Some(dir) = &self.layouts_dir {
            let _ = std::fs::remove_file(dir.join(format!("{id}.toml")));
        }
        self.reload_layouts();
    }

    // ── Arrange mode ──────────────────────────────────────────────────

    /// Arrange mode on (the palette's and the Pane menu's "Arrange", a title drag, Esc in a split's picker).
    pub fn enter_arrange(&mut self) {
        if self.arranging.is_none() {
            self.move_mode = None;
            self.arranging = Some(Arranging { since: self.layout.clone(), edited: false, tray_first: 0, fresh: None });
            self.relayout();
        }
    }

    /// "New design…" (#42): Arrange mode from one empty Pane filling the screen, with Home waiting in the Tray. The
    /// Design on screen stays as it is (in its file too) until Done names the new one; Esc before anything is placed
    /// goes back to it.
    pub fn new_design(&mut self) {
        if self.building() {
            return;
        }
        let before = self.snapshot();
        let history = std::mem::take(&mut self.history);
        // (from Arrange mode: what was arranged is kept as it is, and the build starts from the blank one)
        self.arranging = None;
        self.tile = None;
        self.move_mode = None;
        self.undo_offer = None;
        self.layout = Arrangement::new(Node::empty(), vec![]);
        self.layout_name = None;
        self.focus = self.layout.panes().first().map(|p| p.id);
        self.want_widget_data();
        self.enter_arrange();
        if let Some(a) = self.arranging.as_mut() {
            a.fresh = Some(Box::new(Fresh { before, history }));
        }
    }

    /// Whether a New design… is being built.
    pub fn building(&self) -> bool {
        self.arranging.as_ref().is_some_and(|a| a.fresh.is_some())
    }

    /// The saved Design that was on screen when a New design… began (config.toml keeps naming it meanwhile).
    pub fn building_from(&self) -> Option<String> {
        self.arranging.as_ref()?.fresh.as_ref()?.before.name.clone()
    }

    /// Esc on a New design… before anything is placed: the Design that was on screen, as it was, and its undo history.
    fn cancel_new_design(&mut self) {
        let Some(f) = self.arranging.take().and_then(|a| a.fresh) else { return };
        let Fresh { before, history } = *f;
        self.tile = None;
        self.undo_offer = None;
        self.layout = before.tree;
        self.layout_name = before.name;
        self.focus = before.focus.filter(|p| self.layout.contains(*p)).or_else(|| self.layout.pane_of(Widget::Home));
        self.history = history;
        self.want_widget_data();
        self.relayout();
    }

    /// The name typed on Done's prompt for a New design…: saved as a Design of your own with the current Theme, and
    /// Arrange mode ends. The build's undo steps go with it: the new Design starts its own history. False if it
    /// couldn't be saved (a name that makes no file name, a write error: the bar says why): it is still being built.
    pub fn save_new_design(&mut self, name: &str) -> bool {
        self.save_layout(name);
        // (without a config folder there is nowhere to save it)
        if self.layout_name.is_none() && self.layouts_dir.is_some() {
            return false;
        }
        self.arranging = None;
        self.tile = None;
        self.history = Default::default();
        self.undo_offer = None;
        self.relayout();
        true
    }

    /// The Tray's "+ Widget": the Widget picker, adding the pick to the Tray (#42).
    pub fn open_tray_picker(&mut self) {
        self.open_popup(crate::app::Popup::WidgetPicker(crate::panemenu::WidgetPicker { target: PaneId::UNSET, query: String::new(), sel: 0, after_split: false, tray: true }));
    }

    /// Before each frame: Home always has a Pane outside Arrange mode. A Design whose Home waits in the Tray (one left
    /// that way when the app quit) comes up in Arrange mode, where Done waits for it.
    pub fn arrange_if_homeless(&mut self) {
        // (not while the Design picker previews one: a preview isn't a Design you have)
        if self.arranging.is_none() && self.picker().is_none() && self.layout.unplaced().is_some() {
            self.enter_arrange();
        }
    }

    /// What keeps Done unavailable: "Home still needs a Pane", while it waits in the Tray.
    pub fn done_blocked(&self) -> Option<String> {
        let w = self.layout.unplaced()?;
        let name = crate::ui::pane_name(self.design, w);
        let mut c = name.chars();
        let name = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
        Some(format!("{name} still needs a Pane"))
    }

    /// Done (or Esc): Arrange mode ends, once Home has a Pane. If anything changed in it and the changes went to a
    /// built-in Design's "<Design> (edited)", the name prompt comes now (Esc keeps it unnamed).
    pub fn arrange_done(&mut self) {
        if self.done_blocked().is_some() {
            return;
        }
        // a New design…: Done asks for its name, and Arrange mode stays on until it has one (#42)
        if self.building() {
            if self.naming().is_none() {
                self.open_popup(crate::app::Popup::Naming(Naming::new_design()));
            }
            return;
        }
        let Some(a) = self.arranging.take() else { return };
        self.tile = None;
        self.relayout();
        let changed = a.edited && a.since != self.layout;
        if changed && self.on_edited_copy() && self.naming().is_none() {
            let name = self.suggest_name();
            self.open_popup(crate::app::Popup::Naming(Naming::fork(name)));
        }
    }

    /// Keys in Arrange mode (the shortcuts with Ctrl or Alt work as everywhere): Esc is Done, and the playback keys
    /// work as ever (the player bar stays); no Pane takes keys.
    pub fn arrange_key(&mut self, k: KeyEvent) {
        if k.code == KeyCode::Esc {
            // a New design… with nothing placed yet: back to the Design that was on screen, untouched (#42)
            if self.arranging.as_ref().is_some_and(|a| a.fresh.is_some() && a.since == self.layout) {
                return self.cancel_new_design();
            }
            return self.arrange_done();
        }
        use Action::*;
        if let Some(a) = self.keymap.action(k).filter(|a| matches!(a, TogglePlay | Next | Previous | SeekForward | SeekBack | SeekForwardFine | SeekBackFine | VolumeUp | VolumeDown | Shuffle | Repeat | Like)) {
            self.run(a);
        }
    }

    /// The Tray strip scrolled `by` Widgets (the wheel, its ‹ ›).
    pub fn tray_scroll(&mut self, by: i32) {
        let Some(now) = self.arranging.as_ref().map(|a| a.tray_first) else { return };
        let want = (now as i64 + by as i64).max(0) as usize;
        // no further than the last start that still fills the strip
        let first = tray_layout(self, self.tray_rect(), want).first;
        if let Some(a) = self.arranging.as_mut() {
            a.tray_first = first;
        }
    }

    /// The `×` of Widget `w` in the Tray: it is removed (Home never; one already gone: nothing).
    fn tray_remove(&mut self, w: Widget) {
        if !self.layout.tray().contains(&w) {
            return;
        }
        match self.layout.remove_from_tray(w) {
            Ok(a) => self.commit_layout(a),
            Err(e) => self.refused(e),
        }
    }

    /// Clear Tray: what's in it is removed, but Home.
    fn clear_tray(&mut self) {
        let a = self.layout.clear_tray();
        if a != self.layout {
            self.commit_layout(a);
        }
    }

    /// A mouse event in Arrange mode (on top of everything but popups): title drags, dividers, a Pane's split and
    /// close buttons, the Tray and Done; a right click opens the Pane's menu; the wheel scrolls the Tray. Nothing else
    /// in the Panes takes a click (a click focuses the Pane).
    pub(crate) fn arrange_pointer(&mut self, m: MouseEvent, coarse: bool) {
        if self.tile.is_some() {
            return self.tile_pointer(m);
        }
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => match self.hit_at(x, y) {
                Some(Hit::Title(p) | Hit::TitleName(p)) => self.tile_down(p, x, y),
                Some(Hit::Close(p)) => self.close_pane(p),
                Some(Hit::SplitPane(p)) => {
                    let side = self.default_split(p);
                    self.split_pane(p, side);
                }
                Some(Hit::Divider(i)) => self.grab_divider(i, x, y),
                Some(Hit::TrayItem(w)) => self.tray_down(w, x, y),
                Some(Hit::TrayRemove(w)) => self.tray_remove(w),
                Some(Hit::ClearTray) => self.clear_tray(),
                Some(Hit::TrayAdd) => self.open_tray_picker(),
                Some(Hit::TrayScroll(d)) => self.tray_scroll(d),
                Some(Hit::ArrangeDone) => self.arrange_done(),
                Some(Hit::Action(a @ (Action::UndoLayout | Action::RedoLayout))) => self.run(a),
                _ => {
                    if let Some(p) = self.pane_under(x, y) {
                        self.focus = Some(p);
                    }
                }
            },
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(p) = self.pane_under(x, y) {
                    self.open_pane_menu(p, Some((x, y)));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(Hit::Divider(i)) = self.drag.clone()
                    && let Some(d) = self.dividers.get(i).cloned()
                {
                    let pos = if d.axis == crate::layout::Axis::Vertical { y } else { x };
                    self.drag_divider(&d, (pos as i32 + self.drag_offset).max(0) as u16);
                }
            }
            MouseEventKind::Up(_) => {
                if self.drag.take().is_some() {
                    self.divider_released();
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let rows = if m.kind == MouseEventKind::ScrollUp { -1 } else { 1 };
                self.modal_scroll(Modal::Arrange, rows * if coarse { 10 } else { 1 });
            }
            _ => {}
        }
    }

    /// A press on divider `i`'s grip at (x, y): it follows the pointer until the release.
    pub(crate) fn grab_divider(&mut self, i: usize, x: u16, y: u16) {
        self.divider_from = Some(self.layout.clone());
        if let Some(d) = self.dividers.get(i) {
            let (boundary, grab) = match d.axis {
                crate::layout::Axis::Horizontal => (d.grip.x + 1, x),
                crate::layout::Axis::Vertical => (d.grip.y + 1, y),
            };
            self.drag_offset = boundary as i32 - grab as i32;
        }
        self.drag = Some(Hit::Divider(i));
    }

    /// "Singularity 2", "Singularity 3"…: the first not taken by a Design of the user's.
    fn suggest_name(&mut self) -> String {
        self.reload_layouts();
        let base = DESIGNS[self.design].name;
        (2..).map(|n| format!("{base} {n}")).find(|c| !self.saved_layouts.iter().any(|s| s.id == crate::layouts::file_id(c))).expect("a free name")
    }

    /// Esc on the naming prompt: keep the edit as the one unnamed "<Design> (edited)", replacing the last.
    pub fn keep_edited(&mut self) {
        let d = &DESIGNS[self.design];
        self.write_layout(&crate::layouts::edited_id(d.id), &format!("{} (edited)", d.name), d.id, true);
    }

    fn write_layout(&mut self, id: &str, name: &str, design: &str, edited: bool) {
        let Some(dir) = self.layouts_dir.clone() else { return };
        match crate::layouts::write(&dir, id, name, design, &self.theme.id, &self.layout, edited) {
            Ok(s) => {
                let new = self.layout_name.as_deref() != Some(s.id.as_str());
                self.layout_name = Some(s.id);
                self.reload_layouts();
                if new {
                    self.save_ui();
                }
            }
            Err(e) => self.say(format!("Couldn't save the layout: {e}")),
        }
    }

    /// "Reset design": the built-in Design's own layout again. A Design of the user's stays in the picker as it was
    /// last saved (its file isn't touched); the unnamed edit of a built-in one is dropped.
    pub fn reset_design(&mut self) {
        let before = self.snapshot();
        let (d, theme) = (self.design, self.theme.id.clone());
        let name = DESIGNS[d].name;
        let was = self.layout_name.clone().and_then(|id| self.saved_layouts.iter().find(|s| s.id == id).cloned());
        self.set_design(d);
        self.set_theme(&theme);
        self.layout_name = None;
        self.save_ui();
        match was {
            Some(s) if s.edited => {
                if let Some(dir) = &self.layouts_dir {
                    let _ = std::fs::remove_file(dir.join(format!("{}.toml", s.id)));
                }
                self.reload_layouts();
                self.say(format!("Back to {name}"));
            }
            Some(s) => self.say(format!("Back to {name} · \"{}\" is still in the Design picker", s.name)),
            None => self.say(format!("{name} reset")),
        }
        if before.tree != self.layout || before.name.is_some() {
            self.remember(before);
        }
    }

    // ── close and split ───────────────────────────────────────────────

    pub fn close_pane(&mut self, id: PaneId) {
        let Some(pane) = self.layout.get(id) else { return };
        let name = self.name_of(pane);
        let area = self.pane_area();
        let tree = match self.layout.close(id, area) {
            Ok(t) => t,
            Err(Refused::Required(_)) => return self.say(format!("{name} can be moved, but never closed")),
            Err(Refused::NoRoom | Refused::LastPane) => return self.say(format!("Closing {name} would squash the others")),
            Err(e) => return self.refused(e),
        };
        if self.focus == Some(id) {
            self.focus = tree.pane_of(Widget::Home);
        }
        if self.move_mode == Some(id) {
            self.move_mode = None;
        }
        let built_in = self.layout_name.is_none();
        self.commit_layout(tree);
        // (the bar's one line: what the edit did, and where a built-in Design's edit went)
        match built_in && self.layout_name.is_some() && self.arranging.is_none() {
            true => self.say(format!("Closed {name} · kept as {} (edited)", DESIGNS[self.design].name)),
            false => self.say(format!("Closed {name}")),
        }
    }

    /// What Pane `p` is called on screen (its Widget's title; an empty Pane is "an empty pane").
    pub fn name_of(&self, p: Pane) -> String {
        p.widget.map_or_else(|| "an empty pane".into(), |w| crate::ui::pane_name(self.design, w))
    }

    pub fn split_focused(&mut self, side: Side) {
        if let Some(target) = self.focused_pane() {
            self.split_pane(target, side);
        }
    }

    /// A new empty Pane on `side` of Pane `target`, taking half its room, and the Widget picker on it at once (Esc
    /// leaves it empty). Refused with the reason where it would leave a Pane under the minimum.
    pub fn split_pane(&mut self, target: PaneId, side: Side) {
        let Some(pane) = self.layout.get(target) else { return };
        let area = self.pane_area();
        let tree = match self.layout.split(target, side, None, area) {
            Ok(t) => t,
            Err(Refused::NoRoom) => return self.say(format!("No room for a new Pane {} {}", side.word_beside(), self.name_of(pane))),
            Err(e) => return self.refused(e),
        };
        let Some(new) = tree.panes().into_iter().map(|p| p.id).find(|id| !self.layout.contains(*id)) else { return };
        self.commit_layout(tree);
        self.focus = Some(new);
        self.open_widget_picker(new);
        if let Some(p) = self.widget_picker_mut() {
            p.after_split = true;
        }
    }

    /// The side a split of Pane `id` takes unless told otherwise: along its longer side in real proportions (wider than
    /// tall: to the right, else below), or the other way if only that one has room.
    pub fn default_split(&self, id: PaneId) -> Side {
        let (area, aspect) = (self.pane_area(), self.cell_aspect());
        let rect = self.layout.compute(area).panes.into_iter().find(|(p, _)| p.id == id).map(|(_, r)| r);
        let wide = rect.is_none_or(|r| r.width as f64 >= r.height as f64 * aspect);
        let (long, other) = if wide { (Side::Right, Side::Bottom) } else { (Side::Bottom, Side::Right) };
        let fits = |side| self.layout.split(id, side, None, area).is_ok();
        if !fits(long) && fits(other) { other } else { long }
    }

    /// An arrow of the Pane menu's Split row.
    pub fn menu_split(&mut self, side: Side) {
        if let Some(Menu { target: Target::Pane(id), .. }) = self.take_menu() {
            self.split_pane(id, side);
        }
    }

    /// The Pane's menu (D87): Replace widget… first (not on Home's), then the floor plan's edits.
    pub fn open_pane_menu(&mut self, id: PaneId, at: Option<(u16, u16)>) {
        let mut items = vec![];
        if self.can_replace(id) {
            items.push(("Replace widget…".to_string(), MenuAct::ReplaceWidget));
        }
        items.push((SPLIT_ROW.to_string(), MenuAct::Split(self.default_split(id))));
        if !crate::arrangement::required(self.layout.widget_in(id)) {
            items.push(("Close pane".into(), MenuAct::ClosePane));
        }
        if self.arranging.is_none() {
            items.push(("Arrange".into(), MenuAct::Arrange));
        }
        self.focus = Some(id);
        self.open_popup(crate::app::Popup::Menu(Menu { at: at.unwrap_or((0, 0)), target: Target::Pane(id), items, selected: 0 }));
    }

    pub fn pane_menu_run(&mut self, id: PaneId, act: MenuAct) {
        self.focus = Some(id);
        match act {
            MenuAct::ReplaceWidget => self.open_widget_picker(id),
            MenuAct::Split(side) => self.split_pane(id, side),
            MenuAct::Arrange => self.enter_arrange(),
            MenuAct::ClosePane => self.close_pane(id),
            _ => {}
        }
    }

    // ── Move mode ─────────────────────────────────────────────────────

    pub fn start_move(&mut self) {
        let Some(id) = self.focused_pane() else { return };
        if self.layout.panes().len() < 2 {
            return self.say("Nothing to move it around");
        }
        if self.layout.widget_in(id).is_none() {
            return self.say("An empty Pane has nothing to move");
        }
        self.move_mode = Some(id);
    }

    /// Keys in Move mode: arrows swap the Widget with the neighbour's that way (it moves, the Panes stay; Move mode
    /// follows it), `+` `-` size its Pane, `x` closes it, `u` undoes (`U` redoes), Enter or Esc ends. What can't be done
    /// is said in the bar, and nothing changes. (D66's Shift+arrows, which split it in beside a neighbour or made it a
    /// whole column at the screen's edge, went with the drop zones: the floor plan changes only by its own edits now.)
    pub fn move_key(&mut self, k: KeyEvent) {
        let Some(id) = self.move_mode.filter(|p| self.layout.contains(*p)) else {
            self.move_mode = None;
            return;
        };
        match k.code {
            KeyCode::Esc | KeyCode::Enter => self.move_mode = None,
            KeyCode::Left => self.move_step(id, Side::Left),
            KeyCode::Right => self.move_step(id, Side::Right),
            KeyCode::Up => self.move_step(id, Side::Top),
            KeyCode::Down => self.move_step(id, Side::Bottom),
            KeyCode::Char('+' | '=') => self.resize_step(id, true),
            KeyCode::Char('-' | '_') => self.resize_step(id, false),
            KeyCode::Char('x' | 'X') => self.close_pane(id),
            KeyCode::Char('u') => self.undo_layout(),
            KeyCode::Char('U') => self.redo_layout(),
            _ => {}
        }
    }

    /// One step that way: the Widget swaps with the neighbour's, and Move mode (and the focus) go with it.
    fn move_step(&mut self, id: PaneId, dir: Side) {
        let area = self.pane_area();
        let panes = self.layout.compute(area).panes;
        let Some(pane) = self.layout.get(id) else { return };
        let Some(n) = neighbour(&panes, id, dir) else {
            return self.say(format!("Nothing {} {}", dir.word_beside(), self.name_of(pane)));
        };
        match self.layout.swap(id, n) {
            Ok(tree) => {
                self.focus = Some(n);
                self.move_mode = Some(n);
                self.commit_layout(tree);
            }
            Err(e) => self.refused(e),
        }
    }

    fn resize_step(&mut self, id: PaneId, grow: bool) {
        let area = self.pane_area();
        match self.layout.step_line(id, grow, area) {
            Ok(tree) => self.commit_layout(tree),
            Err(_) => self.say(if grow { "Can't grow it more" } else { "Can't shrink it more" }),
        }
    }
}

// ── drawing ───────────────────────────────────────────────────────────

/// The Pane a release would drop on: its frame lit in the accent, and a label in its middle saying what happens
/// (clear of the chip, `chip`).
pub fn draw_target(buf: &mut Buffer, r: Rect, label: &str, chip: Option<Rect>, th: &Theme) {
    let lit = th.s(th.accent).add_modifier(Modifier::BOLD);
    let (right, bottom) = (r.right().saturating_sub(1), r.bottom().saturating_sub(1));
    let edge = (r.x..r.right()).flat_map(|x| [(x, r.y), (x, bottom)]).chain((r.y..r.bottom()).flat_map(|y| [(r.x, y), (right, y)]));
    for (x, y) in edge {
        if let Some(c) = buf.cell_mut((x, y))
            && c.symbol().chars().next().is_some_and(|ch| ('\u{2500}'..='\u{257f}').contains(&ch))
        {
            c.set_style(lit);
        }
    }
    if let Some(l) = label_rect(r, label, chip) {
        text(buf, l.x, l.y, l.width, &fit(label, l.width), th.chip(th.accent));
    }
}

/// Where a target's label goes: centred in it, cut to fit inside its frame, on the middle row or the nearest one the
/// chip (`chip`) leaves free.
pub fn label_rect(r: Rect, label: &str, chip: Option<Rect>) -> Option<Rect> {
    let w = (label.chars().count() as u16 + 2).min(r.width.saturating_sub(2));
    if r.height < 3 || w < 3 {
        return None;
    }
    let mid = r.y + r.height / 2;
    let at = |y: u16| Rect::new(r.x + (r.width - w) / 2, y, w, 1);
    let free = |l: &Rect| chip.is_none_or(|c| !c.intersects(*l));
    let rows = (r.y + 1..r.bottom() - 1).map(|y| (y.abs_diff(mid), y));
    rows.map(|(_, y)| at(y)).filter(free).min_by_key(|l| l.y.abs_diff(mid)).or(Some(at(mid)))
}

/// ` text ` in `w` columns: cut with an ellipsis if it is longer.
fn fit(s: &str, w: u16) -> String {
    let padded = format!(" {s} ");
    if padded.chars().count() <= w as usize {
        return padded;
    }
    format!(" {}…", s.chars().take((w as usize).saturating_sub(2)).collect::<String>())
}

/// The chip by the pointer: the held Widget's name with its grip.
pub fn draw_chip(buf: &mut Buffer, r: Rect, label: &str, th: &Theme) {
    blank(buf, r.x, r.y, r.width, th.chip(th.accent2));
    text(buf, r.x, r.y, r.width, &fit(label, r.width), th.chip(th.accent2));
}

/// The bar's one line in Move mode: its keys, or what the last key couldn't do.
pub fn draw_move_hint(buf: &mut Buffer, r: Rect, name: &str, said: Option<&str>, th: &Theme) {
    blank(buf, r.x, r.y, r.width, th.base());
    let tag = format!(" MOVE {name} ");
    let w = tag.chars().count() as u16;
    text(buf, r.x + 1, r.y, w, &tag, th.chip(th.accent));
    let (line, st) = match said {
        Some(m) => (m, th.s(th.accent2)),
        None => ("←↑↓→ swap · + − size · x close · u undo · Enter done", th.s(th.dim)),
    };
    text(buf, r.x + w + 2, r.y, r.width.saturating_sub(w + 3), line, st);
}

/// Where the parts of the Tray strip go: each Widget shown (its place in the Tray, its chip, its × unless it is
/// Home), the ‹ › while they don't all fit, "+ Widget", Clear Tray, and the first Widget shown.
pub struct TrayLayout {
    pub items: Vec<(usize, Rect, Option<Rect>)>,
    pub prev: Option<Rect>,
    pub next: Option<Rect>,
    pub add: Option<Rect>,
    pub clear: Option<Rect>,
    pub first: usize,
}

/// The Tray's title on its frame, and its "+ Widget" button beside it (#42).
const TRAY_TITLE: &str = " Tray ";
const TRAY_ADD: &str = " + Widget ";

/// A Widget's chip in the Tray: its grip and name.
fn tray_chip(name: &str) -> String {
    format!(" ⠿ {name} ")
}

/// The Tray strip at `r` (three rows: its frame round one row of Widgets), from Widget `first` on, or as near it as
/// still fills the row: when they don't all fit, ‹ › at the ends scroll it.
pub fn tray_layout(app: &App, r: Rect, first: usize) -> TrayLayout {
    let tray = app.layout.tray();
    let y = r.y + 1;
    let (x0, x1) = (r.x + 2, r.right().saturating_sub(2));
    let width = |w: Widget| tray_chip(&crate::ui::pane_name(app.design, w)).chars().count() as u16 + if crate::arrangement::required(Some(w)) { 0 } else { 2 };
    let widths: Vec<u16> = tray.iter().map(|w| width(*w)).collect();
    let all: u16 = widths.iter().map(|w| w + 1).sum();
    let over = all > x1.saturating_sub(x0) + 1;
    let (from, to) = if over { (x0 + 2, x1.saturating_sub(2)) } else { (x0, x1) };
    // the last place to start from that still fills the row
    let mut last = tray.len().saturating_sub(1);
    let mut used = 0;
    for (i, w) in widths.iter().enumerate().rev() {
        used += w + 1;
        if used > to.saturating_sub(from) + 1 {
            break;
        }
        last = i;
    }
    let first = if over { first.min(last) } else { 0 };
    let mut items = vec![];
    let mut x = from;
    for (i, w) in tray.iter().enumerate().skip(first) {
        let cw = width(*w);
        if x + cw > to {
            break;
        }
        let chip = tray_chip(&crate::ui::pane_name(app.design, *w)).chars().count() as u16;
        let close = (!crate::arrangement::required(Some(*w))).then(|| Rect::new(x + chip, y, 2, 1));
        items.push((i, Rect::new(x, y, chip, 1), close));
        x += cw + 1;
    }
    let shown_to = items.last().map_or(first, |(i, ..)| *i + 1);
    let prev = (over && first > 0).then(|| Rect::new(x0, y, 1, 1));
    let next = (over && shown_to < tray.len()).then(|| Rect::new(x1.saturating_sub(1), y, 1, 1));
    let clear_label = " Clear Tray ";
    let clear = tray.iter().any(|w| !crate::arrangement::required(Some(*w))).then(|| Rect::new(r.right().saturating_sub(clear_label.chars().count() as u16 + 2), r.y, clear_label.chars().count() as u16, 1));
    // "+ Widget" on the frame after the title (where it fits before Clear Tray)
    let add = Rect::new(r.x + 1 + TRAY_TITLE.chars().count() as u16 + 1, r.y, TRAY_ADD.chars().count() as u16, 1);
    let add = (add.right() < clear.map_or(r.right().saturating_sub(1), |c| c.x)).then_some(add);
    TrayLayout { items, prev, next, add, clear, first }
}

/// The Tray strip (Arrange mode, under the player bar): what waits there, each with its ×, Clear Tray, and while a
/// Pane's Widget is held over it, lit with what a release does.
pub fn draw_tray(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    use ratatui::widgets::{Block, Borders, Widget as _};
    if r.height < 3 || r.width < 20 {
        return;
    }
    let lit = app.tile.as_ref().is_some_and(|t| t.live && t.tray);
    let border = if lit { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.border) };
    Block::default().borders(Borders::ALL).border_set(th.borders).border_style(border).title(ratatui::text::Span::styled(TRAY_TITLE, th.s(th.accent))).style(th.base()).render(r, buf);
    let first = app.arranging.as_ref().map_or(0, |a| a.tray_first);
    let l = tray_layout(app, r, first);
    if let Some(a) = app.arranging.as_mut() {
        a.tray_first = l.first;
    }
    let tray = app.layout.tray().to_vec();
    if tray.is_empty() {
        let s = "drop a Widget here to set it aside";
        text(buf, r.x + 2, r.y + 1, r.width.saturating_sub(4), s, th.s(th.dim));
    }
    for (i, chip, close) in &l.items {
        let name = crate::ui::pane_name(app.design, tray[*i]);
        text(buf, chip.x, chip.y, chip.width, &tray_chip(&name), th.chip(th.accent2));
        app.hit(*chip, Hit::TrayItem(tray[*i]));
        if let Some(c) = close {
            text(buf, c.x, c.y, c.width, "× ", th.s(th.dim));
            app.hit(Rect::new(c.x, c.y, 1, 1), Hit::TrayRemove(tray[*i]));
        }
    }
    for (a, glyph, d) in [(l.prev, "‹", -1), (l.next, "›", 1)] {
        if let Some(a) = a {
            text(buf, a.x, a.y, 1, glyph, th.s(th.accent));
            app.hit(a, Hit::TrayScroll(d));
        }
    }
    if let Some(a) = l.add {
        text(buf, a.x, a.y, a.width, TRAY_ADD, th.s(th.dim));
        app.hit(a, Hit::TrayAdd);
    }
    if let Some(c) = l.clear {
        text(buf, c.x, c.y, c.width, " Clear Tray ", th.s(th.dim));
        app.hit(c, Hit::ClearTray);
    }
    if lit {
        let label = " set aside here ";
        let w = label.chars().count() as u16;
        text(buf, r.x + r.width.saturating_sub(w) / 2, r.bottom() - 1, w, label, th.chip(th.accent));
    }
}

/// Arrange mode's line at the bottom: its keys and gestures, then Done (unavailable, saying why, while Home waits in
/// the Tray).
pub fn draw_legend(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    blank(buf, r.x, r.y, r.width, th.base());
    let tag = " ARRANGE ";
    let tw = tag.chars().count() as u16;
    text(buf, r.x + 1, r.y, tw, tag, th.chip(th.accent));
    let done = " Done ";
    let dw = done.chars().count() as u16;
    let dx = r.right().saturating_sub(dw + 1);
    let blocked = app.done_blocked();
    let mut end = dx;
    match &blocked {
        Some(why) => {
            let ww = why.chars().count() as u16;
            let wx = dx.saturating_sub(ww + 2);
            text(buf, wx, r.y, ww, why, th.s(th.accent2));
            text(buf, dx, r.y, dw, done, th.s(th.dim));
            end = wx;
        }
        None => text(buf, dx, r.y, dw, done, th.chip(th.accent)),
    }
    app.hit(Rect::new(dx, r.y, dw, 1), Hit::ArrangeDone);
    let key = |a| app.keymap.label(a);
    let mut parts = vec!["drag a title onto a Pane or the Tray".to_string(), "⊞ split".into(), "× close".into()];
    for (a, what) in [(Action::UndoLayout, "undo"), (Action::RedoLayout, "redo")] {
        let k = key(a);
        if !k.is_empty() {
            parts.push(format!("{k} {what}"));
        }
    }
    parts.push("Esc done".into());
    let x = r.x + tw + 2;
    let room = end.saturating_sub(x + 1) as usize;
    // as many as fit, whole, from the end's (the keys) back to the gestures
    let mut line = parts.join(" · ");
    while line.chars().count() > room && parts.len() > 1 {
        parts.remove(0);
        line = parts.join(" · ");
    }
    if line.chars().count() <= room {
        text(buf, x, r.y, room as u16, &line, th.s(th.dim));
    }
}

/// For a few seconds after a layout edit, the bar's right end offers to undo it (after an undo, to redo it).
pub fn draw_undo_chip(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let Some((at, a)) = app.undo_offer else { return };
    let can = if a == Action::UndoLayout { app.history.can_undo() } else { app.history.can_redo() };
    if at.elapsed() > OFFER || !can || r.width < 40 {
        return;
    }
    let key = app.keymap.label(a);
    let label = format!(" {} {} {key} ", if a == Action::UndoLayout { "↶ undo" } else { "↷ redo" }, if key.is_empty() { "" } else { "·" });
    let w = label.chars().count() as u16;
    let x = r.right().saturating_sub(w + 1);
    blank(buf, x.saturating_sub(1), r.y, w + 2, th.base());
    text(buf, x, r.y, w, &label, th.chip(th.accent));
    app.hit(Rect::new(x, r.y, w, 1), Hit::Action(a));
}

impl Side {
    /// "right of", "below"… for a title.
    pub fn word_beside(self) -> &'static str {
        match self {
            Side::Left => "left of",
            Side::Right => "right of",
            Side::Top => "above",
            Side::Bottom => "below",
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod paths;
