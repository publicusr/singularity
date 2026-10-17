//! Synced lyrics (#14): the current line lit and held near the middle, timestamps like a log (`tail -f lyrics.log`),
//! click a line to seek there. Unsynced lyrics scroll with the wheel; no lyrics is a calm, empty Pane.

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::LyricLine;

/// The line playing at `pos_ms`: the last one that has started.
pub fn current(lines: &[LyricLine], pos_ms: u64) -> Option<usize> {
    let n = lines.partition_point(|l| l.start_ms <= pos_ms);
    n.checked_sub(1)
}

fn stamp(ms: u64) -> String {
    format!("[{:02}:{:02}]", ms / 60_000, ms / 1000 % 60)
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
    if r.height < 2 || w < 6 {
        return;
    }
    let centre = |buf: &mut Buffer, y: u16, s: &str, st| {
        let n = s.chars().count() as u16;
        text(buf, r.x + r.width.saturating_sub(n) / 2, y, r.width, s, st);
    };
    let Some(item) = app.playback.item.clone() else {
        return centre(buf, r.y + r.height / 2, "nothing playing", th.s(th.dim));
    };
    let Some(lyr) = app.lyrics.clone().filter(|l| l.uri == item.uri) else {
        return centre(buf, r.y + r.height / 2, "looking for lyrics…", th.s(th.dim));
    };
    if lyr.lines.is_empty() {
        centre(buf, r.y + r.height / 2 - 1, "♪", th.s(th.border));
        return centre(buf, r.y + r.height / 2 + 1, "no lyrics for this one", th.s(th.dim));
    }
    let body = r.height.saturating_sub(1); // last row: the credit
    let show_stamps = lyr.synced && w >= 30;
    let text_x = if show_stamps { x + 8 } else { x };
    let text_w = w.saturating_sub(text_x - x);
    let pos = app.display_position(singularity_proto::now_ms());
    let now = if lyr.synced { current(&lyr.lines, pos) } else { None };
    // synced: hold the current line about 40 % down; unsynced: the wheel scrolls
    let first = match now {
        Some(i) => i.saturating_sub((body as usize * 2) / 5),
        None if lyr.synced => 0,
        None => app.lyrics_scroll.min(lyr.lines.len().saturating_sub(body as usize)),
    };
    let mut hits = vec![];
    for (row, (i, line)) in lyr.lines.iter().enumerate().skip(first).take(body as usize).enumerate() {
        let y = r.y + row as u16;
        let st = match now {
            Some(c) if c == i => th.s(th.accent).add_modifier(Modifier::BOLD),
            Some(c) if i == c + 1 => th.s(th.fg),
            Some(c) if i < c => th.s(th.border),
            _ if lyr.synced => th.s(th.dim),
            _ => th.s(th.fg),
        };
        if show_stamps {
            text(buf, x, y, 8, &stamp(line.start_ms), th.s(if now == Some(i) { th.dim } else { th.border }));
        }
        let words = if line.text.is_empty() { "♪" } else { line.text.as_str() };
        text(buf, text_x, y, text_w, &clip(words, text_w as usize), st);
        if lyr.synced {
            hits.push((Rect::new(x, y, w, 1), Hit::Lyric(line.start_ms)));
        }
    }
    let credit = if lyr.synced { format!("lyrics: {}", lyr.source) } else { format!("lyrics: {} · not time-synced", lyr.source) };
    let cw = credit.chars().count() as u16;
    text(buf, r.right().saturating_sub(cw + 1), r.bottom() - 1, cw, &credit, th.s(th.border));
    for (rect, h) in hits {
        app.hit(rect, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_line_is_the_last_started() {
        let l: Vec<LyricLine> = [1000u64, 5000, 9000].iter().map(|&s| LyricLine { start_ms: s, text: String::new() }).collect();
        assert_eq!(current(&l, 0), None);
        assert_eq!(current(&l, 1000), Some(0));
        assert_eq!(current(&l, 8999), Some(1));
        assert_eq!(current(&l, 60_000), Some(2));
    }
}
