//! The queue Widget (#12): what's playing, what you queued, what comes next from the context, and what autoplay added
//! (marked as such, story 11). Right-click or Delete edits the queue; play next / remove / clear go through Connect.
//! Songs you queued and the context's coming songs can be dragged into a new order (#32): queued songs among
//! themselves, context songs among themselves or up into your queue, where they become queued songs (#50, D42). The
//! list shows the result while dragging. Autoplay is Spotify's to choose.

use crate::gesture::Held;
use crate::app::{App, Hit};
use singularity_proto::QueueEntry;
use crate::theme::Theme;
use crate::ui::{text, blank};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

#[derive(Default, Clone, Copy)]
pub struct QueueView {
    /// selected entry (index into `playback.queue`)
    pub sel: usize,
    /// first visible line
    pub top: usize,
    /// how many lines are in view (in a big queue, where a song takes two rows, fewer than rows)
    pub visible: usize,
    /// the screen row of the list's first line, as last drawn (in its Pane or its popup): where a drag aims
    pub list_y: u16,
    /// whether it was last drawn big (#59): a song two rows tall, its cover beside it
    pub big: bool,
    /// whether the user put the highlight where it is (a key, the wheel, a click, a menu, a drop): only then does it
    /// follow its song when the queue changes; one never placed stays on the first slot. An empty queue unplaces it.
    pub placed: bool,
}

impl QueueView {
    pub fn move_sel(&mut self, delta: i64, len: usize) {
        self.place((self.sel as i64 + delta).clamp(0, len.saturating_sub(1) as i64) as usize);
    }

    /// The user put the highlight on entry `i`.
    pub fn place(&mut self, i: usize) {
        self.sel = i;
        self.placed = true;
    }

    /// The queue `old` became `new`: a placed highlight follows its song ([`follow`]); one never placed stays where
    /// it is. An empty queue leaves nothing placed.
    pub fn queue_changed(&mut self, old: &[QueueEntry], new: &[QueueEntry]) {
        if new.is_empty() {
            (self.sel, self.placed) = (0, false);
        } else if self.placed {
            self.sel = follow(old, self.sel, new);
        }
    }
}

/// A song being dragged: where it was, where it would land (an index into the queue as it would then be), and
/// whether it would land in your queue (a context song dragged up there, #50).
///
/// The queue can change during the drag (a track change, a phone editing it), so `from` is only where the song was:
/// its `uid` (or, where the player gives none, its URI) finds it again when the drag ends.
#[derive(Clone, Debug, PartialEq)]
pub struct QueueDrag {
    pub from: usize,
    pub to: usize,
    pub into_queue: bool,
    pub uid: String,
    pub uri: String,
}

impl QueueDrag {
    /// Start dragging entry `i`; None if it can't move (autoplay).
    pub fn start(app: &App, i: usize) -> Option<QueueDrag> {
        section(app, i)?;
        let e = &app.playback.queue[i];
        Some(QueueDrag { from: i, to: i, into_queue: false, uid: e.uid.clone(), uri: e.item.uri.clone() })
    }

    /// The dragged song's place in the queue as it is now, found by uid (or URI), not by the index it had.
    pub fn index_in(&self, queue: &[QueueEntry]) -> Option<usize> {
        // with a uid, only that entry is the song: a same-URI duplicate is a different one
        if !self.uid.is_empty() {
            return queue.iter().position(|e| e.uid == self.uid);
        }
        // no uid: the same URI, preferring the old place while it still holds that song
        if queue.get(self.from).is_some_and(|e| e.item.uri == self.uri) {
            return Some(self.from);
        }
        queue.iter().position(|e| e.item.uri == self.uri)
    }
}

/// How many songs at the head of the queue you queued yourself.
pub fn queued(app: &App) -> usize {
    app.playback.queue.iter().take_while(|e| e.source == "queue").count()
}

/// The run of entries around `i` that it belongs to: your queued songs, or the context's. None for autoplay.
pub fn section(app: &App, i: usize) -> Option<(usize, usize)> {
    let q = &app.playback.queue;
    let src = q.get(i)?.source.as_str();
    if src == "autoplay" {
        return None;
    }
    let start = (0..=i).rev().take_while(|&k| q[k].source == src).last()?;
    let len = q[start..].iter().take_while(|e| e.source == src).count();
    Some((start, len))
}

