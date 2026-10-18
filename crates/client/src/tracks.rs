//! Track lists (#9): Liked Songs, playlists and albums, shown in the main Pane.
//!
//! Pages arrive progressively from the Daemon; sorting and filtering are local and only change the *view* order.
//! Playing a row always starts the context at that track's position in the original list.

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::{TrackPage, TrackRow};

/// Liked Songs as the Daemon names it.
pub const LIKED: &str = "spotify:collection";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    Title,
    Artist,
    Album,
    Added,
    Duration,
}

impl Col {
    pub const ALL: [Col; 5] = [Col::Title, Col::Artist, Col::Album, Col::Added, Col::Duration];
    pub fn label(self) -> &'static str {
        match self {
            Col::Title => "Title",
            Col::Artist => "Artist",
            Col::Album => "Album",
            Col::Added => "Added",
            Col::Duration => "Time",
        }
    }
}

/// A track list as received so far.
#[derive(Default, Clone)]
pub struct TrackList {
    pub total: u32,
    pub rows: Vec<TrackRow>,
    pub header: Option<singularity_proto::ListHeader>,
}

impl TrackList {
    pub fn complete(&self) -> bool {
        self.rows.len() as u32 >= self.total
    }

    /// Add a page; pages may repeat (a cache replay) and are placed by offset.
    pub fn add(&mut self, p: &TrackPage) {
        self.total = p.total;
        if p.header.is_some() {
            self.header = p.header.clone();
        }
        let at = p.offset as usize;
        if at > self.rows.len() {
            return; // a gap: ignore until the missing page arrives
        }
        let end = (at + p.tracks.len()).min(self.rows.len());
        self.rows.splice(at..end, p.tracks.iter().cloned());
    }

    /// Put a row at `at` (a like lands on top of Liked Songs; a rollback puts it back where it was).
    pub fn insert(&mut self, at: usize, row: TrackRow) {
        if self.rows.iter().any(|r| r.uri == row.uri) {
            return;
        }
        self.rows.insert(at.min(self.rows.len()), row);
        self.total += 1;
    }

    /// Take a track out; returns where it was and the row.
    pub fn remove(&mut self, uri: &str) -> Option<(usize, TrackRow)> {
        let i = self.rows.iter().position(|r| r.uri == uri)?;
        self.total = self.total.saturating_sub(1);
        Some((i, self.rows.remove(i)))
    }
}

/// How one list is being looked at: sort, filter, selection, scroll.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackView {
    pub uri: String,
    pub title: String,
    /// what it is: "Playlist · Nova Drift", an album's artists…
    pub subtitle: String,
    /// column and descending?
    pub sort: Option<(Col, bool)>,
    pub filter: String,
    /// typing goes to the filter
    pub filtering: bool,
    /// selected position in the view order
    pub sel: usize,
    pub top: usize,
    /// visible rows last frame
    pub visible: usize,
}

impl TrackView {
    pub fn new(uri: &str, title: &str) -> TrackView {
        TrackView { uri: uri.into(), title: title.into(), subtitle: String::new(), sort: None, filter: String::new(), filtering: false, sel: 0, top: 0, visible: 1 }
    }

