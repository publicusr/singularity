//! The library sidebar (#9): Home, Liked Songs, playlists (folders indented), saved albums, followed artists.

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::{text, blank};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

#[derive(Default, Clone, Copy)]
pub struct Sidebar {
    /// selected row: 0 = Home, then library items
    pub sel: usize,
    /// the first row shown
    pub top: usize,
    /// rows shown (all columns), and the columns and rows a column the last frame had (#56)
    pub visible: usize,
    pub cols: usize,
    pub rows: usize,
}

impl Sidebar {
    pub fn move_sel(&mut self, delta: i64, len: usize) {
        self.sel = crate::scroll::moved(self.sel, delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32, len);
        self.top = first(self.sel, self.top, self.rows, self.cols, len);
    }
}

/// How the library uses its Pane (#56), from the Pane's width and height alone: one list in small and medium Panes (an
/// entry a row; from 32 × 24 its name over its subtitle beside a two-row cover), and in Panes wide enough for two
/// columns of [`COL_W`] and at least 24 rows, two or three columns of bigger covers with a short author tag ([`tag`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// columns of entries
    pub cols: u16,
    /// rows an entry takes; its cover is as tall and twice as wide (about square)
    pub pitch: u16,
}

/// The narrowest column: a cover, the name and the author tag beside it.
pub const COL_W: u16 = 38;
/// The gap between columns.
const GAP: u16 = 2;

impl Shape {
    pub fn of(w: u16, h: u16) -> Shape {
        if w < 32 || h < 24 {
            return Shape { cols: 1, pitch: 1 };
        }
        match (w / COL_W).min(3) {
            0 | 1 => Shape { cols: 1, pitch: 2 },
            // covers three rows tall, four in a Pane as tall as a full screen
            cols => Shape { cols, pitch: if h >= 44 { 4 } else { 3 } },
        }
    }

    /// Rows a column for `n` entries in `h` rows: as many as fit; in columns, when they all fit, as few as share them
    /// evenly (rather than one full column and empty ones).
    pub fn rows(self, h: u16, n: usize) -> usize {
        let fit = (h / self.pitch).max(1) as usize;
        let cols = self.cols as usize;
        if cols > 1 && n <= fit * cols { n.div_ceil(cols).max(1) } else { fit }
    }

    /// Column `c`'s part of the Pane's inside `r`.
    pub fn column(self, r: Rect, c: u16) -> Rect {
        if self.cols <= 1 {
            return r;
        }
        let w = r.width.saturating_sub(GAP * (self.cols - 1)) / self.cols;
        Rect::new(r.x + c * (w + GAP), r.y, w, r.height)
    }
}

/// The first row to show so that `sel` stays in view: the window moves only when the highlight leaves it (an entry at
/// a time in one list, a column of `rows` at a time in `cols` columns) and never past the end of `len` rows.
pub fn first(sel: usize, top: usize, rows: usize, cols: usize, len: usize) -> usize {
    let (rows, cols) = (rows.max(1), cols.max(1));
    let (unit, span) = if cols == 1 { (1, rows) } else { (rows, cols) };
    let (at, from) = (sel / unit, top / unit);
    let from = if at < from {
        at
    } else if at >= from + span {
        at + 1 - span
    } else {
        from
    };
    from.min(len.div_ceil(unit).saturating_sub(span)) * unit
}

/// The short author tag under a name in columns (#56): who made it, the first of them only ("Mira Vale & more" for
/// a mix of many), without the kind the subtitle starts with ("Playlist · Nova Drift"); Liked Songs keeps its count.
pub fn tag(kind: &str, subtitle: &str) -> String {
    if matches!(kind, "liked" | "artist" | "folder") {
        return subtitle.to_string();
    }
    // "Playlist · owner", "Podcast · publisher", a book's "<access> · <authors>": the makers come last
    let who = subtitle.rsplit(" · ").next().unwrap_or_default();
    let more = |s: &str| s.strip_suffix(" and more").or_else(|| s.strip_suffix(" & more")).map(str::to_string);
    match who.split_once(", ") {
        Some((one, _)) => format!("{one} & more"),
        None => match more(who) {
            Some(one) => format!("{one} & more"),
            None => who.to_string(),
        },
    }
}

fn glyph(kind: &str) -> &'static str {
    match kind {
        "liked" => "♥",
        "playlist" => "≡",
        "folder" => "▸",
        "album" => "◎",
        "artist" => "◍",
        "show" => "◈",
        _ => "·",
    }
}

/// Rows above the library: Home. Search lives on Home itself (D70, D71).
pub const FIXED: usize = 1;

