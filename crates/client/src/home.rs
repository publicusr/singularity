//! The Home view (#8): the shortcuts grid, Shelves of cover-art Cards, and the feed tail as a grid.
//!
//! Home scrolls by lines (rows cut off at the top or bottom are drawn clipped, covers included) and each Shelf sideways
//! by whole Cards.
//! The layout is a pure function of the feed and the Pane size ([`rows`]); the last frame's rows drive the input.

use crate::app::{App, Hit};
use crate::gfx::{self, ImageKey};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::HomeFeed;
use std::collections::HashMap;

/// A Card by position in the feed: Shelf index, Card index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardRef {
    pub shelf: usize,
    pub card: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// one line of the shortcuts grid
    Shortcuts,
    /// a regular Shelf, scrolled sideways
    Shelf,
    /// one line of the feed tail grid
    Feed,
    /// one line of a titled grid (e.g. one type of search results)
    Grid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    pub title: Option<String>,
    /// every Card in the row, including those scrolled out of view
    pub cards: Vec<CardRef>,
    /// how many Cards fit side by side
    pub per_view: usize,
    pub height: u16,
}

/// Sizes shared by every row at the current Pane width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub card_w: u16,
    pub cover_h: u16,
    pub tile_w: u16,
    pub tile_cover_w: u16,
}

const CARD_MIN: u16 = 16;
const CARD_MAX: u16 = 24;
const CARD_GAP: u16 = 2;
const TILE_MIN: u16 = 18;
/// covers never shrink below this many rows
const COVER_MIN_H: u16 = 4;
const TILE_H: u16 = 2;

/// Lay out the feed for a Pane of `width`×`height` cells; `aspect` is a cell's height / width in pixels (≈2).
pub fn rows(feed: &HomeFeed, width: u16, height: u16, aspect: f64) -> (Vec<Row>, Metrics) {
    let w = width.saturating_sub(2).max(1); // one cell of margin each side
    let mut per_view = (((w + CARD_GAP) / (CARD_MIN + CARD_GAP)) as usize).max(1);
    let mut card_w = ((w + CARD_GAP) / per_view as u16).saturating_sub(CARD_GAP).clamp(1, CARD_MAX);
    // square covers, as tall as the width allows, but a Shelf takes at most about half the Pane
    let mut cover_h = ((card_w as f64 / aspect).round() as u16).max(1);
    let max_h = (height.saturating_sub(4) / 2).max(COVER_MIN_H);
    if cover_h > max_h {
        cover_h = max_h;
        card_w = ((cover_h as f64 * aspect).round() as u16).clamp(1, w);
        per_view = (((w + CARD_GAP) / (card_w + CARD_GAP)) as usize).max(1);
    }
    let cols = ((w + 1) / (TILE_MIN + 1)).clamp(1, 4) as usize;
    let tile_w = (w + 1) / cols as u16 - 1;
    let tile_cover_w = ((TILE_H as f64 * aspect).round() as u16).max(1);
    let m = Metrics { card_w, cover_h, tile_w, tile_cover_w };
    let card_row_h = cover_h + 2 + 1;

    let mut out = vec![];
    let mut feed_cards = vec![];
    for (si, s) in feed.shelves.iter().enumerate() {
        let refs = (0..s.cards.len()).map(|card| CardRef { shelf: si, card });
        match s.kind.as_str() {
            "shortcuts" => {
                let all: Vec<CardRef> = refs.collect();
                let lines = all.len().div_ceil(cols);
                for (i, chunk) in all.chunks(cols).enumerate() {
                    // the grid's lines sit tight; a blank row only after the last one
                    let height = TILE_H + u16::from(i + 1 == lines);
                    out.push(Row { kind: RowKind::Shortcuts, title: None, cards: chunk.to_vec(), per_view: cols, height });
                }
            }
            "feed" => feed_cards.extend(refs),
            "grid" => {
                let all: Vec<CardRef> = refs.collect();
                for (i, chunk) in all.chunks(per_view).enumerate() {
                    let title = (i == 0).then(|| s.title.clone());
                    let height = card_row_h + u16::from(title.is_some());
                    out.push(Row { kind: RowKind::Grid, title, cards: chunk.to_vec(), per_view, height });
                }
            }
            _ => out.push(Row { kind: RowKind::Shelf, title: Some(s.title.clone()), cards: refs.collect(), per_view, height: 1 + card_row_h }),
        }
    }
    for (i, chunk) in feed_cards.chunks(per_view).enumerate() {
        let title = (i == 0).then(|| "More for you".to_string());
        let height = card_row_h + u16::from(title.is_some());
        out.push(Row { kind: RowKind::Feed, title, cards: chunk.to_vec(), per_view, height });
    }
    (out, m)
}

