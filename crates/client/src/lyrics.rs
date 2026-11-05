//! Synced lyrics (#14): the current line lit and held a third of the way down, timestamps like a log
//! (`tail -f lyrics.log`), click a line to seek there. Unsynced lyrics follow the song's progress until the wheel takes
//! over; no lyrics is a calm, empty Pane that says why. Long lines wrap (#30): continuation rows are indented, the
//! timestamp only on the first.
//!
//! Scaling: how the lyrics sit in a Pane comes from its size alone (`fit`): a tiny Pane shows the current line and the
//! next, a tall one spaces the lines out and fades the far ones, a wide one keeps the text in a readable column
//! centred in the Pane. Between lines the view glides (`ease`). The layout pieces (`wrap`, `fit`, `first_row`,
//! `ease`, `tone`) are plain functions, so another lyrics view can lay out the same way.

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use singularity_proto::LyricLine;
use std::time::Instant;
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
/// was a third of the Client's allocations (#71). Also where the view is (it glides between lines) and whether the
/// reader has taken over an unsynced song with the wheel.
#[derive(Default)]
pub struct Wrapped {
    /// (song, text width, lines, gap)
    key: (String, usize, usize, usize),
    /// (line, text); a gap row between lines has no text
    rows: Vec<(usize, String)>,
    /// where each line's first row is
    starts: Vec<usize>,
    /// the row after each line's last
    ends: Vec<usize>,
    /// the top of the view, easing toward where it belongs, and when it was last drawn
    shown: f32,
    at: Option<Instant>,
    /// unsynced: the scroll the follow set last frame (the wheel moving it means the reader took over: `held`)
    followed: Option<usize>,
    held: bool,
}

/// How far continuation rows are indented.
const INDENT: usize = 2;
/// The timestamp column, `[mm:ss] `.
const STAMP_W: u16 = 8;
/// The widest the text gets: a wider Pane centres a column as wide as the song's longest line, up to this (rows much
/// longer than this are hard to follow back to their start).
pub const MEASURE: u16 = 72;
/// Text room from which the lyrics are a centred column rather than flush left like a log.
const CENTRE_FROM: u16 = 60;

/// A line's rows: words fill `w` columns (continuations `w - INDENT`, indent not included); a word too long for a
/// row is broken. Widths are display columns.
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    wrap_indented(s, w, INDENT)
}

/// `wrap` with continuation rows `indent` columns narrower (0: all rows `w` wide).
pub fn wrap_indented(s: &str, w: usize, indent: usize) -> Vec<String> {
    let w = w.max(indent + 1);
    let mut rows = vec![String::new()];
    let room = |rows: &Vec<String>| if rows.len() == 1 { w } else { w - indent };
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
            if !cur.is_empty() && cur.width() + unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) > room(&rows) {
                rows.push(String::new());
            }
            rows.last_mut().unwrap().push(c);
        }
    }
    rows
}

/// How the lyrics sit in a Pane of `w`×`h` cells (inside its frame). It depends on the size and the song's longest
/// line only, so nothing jumps while a song plays. Offsets are from the Pane's left edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// the timestamp column, when there's room for one (synced lyrics only)
    pub stamp_x: Option<u16>,
    pub text_x: u16,
    pub text_w: u16,
    /// rows for the lyrics (the credit's row not included)
    pub body: u16,
    /// blank rows between lines, in a tall Pane
    pub gap: u16,
    /// the source credit on the last row, when there are rows to spare
    pub credit: bool,
    /// the body row the current line starts on: about a third of the way down
    pub anchor: u16,
    /// the current line on a faint band of the accent, across the column (a Pane big enough to lose it in)
    pub band: bool,
}