/// Rows: Home first, then the library. Returns how many rows there are.
pub fn len(app: &App) -> usize {
    FIXED + app.library.as_ref().map(|l| l.items.len()).unwrap_or(0)
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let n = len(app);
    let mut side = app.sidebar;
    // one list, or columns of bigger covers in a wide, big Pane (#56)
    let shape = Shape::of(r.width, r.height);
    let pitch = shape.pitch;
    side.cols = shape.cols as usize;
    side.rows = shape.rows(r.height, n);
    side.visible = side.cols * side.rows;
    // the highlight is on a row there is (a refresh can shorten the library), and the window follows it (the size it
    // was moved with may be another Pane's, or the popup's before its first frame)
    side.sel = side.sel.min(n.saturating_sub(1));
    side.top = first(side.sel, side.top, side.rows, side.cols, n);
    // (its popup, where no Pane shows it, has the keys as a focused Pane has them)
    let focused = app.is_focused(crate::layout::Widget::Library) || app.is_open(crate::app::Modal::Library);
    let open = app.open.view().map(|v| v.uri.clone());
    let page = app.current_page();
    let mut hits = vec![];
    for row in side.top..n.min(side.top + side.visible) {
        let (c, k) = ((row - side.top) / side.rows, (row - side.top) % side.rows);
        let col = shape.column(r, c as u16);
        let e = Rect::new(col.x, col.y + k as u16 * pitch, col.width, pitch);
        let y = e.y;
        let (g, name, sub, depth, current, art) = match row {
            0 => ("⌂", "Home".to_string(), String::new(), 0u16, page == crate::app::Page::Home, None),
            i => {
                let it = &app.library.as_ref().expect("row exists").items[i - FIXED];
                let art = it.image_url.clone().filter(|_| it.kind != "folder");
                let sub = if shape.cols > 1 { tag(&it.kind, &it.subtitle) } else { it.subtitle.clone() };
                (glyph(&it.kind), it.name.clone(), sub, it.depth as u16, open.as_deref() == Some(it.uri.as_str()), art)
            }
        };
        let selected = focused && row == side.sel;
        let bg = if selected { th.selection.color() } else { th.bg.color() };
        for k in 0..pitch {
            blank(buf, e.x, y + k, e.width, th.base().bg(bg));
        }
        let x = e.x + 1 + depth * 2;
        let st = if current { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.fg) };
        hits.push((e, Hit::Side(row)));
        if pitch >= 2 {
            // the name over its subtitle (in columns, the author tag) beside a cover as tall as the entry
            let (cw, gap) = (pitch * 2, if pitch == 2 { 2 } else { 3 });
            let has_art = art.as_ref().is_some_and(|u| app.covers.get(u).is_some());
            match art.filter(|_| has_art) {
                Some(url) => crate::home::cover(buf, Rect::new(x, y, cw.min(e.right().saturating_sub(x)), pitch), &url, g, app, th),
                // the glyph where the cover goes (in the middle of a bigger one)
                None => {
                    let (gx, gy) = if pitch == 2 { (x + 1, y) } else { (x + pitch - 1, y + (pitch - 1) / 2) };
                    text(buf, gx, gy, 2.min(e.right().saturating_sub(gx)), g, th.s(if current { th.accent } else { th.dim }).bg(bg));
                }
            }
            let tx = x + cw + gap;
            let w = e.right().saturating_sub(tx);
            if sub.is_empty() {
                // Home: one line, level with the others' names
                text(buf, tx, y, w, &name, st.bg(bg));
            } else {
                text(buf, tx, y, w, &name, st.add_modifier(Modifier::BOLD).bg(bg));
                text(buf, tx, y + 1, w, &sub, th.s(th.dim).bg(bg));
            }
            continue;
        }
        // a miniature of the cover where there is one and it has arrived (two cells: about square), else the glyph
        let gw = 2.min(e.right().saturating_sub(x));
        match art.filter(|u| app.covers.get(u).is_some()) {
            Some(url) => crate::home::cover(buf, Rect::new(x, y, gw, 1), &url, g, app, th),
            None => text(buf, x, y, gw, g, th.s(if current { th.accent } else { th.dim }).bg(bg)),
        }
        let w = e.right().saturating_sub(x + 3);
        let name_w = name.chars().count() as u16;
        text(buf, x + 3, y, w, &name, st.bg(bg));
        // the subtitle fills whatever room the name leaves
        if name_w + 3 < w && !sub.is_empty() {
            text(buf, x + 4 + name_w, y, w - name_w - 1, &sub, th.s(th.border).bg(bg));
        }
    }
    if app.library.is_none() && r.height > 2 {
        text(buf, r.x + 1, r.y + 2, r.width.saturating_sub(2), "loading your library…", th.s(th.dim));
    }
    for (rect, h) in hits {
        app.hit(rect, h);
    }
    app.sidebar = side;
}

#[cfg(test)]
mod tests;
