//! Synced lyrics (#14): the current line lit and held a third of the way down, timestamps like a log
//! (`tail -f lyrics.log`), click a line to seek there. Unsynced lyrics follow the song's progress until the wheel takes
//! over; no lyrics is a calm, empty Pane that says why. Long lines wrap (#30): continuation rows are indented, the
//! timestamp only on the first.
//!
//! Scaling: how the lyrics sit in a Pane comes from its size alone (`fit`): a tiny Pane shows the current line and the
//! next, a tall one fades the far lines. The text always runs from the Pane's left edge like a log and wraps only at
//! the Pane's width, one row after another with no blank rows between lines (a blank row read as a line split in
//! two, and a centred column left dead space either side: both tried and taken out). Between lines the view glides
//! (`ease`). The layout pieces (`wrap`, `fit`, `first_row`, `ease`, `tone`) are plain functions, so another lyrics
//! view can lay out the same way.
//!
//! A big Pane on the pixel tier draws the same view in big text (`bigtext`, `big_fit`): sized so the song's longer
//! lines reach across the Pane and at least `BIG_LINES` lines fit its height, each row an image placed on cells.

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

/// Every line's rows, kept between frames for the lyrics in hand at one width: wrapping all of a song's lines on every
/// frame was a third of the Client's allocations (#71). Also where the view is (it glides between lines) and whether
/// the reader has taken over an unsynced song with the wheel. `SongLyrics` starts it afresh when new lyrics arrive.
#[derive(Default)]
pub struct Wrapped {
    /// the text width (or the big text's size) the rows are wrapped for
    key: Option<u64>,
    /// the width of the song's longer lines in ems (`typical`; None when the fonts can't draw it), once worked out
    typical: Option<Option<f32>>,
    /// (line, text)
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
    /// synced: the wheel takes the view for a while, then it follows the song again
    pub(crate) follow: crate::scroll::Follow,
    /// unsynced: the first row shown
    pub(crate) scroll: usize,
}

impl Wrapped {
    /// The width of the song's longer lines (`typical`), worked out once for the lyrics in hand.
    fn typical(&mut self, lines: &[LyricLine]) -> Option<f32> {
        *self.typical.get_or_insert_with(|| typical(lines))
    }

    /// Every line's rows for a layout (`key`: the text width, or the big text's size) as `wrap` breaks a line: wrapped
    /// again only for another layout (new lyrics start afresh). Whether they were wrapped just now.
    fn wrap(&mut self, lines: &[LyricLine], key: u64, wrap: impl Fn(&str) -> Vec<String>) -> bool {
        if self.key == Some(key) {
            return false;
        }
        self.key = Some(key);
        self.rows.clear();
        self.starts.clear();
        self.ends.clear();
        for (i, line) in lines.iter().enumerate() {
            self.starts.push(self.rows.len());
            let words = if line.text.trim().is_empty() { "♪" } else { line.text.as_str() };
            self.rows.extend(wrap(words).into_iter().map(|t| (i, t)));
            self.ends.push(self.rows.len());
        }
        true
    }
}

/// How far continuation rows are indented.
const INDENT: usize = 2;
/// The timestamp column, `[mm:ss] `.
const STAMP_W: u16 = 8;

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

/// How the lyrics sit in a Pane of `w`×`h` cells (inside its frame). It depends on the size only, so nothing jumps
/// while a song plays. Offsets are from the Pane's left edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// the timestamp column, when there's room for one (synced lyrics only)
    pub stamp_x: Option<u16>,
    pub text_x: u16,
    pub text_w: u16,
    /// rows for the lyrics (the credit's row not included)
    pub body: u16,
    /// the source credit on the last row, when there are rows to spare
    pub credit: bool,
    /// the body row the current line starts on: about a third of the way down
    pub anchor: u16,
    /// the current line on a faint band of the accent, across the column (a Pane big enough to lose it in)
    pub band: bool,
}

