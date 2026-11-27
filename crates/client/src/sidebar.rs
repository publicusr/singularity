//! The library sidebar (#9): Home, Liked Songs, playlists (folders indented), saved albums, followed artists.

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::{text, blank};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::LibraryItem;

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
    /// whether the wheel has the view (D86)
    pub wheeled: crate::scroll::Wheeled,
}

impl Sidebar {
    pub fn move_sel(&mut self, delta: i64, len: usize) {
        self.sel = crate::scroll::moved(self.sel, delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32, len);
        self.top = first(self.sel, self.top, self.rows, self.cols, len);
    }

    /// The wheel: the view `lines` on (in columns, a column of entries a line), the highlight where it is (D86).
    pub fn scroll(&mut self, lines: i64) {
        let unit = if self.cols > 1 { self.rows.max(1) as i64 } else { 1 };
        self.wheeled.by(&mut self.top, (lines * unit).clamp(i32::MIN as i64, i32::MAX as i64) as i32, self.sel);
    }
}

/// How the library uses its Pane (#56), from the Pane's width and height alone: one list in small and medium Panes (an
/// entry a row; from 32 × 24 its name over its subtitle beside a two-row cover), and in Panes with room for two
/// columns of [`COL_W`] (and the gap between them) and at least 24 rows, two or three columns of bigger covers with a
/// short author tag ([`tag`]) and a blank row between entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// columns of entries
    pub cols: u16,
    /// how many rows tall a cover is (and twice as many cells wide: about square); an entry is as tall
    pub cover: u16,
}

/// The narrowest column: a cover, the name and the author tag beside it.
pub const COL_W: u16 = 38;
/// The gap between columns.
const GAP: u16 = 2;
/// In columns, the cover grows until a column holds only this many entries...
const MIN_ROWS: u16 = 7;
/// ...or the name beside it would have less room than this...
const TEXT_W: u16 = 24;
/// ...and between these.
const COVER: (u16, u16) = (3, 6);

impl Shape {
    pub fn of(w: u16, h: u16) -> Shape {
        if w < 32 || h < 24 {
            return Shape { cols: 1, cover: 1 };
        }
        let cols = ((w + GAP) / (COL_W + GAP)).min(3);
        if cols < 2 {
            return Shape { cols: 1, cover: 2 };
        }
        // as big as the room allows: the height (MIN_ROWS entries and the blank rows between them) and the column's
        // width (a margin, the cover, a gap of three and the name)
        let col_w = (w - GAP * (cols - 1)) / cols;
        let by_h = ((h + 1) / MIN_ROWS).saturating_sub(1);
        let by_w = col_w.saturating_sub(1 + 3 + TEXT_W) / 2;
        Shape { cols, cover: by_h.min(by_w).clamp(COVER.0, COVER.1) }
    }

    /// Rows from one entry's top to the next: in columns, a blank row between entries (covers never touch).
    pub fn pitch(self) -> u16 {
        if self.cols > 1 { self.cover + 1 } else { self.cover }
    }

