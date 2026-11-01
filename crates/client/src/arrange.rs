//! Free tiling in the running Client (D66): dragging a Pane by its title with a live preview, Move mode on the
//! keyboard, close and split, the Widget list for a new Pane, and what an edit does to the Design on screen (a
//! built-in Design is never overwritten: the first edit asks for a name; a Design of the user's own saves itself).
//! The tree edits themselves are pure functions in `tiling`; how Panes move on screen is `motion`, layout undo is
//! `history`.
//!
//! Undo and the Design files (owner, 2026-11-01): every committed edit (a drop, close, split, Move mode step, size
//! step, divider release, Reset design) can be undone, back to 50 edits, for the session. Undo restores the tree and
//! which Design it was, and the files follow: a Design of the user's gets the restored tree written to its file (and
//! one that a Reset dropped comes back). Undoing the first edit of a built-in Design returns to the built-in one: the
//! unnamed "<Design> (edited)" that edit made is removed again (no orphan), while a Design the user named stays in the
//! picker as saved (it was a deliberate save, like after Reset design). With the naming prompt still up, undo just
//! closes it: nothing was written yet. A state that no longer fits the window is refused with a notice.

pub mod history;
pub mod motion;

use crate::app::{Action, App, Hit, Menu, MenuAct, NameFor, Naming, Target};
use crate::design::DESIGNS;
use crate::layout::{Divider, MIN_H, MIN_W, Node, WELL_MIN_H, is_hole};
use crate::theme::{Rgb, Theme};
use crate::tiling::{CORE, Side, Zone, edge_ratio, neighbour, zone_at};
use crate::ui::{blank, centered, popup, text};
use motion::Tween;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::time::{Duration, Instant};

/// A held Pane floats at its own size, but never more than this share of the layout either way.
pub const LIFT_MAX: f32 = 0.45;
/// How long the bar offers "undo" after an edit.
pub const OFFER: Duration = Duration::from_secs(4);

/// A Pane held by its title: where the press was, where the pointer is, and (once it has moved far enough to be a
/// drag rather than a click) the drop zone under it and the layout that dropping there would make.
pub struct TileDrag {
    pub slot: String,
    from: (u16, u16),
    pub at: (u16, u16),
    pub live: bool,
    pub zone: Option<Zone>,
    pub preview: Option<Node>,
    /// the Pane's rectangle when it was picked up
    pub orig: Rect,
}

/// What undo goes back to: the layout, the Design it was (None: the built-in one) and what its file said about it,
/// whether the naming prompt of a first edit was up, and the focus.
#[derive(Clone, Debug)]
pub struct Snapshot {
    tree: Node,
    name: Option<String>,
    /// (name, design, edited) from the Design's file
    file: Option<(String, String, bool)>,
    forking: bool,
    focus: String,
}

/// Where a held Pane floats: its own size, at most `LIFT_MAX` of the layout either way (never under its minimum),
/// with the spot it was grabbed by under the pointer, and inside the layout with room for its shadow.
pub fn float_target(t: &TileDrag, body: Rect) -> Rect {
    let min_h = if is_hole(&t.slot) { WELL_MIN_H } else { MIN_H };
    let cap = |len: u16, total: u16, min: u16| len.min((total as f32 * LIFT_MAX) as u16).max(min).min(total.saturating_sub(1).max(1));
    let (w, h) = (cap(t.orig.width, body.width, MIN_W), cap(t.orig.height, body.height, min_h));
    // the grab point, scaled to the smaller Pane
    let gx = (t.from.0.saturating_sub(t.orig.x) as u32 * w as u32 / t.orig.width.max(1) as u32) as u16;
    let gy = t.from.1.saturating_sub(t.orig.y).min(h.saturating_sub(1));
    let x = t.at.0.saturating_sub(gx).min(body.right().saturating_sub(w + 1)).max(body.x);
    let y = t.at.1.saturating_sub(gy).min(body.bottom().saturating_sub(h + 1)).max(body.y);
    Rect::new(x, y, w, h)
}

/// A floating Pane with its shadow (a cell to the right and below), inside `area`.
pub fn with_shadow(r: Rect, area: Rect) -> Rect {
    Rect::new(r.x, r.y, r.width + 1, r.height + 1).intersection(area)
}

