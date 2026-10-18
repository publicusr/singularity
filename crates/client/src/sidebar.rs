//! The library sidebar (#9): Home, Liked Songs, playlists (folders indented), saved albums, followed artists.

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

#[derive(Default, Clone, Copy)]
pub struct Sidebar {
    /// selected row: 0 = Home, then library items
    pub sel: usize,
    pub top: usize,
    pub visible: usize,
}

impl Sidebar {
    pub fn move_sel(&mut self, delta: i64, len: usize) {
        self.sel = (self.sel as i64 + delta).clamp(0, len.saturating_sub(1) as i64) as usize;
        let vis = self.visible.max(1);
        if self.sel < self.top {
            self.top = self.sel;
        } else if self.sel >= self.top + vis {
            self.top = self.sel + 1 - vis;
        }
    }

}

fn glyph(kind: &str) -> &'static str {
    match kind {
        "liked" => "♥",
        "playlist" => "≡",
        "folder" => "▸",
        "album" => "◎",
        "artist" => "◍",
        _ => "·",
    }
}

/// Rows above the library: Home, Search.
pub const FIXED: usize = 2;

/// Rows: Home and Search first, then the library. Returns how many rows there are.
pub fn len(app: &App) -> usize {
    FIXED + app.library.as_ref().map(|l| l.items.len()).unwrap_or(0)
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let n = len(app);
    let mut side = app.sidebar;
    side.visible = r.height.max(1) as usize;
    side.top = side.top.min(n.saturating_sub(side.visible));
    let focused = app.focus == "library";
    let open = app.view.as_ref().map(|v| v.uri.clone());
    let page = app.current_page();
    let mut hits = vec![];
    for row in side.top..n.min(side.top + side.visible) {
        let y = r.y + (row - side.top) as u16;
        let (g, name, sub, depth, current) = match row {
            0 => ("⌂", "Home".to_string(), String::new(), 0u16, page == crate::app::Page::Home),
            1 => ("⌕", "Search".to_string(), String::new(), 0u16, page == crate::app::Page::Search),
            i => {
                let it = &app.library.as_ref().expect("row exists").items[i - FIXED];
                (glyph(&it.kind), it.name.clone(), it.subtitle.clone(), it.depth as u16, open.as_deref() == Some(it.uri.as_str()))
            }
        };
        let selected = focused && row == side.sel;
        let bg = if selected { th.selection.color() } else { th.bg.color() };
        text(buf, r.x, y, r.width, &" ".repeat(r.width as usize), th.base().bg(bg));
        let x = r.x + 1 + depth * 2;
        let st = if current { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.fg) };
        text(buf, x, y, 2, g, th.s(if current { th.accent } else { th.dim }).bg(bg));
        let w = r.right().saturating_sub(x + 2);
        let name_w = name.chars().count() as u16;
        text(buf, x + 2, y, w, &name, st.bg(bg));
        // the subtitle fills whatever room the name leaves
        if name_w + 3 < w && !sub.is_empty() {
            text(buf, x + 3 + name_w, y, w - name_w - 1, &sub, th.s(th.border).bg(bg));
        }
        hits.push((Rect::new(r.x, y, r.width, 1), Hit::Side(row)));
    }
    if app.library.is_none() && r.height > 2 {
        text(buf, r.x + 1, r.y + 2, r.width.saturating_sub(2), "loading your library…", th.s(th.dim));
    }
    for (rect, h) in hits {
        app.hit(rect, h);
    }
    app.sidebar = side;
}
