//! Free tiling in the running Client (D66): dragging a Pane by its title with a live preview, Move mode on the
//! keyboard, close and split, the Widget list for a new Pane, and what an edit does to the Design on screen (a
//! built-in Design is never overwritten: the first edit asks for a name; a Design of the user's own saves itself).
//! The tree edits themselves are pure functions in `tiling`.

use crate::app::{Action, App, Hit, Menu, MenuAct, Naming, Target};
use crate::design::DESIGNS;
use crate::layout::{Divider, Node, is_hole};
use crate::theme::Theme;
use crate::tiling::{CORE, Side, Zone, edge_ratio, neighbour, zone_at};
use crate::ui::{blank, centered, popup, text};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

/// A Pane held by its title: where the press was, where the pointer is, and (once it has moved far enough to be a
/// drag rather than a click) the drop zone under it and the layout that dropping there would make.
pub struct TileDrag {
    pub slot: String,
    from: (u16, u16),
    pub at: (u16, u16),
    pub live: bool,
    pub zone: Option<Zone>,
    pub preview: Option<Node>,
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
        self.tile = Some(TileDrag { slot, from: (x, y), at: (x, y), live: false, zone: None, preview: None });
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
        t.live = true;
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
        if let Some(tree) = t.preview {
            self.focus = t.slot;
            self.commit_layout(tree);
        }
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
        if self.divider_from.take().is_some_and(|before| before != self.layout) {
            self.edited();
        }
    }

    // ── edits and what they do to the Design ──────────────────────────

    /// Show `tree` as the layout and treat it as an edit.
    fn commit_layout(&mut self, tree: Node) {
        self.layout = tree;
        self.want_widget_data();
        self.edited();
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
    /// split (at the screen's edge if there is no neighbour), `+` `-` size it, `x` closes it, Enter or Esc ends.
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
            _ => {}
        }
    }

    fn move_step(&mut self, slot: &str, dir: Side, shift: bool) {
        let (body, aspect) = (self.pane_area(), self.cell_aspect());
        let slots = self.layout.compute_shaped(body, aspect).slots;
        let zone = match (neighbour(&slots, slot, dir), shift) {
            (Some(n), false) => Zone::Swap(n),
            (Some(n), true) => Zone::Beside(n, dir),
            (None, true) => {
                let Some(me) = slots.iter().find(|(s, _)| s == slot).map(|(_, r)| *r) else { return };
                Zone::Edge(dir, edge_ratio(dir, slot, me, body))
            }
            (None, false) => return self.say("Nothing that way · ⇧ moves it to the edge"),
        };
        match self.layout.moved(slot, &zone).filter(|c| *c != self.layout && self.fits(c)) {
            Some(tree) => {
                self.focus = slot.into();
                self.commit_layout(tree);
            }
            None => self.say("No room for it there"),
        }
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

/// The title that follows the pointer while a Pane is held: where it is, so popups and images can step aside.
pub fn ghost_rect(area: Rect, app: &App) -> Option<Rect> {
    let t = app.tile.as_ref().filter(|t| t.live)?;
    let label = format!(" ⠿ {} ", crate::ui::pane_name(app.design, &t.slot));
    let w = (label.chars().count() as u16).min(area.width);
    let x = (t.at.0 + 1).min(area.right().saturating_sub(w));
    let y = if t.at.1 + 1 < area.bottom() { t.at.1 + 1 } else { t.at.1.saturating_sub(1) };
    Some(Rect::new(x.max(area.x), y.max(area.y), w, 1))
}

pub fn draw_ghost(buf: &mut Buffer, area: Rect, app: &App, th: &Theme) {
    let (Some(r), Some(t)) = (ghost_rect(area, app), app.tile.as_ref()) else { return };
    let label = format!(" ⠿ {} ", crate::ui::pane_name(app.design, &t.slot));
    text(buf, r.x, r.y, r.width, &label, th.chip(if t.zone.is_some() { th.accent } else { th.dim }));
}

/// The bar's one line in Move mode.
pub fn draw_move_hint(buf: &mut Buffer, r: Rect, name: &str, th: &Theme) {
    blank(buf, r.x, r.y, r.width, th.base());
    let tag = format!(" MOVE {name} ");
    let w = tag.chars().count() as u16;
    text(buf, r.x + 1, r.y, w, &tag, th.chip(th.accent));
    text(buf, r.x + w + 2, r.y, r.width.saturating_sub(w + 3), "←↑↓→ swap · ⇧←↑↓→ split beside · + − size · x close · Enter done", th.s(th.dim));
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