    /// Indexes into `rows` in display order.
    pub fn order(&self, rows: &[TrackRow]) -> Vec<usize> {
        let words: Vec<String> = self.filter.to_lowercase().split_whitespace().map(String::from).collect();
        let mut idx: Vec<usize> = (0..rows.len())
            .filter(|&i| {
                if words.is_empty() {
                    return true;
                }
                let r = &rows[i];
                let hay = format!("{} {} {}", r.name, r.artists.join(" "), r.album).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect();
        if let Some((col, desc)) = self.sort {
            let key = |r: &TrackRow| -> String {
                match col {
                    Col::Title => r.name.to_lowercase(),
                    Col::Artist => r.artists.join(", ").to_lowercase(),
                    Col::Album => r.album.to_lowercase(),
                    Col::Added => r.added_at.clone().unwrap_or_default(),
                    Col::Duration => format!("{:012}", r.duration_ms),
                }
            };
            // stable: ties keep the list's own order
            idx.sort_by_cached_key(|&i| key(&rows[i]));
            if desc {
                idx.reverse();
            }
        }
        idx
    }

    /// Click on a column header: ascending → descending → the list's own order.
    pub fn cycle_sort(&mut self, col: Col) {
        self.sort = match self.sort {
            Some((c, false)) if c == col => Some((col, true)),
            Some((c, true)) if c == col => None,
            _ => Some((col, false)),
        };
        self.sel = 0;
        self.top = 0;
    }

    pub fn move_sel(&mut self, delta: i64, len: usize) {
        if len == 0 {
            return;
        }
        self.sel = (self.sel as i64 + delta).clamp(0, len as i64 - 1) as usize;
        self.reveal();
    }

    pub fn reveal(&mut self) {
        let vis = self.visible.max(1);
        if self.sel < self.top {
            self.top = self.sel;
        } else if self.sel >= self.top + vis {
            self.top = self.sel + 1 - vis;
        }
    }

    pub fn set_filter(&mut self, f: String) {
        self.filter = f;
        self.sel = 0;
        self.top = 0;
    }
}

/// Widest a title column gets; artist and album columns stop at `NAME_MAX`.
const TITLE_MAX: u16 = 60;
const NAME_MAX: u16 = 40;

/// Column x-offsets and widths for a list `w` cells wide: (#, title, artist, album, added, time, ♥); 0 = hidden.
fn columns(w: u16) -> [(u16, u16); 7] {
    let num = 5;
    let time = 6;
    let heart = 2;
    let added = if w >= 110 { 12 } else { 0 };
    let rest = w.saturating_sub(num + time + heart + added + 2);
    let (title, artist, album) = if w >= 80 { (rest * 4 / 10, rest * 3 / 10, rest - rest * 4 / 10 - rest * 3 / 10) } else { (rest * 6 / 10, rest - rest * 6 / 10, 0) };
    // on wide Panes the columns stop growing, so the date and time stay near the names they belong to
    let (title, artist, album) = (title.min(TITLE_MAX), artist.min(NAME_MAX), album.min(NAME_MAX));
    let mut x = 0;
    let mut out = [(0, 0); 7];
    for (i, cw) in [num, title, artist, album, added, time, heart].into_iter().enumerate() {
        out[i] = (x, cw);
        x += cw;
    }
    out
}

/// "1 234 567"
pub fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn clip(s: &str, w: u16) -> String {
    let w = w.saturating_sub(1) as usize; // one cell of gutter
    if s.chars().count() <= w {
        return s.to_string();
    }
    let mut out: String = s.chars().take(w.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn fmt_dur(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let Some(mut view) = app.view.take() else { return };
    let empty = TrackList::default();
    // a page has arrived (an empty playlist's says total 0)
    let arrived = app.lists.contains_key(&view.uri);
    let list = app.lists.get(&view.uri).unwrap_or(&empty);
    let order = view.order(&list.rows);
    // hit-boxes are registered once the list is no longer borrowed
    let mut hits: Vec<(Rect, Hit)> = vec![];
    let focused = app.focus == "home";
    let x0 = r.x + 1;
    let w = r.width.saturating_sub(2);
    // header: title, count / progress, filter
    let count = if list.total == 0 && list.rows.is_empty() {
        if arrived { "no songs yet".to_string() } else { "loading…".to_string() }
    } else if list.complete() {
        format!("{} songs", list.total)
    } else {
        format!("{} songs · loading {}/{}", list.total, list.rows.len(), list.total)
    };
    text(buf, x0, r.y, w, &view.title, th.title());
    let tw = view.title.chars().count() as u16 + 2;
    text(buf, x0 + tw, r.y, w.saturating_sub(tw), &count, th.s(th.dim));
    // second line: ▶ Play and what this is
    let play = " ▶ Play ";
    text(buf, x0, r.y + 1, 8, play, th.chip(th.accent));
    hits.push((Rect::new(x0, r.y + 1, 8, 1), Hit::PlayView));
    // ♡ Save / ♥ Saved: the album or playlist in Your Library
    let mut sx = x0 + 9;
    if crate::saved::savable(&view.uri) {
        let saved = app.is_saved(&view.uri) == Some(true);
        let label = if saved { " ♥ Saved " } else { " ♡ Save " };
        let lw = label.chars().count() as u16;
        text(buf, sx, r.y + 1, lw, label, if saved { th.chip(th.accent2) } else { th.s(th.dim) });
        hits.push((Rect::new(sx, r.y + 1, lw, 1), Hit::Save(singularity_proto::Link { name: view.title.clone(), uri: view.uri.clone() })));
        sx += lw + 1;
    }
    // what it is (album: by its artists, year, label), with the artists as links
    let end = x0 + w;
    if let Some(h) = &list.header {
        for (i, a) in h.artists.iter().enumerate() {
            let label = if i == 0 { a.name.clone() } else { format!(", {}", a.name) };
            let lw = (label.chars().count() as u16).min(end.saturating_sub(sx));
            text(buf, sx, r.y + 1, lw, &label, th.s(th.fg));
            let skip = if i == 0 { 0 } else { 2 };
            hits.push((Rect::new(sx + skip, r.y + 1, lw.saturating_sub(skip), 1), Hit::Link(a.clone())));
            sx += lw;
        }
        if !h.artists.is_empty() {
            text(buf, sx, r.y + 1, 3, " · ", th.s(th.border));
            sx += 3;
        }
    }
    let mut sub = list.header.as_ref().map(|h| h.subtitle.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| view.subtitle.clone());
    // a playlist's description follows what it is, on one line
    if let Some(d) = list.header.as_ref().map(|h| h.description.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|d| !d.is_empty()) {
        sub = if sub.is_empty() { d } else { format!("{sub} — {d}") };
    }
    text(buf, sx, r.y + 1, end.saturating_sub(sx), &clip(&sub, end.saturating_sub(sx)), th.s(th.dim));
    let flabel = if view.filtering {
        format!("filter: {}▏", view.filter)
    } else if !view.filter.is_empty() {
        format!("filter: {}  ({} shown)", view.filter, order.len())
    } else {
        "/ filter".to_string()
    };
    let fw = flabel.chars().count() as u16;
    text(buf, r.right().saturating_sub(fw + 2), r.y, fw, &flabel, th.s(if view.filtering || !view.filter.is_empty() { th.accent } else { th.border }));
    // column headers: click to sort
    let cols = columns(w);
    let hy = r.y + 2;
    text(buf, x0 + cols[0].0, hy, cols[0].1, "#", th.s(th.dim));
    for (i, col) in Col::ALL.iter().enumerate() {
        let (cx, cw) = cols[i + 1];
        if cw == 0 {
            continue;
        }
        let arrow = match view.sort {
            Some((c, false)) if c == *col => " ↑",
            Some((c, true)) if c == *col => " ↓",
            _ => "",
        };
        let st = if arrow.is_empty() { th.s(th.dim) } else { th.s(th.accent) };
        text(buf, x0 + cx, hy, cw, &format!("{}{arrow}", col.label()), st);
        hits.push((Rect::new(x0 + cx, hy, cw, 1), Hit::SortCol(*col)));
    }
    for x in 0..w {
        text(buf, x0 + x, hy + 1, 1, "─", th.s(th.border));
    }
    // rows (only the visible slice is drawn, so thousands of tracks cost nothing)
    let body = Rect::new(x0, hy + 2, w, r.bottom().saturating_sub(hy + 2));
    view.visible = body.height.max(1) as usize;
    view.sel = view.sel.min(order.len().saturating_sub(1));
    view.top = view.top.min(order.len().saturating_sub(view.visible));
    let playing = app.playback.item.as_ref().map(|i| i.uri.clone());
    for (vi, &ri) in order.iter().enumerate().skip(view.top).take(view.visible) {
        let y = body.y + (vi - view.top) as u16;
        let row = &list.rows[ri];
        let selected = vi == view.sel;
        let now = playing.as_deref() == Some(row.uri.as_str());
        let bg = if selected && focused { th.selection.color() } else { th.bg.color() };
        // the row first, so the links inside it win
        hits.push((Rect::new(x0, y, w, 1), Hit::Track(vi)));
        let fg = if !row.playable { th.border } else if now { th.accent } else { th.fg };
        text(buf, x0, y, w, &" ".repeat(w as usize), th.base().bg(bg));
        let num = if now { "  ♪".to_string() } else { format!("{:>3}", ri + 1) };
        text(buf, x0 + cols[0].0, y, cols[0].1, &num, th.s(if now { th.accent } else { th.dim }).bg(bg));
        let title_st = th.s(fg).bg(bg).add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() });
        text(buf, x0 + cols[1].0, y, cols[1].1, &clip(&row.name, cols[1].1), title_st);
        text(buf, x0 + cols[2].0, y, cols[2].1, &clip(&row.artists.join(", "), cols[2].1), th.s(if row.playable { th.dim } else { th.border }).bg(bg));
        // each artist name, and the album, opens its page
        let mut ax = x0 + cols[2].0;
        let aend = x0 + cols[2].0 + cols[2].1.saturating_sub(1);
        for (i, (name, uri)) in row.artists.iter().zip(&row.artist_uris).enumerate() {
            if i > 0 {
                ax += 2;
            }
            let nw = (name.chars().count() as u16).min(aend.saturating_sub(ax));
            if nw == 0 {
                break;
            }
            hits.push((Rect::new(ax, y, nw, 1), Hit::Link(singularity_proto::Link { name: name.clone(), uri: uri.clone() })));
            ax += nw;
        }
        if cols[3].1 > 0 {
            text(buf, x0 + cols[3].0, y, cols[3].1, &clip(&row.album, cols[3].1), th.s(if row.playable { th.dim } else { th.border }).bg(bg));
            if let Some(au) = &row.album_uri {
                let aw = (row.album.chars().count() as u16).min(cols[3].1.saturating_sub(1));
                hits.push((Rect::new(x0 + cols[3].0, y, aw, 1), Hit::Link(singularity_proto::Link { name: row.album.clone(), uri: au.clone() })));
            }
        }
        if cols[4].1 > 0 {
            let added = row.added_at.as_deref().map(|d| d.get(..10).unwrap_or(d)).unwrap_or("");
            text(buf, x0 + cols[4].0, y, cols[4].1, added, th.s(th.dim).bg(bg));
        }
        text(buf, x0 + cols[5].0, y, cols[5].1, &format!("{:>5}", fmt_dur(row.duration_ms)), th.s(th.dim).bg(bg));
        let liked = if view.uri == LIKED { Some(true) } else { app.is_saved(&row.uri) };
        hits.push(crate::saved::heart(buf, x0 + cols[6].0 + 1, y, &row.uri, &row.name, liked, bg, th));
    }
    // the ♥s in view (and the next rows) are asked about in one batch
    let ahead: Vec<String> = order.iter().skip(view.top).take(50).map(|&ri| list.rows[ri].uri.clone()).collect();
    if order.is_empty() && list.complete() && list.total > 0 {
        text(buf, x0, body.y + 1, w, "no tracks match the filter", th.s(th.dim));
    }
    for (r, h) in hits {
        app.hit(r, h);
    }
    if view.uri != LIKED {
        app.want_saved(ahead.iter().map(String::as_str), view.visible);
    }
    app.view = Some(view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_stop_growing_on_wide_panes() {
        for w in [60u16, 80, 120, 200, 400] {
            let c = columns(w);
            assert!(c[1].1 <= TITLE_MAX && c[2].1 <= NAME_MAX && c[3].1 <= NAME_MAX, "{w}: {c:?}");
            assert!(c[6].0 + c[6].1 <= w, "{w}: nothing past the Pane");
        }
        // the date and time follow the names instead of the far edge
        let c = columns(400);
        assert_eq!(c[4].0, 5 + TITLE_MAX + 2 * NAME_MAX);
    }

    fn row(i: u32, name: &str, artist: &str, ms: u64) -> TrackRow {
        TrackRow { uri: format!("spotify:track:{i}"), name: name.into(), artists: vec![artist.into()], album: format!("Album {i}"), added_at: Some(format!("2025-01-{:02}", 10 - i)), duration_ms: ms, playable: true, ..Default::default() }
    }

    fn rows() -> Vec<TrackRow> {
        vec![row(0, "Perihelion", "Nova Drift", 200), row(1, "aphelion", "Mira Vale", 100), row(2, "Corona", "Nova Drift", 300)]
    }

    #[test]
    fn sorting_cycles_and_is_case_insensitive() {
        let mut v = TrackView::new("u", "t");
        assert_eq!(v.order(&rows()), [0, 1, 2]);
        v.cycle_sort(Col::Title);
        assert_eq!(v.order(&rows()), [1, 2, 0]);
        v.cycle_sort(Col::Title);
        assert_eq!(v.order(&rows()), [0, 2, 1]);
        v.cycle_sort(Col::Title);
        assert_eq!(v.order(&rows()), [0, 1, 2]);
        v.cycle_sort(Col::Duration);
        assert_eq!(v.order(&rows()), [1, 0, 2]);
        v.cycle_sort(Col::Added);
        assert_eq!(v.order(&rows()), [2, 1, 0]);
    }

    #[test]
    fn filter_matches_all_words_across_fields() {
        let mut v = TrackView::new("u", "t");
        v.set_filter("nova cor".into());
        assert_eq!(v.order(&rows()), [2]);
        v.set_filter("MIRA".into());
        assert_eq!(v.order(&rows()), [1]);
    }

    #[test]
    fn pages_fill_in_by_offset_and_replays_are_harmless() {
        let mut l = TrackList::default();
        let page = |offset: u32, n: u32| TrackPage { uri: "u".into(), offset, total: 5, tracks: (offset..offset + n).map(|i| row(i, "x", "y", 1)).collect(), header: None };
        l.add(&page(0, 3));
        assert!(!l.complete());
        l.add(&page(3, 2));
        assert!(l.complete() && l.rows.len() == 5);
        l.add(&page(0, 3)); // cache replay
        assert_eq!(l.rows.len(), 5);
        assert_eq!(l.rows[4].uri, "spotify:track:4");
    }

    #[test]
    fn selection_scrolls_into_view() {
        let mut v = TrackView::new("u", "t");
        v.visible = 10;
        v.move_sel(25, 100);
        assert_eq!((v.sel, v.top), (25, 16));
        v.move_sel(-20, 100);
        assert_eq!((v.sel, v.top), (5, 5));
        v.move_sel(1000, 100);
        assert_eq!(v.sel, 99);
    }
}
