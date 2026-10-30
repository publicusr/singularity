//! The queue Widget (#12): what's playing, what you queued, what comes next from the context, and what autoplay added
//! (marked as such, story 11). Right-click or Delete edits the queue; play next / remove / clear go through Connect.
//! Songs you queued and the context's coming songs can be dragged into a new order (#32): queued songs among
//! themselves, context songs among themselves or up into your queue, where they become queued songs (#50, D42). The
//! list shows the result while dragging. Autoplay is Spotify's to choose.

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
    pub visible: usize,
}

impl QueueView {
    pub fn move_sel(&mut self, delta: i64, len: usize) {
        self.sel = (self.sel as i64 + delta).clamp(0, len.saturating_sub(1) as i64) as usize;
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
    let Some(pane) = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Focus(s) if s == "queue").then_some(*r)) else { return d };
    // lines start under "now playing" and a blank row, inside the frame
    let first = pane.y + 1 + 2;
    let all = lines(app);
    let line = (y as i64 - first as i64 + app.queue_view.top as i64).clamp(-1, all.len() as i64 - 1);
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

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    if r.height == 0 || w < 8 {
        return;
    }
    // now playing
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
    // while dragging, the list as it would be after the drop
    let q = preview(&app.playback.queue, app.queue_drag.clone());
    let all = lines_of(&q, app.playback.context_name.as_deref());
    if all.is_empty() {
        if y < r.bottom() {
            text(buf, x, y, w, "nothing queued · right-click a track → Add to queue", th.s(th.dim));
        }
        return;
    }
    let body = r.bottom().saturating_sub(y) as usize;
    let mut view = app.queue_view;
    view.visible = body.max(1);
    view.sel = view.sel.min(app.playback.queue.len().saturating_sub(1));
    // keep the selection in view
    if let Some(at) = all.iter().position(|l| *l == Line::Entry(view.sel)) {
        if at < view.top {
            view.top = at.saturating_sub(1);
        } else if at >= view.top + view.visible {
            view.top = at + 1 - view.visible;
        }
    }
    view.top = view.top.min(all.len().saturating_sub(view.visible));
    let focused = app.focus == "queue";
    let mut hits = vec![];
    for line in all.iter().skip(view.top).take(view.visible) {
        match line {
            Line::Heading(h) => text(buf, x, y, w, &clip(h, w as usize), th.s(th.dim).add_modifier(Modifier::BOLD)),
            Line::Entry(i) => {
                let e = &q[*i];
                let dragged = app.queue_drag.as_ref().is_some_and(|d| d.to == *i);
                let selected = (focused && *i == view.sel && app.queue_drag.is_none()) || dragged;
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
                text(buf, x + w - rw, y, rw, &right, th.s(if auto { th.border } else { th.dim }).bg(bg));
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
}