    /// Rows a column for `n` entries in `h` rows: as many as fit (the last needs no blank row under it); in columns,
    /// when they all fit, as few as share them evenly (rather than one full column and empty ones).
    pub fn rows(self, h: u16, n: usize) -> usize {
        let gap = self.pitch() - self.cover;
        let fit = ((h + gap) / self.pitch()).max(1) as usize;
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
/// a mix of many), without the kind the subtitle starts with ("Playlist · Nova Drift"). A book keeps Spotify's access
/// line first (D58); Liked Songs keeps its count. The names are the Daemon's (`authors`, each whole: "Tyler, The
/// Creator" is one); without them only what Spotify already ends with "and more", or a list of three or more, is
/// shortened.
pub fn tag(it: &LibraryItem) -> String {
    let sub = it.subtitle.as_str();
    if matches!(it.kind.as_str(), "liked" | "artist" | "folder") {
        return sub.to_string();
    }
    // "Playlist · owner", "Podcast · publisher", a book's "<access> · <authors>": the makers come last
    let (before, makers) = match sub.rsplit_once(" · ") {
        Some((b, m)) => (Some(b), m),
        None => (None, sub),
    };
    let short = match it.authors.as_slice() {
        [one] => one.clone(),
        [one, ..] => format!("{one} & more"),
        [] => match makers.strip_suffix(" and more").or_else(|| makers.strip_suffix(" & more")) {
            Some(head) => format!("{} & more", head.split(", ").next().unwrap_or(head)),
            None if makers.split(", ").count() >= 3 => format!("{} & more", makers.split(", ").next().unwrap_or(makers)),
            None => makers.to_string(),
        },
    };
    match before {
        Some(b) if !matches!(b, "Playlist" | "Podcast" | "Audiobook") => format!("{b} · {short}"),
        _ => short,
    }
}

/// `s` in `w` display columns: whole when it fits, else cut with an ellipsis.
pub fn clip(s: &str, w: u16) -> String {
    use unicode_width::UnicodeWidthStr;
    let w = w as usize;
    if s.width() <= w {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.to_string().width() + 1 > w {
            break;
        }
        out.push(c);
    }
    if w > 0 {
        out.push('…');
    }
    out
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
    let (cover, pitch) = (shape.cover, shape.pitch());
    side.cols = shape.cols as usize;
    side.rows = shape.rows(r.height, n);
    side.visible = side.cols * side.rows;
    // the highlight is on a row there is (a refresh can shorten the library), and the window follows it (the size it
    // was moved with may be another Pane's, or the popup's before its first frame)
    side.sel = side.sel.min(n.saturating_sub(1));
    // (unless the wheel has the view)
    let end = first(n.saturating_sub(1), 0, side.rows, side.cols, n);
    side.top = if side.wheeled.follows(side.sel, end) { first(side.sel, side.top, side.rows, side.cols, n) } else { side.top.min(end) };
    // (its popup, where no Pane shows it, has the keys as a focused Pane has them)
    let focused = app.is_focused(crate::layout::Widget::Library) || app.is_open(crate::app::Modal::Library);
    let open = app.list_view().map(|v| v.uri.clone());
    let mut hits = vec![];
    for row in side.top..n.min(side.top + side.visible) {
        let (c, k) = ((row - side.top) / side.rows, (row - side.top) % side.rows);
        let col = shape.column(r, c as u16);
        // an entry is as tall as its cover; in columns, a blank row follows it
        let e = Rect::new(col.x, col.y + k as u16 * pitch, col.width, cover);
        let y = e.y;
        let (g, name, sub, depth, current, art) = match row {
            0 => ("⌂", "Home".to_string(), String::new(), 0u16, matches!(app.page, crate::app::Page::Home), None),
            i => {
                let it = &app.library.as_ref().expect("row exists").items[i - FIXED];
                let art = it.image_url.clone().filter(|_| it.kind != "folder");
                let sub = if shape.cols > 1 { tag(it) } else { it.subtitle.clone() };
                (glyph(&it.kind), it.name.clone(), sub, it.depth as u16, open.as_deref() == Some(it.uri.as_str()), art)
            }
        };
        let selected = focused && row == side.sel;
        let bg = if selected { th.selection.color() } else { th.bg.color() };
        for k in 0..cover {
            blank(buf, e.x, y + k, e.width, th.base().bg(bg));
        }
        let x = e.x + 1 + depth * 2;
        let st = if current { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.fg) };
        hits.push((e, Hit::Side(row)));
        if cover >= 2 {
            // the name over its subtitle (in columns, the author tag) beside a cover as tall as the entry
            let (cw, gap) = (cover * 2, if cover == 2 { 2 } else { 3 });
            let has_art = art.as_ref().is_some_and(|u| app.covers.get(u).is_some());
            match art.filter(|_| has_art) {
                Some(url) => crate::home::cover(buf, Rect::new(x, y, cw.min(e.right().saturating_sub(x)), cover), &url, g, app, th),
                // the glyph where the cover goes (across the middle of a bigger one), level with the name
                None => {
                    let gx = if cover == 2 { x + 1 } else { x + cover - 1 };
                    text(buf, gx, y, 2.min(e.right().saturating_sub(gx)), g, th.s(if current { th.accent } else { th.dim }).bg(bg));
                }
            }
            let tx = x + cw + gap;
            let w = e.right().saturating_sub(tx);
            if sub.is_empty() {
                // Home: one line, level with the others' names
                text(buf, tx, y, w, &name, st.bg(bg));
            } else {
                text(buf, tx, y, w, &name, st.add_modifier(Modifier::BOLD).bg(bg));
                // (in columns, a tag too long for its column ends in an ellipsis)
                let sub = if shape.cols > 1 { clip(&sub, w) } else { sub };
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