/// Where the dragged song would land with the pointer on screen row `y`: the queue's own lines decide (headings
/// included). A queued song stays among the queued; a context song stays among the context's, or goes up into your
/// queue: onto a queued row it takes that place, onto the "Next from" heading it goes last of them.
pub fn target(app: &App, d: QueueDrag, y: u16) -> QueueDrag {
    let Some(from) = d.index_in(&app.playback.queue) else { return d };
    let d = QueueDrag { from, ..d };
    let Some((start, len)) = section(app, d.from) else { return d };
    let nq = queued(app);
    let from_context = app.playback.queue[d.from].source != "queue";
    // lines start under "now playing" and a blank row
    let first = app.queue_view.list_y;
    let all = lines(app);
    // only at what's in view: past the list's edge it stays on the first or last line shown (above the first line of
    // the whole list means "first of your queue")
    let line = line_at(&all, &app.queue_view, y as i64 - first as i64);
    let at = |k: i64| all.get(k.max(0) as usize).cloned();
    // the entry a line means: a heading means the entry right after it
    let mut k = line;
    let entry = loop {
        match at(k) {
            Some(Line::Entry(i)) => break Some(i),
            Some(Line::Heading(_)) if k < all.len() as i64 - 1 => k += 1,
            _ => break None,
        }
    };
    let on_heading = matches!(at(line), Some(Line::Heading(_))) && line >= 0;
    let Some(i) = entry else { return d };
    if from_context && nq > 0 && (i < nq || (on_heading && i == nq) || line < 0) {
        // up into your queue: onto queued row i, or after the last queued song from the "Next from" heading
        let to = if line < 0 { 0 } else if i < nq { i } else { nq };
        return QueueDrag { to, into_queue: true, ..d };
    }
    QueueDrag { to: i.clamp(start, start + len - 1), into_queue: false, ..d }
}

/// The line on screen row `dy` of the list (0: its first row), only among what's in view: past the list's edge it
/// stays on the first or last line shown; above the first line of the whole list it is -1.
fn line_at(all: &[Line], view: &QueueView, dy: i64) -> i64 {
    let top = view.top as i64;
    let lo = if top == 0 { -1 } else { top };
    let hi = (top + view.visible.max(1) as i64 - 1).min(all.len() as i64 - 1).max(lo);
    if dy < 0 {
        return lo;
    }
    let mut row = 0;
    for (k, l) in all.iter().enumerate().skip(view.top) {
        row += height(l, view.big) as i64;
        if dy < row {
            return (k as i64).min(hi);
        }
    }
    hi
}

/// The rows line `l` takes: a song two in a big queue (its title, then its artists and album), else one.
fn height(l: &Line, big: bool) -> u16 {
    if big && matches!(l, Line::Entry(_)) { 2 } else { 1 }
}

/// How many lines from `top` fit whole in `rows` rows. In a big queue, not a heading whose first song doesn't fit
/// under it.
fn fitting(all: &[Line], top: usize, rows: usize, big: bool) -> usize {
    let mut used = 0;
    let n = all.iter().skip(top).take_while(|l| {
        used += height(l, big) as usize;
        used <= rows
    }).count();
    let alone = n > 0 && top + n < all.len() && matches!(all[top + n - 1], Line::Heading(_));
    if big && alone { n - 1 } else { n }
}

/// A queue this big shows each song on two rows with its cover beside it, and a cover for what's playing (#59): the
/// area inside a Pane of 90×32 with its frame.
pub const BIG: (u16, u16) = (88, 30);

pub fn is_big(r: Rect) -> bool {
    r.width >= BIG.0 && r.height >= BIG.1
}

/// The queue as it would be after the drag (the dragged song moved, and queued if it went up there).
pub fn preview(q: &[QueueEntry], d: Option<QueueDrag>) -> Vec<QueueEntry> {
    let mut v = q.to_vec();
    if let Some(d) = d.filter(|d| d.from < v.len()) {
        let mut e = v.remove(d.from);
        if d.into_queue {
            e.source = "queue".into();
        }
        v.insert(d.to.min(v.len()), e);
    }
    v
}

/// Where the highlight goes when the queue `old` becomes `new` (a skip, a move, a phone's edit): it stays on its song,
/// not on its slot. The song is found by its uid; where that uid isn't in `new` (the player gives none, or gave it a
/// new one: a "Next from" song lifted into the queue), by its URI, the place nearest its old one. A song that left (it
/// plays now, or was removed) hands the highlight to its nearest neighbour still listed, the next one first. It runs
/// whether or not the highlight shows; `draw` scrolls to it only while it does.
pub fn follow(old: &[QueueEntry], sel: usize, new: &[QueueEntry]) -> usize {
    let find = |e: &QueueEntry, near: usize| -> Option<usize> {
        if !e.uid.is_empty()
            && let Some(i) = new.iter().position(|n| n.uid == e.uid)
        {
            return Some(i);
        }
        // (not an entry whose uid is another song's of the old queue: a second copy of the song is that copy)
        new.iter()
            .enumerate()
            .filter(|(_, n)| n.item.uri == e.item.uri && (n.uid.is_empty() || !old.iter().any(|o| o.uid == n.uid)))
            .map(|(i, _)| i)
            .min_by_key(|&i| i.abs_diff(near))
    };
    let last = new.len().saturating_sub(1);
    if sel >= old.len() {
        return sel.min(last);
    }
    // the song itself, then its neighbours outwards, the next one first
    let around = (1..old.len()).flat_map(|d| [sel.checked_add(d), sel.checked_sub(d)]).flatten().filter(|&i| i < old.len());
    std::iter::once(sel).chain(around).find_map(|i| find(&old[i], i)).unwrap_or(sel.min(last))
}

/// One line of the Widget: a section heading, or entry `i`.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Heading(String),
    Entry(usize),
}

/// Group the queue into sections, in play order: queued, then the context, then autoplay.
pub fn lines(app: &App) -> Vec<Line> {
    lines_of(&app.playback.queue, app.playback.context_name.as_deref())
}

