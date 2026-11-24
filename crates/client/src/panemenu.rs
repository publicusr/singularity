//! The Widget picker (D87): the palette-style list that chooses what a Pane shows. Replace widget… opens it (from the
//! Pane's menu on a right-click, a click on the Widget's name in its title, a key or the palette). Type to filter,
//! scroll like any list. Widgets not on screen come first; those on screen are dimmed below a line, and picking one
//! swaps the two Panes' Widgets (the Panes stay where they are: only what they show changes). Each Widget is on screen
//! at most once, Home's Pane has no Replace widget (Home is the only Required Widget: the library, the queue and lyrics
//! may be replaced away, and the first two then open as popups from the palette), and only what the Design's rules
//! allow today is offered: one black hole at a time. Every other Widget is offered whatever the Pane's size (ADR 0005:
//! every drop is allowed); one bigger than the Pane says so on its row, `needs W × H`, the line it would show there
//! (#36).
//!
//! The picker names a Pane, not a Widget, so it fills an empty one too: a split (← → ↑ ↓, #39) makes an empty Pane and
//! opens the picker on it at once; Esc leaves it empty, and the Pane says so.
//!
//! A Replace or a swap changes only the contents, never a Pane's outline, the gravity well's included (#48: it
//! letterboxes its picture in whatever Pane it gets).

use crate::app::{App, Hit, Modal, Popup};
use crate::layout::{PaneId, Widget, is_hole};
use crate::theme::Theme;
use crate::ui::{blank, centered, popup, text};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// The Widget picker over Pane `target`: what was typed, and the highlighted row.
pub struct WidgetPicker {
    pub target: PaneId,
    pub query: String,
    pub sel: usize,
    /// opened by a split on its new Pane: the pick is part of that edit (one undo step for both)
    pub after_split: bool,
}

/// One row of the Widget picker: a Widget, and the Pane showing it now if it's on screen (picking it swaps).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Choice {
    pub widget: Widget,
    pub from: Option<PaneId>,
}

/// At most this many rows show at once; the list scrolls with its highlight.
const ROWS: usize = 16;

impl App {
    /// Whether Pane `id` offers Replace widget… (Home's doesn't: the main Pane keeps Home).
    pub fn can_replace(&self, id: PaneId) -> bool {
        self.layout.get(id).is_some_and(|p| p.widget != Some(Widget::Home))
    }

    /// Open the Widget picker on Pane `id` (Home's says why not).
    pub fn open_widget_picker(&mut self, id: PaneId) {
        if !self.layout.contains(id) {
            return;
        }
        self.focus = Some(id);
        if !self.can_replace(id) {
            return self.say("Home stays in its Pane · drag its title to move it");
        }
        self.open_popup(Popup::WidgetPicker(WidgetPicker { target: id, query: String::new(), sel: 0, after_split: false }));
    }

    /// Replace widget… on the focused Pane (the key and the palette).
    pub fn replace_focused(&mut self) {
        if let Some(id) = self.focused_pane() {
            self.open_widget_picker(id);
        }
    }

    /// The open picker's rows, filtered by what was typed: the Widgets not on screen first, then those on screen.
    pub fn widget_choices(&self) -> Vec<Choice> {
        let Some(p) = self.widget_picker() else { return vec![] };
        let q = p.query.trim().to_lowercase();
        let named = |c: &Choice| format!("{} {}", crate::ui::pane_name(self.design, c.widget), c.widget.name()).to_lowercase();
        let mut rows: Vec<Choice> = self.choices_for(p.target).into_iter().filter(|c| crate::app::fuzzy(&named(c), &q)).collect();
        // within each part: what you typed at the start first, then anywhere, then loose matches
        rows.sort_by_key(|c| {
            let l = named(c);
            (c.from.is_some(), if l.starts_with(&q) { 0 } else if l.contains(&q) { 1 } else { 2 })
        });
        rows
    }

