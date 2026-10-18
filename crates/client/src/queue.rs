//! The queue Widget (#12): what's playing, what you queued, what comes next from the context, and what autoplay added
//! (marked as such, story 11). Right-click or Delete edits the queue; play next / remove / clear go through Connect.
//! Songs you queued and the context's coming songs can be dragged into a new order (#32), each within its own section:
//! the list shows the new order while dragging. Autoplay is Spotify's to choose.

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::text;
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
    pub fn scroll(&mut self, delta: i64, lines: usize) {
        let max = lines.saturating_sub(self.visible.max(1));
        self.top = (self.top as i64 + delta).clamp(0, max as i64) as usize;
    }
}

/// A song being dragged: where it was and where it would land (indices into the queue, both inside `section`), and
/// the screen row entry 0 would sit on if the section ran up there (rows in a section are consecutive).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueueDrag {
    pub from: usize,
    pub to: usize,
    pub top_y: i32,
    /// the run of entries it can move among: (first index, how many)
    pub section: (usize, usize),
}

impl QueueDrag {
    /// Start dragging entry `i`, shown on screen row `y`; None if it can't move.
    pub fn start(app: &App, i: usize, y: u16) -> Option<QueueDrag> {
        let section = section(app, i)?;
        Some(QueueDrag { from: i, to: i, top_y: y as i32 - i as i32, section })
    }

    /// The pointer is on screen row `y`.
    pub fn over(&mut self, y: u16) {
        let (start, len) = self.section;
        self.to = (y as i32 - self.top_y).clamp(start as i32, (start + len).saturating_sub(1) as i32) as usize;
    }
}

/// The run of entries around `i` that it can be moved among: your queued songs, or the context's. None for autoplay.
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

/// Queue entries in the order shown: as they are, or with the dragged song at its new place.
pub fn order(len: usize, drag: Option<QueueDrag>) -> Vec<usize> {
    let mut o: Vec<usize> = (0..len).collect();
    if let Some(d) = drag.filter(|d| d.from < len && d.to < len) {
        let i = o.remove(d.from);
        o.insert(d.to, i);
    }
    o
}

/// One line of the Widget: a section heading, or entry `i`.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Heading(String),
    Entry(usize),
}

/// Group the queue into sections, in play order: queued, then the context, then autoplay.
pub fn lines(app: &App) -> Vec<Line> {
    let q = &app.playback.queue;
    let mut out = vec![];
    let mut last: Option<&str> = None;
    for (i, e) in q.iter().enumerate() {
        let section = e.source.as_str();
        if last != Some(section) {
            out.push(Line::Heading(match section {
                "queue" => "Next in queue".to_string(),
                "autoplay" => "Autoplay: similar music after this".to_string(),
                _ => match &app.playback.context_name {
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
    let all = lines(app);
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
    let shown = order(app.playback.queue.len(), app.queue_drag);
    let mut hits = vec![];
    for line in all.iter().skip(view.top).take(view.visible) {
        match line {
            Line::Heading(h) => text(buf, x, y, w, &clip(h, w as usize), th.s(th.dim).add_modifier(Modifier::BOLD)),
            Line::Entry(at) => {
                let i = &shown[*at];
                let e = &app.playback.queue[*i];
                let dragged = app.queue_drag.is_some_and(|d| d.from == *i);
                let selected = (focused && *i == view.sel && app.queue_drag.is_none()) || dragged;
                let bg = if selected { th.selection.color() } else { th.bg.color() };
                text(buf, x, y, w, &" ".repeat(w as usize), th.base().bg(bg));
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
    fn a_drag_shows_the_new_order() {
        assert_eq!(order(4, None), [0, 1, 2, 3]);
        assert_eq!(order(4, Some(QueueDrag { from: 2, to: 0, top_y: 0, section: (0, 4) })), [2, 0, 1, 3]);
        assert_eq!(order(4, Some(QueueDrag { from: 0, to: 2, top_y: 0, section: (0, 4) })), [1, 2, 0, 3]);
        let mut d = QueueDrag { from: 1, to: 1, top_y: 10, section: (0, 3) };
        d.over(3);
        assert_eq!(d.to, 0, "above the queued songs: first");
        d.over(40);
        assert_eq!(d.to, 2, "below them: last of the queued, never into the context");
        let mut c = QueueDrag { from: 4, to: 4, top_y: 10, section: (3, 4) };
        c.over(2);
        assert_eq!(c.to, 3, "a context song never goes above the context");
    }
}