/// Scroll and selection state, plus the last frame's layout for input.
#[derive(Default)]
pub struct HomeView {
    /// lines scrolled off the top
    pub offset: u16,
    /// the Pane's height last frame
    pub height: u16,
    /// first visible Card of each scrolled Shelf
    pub scroll: HashMap<usize, usize>,
    pub sel: Option<CardRef>,
    pub rows: Vec<Row>,
    /// rows at least partly in view last frame
    pub visible: usize,
}

impl HomeView {
    fn first(&self, shelf: usize) -> usize {
        self.scroll.get(&shelf).copied().unwrap_or(0)
    }

    /// Where the selection sits: (row, column within the row's Cards).
    fn sel_pos(&self) -> Option<(usize, usize)> {
        let s = self.sel?;
        self.rows.iter().enumerate().find_map(|(r, row)| row.cards.iter().position(|c| *c == s).map(|c| (r, c)))
    }

    /// Where row `i` starts, in lines from the top of the content.
    fn row_y(&self, i: usize) -> u16 {
        self.rows.iter().take(i).map(|r| r.height).sum()
    }

    fn max_offset(&self) -> u16 {
        self.row_y(self.rows.len()).saturating_sub(self.height.max(1))
    }

    pub fn scroll_lines(&mut self, delta: i32) {
        self.offset = (self.offset as i32 + delta).clamp(0, self.max_offset() as i32) as u16;
    }

    pub fn scroll_shelf(&mut self, row: usize, delta: i32) {
        let Some(r) = self.rows.get(row) else { return };
        if r.kind != RowKind::Shelf {
            return;
        }
        let shelf = r.cards[0].shelf;
        let max = r.cards.len().saturating_sub(r.per_view);
        let v = (self.first(shelf) as i32 + delta).clamp(0, max as i32) as usize;
        self.scroll.insert(shelf, v);
    }

    /// Arrow-key movement: `dx` within a row, `dy` between rows (keeping the visible column).
    pub fn move_sel(&mut self, dx: i32, dy: i32) {
        if self.rows.is_empty() {
            return;
        }
        let Some((r, c)) = self.sel_pos() else {
            // the first row that starts in view
            let first = (0..self.rows.len()).find(|&i| self.row_y(i) >= self.offset).unwrap_or(self.rows.len() - 1);
            self.sel = Some(self.rows[first].cards[0]);
            return;
        };
        let (mut r, mut c) = (r as i32, c as i32);
        if dy != 0 {
            // keep the same on-screen column across rows
            let col = c - self.row_first(r as usize) as i32;
            r = (r + dy).clamp(0, self.rows.len() as i32 - 1);
            c = self.row_first(r as usize) as i32 + col;
        }
        c = (c + dx).clamp(0, self.rows[r as usize].cards.len() as i32 - 1);
        self.sel = Some(self.rows[r as usize].cards[c as usize]);
        self.reveal();
    }

    fn row_first(&self, row: usize) -> usize {
        match self.rows.get(row) {
            Some(r) if r.kind == RowKind::Shelf => self.first(r.cards[0].shelf),
            _ => 0,
        }
    }

    /// Scroll so the selected Card is in view.
    pub fn reveal(&mut self) {
        let Some((r, c)) = self.sel_pos() else { return };
        // the whole row in view (its trailing gap may hang off)
        let (top, h) = (self.row_y(r), self.rows[r].height);
        let content = if self.rows[r].kind == RowKind::Shortcuts { TILE_H } else { h.saturating_sub(1) };
        let height = self.height.max(1);
        if top < self.offset {
            self.offset = top;
        } else if top + content > self.offset + height {
            self.offset = (top + content).saturating_sub(height).min(top);
        }
        let row = &self.rows[r];
        if row.kind == RowKind::Shelf {
            let shelf = row.cards[0].shelf;
            let first = self.first(shelf);
            if c < first {
                self.scroll.insert(shelf, c);
            } else if c >= first + row.per_view {
                self.scroll.insert(shelf, c + 1 - row.per_view);
            }
        }
    }
}

