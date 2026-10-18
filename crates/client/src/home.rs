//! The Home view (#8): the shortcuts grid, Shelves of cover-art Cards, and the feed tail as a grid.
//!
//! Home scrolls by whole rows and each Shelf sideways by whole Cards, so a cover is never cut at the Pane edge.
//! The layout is a pure function of the feed and the Pane size ([`rows`]); the last frame's rows drive the input.

use crate::app::{App, Hit};
use crate::gfx::{self, ImageKey};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::{Card, HomeFeed};
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
    /// first visible row
    pub top: usize,
    /// first visible Card of each scrolled Shelf
    pub scroll: HashMap<usize, usize>,
    pub sel: Option<CardRef>,
    pub rows: Vec<Row>,
    /// rows that fit in the Pane last frame
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

    pub fn scroll_rows(&mut self, delta: i32) {
        let max = self.rows.len().saturating_sub(self.visible.max(1));
        self.top = (self.top as i32 + delta).clamp(0, max as i32) as usize;
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
            self.sel = Some(self.rows[self.top.min(self.rows.len() - 1)].cards[0]);
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
        let vis = self.visible.max(1);
        if r < self.top {
            self.top = r;
        } else if r >= self.top + vis {
            self.top = r + 1 - vis;
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
fn clip(s: &str, w: u16) -> String {
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
    let top = view.top.min(view.rows.len().saturating_sub(1));
    view.top = top;
    let x0 = r.x + 1;
    let mut y = r.y;
    let mut shown = 0;
    for ri in top..view.rows.len() {
        let row = view.rows[ri].clone();
        // the trailing gap may hang off the bottom; the content may not
        if y + row.height - 1 > r.bottom() {
            break;
        }
        shown += 1;
        if let Some(t) = &row.title {
            text(buf, x0, y, r.width.saturating_sub(10), &clip(t, r.width.saturating_sub(10)), th.title());
            if row.kind == RowKind::Shelf && row.cards.len() > row.per_view {
                let first = view.first(row.cards[0].shelf);
                let (lx, rx) = (r.right().saturating_sub(6), r.right().saturating_sub(3));
                let can_l = first > 0;
                let can_r = first + row.per_view < row.cards.len();
                text(buf, lx, y, 2, "‹", th.s(if can_l { th.fg } else { th.border }));
                text(buf, rx, y, 2, "›", th.s(if can_r { th.fg } else { th.border }));
                app.hit(Rect::new(lx.saturating_sub(1), y, 3, 1), Hit::ShelfScroll(ri, -1));
                app.hit(Rect::new(rx.saturating_sub(1), y, 3, 1), Hit::ShelfScroll(ri, 1));
            }
            y += 1;
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
            if row.kind == RowKind::Shortcuts {
                let x = x0 + i as u16 * (m.tile_w + 1);
                let cover = Rect::new(x, y, m.tile_cover_w.min(m.tile_w), TILE_H);
                let tw = m.tile_w.saturating_sub(cover.width + 1);
                if selected {
                    text(buf, cover.right(), y, tw + 1, &" ".repeat(tw as usize + 1), th.base().bg(th.selection.color()));
                    text(buf, cover.right(), y + 1, tw + 1, &" ".repeat(tw as usize + 1), th.base().bg(th.selection.color()));
                }
                draw_cover(buf, cover, card, app, th);
                let bg = if selected { th.selection.color() } else { th.bg.color() };
                text(buf, cover.right() + 1, y, tw, &clip(&card.name, tw), name_st.bg(bg));
                text(buf, cover.right() + 1, y + 1, tw, &clip(sub, tw), th.s(th.dim).bg(bg));
                app.hit(Rect::new(x, y, m.tile_w, TILE_H), Hit::Card(*cref));
            } else {
                let x = x0 + i as u16 * (m.card_w + CARD_GAP);
                let cover = Rect::new(x, y, m.card_w, m.cover_h);
                draw_cover(buf, cover, card, app, th);
                let ty = y + m.cover_h;
                let mark = if selected { "▸ " } else { "" };
                text(buf, x, ty, m.card_w, &clip(&format!("{mark}{}", card.name), m.card_w), name_st);
                text(buf, x, ty + 1, m.card_w, &clip(sub, m.card_w), th.s(th.dim));
                app.hit(Rect::new(x, y, m.card_w, m.cover_h + 2), Hit::Card(*cref));
            }
        }
        y += row.height - u16::from(row.title.is_some());
    }
    view.visible = shown.max(1);
    // more above / below
    if top > 0 {
        text(buf, r.right().saturating_sub(1), r.y, 1, "▴", th.s(th.dim));
    }
    if top + shown < view.rows.len() {
        text(buf, r.right().saturating_sub(1), r.bottom().saturating_sub(1), 1, "▾", th.s(th.dim));
    }
    (feed, view)
}

fn draw_cover(buf: &mut Buffer, r: Rect, card: &Card, app: &mut App, th: &Theme) {
    cover(buf, r, card.image_url.as_deref().unwrap_or(""), if card.kind == "artist" { "◍" } else { "♪" }, app, th);
}

/// A cover in the best tier available: kitty pixels, else half-blocks; a quiet placeholder (`glyph`) while it loads.
pub fn cover(buf: &mut Buffer, r: Rect, url: &str, glyph: &str, app: &mut App, th: &Theme) {
    if let Some((cw, ch)) = app.pixel_tier_for(r) {
        if let Some(src) = app.covers.get(url) {
            let (pw, ph) = ((r.width as f64 * cw).round() as u32, (r.height as f64 * ch).round() as u32);
            let key = ImageKey { source: url.to_string(), w: pw, h: ph };
            if let Some(k) = app.kitty.as_mut() {
                k.place(key, r, || gfx::fit(&src, pw, ph));
            }
            // blank cells under the image (it sits below text, so nothing may be drawn here)
            for yy in r.top()..r.bottom() {
                text(buf, r.x, yy, r.width, &" ".repeat(r.width as usize), th.base());
            }
            return;
        }
    } else if let Some(img) = app.covers.scaled(url, r.width as u32, r.height as u32 * 2) {
        gfx::blit_halfblocks(&img, buf, r);
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
    use singularity_proto::Shelf;

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
        v.visible = 2;
        v.move_sel(0, 0);
        assert_eq!(v.sel, Some(CardRef { shelf: 0, card: 0 }));
        v.move_sel(0, 2); // down into the first Shelf
        assert_eq!(v.sel, Some(CardRef { shelf: 1, card: 0 }));
        assert_eq!(v.top, 1, "scrolled so the row is visible");
        let per = v.rows[2].per_view;
        for _ in 0..per {
            v.move_sel(1, 0);
        }
        assert_eq!(v.sel, Some(CardRef { shelf: 1, card: per }));
        assert_eq!(v.first(1), 1, "the Shelf scrolled sideways by one Card");
        v.move_sel(0, 1); // the next row is shorter: clamp to its last Card
        assert_eq!(v.sel, Some(CardRef { shelf: 2, card: 2 }));
        v.scroll_rows(100);
        assert_eq!(v.top, v.rows.len() - 2);
        v.scroll_shelf(2, 100);
        assert_eq!(v.first(1), 10 - per);
    }

    #[test]
    fn clip_adds_an_ellipsis() {
        assert_eq!(clip("Daily Mix 1", 20), "Daily Mix 1");
        assert_eq!(clip("Daily Mix 1", 6), "Daily…");
    }
}