/// The layout for a `w`×`h` Pane: flush left like a log (`[mm:ss] text`), the text as wide as the Pane allows.
pub fn fit(w: u16, h: u16, synced: bool) -> Fit {
    let margin = u16::from(w >= 12);
    let avail = w.saturating_sub(2 * margin);
    let stamps = synced && avail >= 30;
    let gutter = if stamps { STAMP_W } else { 0 };
    let credit = h >= 8;
    let body = h - u16::from(credit);
    Fit { stamp_x: stamps.then_some(margin), text_x: margin + gutter, text_w: avail - gutter, body, credit, anchor: body / 3, band: body >= 12 && avail >= 30 }
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

/// The smallest Pane (inside its frame, credit row not counted) that gets big text: smaller ones are the cells' own.
const BIG_W: u16 = 96;
const BIG_H: u16 = 24;
/// Lines a big Pane shows at least (a line's rows a third down: three sung above, the rest to come).
const BIG_LINES: u16 = 8;
/// Big text at least this many times a cell's height (its pixels to the em), else it isn't worth leaving the cells.
const BIG_MIN: f64 = 1.6;
/// The timestamps' size beside big text, against the lines'.
const STAMP_SCALE: f32 = 0.55;

/// How big text fills a big Pane: the lines' size, the rows' height in cells, the columns in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BigFit {
    /// the lines' and the timestamps' pixels to the em
    pub px: f32,
    pub stamp_px: f32,
    /// cells a row takes, top to bottom
    pub pitch: u16,
    /// pixels from the column's left: where the text starts (after the timestamps)
    pub text_x: f32,
    /// pixels for the text, and the indent of a wrapped line's continuation rows
    pub text_w: f32,
    pub indent: f32,
}

impl BigFit {
    /// What the rows depend on, for the wrap cache (kept apart from the cells' widths by its top bit).
    fn key(&self) -> u64 {
        1 << 63 | (self.px.to_bits() as u64) << 16 | (self.text_w as u64) << 48 | self.pitch as u64
    }
}

/// The width of a song's longer lines at 1 px to the em (bold, as the current line is drawn): the line nine in ten
/// are no wider than, or the longest when it is only a little wider. None when the fonts lack one of its characters.
pub fn typical(lines: &[LyricLine]) -> Option<f32> {
    if !lines.iter().all(|l| crate::bigtext::covers(&l.text)) {
        return None;
    }
    Some(crate::bigtext::typical(lines.iter().map(|l| if l.text.trim().is_empty() { "♪" } else { l.text.as_str() }), 1.5))
}

/// Big text for a Pane laid out as `f` (`w` columns), cells of `cw`×`ch` pixels, a song whose longer lines are
/// `typical` ems wide: as big as lets those lines span the column and `BIG_LINES` lines fit. None for a Pane too small
/// to gain from it.
pub fn big_fit(f: &Fit, w: u16, (cw, ch): (f64, f64), typical: f32) -> Option<BigFit> {
    if w < BIG_W || f.body < BIG_H || cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    let left = f.stamp_x.unwrap_or(f.text_x);
    let avail = (f.text_x + f.text_w - left) as f64 * cw;
    let stamp_em = if f.stamp_x.is_some() { crate::bigtext::width("[00:00]  ", STAMP_SCALE, false) as f64 } else { 0.0 };
    let by_width = avail * 0.97 / (typical.max(1.0) as f64 + stamp_em);
    let by_height = (f.body / BIG_LINES) as f64 * ch / crate::bigtext::line() as f64;
    let px = by_width.min(by_height);
    if px < BIG_MIN * ch {
        return None;
    }
    let px = px.floor() as f32;
    let pitch = ((px * crate::bigtext::line()) as f64 / ch).ceil() as u16;
    let text_x = (stamp_em * px as f64) as f32;
    Some(BigFit { px, stamp_px: px * STAMP_SCALE, pitch, text_x, text_w: avail as f32 - text_x, indent: crate::bigtext::width("  ", px, false) })
}

