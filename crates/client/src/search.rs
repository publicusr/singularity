//! Search (#10): results while you type (debounced), filter chips, songs as a playable list and everything else as
//! Card Shelves, and recent searches (kept locally, owner-only).

use crate::app::{App, Hit};
use crate::home::HomeView;
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::{HomeFeed, SearchResults, Shelf};
use std::time::{Duration, Instant};

/// Typing pauses this long before a search goes out.
pub const DEBOUNCE: Duration = Duration::from_millis(300);
pub const CHIPS: [&str; 6] = ["All", "Songs", "Artists", "Albums", "Playlists", "Podcasts"];
const RECENT_MAX: usize = 12;

#[derive(Default)]
pub struct SearchView {
    pub query: String,
    pub chip: usize,
    pub results: Option<SearchResults>,
    /// the query last sent, and when the user last typed
    sent: String,
    typed_at: Option<Instant>,
    /// the Card Shelves for the current chip
    pub feed: HomeFeed,
    pub shelves: HomeView,
    /// selected song row (None: the selection is in the Shelves)
    pub track_sel: Option<usize>,
    pub recent_sel: usize,
}

impl SearchView {
    pub fn typed(&mut self) {
        self.typed_at = Some(Instant::now());
        self.track_sel = None;
    }

    /// The query to send now, if typing has paused on a new one (2+ characters).
    pub fn due(&mut self) -> Option<String> {
        let q = self.query.trim().to_string();
        let quiet = self.typed_at.is_some_and(|t| t.elapsed() >= DEBOUNCE);
        if quiet && q.chars().count() >= 2 && q != self.sent {
            self.sent = q.clone();
            return Some(q);
        }
        None
    }

    pub fn set_results(&mut self, r: SearchResults) {
        if r.query.trim() != self.query.trim() {
            return; // an older answer: the user has typed on
        }
        self.results = Some(r);
        self.rebuild();
    }

    pub fn set_chip(&mut self, chip: usize) {
        self.chip = chip % CHIPS.len();
        self.track_sel = None;
        self.rebuild();
    }

    /// Songs shown above the Shelves: a few under All, all of them under Songs, none otherwise.
    pub fn songs(&self) -> usize {
        let n = self.results.as_ref().map(|r| r.tracks.len()).unwrap_or(0);
        match self.chip {
            0 => n.min(5),
            1 => n,
            _ => 0,
        }
    }

    fn rebuild(&mut self) {
        let Some(r) = &self.results else { return };
        let shelf = |t: &str, kind: &str, cards: &Vec<singularity_proto::Card>| Shelf { title: t.into(), kind: kind.into(), cards: cards.clone() };
        let shelves = match self.chip {
            0 => vec![shelf("Top results", "shelf", &r.top), shelf("Artists", "shelf", &r.artists), shelf("Albums", "shelf", &r.albums), shelf("Playlists", "shelf", &r.playlists), shelf("Podcasts", "shelf", &r.podcasts)],
            2 => vec![shelf("Artists", "grid", &r.artists)],
            3 => vec![shelf("Albums", "grid", &r.albums)],
            4 => vec![shelf("Playlists", "grid", &r.playlists)],
            5 => vec![shelf("Podcasts", "grid", &r.podcasts)],
            _ => vec![],
        };
        self.feed = HomeFeed { fetched_unix_ms: 0, shelves: shelves.into_iter().filter(|s| !s.cards.is_empty()).collect() };
        self.shelves = HomeView::default();
    }
}

/// Remember `q` at the front of the recent searches (no duplicates, newest first).
pub fn remember(recent: &mut Vec<String>, q: &str) {
    let q = q.trim();
    if q.chars().count() < 2 {
        return;
    }
    recent.retain(|r| !r.eq_ignore_ascii_case(q));
    recent.insert(0, q.to_string());
    recent.truncate(RECENT_MAX);
}

fn recent_file() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".local/state")).join("singularity/recent-searches.json")
}