/// Shorten to `w` columns with an ellipsis.
fn clip_text(s: &str, w: u16) -> String {
    let w = w as usize;
    if s.chars().count() <= w {
        return s.to_string();
    }
    let mut out: String = s.chars().take(w.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Home's own feed.
pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let Some(feed) = app.home.take() else { return };
    let view = std::mem::take(&mut app.home_view);
    let (feed, view) = draw_feed(buf, r, app, th, feed, view);
    app.home = Some(feed);
    app.home_view = view;
}

/// Shelves of Cards (Home, or an artist page's releases): drawn with `view`'s scroll and selection; both handed back.
pub fn draw_feed(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme, feed: HomeFeed, mut view: HomeView) -> (HomeFeed, HomeView) {
    let aspect = app.cell_px.map(|(w, h)| h / w).unwrap_or(2.0);
    let (rows, m) = rows(&feed, r.width, r.height, aspect);
    view.rows = rows;
    let focused = app.focus == "home";
    view.height = r.height;
    view.offset = view.offset.min(view.max_offset());
    let x0 = r.x + 1;
    let (top, bottom) = (r.y as i32, r.bottom() as i32);
    let on = |y: i32| y >= top && y < bottom;
    // a rect from line `y` down `h` lines, cut to the Pane
    let clip = |x: u16, y: i32, w: u16, h: u16| {
        let (y0, y1) = (y.max(top), (y + h as i32).min(bottom));
        (y1 > y0).then(|| Rect::new(x, y0 as u16, w, (y1 - y0) as u16))
    };
    let mut y = top - view.offset as i32;
    let mut shown = 0;
    for ri in 0..view.rows.len() {
        let row = view.rows[ri].clone();
        let (ry, h) = (y, row.height as i32);
        y += h;
        if ry + h <= top {
            continue;
        }
        if ry >= bottom {
            break;
        }
        shown += 1;
        let mut cy = ry;
        if let Some(t) = &row.title {
            if on(cy) {
                let ty = cy as u16;
                text(buf, x0, ty, r.width.saturating_sub(10), &clip_text(t, r.width.saturating_sub(10)), th.title());
                if row.kind == RowKind::Shelf && row.cards.len() > row.per_view {
                    let first = view.first(row.cards[0].shelf);
                    let (lx, rx) = (r.right().saturating_sub(6), r.right().saturating_sub(3));
                    let can_l = first > 0;
                    let can_r = first + row.per_view < row.cards.len();
                    text(buf, lx, ty, 2, "‹", th.s(if can_l { th.fg } else { th.border }));
                    text(buf, rx, ty, 2, "›", th.s(if can_r { th.fg } else { th.border }));
                    app.hit(Rect::new(lx.saturating_sub(1), ty, 3, 1), Hit::ShelfScroll(ri, -1));
                    app.hit(Rect::new(rx.saturating_sub(1), ty, 3, 1), Hit::ShelfScroll(ri, 1));
                }
            }
            cy += 1;
        }
        let first = view.row_first(ri);
        for (i, cref) in row.cards.iter().skip(first).take(row.per_view).enumerate() {
            let card = &feed.shelves[cref.shelf].cards[cref.card];
            let selected = focused && view.sel == Some(*cref);
            let name_st = if selected { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.fg).add_modifier(Modifier::BOLD) };
            // the feed tail's Cards are labelled with their section ("Because you liked …")
            let sub = match row.kind {
                RowKind::Feed if !feed.shelves[cref.shelf].title.is_empty() => feed.shelves[cref.shelf].title.as_str(),
                _ => card.subtitle.as_str(),
            };
            let glyph = if card.kind == "artist" { "◍" } else { "♪" };
            let url = card.image_url.clone().unwrap_or_default();
            if row.kind == RowKind::Shortcuts {
                let x = x0 + i as u16 * (m.tile_w + 1);
                let cw = m.tile_cover_w.min(m.tile_w);
                let tw = m.tile_w.saturating_sub(cw + 1);
                let bg = if selected { th.selection.color() } else { th.bg.color() };
                for (dy, line, st) in [(0, clip_text(&card.name, tw), name_st), (1, clip_text(sub, tw), th.s(th.dim))] {
                    if on(cy + dy) {
                        let ly = (cy + dy) as u16;
                        if selected {
                            text(buf, x + cw, ly, tw + 1, &" ".repeat(tw as usize + 1), th.base().bg(bg));
                        }
                        text(buf, x + cw + 1, ly, tw, &line, st.bg(bg));
                    }
                }
                cover_at(buf, x, cy, cw, TILE_H, r, &url, glyph, app, th);
                if let Some(hr) = clip(x, cy, m.tile_w, TILE_H) {
                    app.hit(hr, Hit::Card(*cref));
                }
            } else {
                let x = x0 + i as u16 * (m.card_w + CARD_GAP);
                cover_at(buf, x, cy, m.card_w, m.cover_h, r, &url, glyph, app, th);
                let ty = cy + m.cover_h as i32;
                let mark = if selected { "▸ " } else { "" };
                if on(ty) {
                    text(buf, x, ty as u16, m.card_w, &clip_text(&format!("{mark}{}", card.name), m.card_w), name_st);
                }
                if on(ty + 1) {
                    text(buf, x, ty as u16 + 1, m.card_w, &clip_text(sub, m.card_w), th.s(th.dim));
                }
                if let Some(hr) = clip(x, cy, m.card_w, m.cover_h + 2) {
                    app.hit(hr, Hit::Card(*cref));
                }
            }
        }
    }
    view.visible = shown.max(1);
    // more above / below
    if view.offset > 0 {
        text(buf, r.right().saturating_sub(1), r.y, 1, "▴", th.s(th.dim));
    }
    if view.offset < view.max_offset() {
        text(buf, r.right().saturating_sub(1), r.bottom().saturating_sub(1), 1, "▾", th.s(th.dim));
    }
    (feed, view)
}

