//! Synced lyrics (#14): the current line lit and held near the middle, timestamps like a log (`tail -f lyrics.log`),
//! click a line to seek there. Unsynced lyrics scroll with the wheel; no lyrics is a calm, empty Pane that says why.
//! Long lines wrap (#30): continuation rows are indented, the timestamp only on the first.

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::LyricLine;
use unicode_width::UnicodeWidthStr;

/// The line playing at `pos_ms`: the last one that has started.
pub fn current(lines: &[LyricLine], pos_ms: u64) -> Option<usize> {
    let n = lines.partition_point(|l| l.start_ms <= pos_ms);
    n.checked_sub(1)
}

fn stamp(ms: u64) -> String {
    format!("[{:02}:{:02}]", ms / 60_000, ms / 1000 % 60)
}

/// Every line's rows, kept between frames for one song at one width: wrapping all of a song's lines on every frame
/// was a third of the Client's allocations (#71).
#[derive(Default)]
pub struct Wrapped {
    /// (song, text width, lines)
    key: (String, usize, usize),
    rows: Vec<(usize, String)>,
    /// where each line's first row is
    starts: Vec<usize>,
}

/// How far continuation rows are indented.
const INDENT: usize = 2;

/// A line's rows: words fill `w` columns (continuations `w - INDENT`, indent not included); a word too long for a
/// row is broken. Widths are display columns.
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(INDENT + 1);
    let mut rows = vec![String::new()];
    let room = |rows: &Vec<String>| if rows.len() == 1 { w } else { w - INDENT };
    for word in s.split_whitespace() {
        let cur = rows.last().unwrap();
        let need = if cur.is_empty() { word.width() } else { cur.width() + 1 + word.width() };
        if need <= room(&rows) {
            let cur = rows.last_mut().unwrap();
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
            continue;
        }
        if !rows.last().unwrap().is_empty() {
            rows.push(String::new());
        }
        // a word wider than a row is broken across rows
        for c in word.chars() {
            let cur = rows.last().unwrap();
            if cur.width() + unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) > room(&rows) {
                rows.push(String::new());
            }
            rows.last_mut().unwrap().push(c);
        }
    }
    rows
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    if r.height < 2 || w < 6 {
        return;
    }
    let centre = |buf: &mut Buffer, y: u16, s: &str, st| {
        let n = s.width() as u16;
        text(buf, r.x + r.width.saturating_sub(n) / 2, y, r.width, s, st);
    };
    let Some(item) = app.playback.item.as_ref() else {
        return centre(buf, r.y + r.height / 2, "nothing playing", th.s(th.dim));
    };
    let Some(lyr) = app.lyrics.as_ref().filter(|l| l.uri == item.uri) else {
        return centre(buf, r.y + r.height / 2, "looking for lyrics…", th.s(th.dim));
    };
    if lyr.lines.is_empty() {
        let why = if lyr.missing.is_empty() { "no lyrics for this one" } else { lyr.missing.as_str() };
        let rows = wrap(why, w as usize);
        let top = (r.y + r.height / 2 + 1).min(r.bottom().saturating_sub(rows.len() as u16)).max(r.y);
        if top > r.y + 1 {
            centre(buf, top - 2, "♪", th.s(th.border));
        }
        for (i, row) in rows.iter().enumerate().take(r.height as usize) {
            centre(buf, top + i as u16, row.trim(), th.s(th.dim));
        }
        return;
    }
    let body = r.height.saturating_sub(1) as usize; // last row: the credit
    let show_stamps = lyr.synced && w >= 30;
    let text_x = if show_stamps { x + 8 } else { x };
    let text_w = w.saturating_sub(text_x - x) as usize;
    let pos = app.display_position(singularity_proto::now_ms());
    // every line's rows, and where each line's first row is: wrapped again only for a new song or width
    let cache = &mut app.lyrics_wrap;
    if cache.key.0 != lyr.uri || (cache.key.1, cache.key.2) != (text_w, lyr.lines.len()) {
        cache.key = (lyr.uri.clone(), text_w, lyr.lines.len());
        cache.rows.clear();
        cache.starts.clear();
        for (i, line) in lyr.lines.iter().enumerate() {
            cache.starts.push(cache.rows.len());
            let words = if line.text.is_empty() { "♪" } else { line.text.as_str() };
            cache.rows.extend(wrap(words, text_w).into_iter().map(|t| (i, t)));
        }
    }
    let (rows, starts) = (&app.lyrics_wrap.rows, &app.lyrics_wrap.starts);
    let now = if lyr.synced { current(&lyr.lines, pos) } else { None };
    // synced: hold the current line about 40 % down, all its rows in view; unsynced: the wheel scrolls by rows
    let first = match now {
        Some(c) => {
            let (top, end) = (starts[c], starts.get(c + 1).copied().unwrap_or(rows.len()));
            let first = top.saturating_sub(body * 2 / 5);
            if end > first + body { end.saturating_sub(body).min(top) } else { first }
        }
        None if lyr.synced => 0,
        None => {
            app.lyrics_scroll = app.lyrics_scroll.min(rows.len().saturating_sub(body));
            app.lyrics_scroll
        }
    };
    let mut hits = vec![];
    for (row, (k, (i, words))) in rows.iter().enumerate().skip(first).take(body).enumerate() {
        let (i, y) = (*i, r.y + row as u16);
        let st = match now {
            Some(c) if c == i => th.s(th.accent).add_modifier(Modifier::BOLD),
            Some(c) if i == c + 1 => th.s(th.fg),
            Some(c) if i < c => th.s(th.border),
            _ if lyr.synced => th.s(th.dim),
            _ => th.s(th.fg),
        };
        let first_row = starts[i] == k;
        if show_stamps && first_row {
            text(buf, x, y, 8, &stamp(lyr.lines[i].start_ms), th.s(if now == Some(i) { th.dim } else { th.border }));
        }
        let indent = if first_row { 0 } else { INDENT as u16 };
        text(buf, text_x + indent, y, text_w as u16 - indent, words, st);
        if lyr.synced {
            hits.push((Rect::new(x, y, w, 1), Hit::Lyric(lyr.lines[i].start_ms)));
        }
    }
    // the longest credit that fits
    let credits = if lyr.synced { vec![format!("lyrics: {}", lyr.source), lyr.source.clone()] } else { vec![format!("lyrics: {} · not time-synced", lyr.source), format!("{} · unsynced", lyr.source), lyr.source.clone()] };
    let credit = credits.iter().find(|c| c.width() < r.width as usize).unwrap_or(credits.last().unwrap());
    let cw = credit.width() as u16;
    text(buf, r.right().saturating_sub(cw + 1), r.bottom() - 1, cw, credit, th.s(th.border));
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

    #[test]
    fn long_lines_wrap_by_words_and_long_words_break() {
        assert_eq!(wrap("short", 20), ["short"]);
        assert_eq!(wrap("we drift past the photon sphere", 12), ["we drift", "past the", "photon", "sphere"]);
        // continuations have INDENT fewer columns
        assert_eq!(wrap("aaaa bbbb cccc", 9), ["aaaa bbbb", "cccc"]);
        assert_eq!(wrap("aaaa bbb cc", 6), ["aaaa", "bbb", "cc"]);
        assert_eq!(wrap("abcdefghij", 5), ["abcde", "fgh", "ij"]);
        // wide characters count two columns
        assert_eq!(wrap("光が曲がる", 6), ["光が曲", "がる"]);
        for w in 3..40 {
            for (k, row) in wrap("Every line a slow orbit, and the long ones fold over onto the next row", w).iter().enumerate() {
                assert!(row.width() <= if k == 0 { w } else { w - INDENT }, "{w}: {row:?}");
            }
        }
    }
}