pub fn load_recent() -> Vec<String> {
    std::fs::read_to_string(recent_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Saved owner-only: what you searched for is personal. (Tests never touch the real file.)
pub fn save_recent(recent: &[String]) {
    if cfg!(test) {
        return;
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let p = recent_file();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&p) {
        let _ = f.write_all(serde_json::to_string(recent).unwrap_or_default().as_bytes());
    }
}

fn clip(s: &str, w: u16) -> String {
    let w = w as usize;
    if s.chars().count() <= w {
        return s.to_string();
    }
    let mut o: String = s.chars().take(w.saturating_sub(1)).collect();
    o.push('…');
    o
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let Some(mut v) = app.search.take() else { return };
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    let focused = app.focus == "home";
    // the query
    text(buf, x, r.y, 2, "⌕", th.s(th.accent));
    let shown = if v.query.is_empty() { "what do you want to listen to?".to_string() } else { v.query.clone() };
    text(buf, x + 2, r.y, w.saturating_sub(2), &format!("{shown}{}", if focused { "▏" } else { "" }), th.s(if v.query.is_empty() { th.dim } else { th.fg }).add_modifier(Modifier::BOLD));
    let hint = "Tab: filter · Esc: back";
    let hw = hint.chars().count() as u16;
    if w > hw + 30 {
        text(buf, x + w - hw, r.y, hw, hint, th.s(th.border));
    }
    // chips
    let mut cx = x;
    let mut hits = vec![];
    for (i, c) in CHIPS.iter().enumerate() {
        let label = format!(" {c} ");
        let lw = label.chars().count() as u16;
        if cx + lw > x + w {
            break;
        }
        let st = if i == v.chip { th.chip(th.accent) } else { th.s(th.dim) };
        text(buf, cx, r.y + 1, lw, &label, st);
        hits.push((Rect::new(cx, r.y + 1, lw, 1), Hit::Chip(i)));
        cx += lw + 1;
    }
    let mut y = r.y + 3;
    if v.query.trim().is_empty() {
        if app.recent.is_empty() {
            text(buf, x, y, w, "Search for songs, artists, albums, playlists and podcasts.", th.s(th.dim));
        } else {
            text(buf, x, y, w, "Recent searches", th.s(th.accent).add_modifier(Modifier::BOLD));
            y += 1;
            for (i, q) in app.recent.iter().enumerate() {
                if y >= r.bottom() {
                    break;
                }
                let sel = focused && i == v.recent_sel;
                let st = if sel { th.s(th.accent).bg(th.selection.color()) } else { th.s(th.fg) };
                text(buf, x, y, w, &format!("  {}", clip(q, w.saturating_sub(2))), st);
                hits.push((Rect::new(x, y, w, 1), Hit::Recent(i)));
                y += 1;
            }
        }
    } else if let Some(res) = v.results.clone() {
        // under All, songs give way so at least one Shelf of Cards fits below them
        let room = r.bottom().saturating_sub(y + 2) as usize;
        let n = if v.chip == 0 && !v.feed.shelves.is_empty() { v.songs().min(room.saturating_sub(10)).max(v.songs().min(2)) } else { v.songs() };
        if n > 0 {
            text(buf, x, y, w, "Songs", th.s(th.accent).add_modifier(Modifier::BOLD));
            y += 1;
            let playing = app.playback.item.as_ref().map(|i| i.uri.clone());
            for (i, t) in res.tracks.iter().take(n).enumerate() {
                if y >= r.bottom() {
                    break;
                }
                let sel = focused && v.track_sel == Some(i);
                let bg = if sel { th.selection.color() } else { th.bg.color() };
                text(buf, x, y, w, &" ".repeat(w as usize), th.base().bg(bg));
                let now = playing.as_deref() == Some(t.uri.as_str());
                hits.push((Rect::new(x, y, w, 1), Hit::SearchTrack(i)));
                let nw = w.saturating_sub(8) / 2;
                text(buf, x + 1, y, nw, &clip(&t.name, nw.saturating_sub(1)), th.s(if now { th.accent } else { th.fg }).bg(bg));
                let aw = w.saturating_sub(8 + nw);
                text(buf, x + 1 + nw, y, aw, &clip(&t.artists.join(", "), aw.saturating_sub(1)), th.s(th.dim).bg(bg));
                // the artists open their pages
                let mut ax = x + 1 + nw;
                for (k, (name, uri)) in t.artists.iter().zip(&t.artist_uris).enumerate() {
                    if k > 0 {
                        ax += 2;
                    }
                    let lw = (name.chars().count() as u16).min((x + 1 + nw + aw).saturating_sub(ax + 1));
                    if lw == 0 {
                        break;
                    }
                    hits.push((Rect::new(ax, y, lw, 1), Hit::Link(singularity_proto::Link { name: name.clone(), uri: uri.clone() })));
                    ax += lw;
                }
                let d = t.duration_ms / 1000;
                text(buf, x + w - 6, y, 6, &format!("{:>2}:{:02}", d / 60, d % 60), th.s(th.dim).bg(bg));
                y += 1;
            }
            y += 1;
        }
        if v.feed.shelves.is_empty() && n == 0 && y < r.bottom() {
            text(buf, x, y, w, &format!("nothing found for “{}”", v.query.trim()), th.s(th.dim));
        }
        if y < r.bottom() && !v.feed.shelves.is_empty() {
            for (rect, h) in hits.drain(..) {
                app.hit(rect, h);
            }
            let feed = std::mem::take(&mut v.feed);
            let view = std::mem::take(&mut v.shelves);
            app.search = Some(v);
            let (feed, view) = crate::home::draw_feed(buf, Rect::new(r.x, y, r.width, r.bottom() - y), app, th, feed, view);
            if let Some(s) = app.search.as_mut() {
                s.feed = feed;
                s.shelves = view;
            }
            return;
        }
    } else {
        text(buf, x, y, w, "searching…", th.s(th.dim));
    }
    for (rect, h) in hits {
        app.hit(rect, h);
    }
    app.search = Some(v);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debounce_waits_for_a_pause_and_two_characters() {
        let mut v = SearchView { query: "d".into(), ..Default::default() };
        v.typed();
        std::thread::sleep(DEBOUNCE + Duration::from_millis(10));
        assert_eq!(v.due(), None, "one character isn't a search");
        v.query.push('a');
        v.typed();
        assert_eq!(v.due(), None, "still typing");
        std::thread::sleep(DEBOUNCE + Duration::from_millis(10));
        assert_eq!(v.due().as_deref(), Some("da"));
        assert_eq!(v.due(), None, "sent once");
    }

    #[test]
    fn recent_searches_are_newest_first_without_duplicates() {
        let mut r = vec![];
        for q in ["daft punk", "nova", "Daft Punk", "x"] {
            remember(&mut r, q);
        }
        assert_eq!(r, ["Daft Punk", "nova"]);
    }

    #[test]
    fn stale_answers_are_ignored() {
        let mut v = SearchView { query: "nova dr".into(), ..Default::default() };
        v.set_results(SearchResults { query: "nova".into(), ..Default::default() });
        assert!(v.results.is_none());
        v.set_results(SearchResults { query: "nova dr".into(), ..Default::default() });
        assert!(v.results.is_some());
    }
}