/// A cover in the best tier available: kitty pixels, else half-blocks; a quiet placeholder (`glyph`) while it loads.
pub fn cover(buf: &mut Buffer, r: Rect, url: &str, glyph: &str, app: &mut App, th: &Theme) {
    cover_at(buf, r.x, r.y as i32, r.width, r.height, r, url, glyph, app, th);
}

/// `cover` for a `w`×`h` cover whose top line is `y` (maybe outside the Pane), drawn only where it's inside `pane`.
#[allow(clippy::too_many_arguments)]
pub fn cover_at(buf: &mut Buffer, x: u16, y: i32, w: u16, h: u16, pane: Rect, url: &str, glyph: &str, app: &mut App, th: &Theme) {
    let (y0, y1) = (y.max(pane.y as i32), (y + h as i32).min(pane.bottom() as i32));
    if y1 <= y0 || w == 0 {
        return;
    }
    let r = Rect::new(x, y0 as u16, w, (y1 - y0) as u16);
    // cover lines scrolled off the top
    let skip = (y0 - y) as u16;
    if let Some((cw, ch)) = app.pixel_tier_for(r) {
        if let Some(src) = app.covers.get(url) {
            let (pw, ph) = ((w as f64 * cw).round() as u32, (h as f64 * ch).round() as u32);
            let key = ImageKey { source: url.to_string(), w: pw, h: ph };
            // only the part in view: the same upload serves every scroll position
            let crop = (r.height < h).then(|| {
                let top = (skip as f64 * ch).round() as u32;
                [0, top, pw, ((r.height as f64 * ch).round() as u32).min(ph - top)]
            });
            if let Some(k) = app.kitty.as_mut() {
                k.place_cropped(key, r, crop, || gfx::fit(&src, pw, ph));
            }
            // blank cells under the image (it sits below text, so nothing may be drawn here)
            for yy in r.top()..r.bottom() {
                text(buf, r.x, yy, r.width, &" ".repeat(r.width as usize), th.base());
            }
            return;
        }
    } else if let Some(img) = app.covers.scaled(url, w as u32, h as u32 * 2) {
        gfx::blit_halfblocks_from(&img, buf, r, skip);
        return;
    }
    for yy in r.top()..r.bottom() {
        text(buf, r.x, yy, r.width, &" ".repeat(r.width as usize), th.base().bg(th.selection.color()));
    }
    text(buf, r.x + r.width / 2, r.y + r.height / 2, 1, glyph, th.s(th.border).bg(th.selection.color()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::{Card, Shelf};

    fn feed() -> HomeFeed {
        let card = |n: usize| Card { uri: format!("spotify:playlist:{n}"), name: format!("Mix {n}"), subtitle: String::new(), image_url: None, kind: "playlist".into() };
        let shelf = |kind: &str, n: usize| Shelf { title: kind.into(), kind: kind.into(), cards: (0..n).map(card).collect() };
        HomeFeed { fetched_unix_ms: 0, shelves: vec![shelf("shortcuts", 8), shelf("shelf", 10), shelf("shelf", 3), shelf("feed", 1), shelf("feed", 1), shelf("feed", 1)] }
    }

    #[test]
    fn rows_follow_the_shelf_structure() {
        let (rows, m) = rows(&feed(), 110, 40, 2.0);
        let kinds: Vec<RowKind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [RowKind::Shortcuts, RowKind::Shortcuts, RowKind::Shelf, RowKind::Shelf, RowKind::Feed]);
        assert_eq!(rows[0].cards.len(), 4, "8 shortcuts in two lines of four");
        assert_eq!(rows[4].cards.len(), 3, "single-card feed sections flow into a grid");
        assert_eq!(rows[4].title.as_deref(), Some("More for you"));
        assert!(m.card_w >= CARD_MIN && m.card_w <= CARD_MAX);
        assert_eq!(m.cover_h, (m.card_w as f64 / 2.0).round() as u16, "square covers");
    }

    #[test]
    fn cards_fill_the_width_evenly() {
        for w in [40u16, 77, 110, 180, 300] {
            let (rows, m) = rows(&feed(), w, 40, 2.0);
            let n = rows[2].per_view as u16;
            assert!(n * m.card_w + (n - 1) * CARD_GAP <= w - 2, "width {w}");
        }
    }

    #[test]
    fn short_panes_get_smaller_square_covers() {
        let (_, tall) = rows(&feed(), 110, 40, 2.0);
        let (rows, short) = rows(&feed(), 110, 16, 2.0);
        assert!(short.cover_h < tall.cover_h && short.cover_h == 6);
        assert_eq!(short.card_w, 12, "still square");
        assert!(rows[2].per_view > 6, "more Cards per Shelf instead");
    }

    #[test]
    fn navigation_moves_by_card_and_row_and_reveals() {
        let mut v = HomeView::default();
        (v.rows, _) = rows(&feed(), 110, 40, 2.0);
        // room for one Shelf, not for the shortcuts above it as well
        v.height = v.rows[2].height + 1;
        v.move_sel(0, 0);
        assert_eq!(v.sel, Some(CardRef { shelf: 0, card: 0 }));
        v.move_sel(0, 2); // down into the first Shelf
        assert_eq!(v.sel, Some(CardRef { shelf: 1, card: 0 }));
        let (top, h) = (v.row_y(2), v.rows[2].height);
        assert!(v.offset <= top && top + h - 1 <= v.offset + v.height, "scrolled so the whole row is in view");
        let per = v.rows[2].per_view;
        for _ in 0..per {
            v.move_sel(1, 0);
        }
        assert_eq!(v.sel, Some(CardRef { shelf: 1, card: per }));
        assert_eq!(v.first(1), 1, "the Shelf scrolled sideways by one Card");
        v.move_sel(0, 1); // the next row is shorter: clamp to its last Card
        assert_eq!(v.sel, Some(CardRef { shelf: 2, card: 2 }));
        v.scroll_lines(1000);
        assert_eq!(v.offset, v.row_y(v.rows.len()) - v.height, "no further than the end");
        v.scroll_lines(-3);
        assert_eq!(v.offset, v.row_y(v.rows.len()) - v.height - 3, "by lines");
        v.scroll_shelf(2, 100);
        assert_eq!(v.first(1), 10 - per);
    }

    #[test]
    fn clip_adds_an_ellipsis() {
        assert_eq!(clip_text("Daily Mix 1", 20), "Daily Mix 1");
        assert_eq!(clip_text("Daily Mix 1", 6), "Daily…");
    }
}