/// The layout for a `w`×`h` Pane; `longest`: the song's longest line in columns.
pub fn fit(w: u16, h: u16, synced: bool, longest: u16) -> Fit {
    let margin = u16::from(w >= 12);
    let avail = w.saturating_sub(2 * margin);
    let stamps = synced && avail >= 30;
    let gutter = if stamps { STAMP_W } else { 0 };
    let room = avail - gutter;
    // wide: a column as wide as the longest line (up to MEASURE), centred with its timestamps; else flush left
    let (text_w, left) = if room >= CENTRE_FROM {
        let tw = longest.clamp(24, MEASURE).min(room);
        (tw, margin + (avail - gutter - tw) / 2)
    } else {
        (room, margin)
    };
    let credit = h >= 8;
    let body = h - u16::from(credit);
    let gap = u16::from(body >= 18);
    Fit { stamp_x: stamps.then_some(left), text_x: left + gutter, text_w, body, gap, credit, anchor: body / 3, band: body >= 12 && avail >= 30 }
}

/// The first row in view for a current line whose rows are `top..end`: it starts on row `anchor` of the `body` rows,
/// and all its rows are in view (from its start, if it has more rows than the view).
pub fn first_row(top: usize, end: usize, body: usize, anchor: usize) -> usize {
    let first = top.saturating_sub(anchor);
    if end > first + body { end.saturating_sub(body).min(top) } else { first }
}

/// The view's top row `shown` moved toward `target` after `dt` seconds: a glide of about a tenth of a second between
/// lines; a jump (a seek, a new song) is taken at once.
pub fn ease(shown: f32, target: f32, dt: f32) -> f32 {
    let d = target - shown;
    if d.abs() > 12.0 || d.abs() < 0.05 {
        return target;
    }
    shown + d * (1.0 - (-dt / 0.045).exp())
}

/// A line's style by where it is from the current one (`d` = line − current): the current line lit and bold, the
/// next one plain, those further on dimmer, the sung ones quietest. `reach`: lines over which the far ones fade (a
/// third of the view, so a tall Pane keeps more context readable).
pub fn tone(th: &Theme, d: i64, reach: usize) -> Style {
    let reach = reach.max(2) as f32;
    match d {
        0 => th.s(th.accent).add_modifier(Modifier::BOLD),
        1 => th.s(th.fg),
        d if d > 1 => th.s(th.dim.lerp(th.border, ((d - 2) as f32 / reach).min(1.0))),
        d => th.s(th.border.lerp(th.bg, ((-d - 1) as f32 / reach).min(0.5))),
    }
}