    /// What Pane `target` could show instead of what it shows, unfiltered: the Widgets off screen it may take, then the
    /// on-screen ones it may trade with, whatever the Pane's size.
    pub fn choices_for(&self, target: PaneId) -> Vec<Choice> {
        let Some(here) = self.layout.get(target) else { return vec![] };
        let mine = here.widget;
        let shown = self.layout.widgets();
        // a Required Widget stays on screen (D87: Home, whose Pane has no picker anyway): its Pane could only trade
        // places, not let it go
        let keeps = crate::arrangement::required(mine);
        // one black hole at a time: the Pane holding it may take the other
        let hole_free = !shown.iter().any(|w| is_hole(Some(*w)) && Some(*w) != mine);
        let mut off: Vec<Widget> = vec![];
        if !keeps {
            let extra = [Widget::Library, Widget::Queue, Widget::Lyrics].into_iter().chain(if hole_free { vec![Widget::Well, Widget::Hole] } else { vec![] });
            for w in crate::layouts::all_widgets().chain(extra) {
                if !shown.contains(&w) && Some(w) != mine && !off.contains(&w) {
                    off.push(w);
                }
            }
        }
        let off = off.into_iter().filter(|w| self.layout.replace(target, *w).is_ok()).map(|widget| Choice { widget, from: None });
        let on = self.layout.panes().into_iter().filter(|p| p.id != target).filter_map(|p| Some(Choice { widget: p.widget?, from: Some(p.id) }));
        off.chain(on).collect()
    }

    /// Where Pane `id` is drawn (at the last frame's size).
    fn pane_rect(&self, id: PaneId) -> Option<Rect> {
        self.layout.compute(self.pane_area()).panes.into_iter().find(|(p, _)| p.id == id).map(|(_, r)| r)
    }

    /// Close the picker without a pick (Esc, a click outside). A split's Pane stays empty, and Arrange mode is on, so
    /// it can be filled by dragging something in (D87).
    pub fn widget_picker_cancel(&mut self) {
        let split = self.widget_picker().is_some_and(|p| p.after_split);
        self.close_if_open(Modal::WidgetPicker);
        if split {
            self.enter_arrange();
        }
    }

    /// The picker's Pane went (a Design reloaded, a resize, an undo): the picker closes instead of offering Widgets
    /// for nothing. True if it did.
    pub fn picker_lost_its_pane(&mut self) -> bool {
        let gone = self.widget_picker().is_some_and(|p| !self.layout.contains(p.target));
        if gone {
            self.close_if_open(Modal::WidgetPicker);
        }
        gone
    }

    /// Pick row `i`: the Widget goes into the picker's Pane (from its own Pane, if it's on screen: they swap).
    pub fn widget_picked(&mut self, i: usize) {
        if self.picker_lost_its_pane() {
            return;
        }
        let Some(c) = self.widget_choices().get(i).copied() else { return };
        let Some(p) = self.take_widget_picker() else { return };
        let tree = match c.from {
            Some(other) => self.layout.swap(p.target, other),
            None => self.layout.replace(p.target, c.widget),
        };
        let tree = match tree {
            Ok(t) => t,
            Err(e) => return self.refused(e),
        };
        self.focus = Some(p.target);
        if p.after_split { self.amend_layout(tree) } else { self.commit_layout(tree) }
    }

    /// Keys in the Widget picker: typing filters, ↑↓ (and PgUp PgDn) move, Enter picks, Esc cancels.
    pub fn widget_picker_key(&mut self, k: KeyEvent) {
        if self.picker_lost_its_pane() {
            return;
        }
        let n = self.widget_choices().len();
        let Some(p) = self.widget_picker_mut() else { return };
        match k.code {
            KeyCode::Esc => self.widget_picker_cancel(),
            KeyCode::Up => p.sel = p.sel.saturating_sub(1),
            KeyCode::Down => p.sel = (p.sel + 1).min(n.saturating_sub(1)),
            KeyCode::PageUp => p.sel = p.sel.saturating_sub(ROWS),
            KeyCode::PageDown => p.sel = (p.sel + ROWS).min(n.saturating_sub(1)),
            KeyCode::Backspace => {
                p.query.pop();
                p.sel = 0;
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) && p.query.chars().count() < 40 => {
                p.query.push(c);
                p.sel = 0;
            }
            KeyCode::Enter => {
                let i = p.sel;
                self.widget_picked(i);
            }
            _ => {}
        }
    }
}

/// The rows the list draws: each choice, and the line before the first on-screen one (where there are both).
fn lines(rows: &[Choice]) -> Vec<Option<usize>> {
    let mut out = vec![];
    for (i, c) in rows.iter().enumerate() {
        if c.from.is_some() && (i == 0 || rows[i - 1].from.is_none()) {
            out.push(None);
        }
        out.push(Some(i));
    }
    out
}

