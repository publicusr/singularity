//! The queue Widget (#12): what's playing, what you queued, what comes next from the context, and what autoplay added
//! (marked as such, story 11). Right-click or Delete edits the queue; play next / remove / clear go through Connect.

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
    let mut hits = vec![];
    for line in all.iter().skip(view.top).take(view.visible) {
        match line {
            Line::Heading(h) => text(buf, x, y, w, &clip(h, w as usize), th.s(th.dim).add_modifier(Modifier::BOLD)),
            Line::Entry(i) => {
                let e = &app.playback.queue[*i];
                let selected = focused && *i == view.sel;
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
                let name_st = if e.source == "queue" { th.s(th.fg).add_modifier(Modifier::BOLD) } else if auto { th.s(th.dim).add_modifier(Modifier::ITALIC) } else { th.s(th.fg) };
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
}