/// `lines` for any list of entries (the preview while dragging).
pub fn lines_of(q: &[QueueEntry], context_name: Option<&str>) -> Vec<Line> {
    let mut out = vec![];
    let mut last: Option<&str> = None;
    for (i, e) in q.iter().enumerate() {
        let section = e.source.as_str();
        if last != Some(section) {
            out.push(Line::Heading(match section {
                "queue" => "Next in queue".to_string(),
                "autoplay" => "Autoplay: similar music after this".to_string(),
                _ => match context_name {
                    Some(n) if !n.is_empty() => format!("Next from {n}"),
                    _ => "Next up".to_string(),
                },
            }));
            last = Some(section);
        }
        out.push(Line::Entry(i));
    }
    out
}

fn fmt_ms(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

fn clip(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        return s.to_string();
    }
    let mut o: String = s.chars().take(w.saturating_sub(1)).collect();
    o.push('…');
    o
}

/// The cover beside a song in a big queue, and the one of what's playing above it (cells: about square).
const COVER: (u16, u16) = (4, 2);
const NOW_COVER: (u16, u16) = (12, 6);

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    let big = is_big(r);
    // (first: a drag aims by it, whatever is drawn)
    app.queue_view.big = big;
    if r.height == 0 || w < 8 {
        return;
    }
    // now playing
    let mut y = r.y;
    match app.playback.item.clone() {
        Some(i) if big => {
            let url = i.cover_url.clone().unwrap_or_default();
            crate::home::cover(buf, Rect::new(x, y, NOW_COVER.0, NOW_COVER.1), &url, "♪", app, th);
            let (tx, tw) = (x + NOW_COVER.0 + 2, w - NOW_COVER.0 - 2);
            let (artists, album) = (i.artists.join(", "), i.album.clone().unwrap_or_default());
            let rows = [("Now playing", th.s(th.dim)), (i.name.as_str(), th.s(th.accent).add_modifier(Modifier::BOLD)), (artists.as_str(), th.s(th.fg)), (album.as_str(), th.s(th.dim))];
            for (k, (s, st)) in rows.into_iter().enumerate() {
                text(buf, tx, y + 1 + k as u16, tw, &crate::sidebar::clip(s, tw), st);
            }
            y += NOW_COVER.1 + 1;
        }
        Some(i) => {
            text(buf, x, y, 2, "♪", th.s(th.accent));
            let line = format!("{} — {}", i.name, i.artists.join(", "));
            text(buf, x + 2, y, w.saturating_sub(2), &clip(&line, w.saturating_sub(2) as usize), th.s(th.accent).add_modifier(Modifier::BOLD));
            y += 2;
        }
        None => {
            text(buf, x, y, w, "nothing playing", th.s(th.dim));
            y += 2;
        }
    }
    // while dragging, the list as it would be after the drop
    let q = preview(&app.playback.queue, app.gesture.queue().cloned());
    let all = lines_of(&q, app.playback.context_name.as_deref());
    if all.is_empty() {
        // the hint, by words, on the rows there are
        for row in crate::lyrics::wrap_indented("nothing queued · right-click a track → Add to queue", w as usize, 0) {
            if y >= r.bottom() {
                break;
            }
            text(buf, x, y, w, &row, th.s(th.dim));
            y += 1;
        }
        return;
    }
    let body = r.bottom().saturating_sub(y) as usize;
    let rows = body.max(1);
    let mut view = app.queue_view;
    view.list_y = y;
    view.sel = view.sel.min(app.playback.queue.len().saturating_sub(1));
    // (its popup, where no Pane shows it, has the keys as a focused Pane has them)
    let focused = app.is_focused(crate::layout::Widget::Queue) || app.is_open(crate::app::Modal::Queue);
    // keep the selection in view while it shows: an unfocused queue keeps its place while its highlight follows its
    // song, and scrolls to it when it gets the focus
    if focused && let Some(at) = all.iter().position(|l| *l == Line::Entry(view.sel)) {
        if at < view.top {
            view.top = at.saturating_sub(1);
        } else {
            while at >= view.top + fitting(&all, view.top, rows, big).max(1) {
                view.top += 1;
            }
        }
    }
    // no empty rows under the list's end
    let last_top = (0..all.len()).find(|&t| fitting(&all, t, rows, big) == all.len() - t).unwrap_or(all.len());
    view.top = view.top.min(last_top);
    view.visible = if big { fitting(&all, view.top, rows, big).max(1) } else { rows };
    let mut hits = vec![];
    // (a Pane a row or two tall has no rows for the list, but keeps its place in it)
    for line in all.iter().skip(view.top).take(fitting(&all, view.top, body, big)) {
        let h = height(line, big);
        match line {
            Line::Heading(h) => text(buf, x, y, w, &clip(h, w as usize), th.s(th.dim).add_modifier(Modifier::BOLD)),
            Line::Entry(i) => {
                let e = &q[*i];
                let dragged = app.gesture.queue().is_some_and(|d| d.to == *i);
                let selected = (focused && *i == view.sel && app.gesture.queue().is_none()) || dragged;
                let bg = if selected { th.selection.color() } else { th.bg.color() };
                for k in 0..h {
                    blank(buf, x, y + k, w, th.base().bg(bg));
                }
                let auto = e.source == "autoplay";
                let name = if e.item.name.is_empty() { "…".to_string() } else { e.item.name.clone() };
                let dur = if e.item.duration_ms > 0 { fmt_ms(e.item.duration_ms) } else { String::new() };
                let tag = if auto { "autoplay " } else { "" };
                let right = format!("{tag}{dur:>5}");
                let rw = right.chars().count() as u16;
                let artists = e.item.artists.join(", ");
                let name_st = if dragged { th.s(th.accent).add_modifier(Modifier::BOLD) } else if e.source == "queue" { th.s(th.fg).add_modifier(Modifier::BOLD) } else if auto { th.s(th.dim).add_modifier(Modifier::ITALIC) } else { th.s(th.fg) };
                if big {
                    // its cover, then the title (and the length) over the artists and album
                    let url = e.item.cover_url.clone().unwrap_or_default();
                    crate::home::cover(buf, Rect::new(x, y, COVER.0, COVER.1), &url, "♪", app, th);
                    let tx = x + COVER.0 + 2;
                    let lw = (x + w).saturating_sub(tx + rw + 1);
                    text(buf, tx, y, lw, &crate::sidebar::clip(&name, lw), name_st.bg(bg));
                    let album = e.item.album.clone().unwrap_or_default();
                    let sub = [artists, album].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                    text(buf, tx, y + 1, lw, &crate::sidebar::clip(&sub, lw), th.s(th.dim).bg(bg));
                } else {
                    let lw = w.saturating_sub(rw + 1) as usize;
                    let left = if artists.is_empty() { name.clone() } else { format!("{name} · {artists}") };
                    text(buf, x, y, lw as u16, &clip(&left, lw), name_st.bg(bg));
                }
                // (a length wider than a narrow Pane goes where a release build always put it, not to a debug panic)
                text(buf, (x + w).wrapping_sub(rw), y, rw, &right, th.s(if auto { th.border } else { th.dim }).bg(bg));
                hits.push((Rect::new(x, y, w, h), Hit::QueueRow(*i)));
            }
        }
        y += h;
    }
    for (rect, h) in hits {
        app.hit(rect, h);
    }
    app.queue_view = view;
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::{Item, QueueEntry};

    fn entry(name: &str, source: &str) -> QueueEntry {
        QueueEntry { item: Item { uri: format!("spotify:track:{name}"), name: name.into(), ..Default::default() }, source: source.into(), uid: format!("u-{name}") }
    }

    #[test]
    fn sections_in_play_order() {
        let mut app = App::new(None);
        app.playback.context_name = Some("Night Drive".into());
        app.playback.queue = vec![entry("a", "queue"), entry("b", "context"), entry("c", "context"), entry("d", "autoplay")];
        let l = lines(&app);
        assert_eq!(l[0], Line::Heading("Next in queue".into()));
        assert_eq!(l[2], Line::Heading("Next from Night Drive".into()));
        assert!(matches!(&l[5], Line::Heading(h) if h.starts_with("Autoplay")));
        assert_eq!(l.len(), 7);
    }

    #[test]
    fn a_drag_previews_the_new_order_and_sections() {
        let q = vec![entry("a", "queue"), entry("b", "queue"), entry("x", "context"), entry("y", "context")];
        let names = |v: &[QueueEntry]| v.iter().map(|e| format!("{}{}", e.item.name, if e.source == "queue" { "q" } else { "" })).collect::<Vec<_>>().join(" ");
        let drag = |from, to, into_queue| QueueDrag { from, to, into_queue, uid: String::new(), uri: String::new() };
        assert_eq!(names(&preview(&q, None)), "aq bq x y");
        assert_eq!(names(&preview(&q, Some(drag(1, 0, false)))), "bq aq x y");
        assert_eq!(names(&preview(&q, Some(drag(3, 2, false)))), "aq bq y x");
        // a context song up into your queue becomes queued there
        assert_eq!(names(&preview(&q, Some(drag(3, 1, true)))), "aq yq bq x");
        assert_eq!(names(&preview(&q, Some(drag(2, 2, true)))), "aq bq xq y", "last of the queued");
        let l = lines_of(&preview(&q, Some(drag(2, 2, true))), Some("Orbits"));
        assert_eq!(l[4], Line::Heading("Next from Orbits".into()), "the moved song shows under Next in queue");
    }

    #[test]
    fn the_highlight_follows_its_song_by_uid_then_by_uri() {
        let e = |n: &str, uid: &str| QueueEntry { uid: uid.into(), ..entry(n, "queue") };
        // a skip: every song up a slot, the highlight with its song
        assert_eq!(follow(&[e("a", "1"), e("b", "2"), e("c", "3")], 1, &[e("b", "2"), e("c", "3")]), 0);
        // a "Next from" song lifted into the queue: the Daemon sent it with no uid, Spotify gave it one
        assert_eq!(follow(&[e("a", "1"), e("x", "")], 1, &[e("x", "9"), e("a", "1")]), 0);
        // …or a new uid for the same song
        assert_eq!(follow(&[e("a", "1"), e("x", "c1")], 1, &[e("a", "1"), e("b", "2"), e("x", "q7")]), 2);
        // a uid still there wins over a same-URI duplicate nearer the old place
        assert_eq!(follow(&[e("a", "1"), e("a", "2")], 1, &[e("a", "2"), e("a", "1")]), 0);
        // no uids: the same URI, the copy nearest its old place
        assert_eq!(follow(&[e("a", ""), e("b", ""), e("a", ""), e("c", "")], 2, &[e("b", ""), e("a", ""), e("c", ""), e("a", "")]), 1);
        assert_eq!(follow(&[e("a", ""), e("b", ""), e("c", ""), e("a", "")], 3, &[e("a", ""), e("b", ""), e("c", ""), e("d", ""), e("a", "")]), 4);
        // the song left: the next one still listed, else the one before
        assert_eq!(follow(&[e("a", "1"), e("b", "2"), e("c", "3")], 0, &[e("b", "2"), e("c", "3")]), 0);
        assert_eq!(follow(&[e("a", "1"), e("b", "2"), e("c", "3")], 2, &[e("a", "1"), e("b", "2")]), 1);
        // a neighbour is looked for near its own old place (c's slot 2), not the highlight's (1): two copies of c
        assert_eq!(follow(&[e("x", ""), e("b", ""), e("c", "")], 1, &[e("c", ""), e("p", ""), e("q", ""), e("c", "")]), 3);
        // a copy whose uid is another song's of the old queue isn't the song: a(u1) plays, b takes the highlight
        assert_eq!(follow(&[e("a", "u1"), e("b", "u2"), e("c", "u4"), e("a", "u3")], 0, &[e("b", "u2"), e("c", "u4"), e("a", "u3")]), 0);
        // an empty queue, before and after; a highlight past the end
        assert_eq!(follow(&[], 0, &[e("a", "1")]), 0);
        assert_eq!(follow(&[e("a", "1")], 0, &[]), 0);
        assert_eq!(follow(&[e("a", "1")], 5, &[e("b", "2"), e("c", "3")]), 1);
        // nothing of the old queue left: the same slot, inside the new one
        assert_eq!(follow(&[e("a", "1"), e("b", "2"), e("c", "3")], 2, &[e("x", "7"), e("y", "8")]), 1);
    }

    #[test]
    fn a_dragged_song_is_found_again_when_the_queue_changes_under_it() {
        let q = vec![entry("a", "queue"), entry("x", "context"), entry("y", "context")];
        let d = QueueDrag { from: 2, to: 1, into_queue: false, uid: "u-y".into(), uri: "spotify:track:y".into() };
        // queued songs appeared and one context song left: the index is another song now, the uid is not
        let now = vec![entry("a", "queue"), entry("b", "queue"), entry("c", "queue"), entry("x", "context"), entry("y", "context")];
        assert_eq!(d.index_in(&now), Some(4));
        assert_eq!(d.index_in(&q), Some(2));
        // a player that gives no uid: the URI, and the old place while it still has it
        let no_uid = QueueDrag { uid: String::new(), ..d.clone() };
        assert_eq!(no_uid.index_in(&now), Some(4));
        let gone = vec![entry("a", "queue")];
        assert_eq!(d.index_in(&gone), None);
    }

    /// The queue as it was drawn before the big look (#59), kept as it was: every size under the gate must still draw
    /// exactly this.
    fn draw_before_59(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
        let (x, w) = (r.x + 1, r.width.saturating_sub(2));
        if r.height == 0 || w < 8 {
            return;
        }
        let mut y = r.y;
        match &app.playback.item {
            Some(i) => {
                text(buf, x, y, 2, "♪", th.s(th.accent));
                let line = format!("{} — {}", i.name, i.artists.join(", "));
                text(buf, x + 2, y, w.saturating_sub(2), &clip(&line, w.saturating_sub(2) as usize), th.s(th.accent).add_modifier(Modifier::BOLD));
            }
            None => text(buf, x, y, w, "nothing playing", th.s(th.dim)),
        }
        y += 2;
        let q = preview(&app.playback.queue, app.gesture.queue().cloned());
        let all = lines_of(&q, app.playback.context_name.as_deref());
        if all.is_empty() {
            for row in crate::lyrics::wrap_indented("nothing queued · right-click a track → Add to queue", w as usize, 0) {
                if y >= r.bottom() {
                    break;
                }
                text(buf, x, y, w, &row, th.s(th.dim));
                y += 1;
            }
            return;
        }
        let body = r.bottom().saturating_sub(y) as usize;
        let mut view = app.queue_view;
        view.list_y = y;
        view.visible = body.max(1);
        view.sel = view.sel.min(app.playback.queue.len().saturating_sub(1));
        if let Some(at) = all.iter().position(|l| *l == Line::Entry(view.sel)) {
            if at < view.top {
                view.top = at.saturating_sub(1);
            } else if at >= view.top + view.visible {
                view.top = at + 1 - view.visible;
            }
        }
        view.top = view.top.min(all.len().saturating_sub(view.visible));
        let focused = app.is_focused(crate::layout::Widget::Queue) || app.is_open(crate::app::Modal::Queue);
        let mut hits = vec![];
        for line in all.iter().skip(view.top).take(body) {
            match line {
                Line::Heading(h) => text(buf, x, y, w, &clip(h, w as usize), th.s(th.dim).add_modifier(Modifier::BOLD)),
                Line::Entry(i) => {
                    let e = &q[*i];
                    let dragged = app.gesture.queue().is_some_and(|d| d.to == *i);
                    let selected = (focused && *i == view.sel && app.gesture.queue().is_none()) || dragged;
                    let bg = if selected { th.selection.color() } else { th.bg.color() };
                    blank(buf, x, y, w, th.base().bg(bg));
                    let auto = e.source == "autoplay";
                    let name = if e.item.name.is_empty() { "…".to_string() } else { e.item.name.clone() };
                    let dur = if e.item.duration_ms > 0 { fmt_ms(e.item.duration_ms) } else { String::new() };
                    let tag = if auto { "autoplay " } else { "" };
                    let right = format!("{tag}{dur:>5}");
                    let rw = right.chars().count() as u16;
                    let lw = w.saturating_sub(rw + 1) as usize;
                    let artists = e.item.artists.join(", ");
                    let left = if artists.is_empty() { name.clone() } else { format!("{name} · {artists}") };
                    let name_st = if dragged { th.s(th.accent).add_modifier(Modifier::BOLD) } else if e.source == "queue" { th.s(th.fg).add_modifier(Modifier::BOLD) } else if auto { th.s(th.dim).add_modifier(Modifier::ITALIC) } else { th.s(th.fg) };
                    text(buf, x, y, lw as u16, &clip(&left, lw), name_st.bg(bg));
                    text(buf, (x + w).wrapping_sub(rw), y, rw, &right, th.s(if auto { th.border } else { th.dim }).bg(bg));
                    hits.push((Rect::new(x, y, w, 1), Hit::QueueRow(*i)));
                }
            }
            y += 1;
        }
        for (rect, h) in hits {
            app.hit(rect, h);
        }
        app.queue_view = view;
    }

    /// A queue of all three kinds, with covers, albums, long and wide titles: what a big queue shows more of.
    fn full_app(n: usize) -> App {
        let mut app = App::new(None);
        app.playback.context_name = Some("Night Drive".into());
        app.playback.item = Some(Item { uri: "spotify:track:now".into(), name: "What Plays".into(), artists: vec!["Nova Drift".into()], album: Some("Orbit EP".into()), cover_url: Some("https://i.scdn.co/image/now".into()), duration_ms: 200_000, ..Default::default() });
        app.playback.queue = (0..n)
            .map(|i| {
                let name = match i % 5 {
                    3 => format!("Song {i} with a title much longer than any Pane has room for, on and on and on and on and on"),
                    4 => format!("夜明けのオーケストラと長い長い日本語のタイトル {i} 夜明けのオーケストラと長い長い日本語のタイトル"),
                    _ => format!("Song {i}"),
                };
                let source = if i < n / 3 { "queue" } else if i < n * 2 / 3 { "context" } else { "autoplay" };
                QueueEntry { item: Item { uri: format!("spotify:track:{i}"), name, artists: vec!["Mira Vale".into()], album: Some(format!("Album {i}")), cover_url: Some(format!("https://i.scdn.co/image/c{i}")), duration_ms: 120_000 + i as u64 * 1000, ..Default::default() }, source: source.into(), uid: format!("u{i}") }
            })
            .collect();
        app.focus_on(crate::layout::Widget::Queue);
        app
    }

    /// What a drawing leaves: the cells, the hits, and the list's (top, visible, list_y).
    type Drawn = (Buffer, Vec<(Rect, Hit)>, (usize, usize, u16));

    fn drawn(r: Rect, app: &mut App, f: fn(&mut Buffer, Rect, &mut App, &Theme)) -> Drawn {
        let th = Theme::singularity();
        let mut buf = Buffer::empty(r);
        app.hits.clear();
        f(&mut buf, r, app, &th);
        let v = app.queue_view;
        (buf, app.hits.clone(), (v.top, v.visible, v.list_y))
    }

    #[test]
    fn every_size_under_the_gate_draws_as_before_cell_for_cell() {
        // the whole range of widths and heights a Pane can give it, but the big ones; a selection at the start, in the
        // middle and at the end (scrolled), something playing or not, a song held in a drag
        let mut sizes = 0;
        // (selection, something playing, a song held in a drag), each an App for the new drawing and one for the old
        let setups = [(0, true, false), (9, false, false), (26, true, false), (9, true, true)];
        let mut apps: Vec<(App, App)> = setups
            .iter()
            .map(|&(sel, playing, drag)| {
                let (mut a, mut b) = (full_app(27), full_app(27));
                for app in [&mut a, &mut b] {
                    app.queue_view.sel = sel;
                    if !playing {
                        app.playback.item = None;
                    }
                    if drag {
                        app.gesture = QueueDrag::start(app, 12).map(|d| crate::gesture::Gesture::Queue(QueueDrag { to: 14, ..d }));
                    }
                }
                (a, b)
            })
            .collect();
        for w in 0..=140u16 {
            for h in 0..=50u16 {
                let r = Rect::new(3, 2, w, h);
                if is_big(r) {
                    continue;
                }
                for ((a, b), setup) in apps.iter_mut().zip(setups) {
                    // from the top of the list each size; two frames: the second starts where the first left it
                    a.queue_view.top = 0;
                    b.queue_view.top = 0;
                    for _ in 0..2 {
                        let new = drawn(r, a, draw);
                        let old = drawn(r, b, draw_before_59);
                        assert!(new == old, "{w}×{h}, {setup:?}: drawn differently");
                    }
                }
                sizes += 1;
            }
        }
        assert!(sizes > 5900, "{sizes}");
        // the gate's edge: one column or row short of it is still the old look
        for (w, h) in [(BIG.0 - 1, BIG.1), (BIG.0, BIG.1 - 1), (BIG.0 - 1, 60), (200, BIG.1 - 1)] {
            let r = Rect::new(0, 0, w, h);
            assert!(!is_big(r));
            assert!(drawn(r, &mut full_app(27), draw).0 == drawn(r, &mut full_app(27), draw_before_59).0, "{w}×{h}");
        }
        assert!(is_big(Rect::new(0, 0, BIG.0, BIG.1)));
    }

    fn row_text(buf: &Buffer, x0: u16, x1: u16, y: u16) -> String {
        (x0..x1).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn a_big_queue_shows_each_song_on_two_rows_beside_its_cover() {
        let r = Rect::new(0, 0, BIG.0, BIG.1);
        let mut app = full_app(27);
        let (buf, hits, (top, visible, list_y)) = drawn(r, &mut app, draw);
        // what's playing: its cover (missing here: a placeholder like Home's), then its words beside it
        assert_eq!(buf[(1 + NOW_COVER.0 / 2, NOW_COVER.1 / 2)].symbol(), "♪", "the now-playing cover's placeholder");
        let tx = 1 + NOW_COVER.0 + 2;
        for (k, want) in ["Now playing", "What Plays", "Nova Drift", "Orbit EP"].into_iter().enumerate() {
            assert!(row_text(&buf, tx, r.width, 1 + k as u16).starts_with(want), "{want}");
        }
        assert_eq!(list_y, NOW_COVER.1 + 1);
        assert!(row_text(&buf, 1, r.width, list_y).starts_with("Next in queue"));
        // each song: two rows, its hit covering both; the cover's placeholder, the title and length, then the rest
        let rows: Vec<Rect> = (0..27).filter_map(|i| hits.iter().find_map(|(rr, h)| (*h == Hit::QueueRow(i)).then_some(*rr))).collect();
        assert!(rows.len() >= 9 && rows.iter().all(|rr| rr.height == 2 && rr.width == r.width - 2), "{rows:?}");
        let first = rows[0];
        assert_eq!(buf[(1 + COVER.0 / 2, first.y + COVER.1 / 2)].symbol(), "♪");
        let tx = 1 + COVER.0 + 2;
        assert!(row_text(&buf, tx, r.width, first.y).starts_with("Song 0 "));
        assert!(row_text(&buf, tx, r.width, first.y).trim_end().ends_with("2:00"));
        assert!(row_text(&buf, tx, r.width, first.y + 1).starts_with("Mira Vale · Album 0"));
        // whole songs only, the selection in view; nothing drawn past the bottom
        assert_eq!(top, 0);
        assert!(rows.last().unwrap().bottom() <= r.bottom());
        assert!(visible >= rows.len());
    }

    #[test]
    fn a_big_queues_long_and_wide_titles_are_cut_by_their_width() {
        let r = Rect::new(0, 0, BIG.0, BIG.1 + 20);
        let mut app = full_app(27);
        let (buf, hits, _) = drawn(r, &mut app, draw);
        let tx = 1 + COVER.0 + 2;
        for i in [3, 4] {
            let rr = hits.iter().find_map(|(rr, h)| (*h == Hit::QueueRow(i)).then_some(*rr)).unwrap();
            let dur = fmt_ms(app.playback.queue[i].item.duration_ms);
            // the title's cells, up to the gap before the length (a cut by characters would lose the ellipsis to the
            // edge: a wide character takes two cells)
            let gap = rr.right() - 5 - 1;
            let title = row_text(&buf, tx, gap, rr.y);
            assert!(title.trim_end().ends_with('…'), "{i}: {title:?}");
            assert_eq!(buf[(gap, rr.y)].symbol(), " ", "{i}: the title runs into its length");
            assert!(row_text(&buf, gap, rr.right(), rr.y).ends_with(&dur), "{i}");
        }
    }

    #[test]
    fn a_big_queues_covers_draw_when_they_arrive() {
        let r = Rect::new(0, 0, BIG.0, BIG.1);
        let mut app = full_app(27);
        let cover = image::RgbImage::from_pixel(8, 8, image::Rgb([200, 40, 40]));
        app.covers.insert("https://i.scdn.co/image/c0", cover.clone());
        app.covers.insert("https://i.scdn.co/image/now", cover);
        let (buf, hits, _) = drawn(r, &mut app, draw);
        let first = hits.iter().find_map(|(rr, h)| (*h == Hit::QueueRow(0)).then_some(*rr)).unwrap();
        // the cell tier: half-blocks in the cover's colour, no placeholder
        for (x, y) in [(1, first.y), (1 + COVER.0 - 1, first.y + 1), (1, 0), (NOW_COVER.0, NOW_COVER.1 - 1)] {
            let c = &buf[(x, y)];
            assert!(c.symbol() == "▀" || c.symbol() == "▄", "({x}, {y}): {:?}", c.symbol());
            assert_eq!(c.fg, ratatui::style::Color::Rgb(200, 40, 40));
        }
        // the next song's cover hasn't come: its placeholder
        let second = hits.iter().find_map(|(rr, h)| (*h == Hit::QueueRow(1)).then_some(*rr)).unwrap();
        assert_eq!(buf[(1 + COVER.0 / 2, second.y + COVER.1 / 2)].symbol(), "♪");
        // the pixel tier: the two that came are placed as images (blank cells under them), the rest wait as placeholders
        app.kitty = Some(crate::gfx::Kitty::new(false));
        app.cell_px = Some((10.0, 20.0));
        let (buf, _, _) = drawn(r, &mut app, draw);
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!((out.matches("a=t,").count(), out.matches("a=p,").count()), (2, 2), "two covers sent and placed");
        assert!((0..COVER.0).all(|dx| buf[(1 + dx, first.y)].symbol() == " " && buf[(1 + dx, first.y + 1)].symbol() == " "));
        assert!(!row_text(&buf, 0, r.width, first.y).contains('▀'), "no half-blocks in the pixel tier");
        assert_eq!(buf[(1 + COVER.0 / 2, second.y + COVER.1 / 2)].symbol(), "♪");
    }

    #[test]
    fn a_big_queue_follows_its_highlight_one_song_at_a_time() {
        let r = Rect::new(0, 0, BIG.0 + 10, BIG.1 + 4);
        let mut app = full_app(60);
        let mut last_top = 0;
        for sel in 0..60 {
            app.queue_view.sel = sel;
            let (_, hits, (top, visible, _)) = drawn(r, &mut app, draw);
            assert!(hits.iter().any(|(_, h)| *h == Hit::QueueRow(sel)), "{sel}: the highlighted song is drawn");
            // the list moves a line or two at a time (a heading comes with its first song), never back while going down
            assert!(top >= last_top && top <= last_top + 2, "{sel}: {last_top} → {top}");
            assert!(visible >= 9);
            last_top = top;
        }
        // at the end, no empty rows under the last song
        let (_, hits, _) = drawn(r, &mut app, draw);
        let last = hits.iter().find_map(|(rr, h)| (*h == Hit::QueueRow(59)).then_some(*rr)).unwrap();
        assert!(r.bottom() - last.bottom() < 2, "{last:?}");
    }

    #[test]
    fn a_big_queue_shows_no_heading_without_its_first_song() {
        // eleven queued songs fill 23 of the 24 rows under what's playing; "Next from" would sit alone on the 24th
        let r = Rect::new(0, 0, BIG.0, BIG.1 + 1);
        let mut app = full_app(33);
        let (buf, _, (top, visible, list_y)) = drawn(r, &mut app, draw);
        assert_eq!((top, list_y), (0, NOW_COVER.1 + 1));
        let screen: Vec<String> = (0..r.height).map(|y| row_text(&buf, 0, r.width, y)).collect();
        assert!(!screen.iter().any(|l| l.contains("Next from")), "{screen:#?}");
        assert_eq!(visible, 12, "the queued songs and their heading");
        // a row more and its first song comes with it
        let (buf, _, _) = drawn(Rect { height: r.height + 2, ..r }, &mut app, draw);
        assert!((0..r.height + 2).any(|y| row_text(&buf, 0, r.width, y).contains("Next from")));
    }

    #[test]
    fn a_big_queue_songs_menu_from_the_keyboard_opens_under_the_song() {
        let r = Rect::new(0, 0, BIG.0, BIG.1);
        let mut app = full_app(27);
        let (_, hits, _) = drawn(r, &mut app, draw);
        // (all drawn inside the queue's Pane)
        app.hits_in_panes = hits.len();
        app.hits = hits;
        let rr = app.hit_rect(&Hit::QueueRow(2)).unwrap();
        app.open_queue_menu(2, None);
        match app.popup.as_ref() {
            Some(crate::app::Popup::Menu(m)) => assert_eq!(m.at, (rr.x + 4, rr.bottom())),
            _ => panic!("no menu"),
        }
    }

    #[test]
    fn a_drag_in_a_big_queue_aims_at_the_song_under_either_of_its_rows() {
        let r = Rect::new(0, 0, BIG.0, BIG.1);
        let mut app = full_app(27);
        let (_, hits, _) = drawn(r, &mut app, draw);
        let row = |i: usize| hits.iter().find_map(|(rr, h)| (*h == Hit::QueueRow(i)).then_some(*rr)).unwrap();
        let d = QueueDrag::start(&app, 1).unwrap();
        // queued songs 0..9: onto song 5's title row and its second row alike
        for y in [row(5).y, row(5).y + 1] {
            assert_eq!(target(&app, d.clone(), y).to, 5, "row {y}");
        }
        // above the list: first of your queue; below what's shown: the last queued song in view
        assert_eq!(target(&app, d.clone(), 0).to, 0);
        let c = QueueDrag::start(&app, 12).unwrap();
        assert_eq!((target(&app, c.clone(), row(2).y + 1).to, target(&app, c.clone(), row(2).y + 1).into_queue), (2, true));
        assert_eq!(target(&app, c.clone(), r.bottom() + 5), target(&app, c, r.bottom() - 1), "past the bottom: the last line in view");
    }
}