/// The Widget list for a new Pane: what to split, which side, and the Widgets that fit.
pub struct WidgetPick {
    pub target: String,
    pub side: Side,
    pub items: Vec<&'static str>,
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

    /// Whether `tree` could be shown at the size we draw at.
    fn fits(&self, tree: &Node) -> bool {
        tree.valid(self.pane_area(), self.cell_aspect())
    }

    /// The Pane to act on: the focused one, or Home if the focus is on something this layout hasn't got.
    pub fn focused_slot(&self) -> String {
        if self.layout.has(&self.focus) { self.focus.clone() } else { "home".into() }
    }

    /// The layout to draw: the one a drop would make while a Pane is held over a zone, else the real one.
    pub fn shown_layout(&self) -> std::borrow::Cow<'_, Node> {
        match self.tile.as_ref().and_then(|t| t.preview.as_ref()) {
            Some(p) => std::borrow::Cow::Borrowed(p),
            None => std::borrow::Cow::Borrowed(&self.layout),
        }
    }

    /// The Pane being moved this frame (held by the mouse, or in Move mode): drawn highlighted.
    pub fn moving(&self) -> Option<&str> {
        self.tile.as_ref().filter(|t| t.live).map(|t| t.slot.as_str()).or(self.move_mode.as_deref())
    }

    // ── the mouse ─────────────────────────────────────────────────────

    /// A press on a Pane's title bar: a click unless the pointer moves a few cells before the release.
    pub fn tile_down(&mut self, slot: String, x: u16, y: u16) {
        self.focus = slot.clone();
        let orig = self.layout.compute_shaped(self.pane_area(), self.cell_aspect()).slots.into_iter().find(|(s, _)| *s == slot).map(|(_, r)| r).unwrap_or_default();
        self.tile = Some(TileDrag { slot, from: (x, y), at: (x, y), live: false, zone: None, preview: None, orig });
    }

    /// The held Pane's rectangle this frame: lifted from its place (it shrinks to its floating size on the way) and
    /// following the pointer; after a release, flying into its slot. None when nothing floats.
    pub fn float_rect(&mut self, body: Rect, now: Instant) -> Option<Rect> {
        match self.tile.as_ref().filter(|t| t.live) {
            Some(t) => {
                let to = float_target(t, body);
                if !self.motion.on {
                    return Some(to);
                }
                let tw = self.motion.float.get_or_insert(Tween { from: t.orig, to, start: now });
                tw.follow(to);
                Some(motion::clamp(tw.at(now), body))
            }
            None if self.motion.landing.is_some() => self.motion.float_at(now),
            None => {
                self.motion.float = None;
                None
            }
        }
    }

    /// The held Pane let go (dropped or put back): it flies from where it floats into its place in the layout.
    fn land(&mut self, slot: &str) {
        let now = Instant::now();
        let from = self.motion.float.take().map(|t| t.at(now));
        self.motion.landing = None;
        let to = self.layout.compute_shaped(self.pane_area(), self.cell_aspect()).slots.into_iter().find(|(s, _)| s == slot).map(|(_, r)| r);
        if let (Some(from), Some(to), true) = (from, to, self.motion.on) {
            self.motion.float = Some(Tween { from, to, start: now });
            self.motion.landing = Some(slot.into());
        }
    }

    /// Esc while a Pane is held: it goes back where it was.
    pub fn tile_cancel(&mut self) {
        if let Some(t) = self.tile.take().filter(|t| t.live) {
            self.land(&t.slot);
        }
    }

    /// Mouse events while a Pane is held.
    pub fn tile_pointer(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::Drag(MouseButton::Left) => self.tile_move(m.column, m.row),
            MouseEventKind::Up(MouseButton::Left) => self.tile_up(),
            MouseEventKind::Up(_) => self.tile = None,
            _ => {}
        }
    }

    fn tile_move(&mut self, x: u16, y: u16) {
        let Some(t) = self.tile.as_mut() else { return };
        if !t.live && x.abs_diff(t.from.0) < 3 && y.abs_diff(t.from.1) < 2 {
            return;
        }
        if !t.live {
            // a fresh lift: whatever was still landing is down
            t.live = true;
            self.motion.float = None;
            self.motion.landing = None;
        }
        let Some(t) = self.tile.as_mut() else { return };
        t.at = (x, y);
        let slot = t.slot.clone();
        let (body, aspect) = (self.pane_area(), self.cell_aspect());
        // zones come from the layout as it stands, not the preview: what's under the pointer can't change because
        // of what the preview did
        let slots = self.layout.compute_shaped(body, aspect).slots;
        let zone = zone_at(&slots, body, &slot, x, y);
        let preview = zone.as_ref().and_then(|z| self.layout.moved(&slot, z)).filter(|c| *c != self.layout && self.fits(c));
        if let Some(t) = self.tile.as_mut() {
            // a zone that wouldn't fit isn't offered: nothing highlighted, the original layout shown
            t.zone = zone.filter(|_| preview.is_some());
            t.preview = preview;
        }
    }

    fn tile_up(&mut self) {
        let Some(t) = self.tile.take() else { return };
        if !t.live {
            // a click on the title: the focus moved at the press; the hole's Pane also changes its look
            if is_hole(&t.slot) {
                self.run(Action::NextLook);
            }
            return;
        }
        // checked again: the terminal can have changed size since the last move
        if let Some(tree) = t.preview.filter(|c| self.fits(c)) {
            self.focus = t.slot.clone();
            self.commit_layout(tree);
        }
        self.land(&t.slot);
    }

    /// A divider dragged to `pos`, unless that would leave a Pane too small (or the well out of shape).
    pub fn drag_divider(&mut self, d: &Divider, pos: u16) {
        let mut t = self.layout.clone();
        t.drag(d, pos);
        if self.fits(&t) || !self.fits(&self.layout) {
            self.layout = t;
        }
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
    fn commit_layout(&mut self, tree: Node) {
        let was = self.snapshot();
        self.layout = tree;
        self.remember(was);
        self.want_widget_data();
        self.edited();
    }

    fn snapshot(&self) -> Snapshot {
        let file = self.layout_name.as_ref().and_then(|id| self.saved_layouts.iter().find(|s| s.id == *id)).map(|s| (s.name.clone(), s.design.clone(), s.edited));
        let forking = self.naming.as_ref().is_some_and(|n| n.purpose == NameFor::Fork);
        Snapshot { tree: self.layout.clone(), name: self.layout_name.clone(), file, forking, focus: self.focus.clone() }
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
        // a held Pane is put back first; the keyboard and the bar never reach here mid-drag, but be sure
        self.tile = None;
        let (area, aspect) = (self.pane_area(), self.cell_aspect());
        let now = self.snapshot();
        let fits = |s: &Snapshot| s.tree.valid(area, aspect);
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
        if self.naming.as_ref().is_some_and(|n| n.purpose == NameFor::Fork) {
            self.naming = None;
        }
        self.layout = s.tree;
        self.focus = if self.layout.has(&s.focus) { s.focus } else { "home".into() };
        if self.move_mode.as_ref().is_some_and(|m| !self.layout.has(m)) {
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
                if self.layouts_dir.is_some() && self.naming.is_none() {
                    let name = self.suggest_name();
                    self.naming = Some(Naming::fork(name));
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

    pub fn close_pane(&mut self, slot: &str) {
        if !self.layout.has(slot) {
            return;
        }
        let name = crate::ui::pane_name(self.design, slot);
        if CORE.contains(&slot) {
            return self.say(format!("{name} can be moved, but never closed"));
        }
        let Some(tree) = self.layout.closed(slot).filter(|t| self.fits(t)) else {
            return self.say(format!("Closing {name} would squash the others"));
        };
        if self.focus == slot {
            self.focus = "home".into();
        }
        if self.move_mode.as_deref() == Some(slot) {
            self.move_mode = None;
        }
        self.commit_layout(tree);
        self.say(format!("Closed {name}"));
    }

    pub fn split_focused(&mut self, side: Side) {
        let target = self.focused_slot();
        self.split_pane(&target, side);
    }

    /// Open the Widget list for a new Pane beside `target`: Widgets not on screen, and only those that fit there.
    pub fn split_pane(&mut self, target: &str, side: Side) {
        let holeless = !self.layout.has("well") && !self.layout.has("hole");
        let mut items: Vec<&'static str> = crate::layouts::all_widgets().chain(if holeless { ["well", "hole"].into_iter().collect::<Vec<_>>() } else { vec![] }).collect();
        items.dedup();
        items.retain(|w| !self.layout.has(w) && self.layout.beside(target, w, side).is_some_and(|t| self.fits(&t)));
        if items.is_empty() {
            let name = crate::ui::pane_name(self.design, target);
            return self.say(format!("No room to split {name} here"));
        }
        self.widget_pick = Some(WidgetPick { target: target.into(), side, items, sel: 0 });
    }

    pub fn widget_key(&mut self, k: KeyEvent) {
        let Some(p) = self.widget_pick.as_mut() else { return };
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
            KeyCode::Esc | KeyCode::Char('q') => self.widget_pick = None,
            _ => {}
        }
    }

    pub fn widget_chosen(&mut self, i: usize) {
        let Some(p) = self.widget_pick.take() else { return };
        let Some(w) = p.items.get(i).copied() else { return };
        match self.layout.beside(&p.target, w, p.side).filter(|t| self.fits(t)) {
            Some(tree) => {
                self.focus = w.into();
                self.commit_layout(tree);
            }
            None => self.say("No room for it there"),
        }
    }

    pub fn open_pane_menu(&mut self, slot: String, at: Option<(u16, u16)>) {
        let mut items = vec![("Split right".to_string(), MenuAct::SplitRight), ("Split down".into(), MenuAct::SplitDown), ("Move pane".into(), MenuAct::MovePane)];
        if !CORE.contains(&slot.as_str()) {
            items.push(("Close pane".into(), MenuAct::ClosePane));
        }
        self.focus = slot.clone();
        self.menu = Some(Menu { at: at.unwrap_or((0, 0)), target: Target::Pane(slot), items, selected: 0 });
    }

    pub fn pane_menu_run(&mut self, slot: &str, act: MenuAct) {
        self.focus = slot.into();
        match act {
            MenuAct::SplitRight => self.split_pane(slot, Side::Right),
            MenuAct::SplitDown => self.split_pane(slot, Side::Bottom),
            MenuAct::MovePane => self.start_move(),
            MenuAct::ClosePane => self.close_pane(slot),
            _ => {}
        }
    }

    // ── Move mode ─────────────────────────────────────────────────────

    pub fn start_move(&mut self) {
        let slot = self.focused_slot();
        if self.layout.slot_names().len() < 2 {
            return self.say("Nothing to move it around");
        }
        self.move_mode = Some(slot);
    }

    /// Keys in Move mode: arrows swap with the neighbour that way, Shift+arrows put the Pane on that side of it as a
    /// split (at the screen's edge if there is no neighbour), `+` `-` size it, `x` closes it, `u` undoes (`U`
    /// redoes), Enter or Esc ends. What can't be done is said in the bar, and nothing changes.
    pub fn move_key(&mut self, k: KeyEvent) {
        let Some(slot) = self.move_mode.clone().filter(|s| self.layout.has(s)) else {
            self.move_mode = None;
            return;
        };
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Esc | KeyCode::Enter => self.move_mode = None,
            KeyCode::Left => self.move_step(&slot, Side::Left, shift),
            KeyCode::Right => self.move_step(&slot, Side::Right, shift),
            KeyCode::Up => self.move_step(&slot, Side::Top, shift),
            KeyCode::Down => self.move_step(&slot, Side::Bottom, shift),
            KeyCode::Char('+' | '=') => self.resize_step(&slot, true),
            KeyCode::Char('-' | '_') => self.resize_step(&slot, false),
            KeyCode::Char('x' | 'X') => self.close_pane(&slot),
            KeyCode::Char('u') => self.undo_layout(),
            KeyCode::Char('U') => self.redo_layout(),
            _ => {}
        }
    }

    /// One step that way: a swap with the neighbour, or (Shift) on its far side as a split, or with nothing that
    /// way, a whole column / row at the screen's edge. Refused, with the reason, if it wouldn't fit or would only
    /// change sizes (a Pane already spanning that edge).
    fn move_step(&mut self, slot: &str, dir: Side, shift: bool) {
        let (body, aspect) = (self.pane_area(), self.cell_aspect());
        let slots = self.layout.compute_shaped(body, aspect).slots;
        let name = crate::ui::pane_name(self.design, slot);
        let zone = match (neighbour(&slots, slot, dir), shift) {
            (Some(n), false) => Zone::Swap(n),
            (Some(n), true) => Zone::Beside(n, dir),
            (None, true) => {
                let Some(me) = slots.iter().find(|(s, _)| s == slot).map(|(_, r)| *r) else { return };
                Zone::Edge(dir, edge_ratio(dir, slot, me, body))
            }
            (None, false) => return self.say(format!("Nothing {} {name} · ⇧{} makes it a whole {}", dir.word_beside(), dir.arrow(), dir.line())),
        };
        let Some(tree) = self.layout.moved_any(slot, &zone) else { return self.say("It can't go there") };
        if tree.same_arrangement(&self.layout) {
            return self.say(format!("{name} already spans the {} edge", dir.edge()));
        }
        if !self.fits(&tree) {
            return self.say(match &zone {
                Zone::Beside(n, _) => format!("No room for {name} {} {} · {} swaps", dir.word_beside(), crate::ui::pane_name(self.design, n), dir.arrow()),
                Zone::Edge(..) => format!("No room for {name} as a whole {}", dir.line()),
                Zone::Swap(n) => format!("{name} won't fit where {} is", crate::ui::pane_name(self.design, n)),
            });
        }
        self.focus = slot.into();
        self.commit_layout(tree);
    }

    fn resize_step(&mut self, slot: &str, grow: bool) {
        match self.layout.resized(slot, grow, self.pane_area()).filter(|t| self.fits(t)) {
            Some(tree) => self.commit_layout(tree),
            None => self.say(if grow { "Can't grow it more" } else { "Can't shrink it more" }),
        }
    }
}

// ── drawing ───────────────────────────────────────────────────────────

pub fn widgets_rect(area: Rect, p: &WidgetPick) -> Rect {
    centered(area, 44, (p.items.len() as u16 + 4).min(area.height))
}

/// The list of Widgets a new Pane can show.
pub fn draw_widgets(buf: &mut Buffer, area: Rect, app: &mut App, th: &Theme) {
    let Some(p) = app.widget_pick.as_ref() else { return };
    let r = widgets_rect(area, p);
    let inner = popup(buf, r, th, &format!("New pane {} {}", p.side.word_beside(), crate::ui::pane_name(app.design, &p.target)));
    let (items, sel) = (p.items.clone(), p.sel);
    let rows = inner.height.saturating_sub(1).max(1) as usize;
    let first = sel.saturating_sub(rows - 1).min(items.len().saturating_sub(rows));
    for (i, w) in items.iter().enumerate().skip(first).take(rows) {
        let y = inner.y + (i - first) as u16;
        let bg = if i == sel { th.selection.color() } else { th.bg.color() };
        blank(buf, inner.x, y, inner.width, th.base().bg(bg));
        text(buf, inner.x + 1, y, 2, if i == sel { "›" } else { " " }, th.s(th.accent).bg(bg));
        text(buf, inner.x + 3, y, 24, &crate::ui::pane_name(app.design, w), (if i == sel { th.title() } else { th.s(th.fg) }).bg(bg));
        text(buf, inner.x + 28, y, inner.width.saturating_sub(29), w, th.s(th.dim).bg(bg));
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::WidgetItem(i));
    }
    text(buf, inner.x + 1, inner.bottom().saturating_sub(1), inner.width.saturating_sub(2), "↑↓ choose · Enter add · Esc cancel", th.s(th.dim));
}

