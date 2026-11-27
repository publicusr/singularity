//! Search (#10): results while you type (debounced), filter chips, songs as a playable list and everything else as
//! Card Shelves, and recent searches (kept locally, owner-only).

use crate::app::{App, Hit};
use crate::home::HomeView;
use crate::scroll::Wheeled;
use crate::theme::Theme;
use crate::ui::{text, blank};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::{HomeFeed, SearchResults, Shelf};
use std::time::{Duration, Instant};

/// Typing pauses this long before a search goes out.
pub const DEBOUNCE: Duration = Duration::from_millis(300);
pub const CHIPS: [&str; 7] = ["All", "Songs", "Artists", "Albums", "Playlists", "Podcasts", "Audiobooks"];
const RECENT_MAX: usize = 12;

/// Results kept this long by query, so going back or picking a recent search shows them at once; a fresh answer is
/// still asked for (the Daemon keeps its own for 5 minutes).
pub const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const CACHE_MAX: usize = 16;
/// A Search Pane under this many rows (inside its frame) is short: no headings or blank rows, the songs or the Shelves
/// under the chips (the smallest Homes, #55, have 4 and 5)
const SHORT_H: u16 = 6;
/// Song rows narrower than this show the name alone (beside the artists and the time it would get under 8 columns).
const NARROW_W: u16 = 24;

/// Search (D74): the query and everything that decides what goes out and when, the recent searches, results kept by
/// query, and the page's own view state. App owns one for its whole life; leaving the page keeps it as it was.
#[derive(Default)]
pub struct Search {
    query: String,
    results: Option<SearchResults>,
    /// the last search failed: why
    failed: Option<String>,
    /// the query last sent (empty: nothing is out), and when the user last typed
    sent: String,
    typed_at: Option<Instant>,
    /// a picked recent search or Enter: goes out on the next `due` without waiting for a pause
    skip_pause: bool,
    recent: Vec<String>,
    /// results by `key`, newest last
    cache: Vec<(String, Instant, SearchResults)>,
    pub chip: usize,
    /// the Card Shelves for the current chip
    pub feed: HomeFeed,
    pub shelves: HomeView,
    /// selected song row (None: the selection is in the Shelves)
    pub track_sel: Option<usize>,
    pub recent_sel: usize,
    /// the first song and recent search shown, and whether the wheel has those views (D86)
    pub track_first: usize,
    pub track_wheeled: Wheeled,
    pub recent_first: usize,
    pub recent_wheeled: Wheeled,
    /// a short Pane: the wheel showed the Shelves (true) or the songs (false); None: where the highlight is
    pub shelves_shown: Option<bool>,
    /// the Pane was short last frame (under `SHORT_H` rows): the songs and the Shelves take turns under the chips
    pub short: bool,
    /// how many song rows showed last frame: a PgUp or PgDn's step
    pub song_rows: usize,
}

/// How queries are matched, as the Daemon matches them.
fn key(q: &str) -> String {
    q.trim().to_lowercase()
}

