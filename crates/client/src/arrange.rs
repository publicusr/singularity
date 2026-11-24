//! Arranging in the running Client (D87, ADR 0005): a title drag that moves a Widget onto another Pane, Move mode on
//! the keyboard, close and split, the Widget list for a new Pane, and what an edit does to the Design on screen (a
//! built-in Design is never overwritten: the first edit asks for a name; a Design of the user's own saves itself).
//! The edits themselves are `Arrangement`'s; layout undo is `history`.
//!
//! A drag only moves contents: while a Widget is held nothing on screen moves but a highlight on the Pane under the
//! pointer (at once, no dwell, saying what a release would do) and a chip with the Widget's name by the pointer. The
//! release drops on the Pane under the pointer: the two Widgets swap (or an empty Pane is filled). On its own Pane,
//! off the Panes, on Esc, or when the release was never heard, the drag ends and nothing changes. Every drop is
//! allowed: a swap never changes the floor plan.
//!
//! Undo and the Design files (owner, 2026-11-01): every committed edit (a drop, close, split, Move mode step, size
//! step, divider release, Reset design) can be undone, back to 50 edits, for the session. Undo restores the
//! Arrangement and which Design it was, and the files follow: a Design of the user's gets the restored tree written
//! to its file (and one that a Reset dropped comes back). Undoing the first edit of a built-in Design returns to the
//! built-in one: the unnamed "<Design> (edited)" that edit made is removed again (no orphan), while a Design the user
//! named stays in the picker as saved (it was a deliberate save, like after Reset design). With the naming prompt
//! still up, undo just closes it: nothing was written yet. A state whose floor plan no longer fits the window is
//! refused with a notice.

pub mod history;

use crate::app::{Action, App, Hit, Menu, MenuAct, NameFor, Naming, Target};
use crate::design::DESIGNS;
use crate::arrangement::{Arrangement, Refused};
use crate::layout::{Divider, Pane, PaneId, Widget, is_hole};
use crate::theme::Theme;
use crate::tiling::{CORE, Side, neighbour};
use crate::ui::{blank, centered, popup, text};
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

/// A Widget held by its Pane's title: where the press was, where the pointer is, and (once it has moved far enough to
/// be a drag rather than a click) the Pane under the pointer, if it is another one: the one a release drops on.
pub struct TileDrag {
    /// the Pane picked up (its Widget is what moves)
    pub pane: Pane,
    from: (u16, u16),
    pub at: (u16, u16),
    pub live: bool,
    /// the Pane under the pointer, other than its own: highlighted, and where a release drops
    pub over: Option<PaneId>,
}

/// What undo goes back to: the layout, the Design it was (None: the built-in one) and what its file said about it,
/// whether the naming prompt of a first edit was up, and the focus.
#[derive(Clone, Debug)]
pub struct Snapshot {
    tree: Arrangement,
    name: Option<String>,
    /// (name, design, edited) from the Design's file
    file: Option<(String, String, bool)>,
    forking: bool,
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

/// The Widget list for a new Pane: what to split, which side, and the Widgets that fit.
pub struct WidgetPick {
    pub target: PaneId,
    pub side: Side,
    pub items: Vec<Widget>,
    pub sel: usize,
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
        self.tile = Some(TileDrag { pane, from: (x, y), at: (x, y), live: false, over: None });
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
    /// and the Pane under it is the one highlighted, at once.
    fn tile_move(&mut self, x: u16, y: u16) {
        let Some(t) = self.tile.as_ref() else { return };
        if !t.live && (x.abs_diff(t.from.0) < 3 && y.abs_diff(t.from.1) < 2 || t.pane.widget.is_none()) {
            return;
        }
        let own = t.pane.id;
        let over = self.pane_under(x, y).filter(|p| *p != own);
        if let Some(t) = self.tile.as_mut() {
            t.live = true;
            t.at = (x, y);
            t.over = over;
        }
    }