/// The empty slot a held Pane leaves where it would land: a dashed outline, tinted, saying so where the floating Pane
/// (`float`) doesn't hide it. `ok`: a drop there changes the layout; otherwise this is where it goes back to.
pub fn draw_slot(buf: &mut Buffer, r: Rect, ok: bool, float: Option<Rect>, th: &Theme) {
    let col = if ok { th.accent } else { th.dim };
    let tint = mix(th.bg, col, if ok { 0.12 } else { 0.05 });
    for y in r.top()..r.bottom() {
        blank(buf, r.x, y, r.width, th.base().bg(tint));
    }
    if r.width < 2 || r.height < 2 {
        return;
    }
    let st = th.s(col).bg(tint);
    let mut put = |x: u16, y: u16, c: char| {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(c).set_style(st);
        }
    };
    for x in r.x + 1..r.right() - 1 {
        put(x, r.y, '╌');
        put(x, r.bottom() - 1, '╌');
    }
    for y in r.y + 1..r.bottom() - 1 {
        put(r.x, y, '╎');
        put(r.right() - 1, y, '╎');
    }
    put(r.x, r.y, '╭');
    put(r.right() - 1, r.y, '╮');
    put(r.x, r.bottom() - 1, '╰');
    put(r.right() - 1, r.bottom() - 1, '╯');
    let label = if ok { "drop here" } else { "back here" };
    let w = label.chars().count() as u16;
    if r.height >= 3 && r.width >= w + 4 {
        // the middle, else the spot nearest it the floating Pane leaves free (its middle, then its left and right)
        let xs = [r.x + (r.width - w) / 2, r.x + 2, r.right() - w - 2];
        let spots = (r.y + 1..r.bottom() - 1).flat_map(|y| xs.iter().enumerate().map(move |(k, &x)| ((y.abs_diff(r.y + r.height / 2), k), x, y)));
        let free = |x: u16, y: u16| float.is_none_or(|f| Rect::new(x, y, w, 1).intersection(with_shadow(f, f.union(r))).is_empty());
        if let Some((_, x, y)) = spots.filter(|&(_, x, y)| free(x, y)).min() {
            text(buf, x, y, w, label, th.s(col).bg(tint));
        }
    }
}