impl Search {
    /// Search with these recent searches (newest first).
    pub fn with_recent(recent: Vec<String>) -> Self {
        Self { recent, ..Default::default() }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// The answer for the query on screen (None: none yet).
    pub fn results(&self) -> Option<&SearchResults> {
        self.results.as_ref()
    }

    /// The results on screen answer the query as it is now.
    pub fn answered(&self) -> bool {
        self.results.as_ref().is_some_and(|r| key(&r.query) == key(&self.query))
    }

    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    pub fn recent(&self) -> &[String] {
        &self.recent
    }

    /// Typed (or pasted) text, at the end of the query.
    pub fn type_text(&mut self, t: &str) {
        self.query.push_str(t);
        self.edited();
    }

    /// Backspace: false when the query was already empty (Backspace then means back).
    pub fn erase(&mut self) -> bool {
        if self.query.pop().is_none() {
            return false;
        }
        self.edited();
        true
    }

    /// Esc: false when the query was already empty (Esc then means back).
    pub fn clear(&mut self) -> bool {
        if self.query.is_empty() {
            return false;
        }
        self.query.clear();
        self.edited();
        true
    }

    /// Recent search `i` runs again, at once.
    pub fn pick(&mut self, i: usize) {
        let Some(q) = self.recent.get(i).cloned() else { return };
        self.query = q;
        self.edited();
        self.run_now();
    }

    /// Enter: the recent search under the highlight while the query is empty, else the query as it is, at once.
    pub fn submit(&mut self) {
        if self.query.trim().is_empty() {
            self.pick(self.recent_sel);
        } else {
            self.run_now();
        }
    }

    /// The query changed: what was out no longer counts for it (#8: an emptied query that was sent before must go out
    /// again when it comes back). The results on screen stay until the new answer, unless the query is now empty.
    fn edited(&mut self) {
        self.failed = None;
        self.typed_at = Some(Instant::now());
        self.songs_from_top();
        self.skip_pause = false;
        if key(&self.query) != key(&self.sent) {
            self.sent.clear();
        }
        if self.query.trim().is_empty() {
            self.results = None;
            self.rebuild();
        }
    }

    /// Go out on the next `due`, even if these words went out before (a fresh answer replaces a kept one).
    fn run_now(&mut self) {
        self.failed = None;
        self.sent.clear();
        self.skip_pause = true;
    }

    /// The query to send now, if any: when typing has paused on a new one (2+ characters), or at once after a pick or
    /// Enter. Kept results for it show straight away.
    pub fn due(&mut self) -> Option<String> {
        self.due_at(Instant::now())
    }

    fn due_at(&mut self, now: Instant) -> Option<String> {
        let q = self.query.trim().to_string();
        let quiet = self.skip_pause || self.typed_at.is_some_and(|t| now.duration_since(t) >= DEBOUNCE);
        if !quiet || q.chars().count() < 2 || key(&q) == key(&self.sent) {
            return None;
        }
        self.skip_pause = false;
        self.sent = q.clone();
        self.cache.retain(|(_, at, _)| now.duration_since(*at) < CACHE_TTL);
        if let Some((_, _, r)) = self.cache.iter().find(|(k, _, _)| *k == key(&q)) {
            self.results = Some(r.clone());
            self.rebuild();
        }
        Some(q)
    }

    /// An answer from the Daemon. One for other words is kept for later but not shown.
    pub fn receive(&mut self, r: SearchResults) {
        self.receive_at(r, Instant::now());
    }

    fn receive_at(&mut self, r: SearchResults, now: Instant) {
        let current = key(&r.query) == key(&self.query);
        if let Some(e) = r.error {
            if current {
                // shown instead of "searching…" and of older words' results; the same words go out again on Enter
                // (or once typing changes them), never by themselves
                self.failed = Some(e);
                if !self.answered() {
                    self.results = None;
                    self.rebuild();
                }
            }
            return;
        }
        self.cache.retain(|(k, _, _)| *k != key(&r.query));
        self.cache.push((key(&r.query), now, r.clone()));
        if self.cache.len() > CACHE_MAX {
            self.cache.remove(0);
        }
        // the same answer again (a fresh one for kept results, usually unchanged) leaves the page as the user has it
        if current && self.results.as_ref() != Some(&r) {
            self.failed = None;
            self.results = Some(r);
            self.songs_from_top();
            self.rebuild();
        }
    }

    /// A result was opened or played: the query is worth remembering (newest first, no duplicates). Saved on disk.
    pub fn remember(&mut self) {
        let q = self.query.trim();
        if q.chars().count() < 2 {
            return;
        }
        self.recent.retain(|r| key(r) != key(q));
        self.recent.insert(0, q.to_string());
        self.recent.truncate(RECENT_MAX);
        save_recent(&self.recent);
    }

    /// Other songs: no highlight, shown from the first.
    fn songs_from_top(&mut self) {
        self.track_sel = None;
        (self.track_first, self.track_wheeled, self.shelves_shown) = (0, Wheeled::default(), None);
    }

    /// A short Pane shows the Shelves (not the songs): where the wheel took it, else where the highlight is.
    pub fn showing_shelves(&self) -> bool {
        self.shelves_shown.unwrap_or(self.shelves.sel.is_some())
    }

    /// The highlight among the songs, as the songs' view knows it (None: in the Shelves).
    pub fn track_key(&self) -> usize {
        self.track_sel.unwrap_or(usize::MAX)
    }

    pub fn set_chip(&mut self, chip: usize) {
        self.chip = chip % CHIPS.len();
        self.songs_from_top();
        self.rebuild();
    }

    /// The rows above the Shelves: songs, or under Podcasts its episodes (#61).
    pub fn rows(&self) -> &[singularity_proto::TrackRow] {
        match &self.results {
            Some(r) if self.chip == 5 => &r.episodes,
            Some(r) => &r.tracks,
            None => &[],
        }
    }

    /// Rows shown above the Shelves: a few songs under All, all of them under Songs, the episodes under Podcasts.
    pub fn songs(&self) -> usize {
        let n = self.rows().len();
        match self.chip {
            0 => n.min(5),
            1 | 5 => n,
            _ => 0,
        }
    }

    fn rebuild(&mut self) {
        let Some(r) = &self.results else {
            self.feed = HomeFeed::default();
            self.shelves = HomeView::default();
            return;
        };
        let shelf = |t: &str, kind: &str, cards: &Vec<singularity_proto::Card>| Shelf { id: String::new(), title: t.into(), kind: kind.into(), cards: cards.clone() };
        let shelves = match self.chip {
            0 => vec![shelf("Top results", "shelf", &r.top), shelf("Artists", "shelf", &r.artists), shelf("Albums", "shelf", &r.albums), shelf("Playlists", "shelf", &r.playlists), shelf("Podcasts", "shelf", &r.podcasts), shelf("Audiobooks", "shelf", &r.audiobooks)],
            2 => vec![shelf("Artists", "grid", &r.artists)],
            3 => vec![shelf("Albums", "grid", &r.albums)],
            4 => vec![shelf("Playlists", "grid", &r.playlists)],
            5 => vec![shelf("Podcasts", "grid", &r.podcasts)],
            6 => vec![shelf("Audiobooks", "grid", &r.audiobooks)],
            _ => vec![],
        };
        self.feed = HomeFeed { fetched_unix_ms: 0, shelves: shelves.into_iter().filter(|s| !s.cards.is_empty()).collect() };
        self.shelves = HomeView::default();
    }
}

fn recent_file() -> std::path::PathBuf {
    // demo mode keeps its own (D90): the real ones show as soon as Search opens
    let name = if crate::demo::on() { "recent-searches-demo.json" } else { "recent-searches.json" };
    singularity_config::app_folders::AppFolders::this().state.join(name)
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

/// The search bar (D70, D72): one filled line, the same on Home and on the Search page, so opening Search brings it
/// to life in place, on the first line of `r`. `active` shows the cursor; `hint` sits at the right when there is room.
pub fn bar(buf: &mut Buffer, r: Rect, query: &str, active: bool, hint: &str, th: &Theme) {
    let (x, y, w) = (r.x, r.y, r.width);
    if w < 4 {
        return;
    }
    let fill = th.base().bg(th.selection.color());
    blank(buf, x, y, w, fill);
    text(buf, x + 1, y, 2, "⌕", th.s(if active { th.accent } else { th.fg }).bg(th.selection.color()));
    let hw = hint.chars().count() as u16;
    let room = w.saturating_sub(4);
    let show_hint = !hint.is_empty() && room > hw + 12;
    let tw = if show_hint { room - hw - 2 } else { room };
    let (shown, colour) = if query.is_empty() { ("Search".to_string(), th.dim) } else { (query.to_string(), th.fg) };
    // the end of a long query stays in view, where the typing happens
    let n = shown.chars().count();
    let fit = (tw as usize).saturating_sub(1);
    let shown: String = if n > fit { shown.chars().skip(n - fit).collect() } else { shown };
    let cursor = if active { "▏" } else { "" };
    let line = if query.is_empty() && active { format!("{cursor}{shown}") } else { format!("{shown}{cursor}") };
    text(buf, x + 3, y, tw, &line, th.s(colour).add_modifier(Modifier::BOLD).bg(th.selection.color()));
    if show_hint {
        text(buf, x + w - hw - 1, y, hw, hint, th.s(th.dim).bg(th.selection.color()));
    }
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if !matches!(app.page, crate::app::Page::Search) {
        return;
    }
    let mut v = std::mem::take(&mut app.search);
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    let focused = app.is_focused(crate::layout::Widget::Home);
    // the query, in the same bar Home shows
    bar(buf, Rect::new(x, r.y, w, 1), v.query(), focused, "Tab: filter · Esc: back", th);
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
    // a short Pane (the smallest Homes, #55): no blank rows or headings, what was found right under the chips; the
    // songs, or the Shelves once the highlight or the wheel is in them (owner: in a tiny Home the results didn't show)
    v.short = r.height < SHORT_H;
    let mut y = r.y + if v.short { 2 } else { 3 };
    if v.query().trim().is_empty() {
        if v.recent().is_empty() {
            text(buf, x, y, w, "Search for songs, artists, albums, playlists, podcasts and audiobooks.", th.s(th.dim));
        } else {
            if !v.short {
                text(buf, x, y, w, "Recent searches", th.s(th.accent).add_modifier(Modifier::BOLD));
                y += 1;
            }
            // a short Pane shows a window that follows the highlight (↑ ↓), or where the wheel put it
            let (rows, len) = (r.bottom().saturating_sub(y) as usize, v.recent().len());
            let end = len.saturating_sub(rows);
            let first = if v.recent_wheeled.follows(v.recent_sel, end) { crate::scroll::window(v.recent_sel, rows, len) } else { v.recent_first.min(end) };
            v.recent_first = first;
            for (i, q) in v.recent().iter().enumerate().skip(first) {
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
    } else if let Some(res) = v.results().cloned() {
        // under All, songs give way so at least one Shelf of Cards fits below them
        let room = r.bottom().saturating_sub(y + 2) as usize;
        // (with no Shelves below, the songs take the Pane)
        let n = if v.short {
            if !v.showing_shelves() || v.feed.shelves.is_empty() { v.songs().min(r.bottom().saturating_sub(y) as usize) } else { 0 }
        } else if !v.feed.shelves.is_empty() {
            v.songs().min(room.saturating_sub(10)).max(v.songs().min(2))
        } else {
            v.songs().min(room + 1)
        };
        v.song_rows = n;
        // (the Shelves hidden behind the songs: laid out all the same, so ↓ and the wheel can go into them)
        if v.short && n > 0 {
            let aspect = app.cell_px.map(|(cw, ch)| ch / cw).unwrap_or(2.0);
            v.shelves.rows = crate::home::rows(&v.feed, r.width, r.bottom().saturating_sub(y), aspect).0;
        }
        let episodes = v.chip == 5;
        let rows = if episodes { &res.episodes } else { &res.tracks };
        if n > 0 {
            if !v.short {
                text(buf, x, y, w, if episodes { "Episodes" } else { "Songs" }, th.s(th.accent).add_modifier(Modifier::BOLD));
                y += 1;
            }
            let playing = app.playback.item.as_ref().map(|i| i.uri.clone());
            // more songs than fit: a window that follows the highlight (↑ ↓), or where the wheel put it
            let end = v.songs().saturating_sub(n);
            let skip = if v.track_wheeled.follows(v.track_key(), end) { crate::scroll::window(v.track_sel.unwrap_or(0), n, v.songs()) } else { v.track_first.min(end) };
            v.track_first = skip;
            if !episodes {
                app.want_saved(rows.iter().skip(skip).take(n).map(|t| t.uri.as_str()), n);
            }
            for (i, t) in rows.iter().enumerate().skip(skip).take(n) {
                if y >= r.bottom() {
                    break;
                }
                let sel = focused && v.track_sel == Some(i);
                let bg = if sel { th.selection.color() } else { th.bg.color() };
                blank(buf, x, y, w, th.base().bg(bg));
                let now = playing.as_deref() == Some(t.uri.as_str());
                hits.push((Rect::new(x, y, w, 1), Hit::SearchTrack(i)));
                // too narrow for the name beside the artists and the time (under 8 columns for it): the name alone,
                // and a song's ♥ (owner: a tiny Home's results were only "…")
                if w < NARROW_W {
                    let nw = w.saturating_sub(3);
                    text(buf, x + 1, y, nw, &clip(&t.name, nw), th.s(if now { th.accent } else { th.fg }).bg(bg));
                    if !episodes {
                        hits.push(crate::saved::heart(buf, x + w - 1, y, &t.uri, &t.name, app.is_saved(&t.uri), bg, th));
                    }
                    y += 1;
                    continue;
                }
                let nw = w.saturating_sub(8) / 2;
                text(buf, x + 1, y, nw, &clip(&t.name, nw.saturating_sub(1)), th.s(if now { th.accent } else { th.fg }).bg(bg));
                let aw = w.saturating_sub(10 + nw);
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
                if episodes {
                    // an episode: how far you are and how long, where a song has its time and ♥
                    let p = crate::tracks::progress_label(t.progress, t.duration_ms);
                    let pw = p.chars().count() as u16;
                    let tail = format!("{:>3} min", (d / 60).max(1));
                    text(buf, x + w - 8 - pw - 2, y, pw, &p, th.s(if t.progress.is_some_and(|p| p.done) { th.border } else { th.accent2 }).bg(bg));
                    text(buf, x + w - 8, y, 8, &tail, th.s(th.dim).bg(bg));
                } else {
                    text(buf, x + w - 8, y, 6, &format!("{:>2}:{:02}", d / 60, d % 60), th.s(th.dim).bg(bg));
                    hits.push(crate::saved::heart(buf, x + w - 1, y, &t.uri, &t.name, app.is_saved(&t.uri), bg, th));
                }
                y += 1;
            }
            y += 1;
        }
        if v.feed.shelves.is_empty() && n == 0 && y < r.bottom() {
            text(buf, x, y, w, &format!("nothing found for “{}”", v.query().trim()), th.s(th.dim));
        }
        if y < r.bottom() && !v.feed.shelves.is_empty() {
            for (rect, h) in hits.drain(..) {
                app.hit(rect, h);
            }
            let feed = std::mem::take(&mut v.feed);
            let view = std::mem::take(&mut v.shelves);
            app.search = v;
            let (feed, view) = crate::home::draw_feed(buf, Rect::new(r.x, y, r.width, r.bottom() - y), app, th, feed, view);
            app.search.feed = feed;
            app.search.shelves = view;
            return;
        }
    } else if let Some(e) = v.failed() {
        text(buf, x, y, w, &format!("search failed: {e} · Enter tries again"), th.s(th.dim));
    } else {
        text(buf, x, y, w, "searching…", th.s(th.dim));
    }
    for (rect, h) in hits {
        app.hit(rect, h);
    }
    app.search = v;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(q: &str) -> SearchResults {
        SearchResults { query: q.into(), tracks: vec![singularity_proto::TrackRow { uri: format!("spotify:track:{q}"), name: q.into(), ..Default::default() }], ..Default::default() }
    }

    /// `s` ran `q` (typed it, waited, sent it) and its answer arrived.
    fn ran(s: &mut Search, q: &str) {
        s.type_text(q);
        assert_eq!(s.due_at(Instant::now() + DEBOUNCE).as_deref(), Some(q));
        s.receive(answer(q));
    }

    #[test]
    fn debounce_waits_for_a_pause_and_two_characters() {
        let mut s = Search::default();
        s.type_text("d");
        std::thread::sleep(DEBOUNCE + Duration::from_millis(10));
        assert_eq!(s.due(), None, "one character isn't a search");
        s.type_text("a");
        assert_eq!(s.due(), None, "still typing");
        std::thread::sleep(DEBOUNCE + Duration::from_millis(10));
        assert_eq!(s.due().as_deref(), Some("da"));
        assert_eq!(s.due(), None, "sent once");
    }

    #[test]
    fn a_cleared_search_picked_again_goes_out_at_once() {
        // #8: after Esc the old "sent" made the picked search look already asked, so it never went out
        let mut s = Search::with_recent(vec!["nova".into()]);
        ran(&mut s, "nova");
        assert!(s.clear());
        s.pick(0);
        assert_eq!(s.query(), "nova");
        assert_eq!(s.due().as_deref(), Some("nova"), "no debounce for a pick, and it goes out again");
    }

    #[test]
    fn erasing_to_empty_and_typing_the_same_words_asks_again() {
        let mut s = Search::default();
        ran(&mut s, "nova");
        while s.erase() {}
        assert!(s.results().is_none(), "an empty query shows the recent searches, not old results");
        s.type_text("nova");
        assert_eq!(s.due_at(Instant::now() + DEBOUNCE).as_deref(), Some("nova"));
    }

    #[test]
    fn a_picked_search_shows_its_kept_results_at_once_and_still_asks() {
        let mut s = Search::with_recent(vec!["nova".into(), "orbit".into()]);
        ran(&mut s, "orbit");
        s.clear();
        ran(&mut s, "nova");
        s.clear();
        s.pick(1);
        assert_eq!(s.due().as_deref(), Some("orbit"), "a fresh answer is asked for");
        assert_eq!(s.results().map(|r| r.query.as_str()), Some("orbit"), "the kept one shows meanwhile");
        assert_eq!(s.rows().len(), 1);
    }

    #[test]
    fn kept_results_expire() {
        let mut s = Search::with_recent(vec!["nova".into()]);
        ran(&mut s, "nova");
        s.clear();
        s.pick(0);
        assert_eq!(s.due_at(Instant::now() + CACHE_TTL + Duration::from_secs(1)).as_deref(), Some("nova"));
        assert!(s.results().is_none(), "too old to show");
    }

    #[test]
    fn enter_sends_typed_words_without_waiting() {
        let mut s = Search::default();
        s.type_text("nova");
        assert_eq!(s.due(), None);
        s.submit();
        assert_eq!(s.due().as_deref(), Some("nova"));
    }

    #[test]
    fn enter_on_an_empty_query_runs_the_highlighted_recent_search() {
        let mut s = Search::with_recent(vec!["nova".into(), "orbit".into()]);
        s.recent_sel = 1;
        s.submit();
        assert_eq!(s.due().as_deref(), Some("orbit"));
    }

    #[test]
    fn a_failed_search_can_go_out_again() {
        let mut s = Search::default();
        s.type_text("nova");
        assert!(s.due_at(Instant::now() + DEBOUNCE).is_some());
        s.receive(SearchResults { query: "nova".into(), error: Some("no answer".into()), ..Default::default() });
        assert_eq!(s.failed(), Some("no answer"));
        assert!(s.results().is_none());
        assert_eq!(s.due_at(Instant::now() + DEBOUNCE), None, "a failure waits for Enter or typing, it doesn't retry by itself");
        s.submit();
        assert_eq!(s.failed(), None);
        assert_eq!(s.due().as_deref(), Some("nova"));
    }

    #[test]
    fn a_failure_for_new_words_shows_instead_of_older_results() {
        let mut s = Search::default();
        ran(&mut s, "nova");
        s.type_text(" drift");
        assert!(s.due_at(Instant::now() + DEBOUNCE).is_some());
        assert!(s.results().is_some(), "the older results stay while the new words are out");
        s.receive(SearchResults { query: "nova drift".into(), error: Some("no answer".into()), ..Default::default() });
        assert!(s.results().is_none() && s.failed().is_some(), "the failure is what shows");
    }

    #[test]
    fn a_fresh_answer_like_the_kept_one_leaves_the_highlight_alone() {
        let mut s = Search::with_recent(vec!["nova".into()]);
        ran(&mut s, "nova");
        s.clear();
        s.pick(0);
        assert!(s.due().is_some());
        s.track_sel = Some(0);
        s.shelves.offset = 3;
        s.receive(answer("nova"));
        assert_eq!((s.track_sel, s.shelves.offset), (Some(0), 3), "the same answer again changes nothing on screen");
        // a different answer starts the page afresh
        s.receive(SearchResults { query: "nova".into(), ..Default::default() });
        assert_eq!((s.track_sel, s.shelves.offset), (None, 0));
    }

    #[test]
    fn recent_searches_match_as_queries_do() {
        let mut s = Search::default();
        for q in ["Étoile", "étoile"] {
            s.clear();
            s.type_text(q);
            s.remember();
        }
        assert_eq!(s.recent(), ["étoile"]);
    }

    #[test]
    fn an_answer_for_older_words_is_kept_but_not_shown() {
        let mut s = Search::default();
        s.type_text("nova dr");
        s.receive(answer("nova"));
        assert!(s.results().is_none());
        s.receive(answer("Nova Dr "));
        assert!(s.answered(), "matched as the Daemon matches: case and outer spaces don't count");
        // the older answer was kept: going back to those words shows it at once
        for _ in 0..3 {
            s.erase();
        }
        assert_eq!(s.due_at(Instant::now() + DEBOUNCE).as_deref(), Some("nova"));
        assert!(s.answered());
    }

    #[test]
    fn recent_searches_are_newest_first_without_duplicates() {
        let mut s = Search::default();
        for q in ["daft punk", "nova", "Daft Punk", "x"] {
            s.clear();
            s.type_text(q);
            s.remember();
        }
        assert_eq!(s.recent(), ["Daft Punk", "nova"]);
    }
}