    /// The terminal took a new size (`w` × `h`): the Panes' area is known at once, so a release in the same batch of
    /// input finds the Pane under it in the new window, not the old (#40 review).
    pub fn window_resized(&mut self, w: u16, h: u16) {
        // the top bar above, the player (3 rows) and the hints below: as `ui::draw` lays them out
        self.body = Rect::new(0, 1, w, h.saturating_sub(5));
        self.tile_resized();
    }

    /// The window changed size while a Widget is held: the Panes moved under the pointer, so the one under it is found
    /// again (#21: a release dropped on a Pane of the old window).
    pub fn tile_resized(&mut self) {
        let Some((own, (x, y))) = self.tile.as_ref().filter(|t| t.live).map(|t| (t.pane.id, t.at)) else { return };
        let over = self.pane_under(x, y).filter(|p| *p != own);
        if let Some(t) = self.tile.as_mut() {
            t.over = over;
        }
    }

    /// Let go at (x, y): on another Pane the two Widgets swap (an empty one is filled); anywhere else nothing changes.
    fn tile_up(&mut self, x: u16, y: u16) {
        let Some(t) = self.tile.as_ref() else { return };
        if !t.live {
            let (id, from, w) = (t.pane.id, t.from, t.pane.widget);
            self.tile = None;
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
        let Some(to) = t.over else { return };
        match self.layout.swap(t.pane.id, to) {
            Ok(a) => {
                self.focus = Some(to);
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
            _ if self.tile.is_none() && self.modal().is_none() => self.grip_hover,
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
    pub(crate) fn commit_layout(&mut self, tree: Arrangement) {
        let was = self.snapshot();
        self.layout = tree;
        self.remember(was);
        self.want_widget_data();
        self.edited();
    }

    fn snapshot(&self) -> Snapshot {
        let file = self.layout_name.as_ref().and_then(|id| self.saved_layouts.iter().find(|s| s.id == *id)).map(|s| (s.name.clone(), s.design.clone(), s.edited));
        let forking = self.naming().is_some_and(|n| n.purpose == NameFor::Fork);
        Snapshot { tree: self.layout.clone(), name: self.layout_name.clone(), file, forking, focus: self.focus }
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
        // the first edit's naming prompt: nothing was written yet, and the edit is going
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
                // redoing a first edit asks for its name again
                if s.forking {
                    self.edited();
                }
            }
        }
    }

    /// The layout was edited. A Design of the user's own (or the unnamed edit of a built-in one) follows in its file;
    /// the first edit of a built-in Design asks what to call the Design it makes.
    pub fn edited(&mut self) {
        let own = self.layout_name.clone().and_then(|id| self.saved_layouts.iter().find(|s| s.id == id).cloned());
        match own {
            Some(s) => self.write_layout(&s.id, &s.name, &s.design, s.edited),
            None => {
                self.layout_name = None;
                if self.layouts_dir.is_some() && self.naming().is_none() {
                    let name = self.suggest_name();
                    self.open_popup(crate::app::Popup::Naming(Naming::fork(name)));
                }
            }
        }
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
        self.commit_layout(tree);
        self.say(format!("Closed {name}"));
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

    /// Open the Widget list for a new Pane beside `target`: Widgets not on screen, and only those that fit there.
    pub fn split_pane(&mut self, target: PaneId, side: Side) {
        let holeless = !self.layout.has(Widget::Well) && !self.layout.has(Widget::Hole);
        let mut items: Vec<Widget> = crate::layouts::all_widgets().chain(if holeless { vec![Widget::Well, Widget::Hole] } else { vec![] }).collect();
        items.dedup();
        let area = self.pane_area();
        items.retain(|w| self.layout.split(target, side, Some(*w), area).is_ok());
        if items.is_empty() {
            let name = self.layout.get(target).map(|p| self.name_of(p)).unwrap_or_default();
            return self.say(format!("No room to split {name} here"));
        }
        self.open_popup(crate::app::Popup::Widgets(WidgetPick { target, side, items, sel: 0 }));
    }

    pub fn widget_key(&mut self, k: KeyEvent) {
        let Some(p) = self.widget_pick_mut() else { return };
        let n = p.items.len();
        match k.code {
            KeyCode::Up => p.sel = p.sel.saturating_sub(1),
            KeyCode::Down => p.sel = (p.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Home => p.sel = 0,
            KeyCode::End => p.sel = n.saturating_sub(1),
            KeyCode::Enter => {
                let i = p.sel;
                self.widget_chosen(i);
            }
            KeyCode::Esc | KeyCode::Char('q') => self.close_if_open(crate::app::Modal::Widgets),
            _ => {}
        }
    }

    pub fn widget_chosen(&mut self, i: usize) {
        let Some(p) = self.take_widget_pick() else { return };
        let Some(w) = p.items.get(i).copied() else { return };
        let area = self.pane_area();
        match self.layout.split(p.target, p.side, Some(w), area) {
            Ok(tree) => {
                self.focus = tree.pane_of(w);
                self.commit_layout(tree);
            }
            Err(_) => self.say("No room for it there"),
        }
    }

    /// The Pane's menu (D87): Replace widget… first (not on Home's), then the floor plan's edits.
    pub fn open_pane_menu(&mut self, id: PaneId, at: Option<(u16, u16)>) {
        let mut items = vec![];
        if self.can_replace(id) {
            items.push(("Replace widget…".to_string(), MenuAct::ReplaceWidget));
        }
        items.extend([("Split right".to_string(), MenuAct::SplitRight), ("Split down".into(), MenuAct::SplitDown)]);
        if !self.layout.widget_in(id).is_some_and(|w| CORE.contains(&w)) {
            items.push(("Close pane".into(), MenuAct::ClosePane));
        }
        // an empty Pane has nothing to move
        if self.layout.widget_in(id).is_some() {
            items.push(("Move pane".into(), MenuAct::MovePane));
        }
        self.focus = Some(id);
        self.open_popup(crate::app::Popup::Menu(Menu { at: at.unwrap_or((0, 0)), target: Target::Pane(id), items, selected: 0 }));
    }

    pub fn pane_menu_run(&mut self, id: PaneId, act: MenuAct) {
        self.focus = Some(id);
        match act {
            MenuAct::ReplaceWidget => self.open_widget_picker(id),
            MenuAct::SplitRight => self.split_pane(id, Side::Right),
            MenuAct::SplitDown => self.split_pane(id, Side::Bottom),
            MenuAct::MovePane => self.start_move(),
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

pub fn widgets_rect(area: Rect, p: &WidgetPick) -> Rect {
    centered(area, 44, (p.items.len() as u16 + 4).min(area.height))
}

/// The list of Widgets a new Pane can show.
pub fn draw_widgets(buf: &mut Buffer, area: Rect, app: &mut App, th: &Theme) {
    let Some(p) = app.widget_pick() else { return };
    let r = widgets_rect(area, p);
    let target = app.layout.get(p.target).map(|t| app.name_of(t)).unwrap_or_default();
    let inner = popup(buf, r, th, &format!("New pane {} {target}", p.side.word_beside()));
    let (items, sel) = (p.items.clone(), p.sel);
    let rows = inner.height.saturating_sub(1).max(1) as usize;
    let first = crate::scroll::window(sel, rows, items.len());
    for (i, w) in items.iter().enumerate().skip(first).take(rows) {
        let y = inner.y + (i - first) as u16;
        let bg = if i == sel { th.selection.color() } else { th.bg.color() };
        blank(buf, inner.x, y, inner.width, th.base().bg(bg));
        text(buf, inner.x + 1, y, 2, if i == sel { "›" } else { " " }, th.s(th.accent).bg(bg));
        text(buf, inner.x + 3, y, 24, &crate::ui::pane_name(app.design, *w), (if i == sel { th.title() } else { th.s(th.fg) }).bg(bg));
        text(buf, inner.x + 28, y, inner.width.saturating_sub(29), w.name(), th.s(th.dim).bg(bg));
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::WidgetItem(i));
    }
    text(buf, inner.x + 1, inner.bottom().saturating_sub(1), inner.width.saturating_sub(2), "↑↓ choose · Enter add · Esc cancel", th.s(th.dim));
}

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
    fn word_beside(self) -> &'static str {
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