/// `s` wrapped by words and centred in `r`, under a ♪ when there's room: a calm Pane that says why it's empty.
fn message(buf: &mut Buffer, r: Rect, s: &str, th: &Theme) {
    let rows = wrap_indented(s, r.width as usize, 0);
    let note = r.height as usize >= rows.len() + 3;
    let n = (rows.len() + if note { 2 } else { 0 }).min(r.height as usize) as u16;
    let mut y = r.y + (r.height - n) / 2;
    let centre = |buf: &mut Buffer, y: u16, s: &str, st| {
        let w = (s.width() as u16).min(r.width);
        text(buf, r.x + (r.width - w) / 2, y, w, s, st);
    };
    if note {
        centre(buf, y, "♪", th.s(th.border));
        y += 2;
    }
    for row in rows.iter().take((r.bottom() - y) as usize) {
        centre(buf, y, row, th.s(th.dim));
        y += 1;
    }
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.height == 0 || r.width < 4 {
        return;
    }
    let Some(item) = app.playback.item.as_ref() else {
        return message(buf, r, "nothing playing", th);
    };
    let Some(lyr) = app.lyrics.as_ref().filter(|l| l.uri == item.uri) else {
        return message(buf, r, "looking for lyrics…", th);
    };
    if lyr.lines.is_empty() {
        let why = if lyr.missing.is_empty() { "no lyrics for this one" } else { lyr.missing.as_str() };
        return message(buf, r, why, th);
    }
    let duration = item.duration_ms;
    let longest = lyr.lines.iter().map(|l| l.text.width()).max().unwrap_or(0).min(u16::MAX as usize) as u16;
    let f = fit(r.width, r.height, lyr.synced, longest);
    let body = f.body as usize;
    let pos = app.display_position(singularity_proto::now_ms());
    // every line's rows, and where each line's rows are: wrapped again only for a new song, width or spacing
    let cache = &mut app.lyrics_wrap;
    let key = (lyr.uri.clone(), f.text_w as usize, lyr.lines.len(), f.gap as usize);
    let fresh = cache.key != key;
    if cache.key.0 != lyr.uri {
        (cache.held, cache.followed) = (false, None);
    }
    if fresh {
        cache.key = key;
        cache.rows.clear();
        cache.starts.clear();
        cache.ends.clear();
        for (i, line) in lyr.lines.iter().enumerate() {
            if i > 0 {
                cache.rows.extend((0..f.gap).map(|_| (i, String::new())));
            }
            cache.starts.push(cache.rows.len());
            let words = if line.text.trim().is_empty() { "♪" } else { line.text.as_str() };
            cache.rows.extend(wrap(words, f.text_w as usize).into_iter().map(|t| (i, t)));
            cache.ends.push(cache.rows.len());
        }
    }
    let now = if lyr.synced { current(&lyr.lines, pos) } else { None };
    let total = cache.rows.len();
    let first = match now {
        // synced: the current line on the anchor row, gliding there from the last one, and always whole in view
        Some(c) => {
            let (top, end) = (cache.starts[c], cache.ends[c]);
            let target = first_row(top, end, body, f.anchor as usize);
            let dt = cache.at.map_or(1.0, |t| t.elapsed().as_secs_f32());
            cache.shown = if fresh { target as f32 } else { ease(cache.shown, target as f32, dt) };
            let first = cache.shown.round().max(0.0) as usize;
            if end - top <= body { first.clamp(end - body.min(end), top) } else { top }
        }
        None if lyr.synced => {
            cache.shown = 0.0;
            0
        }
        // unsynced: the reading place follows the song's progress until the wheel moves the view
        None => {
            let max = total.saturating_sub(body);
            if cache.followed.is_some_and(|v| v != app.lyrics_scroll) || (cache.followed.is_none() && app.lyrics_scroll != 0) {
                cache.held = true;
            }
            if !cache.held && duration > 0 {
                let at = (pos.min(duration) as f64 / duration as f64 * total as f64) as usize;
                app.lyrics_scroll = at.saturating_sub(f.anchor as usize).min(max);
                cache.followed = Some(app.lyrics_scroll);
            }
            app.lyrics_scroll = app.lyrics_scroll.min(max);
            app.lyrics_scroll
        }
    };
    cache.at = Some(Instant::now());
    let cache = &app.lyrics_wrap;
    let reach = body / 3;
    let mut hits = vec![];
    for (k, (i, words)) in cache.rows.iter().enumerate().skip(first).take(body) {
        if words.is_empty() {
            continue;
        }
        let (i, y) = (*i, r.y + (k - first) as u16);
        let st = match now {
            Some(c) => tone(th, i as i64 - c as i64, reach),
            None if lyr.synced => tone(th, i as i64 + 1, reach),
            None => th.s(th.fg),
        };
        let first_row = cache.starts[i] == k;
        if f.band && now == Some(i) {
            // a cell of the band either side of the column, inside the Pane
            let from = r.x + f.stamp_x.unwrap_or(f.text_x).saturating_sub(1);
            let to = (r.x + f.text_x + f.text_w + 1).min(r.right());
            crate::ui::blank(buf, from, y, to - from, th.base().bg(th.bg.lerp(th.accent, 0.12).color()));
        }
        if let Some(sx) = f.stamp_x
            && first_row
        {
            let st = if now == Some(i) { th.s(th.accent) } else { th.s(th.border) };
            text(buf, r.x + sx, y, STAMP_W, &stamp(lyr.lines[i].start_ms), st);
        }
        let indent = if first_row { 0 } else { (INDENT as u16).min(f.text_w.saturating_sub(1)) };
        text(buf, r.x + f.text_x + indent, y, f.text_w - indent, words, st);
        if lyr.synced {
            hits.push((Rect::new(r.x, y, r.width, 1), Hit::Lyric(lyr.lines[i].start_ms)));
        }
    }
    if f.credit {
        // the longest credit that fits
        let credits = if lyr.synced { vec![format!("lyrics: {}", lyr.source), lyr.source.clone()] } else { vec![format!("lyrics: {} · not time-synced", lyr.source), format!("{} · unsynced", lyr.source), lyr.source.clone()] };
        let credit = credits.iter().find(|c| c.width() < r.width as usize).unwrap_or(credits.last().unwrap());
        let cw = (credit.width() as u16).min(r.width);
        text(buf, r.right().saturating_sub(cw + 1).max(r.x), r.bottom() - 1, cw, credit, th.s(th.border));
    }
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
        // no indent: every row as wide; a wide character never makes an empty row
        assert_eq!(wrap_indented("aaaa bbbb cccc", 9, 0), ["aaaa bbbb", "cccc"]);
        assert_eq!(wrap_indented("光光", 1, 0), ["光", "光"]);
    }

    #[test]
    fn the_layout_fits_every_size() {
        for w in 0..=300u16 {
            for h in 0..=100u16 {
                for synced in [true, false] {
                    for longest in [0u16, 30, 200] {
                        let f = fit(w, h, synced, longest);
                        let ctx = format!("{w}x{h} synced {synced} longest {longest}: {f:?}");
                        assert!(f.text_x + f.text_w <= w, "{ctx}");
                        assert!(f.body + u16::from(f.credit) <= h && f.anchor < f.body.max(1), "{ctx}");
                        if let Some(sx) = f.stamp_x {
                            assert!(sx + STAMP_W <= f.text_x, "{ctx}");
                        }
                        // the column is centred: as much room on its right as on its left (give or take a cell)
                        let right = w - f.text_x - f.text_w;
                        let left = f.stamp_x.unwrap_or(f.text_x);
                        assert!(right >= left && right <= left + 1 + u16::from(w >= 12), "{ctx}");
                        if w >= 12 {
                            assert!(f.text_w >= 1 && left >= 1, "{ctx}");
                        }
                    }
                }
            }
        }
        // a wide Pane: a readable column in the middle, not text running the whole width
        let f = fit(256, 64, true, 50);
        assert_eq!(f.text_w, 50);
        assert!(f.stamp_x.unwrap() > 90 && f.gap == 1 && f.anchor == 21, "{f:?}");
        // a short one: no credit, the current line on the first row
        let f = fit(10, 2, true, 50);
        assert!(!f.credit && f.anchor == 0 && f.stamp_x.is_none() && f.body == 2, "{f:?}");
    }

    #[test]
    fn the_current_line_starts_a_third_down_and_is_always_whole() {
        assert_eq!(first_row(30, 31, 30, 10), 20);
        assert_eq!(first_row(3, 4, 30, 10), 0, "the start of the song: from the top");
        // a line of 6 rows in a view of 4: from its start
        assert_eq!(first_row(30, 36, 4, 1), 30);
        // a line of 3 rows a row under the anchor's reach: moved up just enough
        assert_eq!(first_row(30, 33, 4, 2), 29);
        for body in 1..40 {
            for anchor in 0..body {
                for top in 0..60 {
                    for len in 1..8 {
                        let f = first_row(top, top + len, body, anchor);
                        assert!(f <= top, "the line's start in view");
                        if len <= body {
                            assert!(top + len <= f + body, "the whole line in view");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_view_glides_and_jumps_when_far() {
        let mut s = 10.0;
        let mut steps = 0;
        while s != 12.0 {
            s = ease(s, 12.0, 1.0 / 60.0);
            steps += 1;
            assert!(steps < 60, "it gets there");
        }
        assert!(steps >= 4, "over a few frames, not at once: {steps}");
        assert_eq!(ease(10.0, 40.0, 0.016), 40.0, "a seek is taken at once");
        assert_eq!(ease(10.0, 12.0, 5.0), 12.0, "after a long pause, there");
    }

    #[test]
    fn far_lines_fade_and_the_current_one_stands_out() {
        let th = Theme::singularity();
        assert_eq!(tone(&th, 0, 5).fg, th.s(th.accent).fg);
        assert!(tone(&th, 0, 5).add_modifier.contains(Modifier::BOLD));
        assert_eq!(tone(&th, 1, 5).fg, th.s(th.fg).fg);
        assert_ne!(tone(&th, 2, 5).fg, tone(&th, 6, 5).fg, "further is fainter");
        assert_ne!(tone(&th, -1, 5).fg, tone(&th, -5, 5).fg);
    }
}