/// A line's colour by where it is from the current one, and whether it is bold: `tone`'s colours.
pub fn tone_rgb(th: &Theme, d: i64, reach: usize) -> (crate::theme::Rgb, bool) {
    let reach = reach.max(2) as f32;
    match d {
        0 => (th.accent, true),
        1 => (th.fg, false),
        d if d > 1 => (th.dim.lerp(th.border, ((d - 2) as f32 / reach).min(1.0)), false),
        d => (th.border.lerp(th.bg, ((-d - 1) as f32 / reach).min(0.5)), false),
    }
}

/// A line's style by where it is from the current one (`d` = line − current): the current line lit and bold, the
/// next one plain, those further on dimmer, the sung ones quietest. `reach`: lines over which the far ones fade (a
/// third of the view, so a tall Pane keeps more context readable).
pub fn tone(th: &Theme, d: i64, reach: usize) -> Style {
    let (c, bold) = tone_rgb(th, d, reach);
    if bold { th.s(c).add_modifier(Modifier::BOLD) } else { th.s(c) }
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
    let Some(lyr) = app.lyrics.for_song(&item.uri) else {
        return message(buf, r, "looking for lyrics…", th);
    };
    if lyr.lines.is_empty() {
        let why = if lyr.missing.is_empty() { "no lyrics for this one" } else { lyr.missing.as_str() };
        return message(buf, r, why, th);
    }
    let duration = item.duration_ms;
    let f = fit(r.width, r.height, lyr.synced);
    let pos = app.lyrics_position(singularity_proto::now_ms());
    // a big Pane on the pixel tier: big text, rows `pitch` cells tall
    let cpx = app.pixel_text_for(r);
    let Some((lyr, cache)) = app.lyrics.lyrics_view() else { return };
    let big = cpx.and_then(|cpx| cache.typical(&lyr.lines).map(|t| (cpx, t))).and_then(|(cpx, t)| big_fit(&f, r.width, cpx, t).map(|b| (b, cpx)));
    let (body, anchor) = match big {
        Some((b, _)) => ((f.body / b.pitch) as usize, (f.body / b.pitch) as usize / 3),
        None => (f.body as usize, f.anchor as usize),
    };
    // every line's rows, and where each line's rows are
    let fresh = match big {
        Some((b, _)) => cache.wrap(&lyr.lines, b.key(), |s| crate::bigtext::wrap(s, b.text_w, b.indent, |s| crate::bigtext::width(s, b.px, true))),
        None => cache.wrap(&lyr.lines, f.text_w as u64, |s| wrap(s, f.text_w as usize)),
    };
    let now = if lyr.synced { current(&lyr.lines, pos) } else { None };
    let total = cache.rows.len();
    // big text glides by cells: where the view's top is between rows (-1: on a row)
    let mut glide: f32 = -1.0;
    let first = match now {
        // synced: the current line on the anchor row, gliding there from the last one, and always whole in view
        Some(c) => {
            let (top, end) = (cache.starts[c], cache.ends[c]);
            let target = first_row(top, end, body, anchor);
            let dt = cache.at.map_or(1.0, |t| t.elapsed().as_secs_f32());
            cache.shown = if fresh { target as f32 } else { ease(cache.shown, target as f32, dt) };
            if big.is_some() {
                let lo = if end - top <= body { (end - body.min(end)) as f32 } else { top as f32 };
                glide = cache.shown.max(lo).min(top as f32).max(0.0);
            }
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
            if cache.followed.is_some_and(|v| v != cache.scroll) || (cache.followed.is_none() && cache.scroll != 0) {
                cache.held = true;
            }
            if !cache.held && duration > 0 {
                let at = (pos.min(duration) as f64 / duration as f64 * total as f64) as usize;
                cache.scroll = at.saturating_sub(anchor).min(max);
                cache.followed = Some(cache.scroll);
            }
            cache.scroll = cache.scroll.min(max);
            cache.scroll
        }
    };
    // synced: the wheel can hold the view elsewhere for a while (`scroll::Follow`); it glides back from there
    let first = if lyr.synced {
        let held = cache.follow.first(first, total.saturating_sub(body).max(first));
        if cache.follow.held() {
            cache.shown = held as f32;
            glide = held as f32;
        }
        held
    } else {
        first
    };
    cache.at = Some(Instant::now());
    if let Some((b, cpx)) = big {
        let top = if glide >= 0.0 { glide } else { first as f32 };
        let hits = draw_big(buf, r, app, th, &f, &b, cpx, top, now, body);
        return finish(buf, r, app, th, &f, hits);
    }
    let reach = body / 3;
    let mut hits = vec![];
    for (k, (i, words)) in cache.rows.iter().enumerate().skip(first).take(body) {
        let (i, y) = (*i, r.y + (k - first) as u16);
        // the tail of a line that starts above the view isn't shown: alone on the top row it read as a line of its own
        // (synced only: unsynced lyrics scroll by rows under the reader's wheel)
        if lyr.synced && cache.starts[i] < first && now != Some(i) {
            continue;
        }
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
    finish(buf, r, app, th, &f, hits);
}

/// The credit on the last row, and the lines' click targets.
fn finish(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme, f: &Fit, hits: Vec<(Rect, Hit)>) {
    let Some(lyr) = app.lyrics.current() else { return };
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

/// The big-text view: each row in view an image `b.pitch` cells tall, the view's top `top` rows down the song (a
/// fraction while it glides: it moves by cells). Returns the lines' click targets.
#[allow(clippy::too_many_arguments)]
fn draw_big(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme, f: &Fit, b: &BigFit, (cw, ch): (f64, f64), top: f32, now: Option<usize>, body: usize) -> Vec<(Rect, Hit)> {
    use crate::bigtext::{Span, key, render, width};
    let Some((lyr, cache)) = app.lyrics.lyrics_view() else { return vec![] };
    let pitch = b.pitch as i32;
    let off = (top * pitch as f32).round() as i32;
    let first = (off / pitch).max(0) as usize;
    let cells = f.body as i32;
    let left = r.x + f.stamp_x.unwrap_or(f.text_x);
    let room = r.right().saturating_sub(left);
    let img_h = (pitch as f64 * ch).round() as u32;
    // the text in the middle of its rows
    let baseline = ((img_h as f32 - b.px * crate::bigtext::line()) / 2.0 + b.px * crate::bigtext::ascent()).round();
    let reach = body / 3;
    let mut hits = vec![];
    let mut places = vec![];
    for (k, (i, words)) in cache.rows.iter().enumerate().skip(first) {
        let i = *i;
        let y = k as i32 * pitch - off;
        // a row cut off at the bottom only while the view glides
        if y >= cells || (y + pitch > cells && off % pitch == 0) {
            break;
        }
        // (as the cells do: the tail of a line that starts above the view isn't shown)
        if lyr.synced && cache.starts[i] < first && now != Some(i) {
            continue;
        }
        let (rgb, bold) = match now {
            Some(c) => tone_rgb(th, i as i64 - c as i64, reach),
            None if lyr.synced => tone_rgb(th, i as i64 + 1, reach),
            None => (th.fg, false),
        };
        let (y0, y1) = (y.max(0), (y + pitch).min(cells));
        let (ry, rh) = (r.y + y0 as u16, (y1 - y0) as u16);
        if f.band && now == Some(i) {
            let from = r.x + f.stamp_x.unwrap_or(f.text_x).saturating_sub(1);
            let to = (r.x + f.text_x + f.text_w + 1).min(r.right());
            for yy in ry..ry + rh {
                crate::ui::blank(buf, from, yy, to - from, th.base().bg(th.bg.lerp(th.accent, 0.12).color()));
            }
        }
        let first_row = cache.starts[i] == k;
        let st = stamp(lyr.lines[i].start_ms);
        let mut spans = vec![];
        if f.stamp_x.is_some() && first_row {
            let c = if now == Some(i) { th.accent } else { th.border };
            spans.push(Span { x: 0.0, text: &st, rgb: c, px: b.stamp_px, bold: false });
        }
        let x = b.text_x + if first_row { 0.0 } else { b.indent };
        spans.push(Span { x, text: words, rgb, px: b.px, bold });
        // as wide as the text (and a little for the last letter's overhang), in whole cells
        let w_cells = (((x + width(words, b.px, bold) + b.px * 0.15) as f64 / cw).ceil() as u16).clamp(1, room.max(1));
        let img_w = (w_cells as f64 * cw).round() as u32;
        let crop = (rh as i32) < pitch || y < 0;
        let crop = crop.then(|| {
            let t = ((y0 - y) as f64 * ch).round() as u32;
            [0, t, img_w, ((rh as f64 * ch).round() as u32).min(img_h - t)]
        });
        let k = key(&spans, img_w, img_h, baseline);
        // rendered only when the terminal doesn't hold it yet
        let px = (!app.kitty.as_ref().is_some_and(|kt| kt.holds(&k))).then(|| render(&spans, img_w, img_h, baseline));
        places.push((k, Rect::new(left, ry, w_cells, rh), crop, img_w, px));
        if lyr.synced {
            hits.push((Rect::new(r.x, ry, r.width, rh), Hit::Lyric(lyr.lines[i].start_ms)));
        }
    }
    if let Some(kt) = app.kitty.as_mut() {
        for (k, rect, crop, img_w, px) in places {
            kt.place_rgba(k, rect, crop, || (img_w, img_h, px.unwrap_or_default()));
        }
    }
    hits
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn the_current_line_is_the_last_started() {
        let l: Vec<LyricLine> = [1000u64, 5000, 9000].iter().map(|&s| LyricLine { start_ms: s, ..Default::default() }).collect();
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
                    let f = fit(w, h, synced);
                    let ctx = format!("{w}x{h} synced {synced}: {f:?}");
                    assert!(f.text_x + f.text_w <= w, "{ctx}");
                    assert!(f.body + u16::from(f.credit) <= h && f.anchor < f.body.max(1), "{ctx}");
                    if let Some(sx) = f.stamp_x {
                        assert!(sx + STAMP_W <= f.text_x, "{ctx}");
                    }
                    // flush left like a log, as wide as the Pane: a cell of margin either side, no dead space
                    let left = f.stamp_x.unwrap_or(f.text_x);
                    assert_eq!(left, u16::from(w >= 12), "{ctx}");
                    assert_eq!(w - f.text_x - f.text_w, left, "{ctx}");
                }
            }
        }
        // the owner's Terminal design: `[mm:ss] text` from the left edge, wrapped only at the Pane's width
        let f = fit(72, 27, true);
        assert!(f.stamp_x == Some(1) && f.text_x == 9 && f.text_w == 62 && f.anchor == 8, "{f:?}");
        // a short one: no credit, the current line on the first row
        let f = fit(10, 2, true);
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

    /// An App on the pixel tier (cells of 10×20 pixels) playing a song of `n` synced lines, paused at `pos`.
    fn pixel_app(n: u64, pos: u64) -> App {
        use singularity_proto::{Item, Lyrics};
        let mut app = App::new(None);
        app.playback.item = Some(Item { uri: "spotify:track:b".into(), name: "Song".into(), duration_ms: 400_000, ..Default::default() });
        app.playback.position_ms = pos;
        let texts = ["Light bends where the music falls", "We drift past the photon sphere", "", "Every line a slow orbit, and the long ones fold over onto the next row when the Pane is narrow", "Hold on"];
        let lines = (0..n).map(|i| LyricLine { start_ms: i * 4_000, text: texts[i as usize % texts.len()].into(), ..Default::default() }).collect();
        app.lyrics_event(Lyrics { uri: "spotify:track:b".into(), synced: true, lines, source: "fake".into(), ..Default::default() });
        app.cell_px = Some((10.0, 20.0));
        app.kitty = Some(crate::gfx::Kitty::new(false));
        app
    }

    /// Draw the lyrics into a `w`×`h` Pane at (1, 1); the cells' text and what went to the terminal.
    fn frame(app: &mut App, w: u16, h: u16) -> (Vec<String>, String) {
        let th = Theme::singularity();
        let mut buf = Buffer::empty(Rect::new(0, 0, w + 2, h + 2));
        app.hits.clear();
        draw(&mut buf, Rect::new(1, 1, w, h), app, &th);
        let rows = (0..h + 2).map(|y| (0..w + 2).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect();
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        (rows, String::from_utf8_lossy(&out).into_owned())
    }

    /// Every placement in `out`: (column, row, columns, rows), 0-based.
    pub(crate) fn placements(out: &str) -> Vec<(u16, u16, u16, u16)> {
        out.split("\x1b[")
            .filter_map(|s| {
                let (pos, rest) = s.split_once('H')?;
                let rest = rest.strip_prefix("\x1b_Ga=p,")?;
                let (y, x) = pos.split_once(';')?;
                let num = |k: &str| rest.split(',').find_map(|kv| kv.strip_prefix(k)?.parse::<u16>().ok());
                Some((x.parse::<u16>().ok()? - 1, y.parse::<u16>().ok()? - 1, num("c=")?, num("r=")?))
            })
            .collect()
    }

    #[test]
    fn big_text_suits_every_big_pane() {
        for typical in [6.0f32, 14.0, 30.0, 60.0] {
            for w in 0..=300u16 {
                for h in 0..=100u16 {
                    let f = fit(w, h, true);
                    let Some(b) = big_fit(&f, w, (10.0, 20.0), typical) else {
                        continue;
                    };
                    let ctx = format!("{w}x{h} typical {typical}: {b:?}");
                    assert!(w >= BIG_W && f.body >= BIG_H, "{ctx}");
                    assert!(b.px as f64 >= BIG_MIN * 20.0 - 1.0 && b.pitch >= 3, "worth leaving the cells: {ctx}");
                    assert!(f.body / b.pitch >= BIG_LINES, "at least {BIG_LINES} lines fit: {ctx}");
                    assert!(b.px * crate::bigtext::line() <= b.pitch as f32 * 20.0, "a row holds its text: {ctx}");
                    assert!(b.text_x + b.text_w <= (w - 2) as f32 * 10.0 + 0.01, "{ctx}");
                    assert!(b.text_w >= typical * b.px * 0.96, "the longer lines fit across: {ctx}");
                }
            }
        }
        // the owner's big window: about 240×65 cells
        let f = fit(240, 65, true);
        let b = big_fit(&f, 240, (10.0, 20.0), crate::bigtext::width("Light bends where the music falls", 1.0, true)).unwrap();
        assert!(b.px >= 80.0 && b.pitch >= 6, "{b:?}");
        // the Terminal design's lyrics Pane stays the cells' own
        assert!(big_fit(&fit(72, 27, true), 72, (10.0, 20.0), 10.0).is_none());
    }

    #[test]
    fn a_big_pane_on_the_pixel_tier_fills_with_big_text() {
        let mut app = pixel_app(40, 41_000);
        let (rows, out) = frame(&mut app, 240, 65);
        // the lyrics are images now, not cells (the credit is still text)
        assert!(!rows.iter().any(|r| r.contains("photon") || r.contains("[00:")), "{rows:#?}");
        assert!(rows.iter().any(|r| r.contains("lyrics: fake")));
        assert!(out.contains("f=32") && out.matches("a=t,").count() >= 8, "{out:.300}");
        let placed = placements(&out);
        assert!(placed.len() >= 8, "{placed:?}");
        for &(x, y, c, r) in &placed {
            assert!(x >= 1 && x + c <= 241 && y >= 1 && y + r <= 66, "inside the Pane: {placed:?}");
        }
        // reaching across: the widest row spans most of the Pane
        assert!(placed.iter().map(|p| p.2).max().unwrap() >= 150, "{placed:?}");
        // the current line (line 10, at 40 s) a third of the way down
        let cur = app.hits.iter().find(|(_, h)| *h == Hit::Lyric(40_000)).map(|(r, _)| *r).unwrap();
        assert!(cur.height >= 6 && (16..=26).contains(&cur.y), "{cur:?}");
        // steady: the next frame sends nothing new and renders no glyph
        let before = crate::bigtext::rasterized();
        let (_, out) = frame(&mut app, 240, 65);
        assert!(!out.contains("a=t,"), "nothing re-sent: {out:.300}");
        assert_eq!(crate::bigtext::rasterized(), before);
        // a popup over the Pane: the cells take over and the images go
        app.overlays = vec![Rect::new(50, 20, 40, 10)];
        let (rows, out) = frame(&mut app, 240, 65);
        assert!(rows.iter().any(|r| r.contains("[00:40] Light bends")), "{rows:#?}");
        assert!(out.contains("a=d,d=i") && placements(&out).is_empty(), "{out:.300}");
        app.overlays.clear();
        // a small Pane: the cells, as before
        let (rows, out) = frame(&mut app, 72, 27);
        assert!(rows.iter().any(|r| r.contains("[00:40] Light bends")) && placements(&out).is_empty());
    }

    #[test]
    fn big_text_draws_at_every_size_and_moment_inside_its_pane() {
        let mut app = pixel_app(60, 0);
        for &(w, h) in &[(96u16, 25u16), (100, 30), (120, 36), (160, 45), (200, 50), (240, 65), (256, 64), (260, 70), (300, 100), (97, 90), (300, 26)] {
            for pos in [0u64, 1_000, 13_000, 14_100, 77_000, 239_000, 400_000] {
                app.playback.position_ms = pos;
                let (_, out) = frame(&mut app, w, h);
                for &(x, y, c, r) in &placements(&out) {
                    assert!(x >= 1 && x + c <= w + 1 && y >= 1 && y + r <= h + 1, "{w}x{h} at {pos}: ({x},{y},{c},{r})");
                }
                for (r, _) in &app.hits {
                    assert!(r.y >= 1 && r.bottom() <= h + 1, "{w}x{h} at {pos}: {r:?}");
                }
            }
        }
        // a song the fonts can't draw keeps to the cells
        let mut other = app.lyrics.current().unwrap().clone();
        other.lines[3].text = "光が曲がる".into();
        other.uri = "spotify:track:c".into();
        app.playback.item.as_mut().unwrap().uri = "spotify:track:c".into();
        app.lyrics_event(other);
        app.playback.position_ms = 12_100;
        let (rows, out) = frame(&mut app, 240, 65);
        assert!(placements(&out).is_empty() && rows.iter().any(|r| r.contains("光")), "{rows:#?}");
    }

    /// What big text costs (run in release: `cargo test --release -p singularity -- --ignored big_text_cost
    /// --nocapture`): a steady frame, a frame where the line changes, against the cells.
    #[test]
    #[ignore]
    fn big_text_cost() {
        let time = |label: &str, n: u32, f: &mut dyn FnMut(u32)| {
            let t = Instant::now();
            for i in 0..n {
                f(i);
            }
            println!("{label}: {:.1} µs a frame", t.elapsed().as_secs_f64() * 1e6 / n as f64);
        };
        for (w, h) in [(240u16, 65u16), (120, 36)] {
            let mut app = pixel_app(60, 41_000);
            let _ = frame(&mut app, w, h);
            time(&format!("{w}x{h} big, steady"), 2_000, &mut |_| {
                let _ = frame(&mut app, w, h);
            });
            // every frame a new line (worst case: each a fresh set of images, glyphs cached)
            time(&format!("{w}x{h} big, a new line every frame"), 200, &mut |i| {
                app.playback.position_ms = (i as u64 % 55) * 4_000 + 100;
                let _ = frame(&mut app, w, h);
            });
            let mut cells = pixel_app(60, 41_000);
            cells.cell_px = None;
            time(&format!("{w}x{h} cells, steady"), 2_000, &mut |_| {
                let _ = frame(&mut cells, w, h);
            });
        }
    }
}