pub fn rect(area: Rect, app: &App) -> Rect {
    let n = lines(&app.widget_choices()).len().clamp(1, ROWS) as u16;
    // borders, the typed line, the list, the hint
    centered(area, 60, n + 4)
}

/// The Widget picker, palette-style: what was typed, the Widgets, the hint.
pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, th: &Theme) {
    if app.picker_lost_its_pane() {
        return;
    }
    let Some(p) = app.widget_picker() else { return };
    let (target, query, sel) = (p.target, p.query.clone(), p.sel);
    let rows = app.widget_choices();
    let here = app.layout.get(target);
    let room = app.pane_rect(target);
    let title = match here.and_then(|h| h.widget) {
        Some(_) => format!("Replace {}", here.map(|h| app.name_of(h)).unwrap_or_default()),
        None => "Choose a widget".to_string(),
    };
    let r = rect(area, app);
    let inner = popup(buf, r, th, &title);
    text(buf, inner.x + 1, inner.y, inner.width.saturating_sub(2), &format!("› {query}▏"), th.s(th.fg).add_modifier(Modifier::BOLD));
    let list = lines(&rows);
    let shown = (inner.height.saturating_sub(2) as usize).max(1);
    let at = list.iter().position(|l| *l == Some(sel)).unwrap_or(0);
    let first = crate::scroll::window(at, shown, list.len());
    for (k, line) in list.iter().enumerate().skip(first).take(shown) {
        let y = inner.y + 1 + (k - first) as u16;
        let Some(i) = *line else {
            // the line: what's below it is on screen already
            let label = " on screen · picking one swaps ";
            let w = inner.width.saturating_sub(2);
            let dashes = |n: u16| "─".repeat(n as usize);
            let lw = label.chars().count() as u16;
            text(buf, inner.x + 1, y, w, &format!("{}{label}{}", dashes(2), dashes(w.saturating_sub(lw + 2))), th.s(th.border));
            continue;
        };
        let c = rows[i];
        let on = c.from.is_some();
        let st = match (i == sel, on) {
            (true, _) => th.s(th.accent).bg(th.selection.color()),
            (false, true) => th.s(th.dim),
            (false, false) => th.s(th.fg),
        };
        blank(buf, inner.x, y, inner.width, st);
        text(buf, inner.x + 2, y, 28, &crate::ui::pane_name(app.design, c.widget), st);
        text(buf, inner.x + 31, y, inner.width.saturating_sub(33), c.widget.name(), if i == sel { st } else { th.s(th.dim) });
        // too big for this Pane: the line it would show there
        if room.is_some_and(|r| crate::needs::too_small(c.widget, r)) {
            let (w, h) = crate::needs::smallest(c.widget);
            let needs = format!("needs {w} × {h}");
            let nw = needs.chars().count() as u16;
            text(buf, inner.right().saturating_sub(nw + 2), y, nw, &needs, if i == sel { st } else { th.s(th.dim) });
        }
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::PickWidget(i));
    }
    if rows.is_empty() {
        let why = if query.is_empty() { "nothing else to show here" } else { "nothing matches" };
        text(buf, inner.x + 2, inner.y + 1, inner.width.saturating_sub(4), why, th.s(th.dim));
    }
    let keeps = crate::arrangement::required(here.and_then(|h| h.widget));
    let empty = here.is_some_and(|h| h.widget.is_none());
    let hint = match rows.get(sel) {
        _ if keeps && query.is_empty() => "it stays on screen: pick one to swap · Esc cancel",
        // an empty Pane (a split's): Esc keeps it, empty
        Some(c) if empty && c.from.is_some() => "Enter moves it here · Esc leaves it empty",
        _ if empty => "type to filter · Enter fills it · Esc leaves it empty",
        Some(c) if c.from.is_some() => "type to filter · Enter swaps it here · Esc cancel",
        _ => "type to filter · Enter puts it here · Esc cancel",
    };
    text(buf, inner.x + 1, inner.bottom().saturating_sub(1), inner.width.saturating_sub(2), hint, th.s(th.dim));
}

#[cfg(test)]
mod tests;
