//! The library and the queue without a Pane (D87). Home is the only Required Widget, so a Design of your own may let
//! them go; the palette's "Your Library" and "Queue" then open them as popups (with a Pane, the entries focus it).
//! Each popup is its Pane's Widget in a box: the same drawing (`sidebar::draw`, `queue::draw`), the same selection
//! and the same keys. Liked Songs and playlists open from the library into Home's Pane, as a click in the library Pane
//! does; the queue's songs are seen, dragged into a new order and removed.

use crate::app::{App, Modal, Popup};
use crate::layout::Widget;
use crate::theme::Theme;
use crate::ui::{centered, popup, text};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;

/// The popups stay compact, as the palette is: at most this many rows, the list scrolls with its highlight (and the
/// library's list stays one row an entry, below the size where its Pane goes two).
const MAX_H: u16 = 24;

impl App {
    /// "Your Library": focus its Pane, or open it as a popup where it has none.
    pub fn show_library(&mut self) {
        match self.layout.pane_of(Widget::Library) {
            Some(p) => self.focus = Some(p),
            None => self.open_popup(Popup::Library),
        }
    }

    /// "Queue": focus its Pane, or open it as a popup where it has none.
    pub fn show_queue(&mut self) {
        match self.layout.pane_of(Widget::Queue) {
            Some(p) => self.focus = Some(p),
            None => self.open_popup(Popup::Queue),
        }
    }

    /// Keys in the library popup: its Pane's (↑↓, Enter or → opens, the Menu key its menu), Esc closes.
    pub fn library_popup_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.close_if_open(Modal::Library),
            KeyCode::Enter | KeyCode::Right => {
                let row = self.sidebar.sel;
                self.library_popup_open(row);
            }
            _ => {
                self.side_key(k);
            }
        }
    }

    /// Row `row` picked in the library popup: it opens in Home's Pane and the popup has done its job. A folder doesn't
    /// open (it says why), so the popup stays.
    pub fn library_popup_open(&mut self, row: usize) {
        let folder = row.checked_sub(crate::sidebar::FIXED).and_then(|i| self.library.as_ref()?.items.get(i)).is_some_and(|it| it.kind == "folder");
        if !folder {
            self.close_if_open(Modal::Library);
            self.focus_on(Widget::Home);
        }
        self.open_side(row);
    }

    /// Keys in the queue popup: its Pane's (↑↓, Delete removes the song, Enter its menu), Esc closes.
    pub fn queue_popup_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.close_if_open(Modal::Queue),
            _ => {
                self.queue_key(k);
            }
        }
    }
}

pub fn library_rect(area: Rect, app: &App) -> Rect {
    // borders, the list, the hint
    let n = crate::sidebar::len(app) as u16;
    centered(area, 56, (n + 3).clamp(6, MAX_H))
}

pub fn queue_rect(area: Rect, app: &App) -> Rect {
    // borders, what's playing and a gap, the list, the hint
    let n = crate::queue::lines(app).len().max(1) as u16;
    centered(area, 64, (n + 5).clamp(7, MAX_H))
}

/// The library popup: the library Pane's list in a box, titled as the Design titles its Pane.
pub fn draw_library(buf: &mut Buffer, area: Rect, app: &mut App, th: &Theme) {
    let r = library_rect(area, app);
    let inner = popup(buf, r, th, &crate::ui::pane_name(app.design, Widget::Library));
    let list = Rect { height: inner.height.saturating_sub(1), ..inner };
    crate::sidebar::draw(buf, list, app, th);
    text(buf, inner.x + 1, inner.bottom().saturating_sub(1), inner.width.saturating_sub(2), "Enter or click: open in Home · Esc close", th.s(th.dim));
}

/// The queue popup: the queue Pane's list in a box, titled as the Design titles its Pane.
pub fn draw_queue(buf: &mut Buffer, area: Rect, app: &mut App, th: &Theme) {
    let r = queue_rect(area, app);
    let inner = popup(buf, r, th, &crate::ui::pane_name(app.design, Widget::Queue));
    let list = Rect { height: inner.height.saturating_sub(1), ..inner };
    crate::queue::draw(buf, list, app, th);
    text(buf, inner.x + 1, inner.bottom().saturating_sub(1), inner.width.saturating_sub(2), "drag to reorder · Delete remove · Enter menu · Esc close", th.s(th.dim));
}

#[cfg(test)]
mod tests;