/// A floating Pane's shadow: the cell column to its right and the row below it, darkened.
pub fn draw_shadow(buf: &mut Buffer, r: Rect, area: Rect, th: &Theme) {
    let dark = |c: Color, base: Rgb| {
        let Rgb(r, g, b) = match c {
            Color::Rgb(r, g, b) => Rgb(r, g, b),
            _ => base,
        };
        // on a black background a darker black is no shadow: lift it a little instead, toward the theme's border
        if (r as u16 + g as u16 + b as u16) < 40 { mix(Rgb(r, g, b), th.border, 0.35) } else { Color::Rgb(r / 3, g / 3, b / 3) }
    };
    let cells = (r.y + 1..=r.bottom()).map(|y| (r.right(), y)).chain((r.x + 1..r.right()).map(|x| (x, r.bottom())));
    for (x, y) in cells {
        if x >= area.right() || y >= area.bottom() {
            continue;
        }
        if let Some(c) = buf.cell_mut((x, y)) {
            let (fg, bg) = (c.fg, c.bg);
            c.set_bg(dark(bg, th.bg));
            if let Color::Rgb(r, g, b) = fg {
                c.set_fg(Color::Rgb(r / 3, g / 3, b / 3));
            }
        }
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Color {
    let m = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color::Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// The bar's one line in Move mode: its keys, or what the last key couldn't do.
pub fn draw_move_hint(buf: &mut Buffer, r: Rect, name: &str, said: Option<&str>, th: &Theme) {
    blank(buf, r.x, r.y, r.width, th.base());
    let tag = format!(" MOVE {name} ");
    let w = tag.chars().count() as u16;
    text(buf, r.x + 1, r.y, w, &tag, th.chip(th.accent));
    let (line, st) = match said {
        Some(m) => (m, th.s(th.accent2)),
        None => ("←↑↓→ swap · ⇧←↑↓→ split beside · + − size · x close · u undo · Enter done", th.s(th.dim)),
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
    fn arrow(self) -> &'static str {
        match self {
            Side::Left => "←",
            Side::Right => "→",
            Side::Top => "↑",
            Side::Bottom => "↓",
        }
    }
    /// "column" or "row": what a Pane at this screen edge becomes.
    fn line(self) -> &'static str {
        if self.axis() == crate::layout::Axis::Horizontal { "column" } else { "row" }
    }
    fn edge(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Top => "top",
            Side::Bottom => "bottom",
        }
    }
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
