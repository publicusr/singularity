//! The Karaoke Widget: the line being sung big and centred, its words filling with the accent as they are sung (left
//! to right, a character at a time), and a ball hopping from word to word above them, landing on each as it starts.
//! The next lines wait dimmed below, the last one fades above. Before a line that follows an instrumental, dots
//! count it in; during the instrumental the Pane says what comes next and when.
//!
//! It scales with its Pane: one or two rows give the line alone (scrolled to the word being sung); a few more add the
//! ball's lane; a big Pane draws the line in the marquee's pixel font (two sizes), and on the pixel tier the ball is
//! a smooth image of its own instead of a `●` stepping through cells. On the pixel tier a bigger Pane draws the line in
//! big text instead (`bigtext`, `plan_px`): one size for the whole song, as big as lets its longer lines span the
//! Pane, long lines wrapped; the line is two images (sung and waiting) split where the fill has got to, and the ball
//! lands on each word's middle as the font measures it. A narrow, tall Pane runs the rest of the song above and
//! below the line like a lyric sheet, each line wrapped whole (`narrow_tall`). Word times come with the lyrics when
//! the source has them, else they're estimated (`timing.rs`); the credit line says which.

pub mod timing;

use crate::app::{App, Hit};
use crate::theme::{Rgb, Theme};
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::LyricLine;
use std::time::{Duration, Instant};
use timing::{Line, Stage, Word};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The ball's Visual id on the pixel tier (the hole is 1, the idle takeover 2, glow 3).
const BALL_ID: u32 = 4;

/// A Pane's columns and rows, and the cells' size in pixels (as bits).
type PaneCells = (u16, u16, (u64, u64));

/// What the Widget keeps between frames: the song's timed lines and sizes (worked out once for the lyrics in hand:
/// `SongLyrics` starts them afresh when new ones arrive), and its view.
#[derive(Default)]
pub struct State {
    /// the timed lines, and the song's length they were timed for
    lines: Option<(u64, Vec<Line>)>,
    /// the ball is moving on the pixel tier until then: frames come quicker meanwhile
    smooth_until: Option<Instant>,
    /// unsynced lyrics follow the song's progress; the wheel takes them for a while
    pub(crate) follow: crate::scroll::Follow,
    /// the song's big-text size for a Pane: (columns, rows, cell size) → pixels to the em
    size: Option<(PaneCells, Option<f32>)>,
    /// the song's pixel-font scale for a Pane: (columns, rows) → scale (0: plain text)
    scale: Option<((u16, u16), u16)>,
}

impl State {
    /// Whether the ball needs frames faster than the usual 30 a second right now.
    pub fn smooth(&self) -> bool {
        self.smooth_until.is_some_and(|t| Instant::now() < t)
    }

    /// The ball is moving on the pixel tier: quicker frames for a moment.
    pub(crate) fn moving(&mut self) {
        self.smooth_until = Some(Instant::now() + Duration::from_millis(200));
    }

    /// New lyrics arrived: what was worked out from the old ones goes (the wheel's hold stays).
    pub(crate) fn new_lyrics(&mut self) {
        (self.lines, self.size, self.scale) = (None, None, None);
    }

    /// The timed lines of `lyrics` (a song `duration` ms long), and for a `w`×`h` Pane the song's big-text size (cells
    /// of `cpx` pixels, the pixel tier) and pixel-font scale: each worked out once for these lyrics.
    pub(crate) fn song(&mut self, lyrics: &[LyricLine], duration: u64, w: u16, h: u16, cpx: Option<(f64, f64)>) -> (&[Line], Option<f32>, u16) {
        if self.lines.as_ref().is_none_or(|(d, _)| *d != duration) {
            self.lines = Some((duration, timing::timed(lyrics, duration)));
            (self.size, self.scale) = (None, None);
        }
        let lines = &self.lines.as_ref().expect("timed above").1;
        let credit = u16::from(h >= 7);
        // big text on the pixel tier: the song's size for this Pane
        let px = cpx.and_then(|c| {
            let key = (w, h, (c.0.to_bits(), c.1.to_bits()));
            match self.size {
                Some((k, px)) if k == key => px,
                _ => {
                    // (a narrow, tall Pane: its own, #54)
                    let px = if narrow_tall(w, h) { narrow_px(lines, w.saturating_sub(2), c) } else { song_px(lines, w.saturating_sub(2), h - credit, c) };
                    self.size = Some((key, px));
                    px
                }
            }
        });
        // the cells' font: the song's pixel-font scale for this Pane
        let scale = match self.scale {
            Some((k, s)) if k == (w, h) => s,
            _ => {
                let s = song_scale(lines, w.saturating_sub(2), h - credit);
                self.scale = Some(((w, h), s));
                s
            }
        };
        (lines, px, scale)
    }
}

/// How the line on stage is drawn: plain text, or the pixel font at a scale (a font pixel is `s` columns by `s`
/// half-rows: square on a terminal's 1:2 cells).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Font {
    Plain,
    Big(u16),
    /// big text (pixel tier): its pixels to the em, the cells a row takes and the columns between count-in dots
    Px { px: f32, pitch: u16, dot: u16 },
}

impl Font {
    /// Columns per character (the pixel font's include the gap after it).
    fn char_w(self) -> u16 {
        match self {
            Font::Plain => 1,
            Font::Big(s) => 4 * s,
            Font::Px { dot, .. } => dot,
        }
    }
    /// Extra columns between two words.
    fn space(self) -> u16 {
        match self {
            Font::Plain | Font::Px { .. } => 1,
            Font::Big(s) => 2 * s,
        }
    }
    /// Text rows a line of it takes.
    fn rows(self) -> u16 {
        match self {
            Font::Plain => 1,
            Font::Big(s) => (5 * s).div_ceil(2),
            Font::Px { pitch, .. } => pitch,
        }
    }
    fn width(self, s: &str) -> u16 {
        match self {
            Font::Plain | Font::Px { .. } => s.width() as u16,
            Font::Big(_) => s.chars().count() as u16 * self.char_w(),
        }
    }
}

/// The pixel font's form of a character, if it has one (curly quotes, dashes and a few more are mapped first).
fn big_char(c: char) -> Option<char> {
    let c = match c {
        '’' | '‘' | '`' | '"' | '“' | '”' | '´' => '\'',
        ';' => ',',
        '…' => '.',
        '–' | '—' => '-',
        c => c,
    };
    if c.is_ascii_alphanumeric() || " .,!?'-&()/:".contains(c) {
        Some(c)
    } else {
        crate::ambient::plain(c)
    }
}

/// Where a piece of the line sits: its text row (of the wrapped line) and columns, from the Pane's left.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Spot {
    row: u16,
    x: u16,
    w: u16,
}

/// The line's pieces in rows of at most `max` columns, pieces sung as one word kept together, each row centred.
/// Returns the spots and the number of rows.
fn lay_out(words: &[Word], font: Font, max: u16) -> (Vec<Spot>, u16) {
    let mut spots: Vec<Spot> = Vec::with_capacity(words.len());
    let mut rows: Vec<u16> = vec![0];
    let mut k = 0;
    while k < words.len() {
        // a word: this piece and those glued to it
        let mut end = k + 1;
        while end < words.len() && words[end].glued {
            end += 1;
        }
        let gw: u16 = words[k..end].iter().map(|w| font.width(&w.text)).fold(0u16, u16::saturating_add);
        let cur = *rows.last().unwrap();
        let mut x = if cur == 0 { 0 } else { cur.saturating_add(font.space()) };
        if cur > 0 && x.saturating_add(gw) > max {
            rows.push(0);
            x = 0;
        }
        let row = rows.len() as u16 - 1;
        for w in &words[k..end] {
            let pw = font.width(&w.text);
            spots.push(Spot { row, x, w: pw });
            x = x.saturating_add(pw);
        }
        *rows.last_mut().unwrap() = x;
        k = end;
    }
    // centre each row (the pixel font's last gap doesn't count)
    let trail = if let Font::Big(s) = font { s } else { 0 };
    for s in &mut spots {
        let used = rows[s.row as usize].saturating_sub(trail);
        s.x += max.saturating_sub(used) / 2;
    }
    (spots, rows.len() as u16)
}

/// How the line on stage fits a Pane: its font, the ball's lane above each text row, where its pieces go.
#[derive(Debug, Clone, PartialEq)]
struct Plan {
    font: Font,
    /// rows above each text row for the ball (0: no ball)
    lane: u16,
    spots: Vec<Spot>,
    rows: u16,
    /// big text: each word's left and width in pixels from the line's left column, and each row's text, its first
    /// column and its width in columns
    px: Vec<(f32, f32)>,
    row_px: Vec<(String, u16, u16)>,
}

impl Plan {
    /// Rows the whole line takes, lanes included.
    fn height(&self) -> u16 {
        self.rows.saturating_mul(self.lane + self.font.rows())
    }
}

/// The pixel font's sizes in cells: (scale, lane, rows a line keeps to when it can, rows to keep free: the big size
/// wants two next lines, the small one).
const BIG_SIZES: [(u16, u16, u16, u16); 2] = [(2, 4, 2, 4), (1, 3, 3, 2)];

/// The line in the pixel font at `s` in `w` × `h`, if it fits: within `most` rows, or (`stretch`) in as many as the
/// height takes.
fn plan_big(words: &[Word], w: u16, h: u16, (s, lane, most, spare): (u16, u16, u16, u16), stretch: bool) -> Option<Plan> {
    if words.is_empty() || !words.iter().all(|x| x.text.chars().all(|c| big_char(c).is_some())) {
        return None;
    }
    let font = Font::Big(s);
    let (spots, rows) = lay_out(words, font, w);
    let fits = spots.iter().all(|p| p.x + p.w <= w + s);
    let p = Plan { font, lane, spots, rows, px: vec![], row_px: vec![] };
    (fits && (stretch || rows <= most) && p.height() + spare <= h).then_some(p)
}

/// The biggest pixel-font scale every line of the song fits `w` × `h` at (a long line may take more rows), 0 for plain
/// text: one font for the whole song, so a long line doesn't switch to another (the owner, 2026-11-06). Lines the
/// pixel font has no letters for (CJK) don't count: they are plain text whatever the rest is.
fn song_scale(lines: &[Line], w: u16, h: u16) -> u16 {
    let sung = || lines.iter().filter(|l| !l.words.is_empty() && l.words.iter().all(|x| x.text.chars().all(|c| big_char(c).is_some())));
    BIG_SIZES.iter().find(|&&size| sung().all(|l| plan_big(&l.words, w, h, size, true).is_some())).map_or(0, |&(s, ..)| s)
}

/// The line in `w` × `h` with room left for the next line and the credit, in the pixel font at the song's scale (at
/// most `scale`; 0: plain text): in its usual rows if it can, else wrapped further at the same size.
fn plan(words: &[Word], w: u16, h: u16, scale: u16) -> Plan {
    // the song's own size, in its usual rows, else in more; a smaller one only if the song's size can't hold it
    let tries = BIG_SIZES.iter().filter(|(s, ..)| *s <= scale).flat_map(|&size| [(size, false), (size, true)]);
    if let Some(p) = tries.into_iter().find_map(|(size, stretch)| plan_big(words, w, h, size, stretch)) {
        return p;
    }
    let (spots, rows) = lay_out(words, Font::Plain, w);
    let lane = if rows * 3 + 3 <= h {
        2
    } else if rows * 2 < h {
        1
    } else {
        0
    };
    Plan { font: Font::Plain, lane, spots, rows, px: vec![], row_px: vec![] }
}

/// The lane over big text `pitch` cells tall.
fn px_lane(pitch: u16) -> u16 {
    (pitch * 3 / 5).clamp(2, 8)
}

/// The lines before and after the one on stage, beside big text of `px`: their size and the cells a row of them takes
/// (under 2: they stay terminal text).
fn side(px: f32, ch: f64) -> (f32, u16) {
    let sp = px * 0.42;
    (sp, ((sp * crate::bigtext::line()) as f64 / ch).ceil() as u16)
}

/// Rows kept free around big text of `px`: the line before above it and the next one under it, a row apart.
fn px_spare(px: f32, ch: f64) -> u16 {
    match side(px, ch) {
        (_, sp) if sp >= 2 => 2 * sp + 2,
        _ => 4,
    }
}

/// The smallest big text worth leaving the cells for, against a cell's height.
const PX_MIN: f64 = 1.25;

/// The song's big-text size for a `w`×`h` Pane (credit row not counted): its longer lines span the Pane, and a row of it
/// with its lane and the lines around it fits the height. None when it would hardly beat the cells, or the fonts
/// lack a character.
fn song_px(lines: &[Line], w: u16, h: u16, (cw, ch): (f64, f64)) -> Option<f32> {
    if lines.is_empty() || !lines.iter().all(|l| l.words.iter().all(|x| crate::bigtext::covers(&x.text))) {
        return None;
    }
    let texts: Vec<String> = lines.iter().map(|l| plain_text(&l.words)).collect();
    let typical = crate::bigtext::typical(texts.iter().map(String::as_str), 1.1).max(1.0);
    let by_width = (w as f64 * cw * 0.94 / typical as f64) as f32;
    // the tallest row that fits with its lane and the lines around it
    let mut by_height = 0.0f32;
    for pitch in 2..=h {
        let px = (pitch as f64 * ch / crate::bigtext::line() as f64 * 0.96) as f32;
        if pitch + px_lane(pitch) + px_spare(px, ch) > h {
            break;
        }
        by_height = px;
    }
    let px = by_width.min(by_height);
    (px as f64 >= PX_MIN * ch).then_some(px.floor())
}

/// Words in rows of at most `max` pixels at `px` (pieces sung as one word kept together): each word's row, its left
/// and width from its row's left, and each row's width.
fn lay_out_px(words: &[Word], px: f32, max: f32) -> (Vec<(u16, f32, f32)>, Vec<f32>) {
    use crate::bigtext::width;
    let space = width(" ", px, true);
    let mut out = Vec::with_capacity(words.len());
    let mut rows = vec![0.0f32];
    let mut k = 0;
    while k < words.len() {
        let mut end = k + 1;
        while end < words.len() && words[end].glued {
            end += 1;
        }
        let ws: Vec<f32> = words[k..end].iter().map(|w| width(&w.text, px, true)).collect();
        let gw: f32 = ws.iter().sum();
        let cur = *rows.last().unwrap();
        let mut x = if cur == 0.0 { 0.0 } else { cur + space };
        if cur > 0.0 && x + gw > max {
            rows.push(0.0);
            x = 0.0;
        }
        let row = rows.len() as u16 - 1;
        for w in ws {
            out.push((row, x, w));
            x += w;
        }
        *rows.last_mut().unwrap() = x;
        k = end;
    }
    (out, rows)
}

/// The line on stage in big text at the song's size `px` in `w` × `h` cells: in at most three rows, shrunk if it needs;
/// a line too long for that keeps the song's size in more rows, then shrinks on down to about the cells' own size, so
/// it stays in the song's font. None only when not even that fits.
fn plan_px(words: &[Word], w: u16, h: u16, cpx: (f64, f64), px: f32) -> Option<Plan> {
    if words.is_empty() {
        return None;
    }
    let ch = cpx.1;
    let tries = |most: u16, floor: f64| {
        let mut out = vec![];
        let mut p = px;
        while p as f64 >= floor * ch {
            out.push((p, most));
            p *= 0.9;
        }
        out
    };
    tries(3, PX_MIN).into_iter().chain(tries(u16::MAX, PX_FLOOR)).find_map(|(p, most)| plan_px_at(words, w, h, cpx, p, most))
}

/// The smallest big text a long line shrinks to before the song's font gives way to the cells', against a cell's
/// height.
const PX_FLOOR: f64 = 0.9;

/// The line in big text of `px` in `w` × `h` cells, if it fits in at most `most` rows.
fn plan_px_at(words: &[Word], w: u16, h: u16, cpx: (f64, f64), px: f32, most: u16) -> Option<Plan> {
    plan_px_with(words, w, h, cpx, px, most, px_spare(px, cpx.1))
}

/// `plan_px_at` keeping `spare` rows free.
#[allow(clippy::too_many_arguments)]
fn plan_px_with(words: &[Word], w: u16, h: u16, (cw, ch): (f64, f64), px: f32, most: u16, spare: u16) -> Option<Plan> {
    let max = (w as f64 * cw) as f32;
    let pitch = ((px * crate::bigtext::line()) as f64 / ch).ceil() as u16;
    let lane = px_lane(pitch);
    let (laid, rows) = lay_out_px(words, px, max);
    let n = rows.len() as u16;
    // a word wider than the Pane can't be drawn whole: smaller then
    if n > most || n * (lane + pitch) + spare > h || rows.iter().any(|&rw| rw > max) {
        return None;
    }
    // each row centred on whole columns, a little room for the last letter's overhang
    let row_px: Vec<(String, u16, u16)> = rows
        .iter()
        .enumerate()
        .map(|(ri, &rw)| {
            let cols = (((rw + px * 0.15) as f64 / cw).ceil() as u16).clamp(1, w);
            let mut text = String::new();
            for (_, word) in words.iter().enumerate().filter(|(i, _)| laid[*i].0 == ri as u16) {
                if !text.is_empty() && !word.glued {
                    text.push(' ');
                }
                text.push_str(&word.text);
            }
            (text, (w - cols) / 2, cols)
        })
        .collect();
    let mut spots = Vec::with_capacity(words.len());
    let mut pxs = Vec::with_capacity(words.len());
    for &(row, x, ww) in &laid {
        let left = row_px[row as usize].1 as f32 * cw as f32 + x;
        pxs.push((left, ww));
        spots.push(Spot { row, x: (left as f64 / cw) as u16, w: ((ww as f64 / cw).ceil() as u16).max(1) });
    }
    let dot = ((px as f64 * 0.6 / cw).round() as u16).max(2);
    Some(Plan { font: Font::Px { px, pitch, dot }, lane, spots, rows: n, px: pxs, row_px })
}

/// A Pane of `w` × `h` cells that is taller than wide on screen (a cell is about twice as tall as wide) and too
/// narrow for a line in one row (#54): the song runs above and below the line on stage like a lyric sheet.
fn narrow_tall(w: u16, h: u16) -> bool {
    (16..40).contains(&w) && h >= 12 && h * 2 > w
}

/// A narrow, tall Pane's big-text size for the song (`w` × `h` cells of `cpx` pixels): a little over the cells' own
/// size, its longest word no wider than the Pane. None when that would hardly beat the cells, or the fonts lack a
/// character.
fn narrow_px(lines: &[Line], w: u16, (cw, ch): (f64, f64)) -> Option<f32> {
    if lines.is_empty() || !lines.iter().all(|l| l.words.iter().all(|x| crate::bigtext::covers(&x.text))) {
        return None;
    }
    let widest = lines.iter().flat_map(|l| l.words.iter()).map(|x| crate::bigtext::width(x.text.trim(), 1.0, true)).fold(0.0f32, f32::max).max(0.1);
    let px = ((w as f64 * cw * 0.94 / widest as f64) as f32).min((ch * NARROW_PX) as f32);
    (px as f64 >= PX_MIN * ch).then_some(px.floor())
}

/// Big text's size in a narrow, tall Pane, against a cell's height: the line on stage a little bigger than the cells,
/// leaving most of the Pane to the lines around it.
const NARROW_PX: f64 = 1.35;

/// The line on stage in a narrow, tall Pane: big text at `px` when there is some (shrunk if the line needs), else
/// the cells' as in any Pane.
fn narrow_plan(words: &[Word], w: u16, h: u16, big: Option<((f64, f64), f32)>, scale: u16) -> Plan {
    if let Some((cpx, px)) = big {
        let mut p = px;
        while p as f64 >= PX_FLOOR * cpx.1 {
            if let Some(plan) = plan_px_with(words, w, h, cpx, p, u16::MAX, 4) {
                return plan;
            }
            p *= 0.9;
        }
    }
    plan(words, w, h, scale)
}

/// The lines around the one on stage in a narrow, tall Pane, each wrapped whole and a row apart, fading the further
/// they are: upwards from `top` and downwards from `below` to `bottom`, running on past the Pane's edges like a sheet
/// (the rows that fit of a line cut there).
fn narrow_context(buf: &mut Buffer, r: Rect, lines: &[Line], i: usize, (top, below, bottom): (u16, u16, u16), th: &Theme, hits: &mut Vec<(Rect, Hit)>) {
    let w = r.width.saturating_sub(2) as usize;
    let rows_of = |k: usize| crate::lyrics::wrap_indented(&plain_text(&lines[k].words), w, 0);
    let fade = |d: usize| match d {
        0 => th.dim,
        1 => th.dim.lerp(th.border, 0.5),
        _ => th.border,
    };
    let (ry0, bottom) = (r.y as i32, bottom as i32);
    let mut y = top as i32;
    for (d, k) in (0..i).rev().enumerate() {
        let rows = rows_of(k);
        let y0 = y - 1 - rows.len() as i32;
        if y - 1 <= ry0 {
            break;
        }
        for (j, row) in rows.iter().enumerate() {
            if y0 + j as i32 >= ry0 {
                centre(buf, r, (y0 + j as i32) as u16, row.trim(), th.s(fade(d + 1)));
            }
        }
        let from = y0.max(ry0) as u16;
        hits.push((Rect::new(r.x, from, r.width, (y - 1) as u16 - from), Hit::Lyric(lines[k].start)));
        y = y0;
    }
    let mut y = below as i32 + 1;
    for (d, k) in (i + 1..lines.len()).enumerate() {
        if y >= bottom {
            break;
        }
        let rows = rows_of(k);
        for (j, row) in rows.iter().enumerate() {
            if y + (j as i32) < bottom {
                centre(buf, r, (y + j as i32) as u16, row.trim(), th.s(fade(d)));
            }
        }
        let n = rows.len() as i32;
        hits.push((Rect::new(r.x, y as u16, r.width, ((y + n).min(bottom) - y) as u16), Hit::Lyric(lines[k].start)));
        y += n + 1;
    }
}

/// `s` cut to `w` columns, with an ellipsis when cut.
fn fit(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.width().unwrap_or(0) + 1 > w {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

fn centre(buf: &mut Buffer, r: Rect, y: u16, s: &str, st: ratatui::style::Style) {
    if y < r.y || y >= r.bottom() {
        return;
    }
    let s = fit(s, r.width as usize);
    let n = s.width() as u16;
    text(buf, r.x + r.width.saturating_sub(n) / 2, y, r.width, &s, st);
}

/// The line's text as one string (pieces glued where they're sung as one word).
fn plain_text(words: &[Word]) -> String {
    let mut s = String::new();
    for w in words {
        if !s.is_empty() && !w.glued {
            s.push(' ');
        }
        s.push_str(&w.text);
    }
    s
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.width < 4 || r.height == 0 {
        return;
    }
    let mid = r.y + r.height.saturating_sub(1) / 2;
    let Some(item) = app.playback.item.as_ref() else {
        return centre(buf, r, mid, "nothing playing · play a song to sing along", th.s(th.dim));
    };
    let Some(lyr) = app.lyrics.for_song(&item.uri) else {
        return centre(buf, r, mid, "looking for lyrics…", th.s(th.dim));
    };
    if lyr.lines.is_empty() {
        let why = if lyr.missing.is_empty() { "no lyrics for this one" } else { lyr.missing.as_str() };
        if r.height >= 3 {
            centre(buf, r, mid.saturating_sub(1), "♪", th.s(th.border));
        }
        centre(buf, r, mid, "nothing to sing along to", th.s(th.fg));
        return centre(buf, r, mid + 1, why, th.s(th.dim));
    }
    let pos = app.lyrics_position(singularity_proto::now_ms());
    if !lyr.synced {
        return unsynced(buf, r, app, pos, th);
    }
    let duration = item.duration_ms;
    let playing = app.playback.status == singularity_proto::PlayStatus::Playing;
    let mut hits = vec![];
    // the song's timed lines, and its sizes for this Pane (big text on the pixel tier, the cells' pixel font)
    let cpx = app.pixel_text_for(r);
    let Some(sung) = app.lyrics.sung(duration, r.width, r.height, cpx) else { return };
    let big = match (cpx.zip(sung.px), app.kitty.as_mut()) {
        (Some((cpx, px)), Some(kitty)) => Some(Big { cpx, px, kitty }),
        _ => None,
    };
    let ball = synced(buf, r, sung.lines, pos, &sung.lyrics.source, th, &mut hits, big, sung.scale);
    // the pixel tier's ball, or the cells' `●`
    let mut smooth = false;
    if let Some(b) = ball {
        smooth = playing && (b.height > 0.0 || b.moving);
        match app.pixel_tier_for(r) {
            Some(px) if b.lane >= 2 => pixel_ball(app, r, b, px, th),
            _ => cell_ball(buf, r, b, th),
        }
    }
    if smooth && app.pixel_tier_for(r).is_some() {
        app.lyrics.ball_moving();
    }
    for (rect, h) in hits {
        app.hit(rect, h);
    }
}

/// Big text for the line on stage: the cell size, the song's size and where the images go.
struct Big<'a> {
    cpx: (f64, f64),
    px: f32,
    kitty: &'a mut crate::gfx::Kitty,
}

/// Where to draw the ball: a column and the bottom row of its lane (fractions: between words, between rows), how
/// high in the lane (0..1), and the lane's height; over big text, its radius in pixels too.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ball {
    x: f32,
    base: f32,
    height: f32,
    lane: u16,
    rad: Option<f64>,
    /// gliding along the line (not waiting before it or resting after it)
    moving: bool,
}

fn cell_ball(buf: &mut Buffer, r: Rect, b: Ball, th: &Theme) {
    let x = b.x.round() as i32;
    let y = (b.base - b.height * (b.lane.saturating_sub(1)) as f32).round() as i32;
    if x >= r.x as i32 && x < r.right() as i32 && y >= r.y as i32 && y < r.bottom() as i32 {
        text(buf, x as u16, y as u16, 1, "●", th.s(th.accent2).add_modifier(Modifier::BOLD));
    }
}

/// The ball as a small image of its own, placed to the pixel: smooth hops at the motion frame rate.
fn pixel_ball(app: &mut App, r: Rect, b: Ball, (cw, ch): (f64, f64), th: &Theme) {
    // bigger over the pixel font's taller lanes
    let rad = b.rad.unwrap_or(ch * [0.28, 0.28, 0.28, 0.36, 0.42][b.lane.min(4) as usize]).max(2.0);
    // its centre in pixels from the screen's corner: resting, it sits on the bottom of the lane's last row
    let cx = (b.x as f64 + 0.5) * cw;
    let rest = (b.base as f64 + 1.0) * ch - rad - ch * 0.1;
    let cy = rest - b.height as f64 * (b.lane.saturating_sub(1) as f64 * ch);
    // the cells it covers, kept inside the Pane
    let c0 = (((cx - rad) / cw).floor().max(r.x as f64)) as u16;
    let c1 = (((cx + rad) / cw).ceil().min(r.right() as f64)) as u16;
    let r0 = (((cy - rad) / ch).floor().max(r.y as f64)) as u16;
    let r1 = (((cy + rad) / ch).ceil().min(r.bottom() as f64)) as u16;
    if c1 <= c0 || r1 <= r0 {
        return;
    }
    let cell = Rect::new(c0, r0, c1 - c0, r1 - r0);
    let (w, h) = ((cell.width as f64 * cw).round() as u32, (cell.height as f64 * ch).round() as u32);
    let (ox, oy) = (cx - c0 as f64 * cw, cy - r0 as f64 * ch);
    let col = th.accent2;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for py in 0..h {
        for px in 0..w {
            // a soft edge a pixel wide, and a highlight up and left
            let (dx, dy) = (px as f64 + 0.5 - ox, py as f64 + 0.5 - oy);
            let d = (dx * dx + dy * dy).sqrt();
            let a = (rad + 0.5 - d).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let shine = (1.0 - ((dx + rad * 0.35).powi(2) + (dy + rad * 0.35).powi(2)).sqrt() / rad).clamp(0.0, 1.0) * 0.55;
            let c = col.lerp(Rgb(255, 255, 255), shine as f32);
            let i = ((py * w + px) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[c.0, c.1, c.2, (a * 255.0) as u8]);
        }
    }
    if let Some(k) = app.kitty.as_mut() {
        k.frame(BALL_ID, cell, w, h, rgba.into());
    }
}

/// Unsynced lyrics: shown whole and centred, no ball; long ones move along with the song (they have no times).
fn unsynced(buf: &mut Buffer, r: Rect, app: &App, pos: u64, th: &Theme) {
    let Some(lyr) = app.lyrics.current() else { return };
    let body = r.height.saturating_sub(1);
    let rows: Vec<String> = lyr.lines.iter().flat_map(|l| crate::lyrics::wrap(if l.text.is_empty() { "♪" } else { &l.text }, r.width.saturating_sub(2) as usize)).collect();
    let spare = rows.len().saturating_sub(body as usize);
    let dur = app.duration_ms().max(1);
    let first = (spare as f64 * (pos as f64 / dur as f64).clamp(0.0, 1.0)) as usize;
    let first = app.lyrics.karaoke().follow.first(first, spare);
    for (k, row) in rows.iter().skip(first).take(body as usize).enumerate() {
        centre(buf, r, r.y + k as u16, row.trim(), th.s(th.fg));
    }
    if r.height >= 2 {
        centre(buf, r, r.bottom() - 1, &format!("{} · not time-synced: no ball", lyr.source), th.s(th.border));
    }
}

/// The synced view; returns where the ball goes (if anywhere). Seek targets go to `hits`.
#[allow(clippy::too_many_arguments)]
fn synced(buf: &mut Buffer, r: Rect, lines: &[Line], pos: u64, source: &str, th: &Theme, hits: &mut Vec<(Rect, Hit)>, mut big: Option<Big>, scale: u16) -> Option<Ball> {
    let stage = timing::at(lines, pos);
    // one or two rows: the line alone, scrolled to the word being sung
    if r.height <= 2 || r.width < 16 {
        tiny(buf, r, lines, stage, pos, th);
        return None;
    }
    let (i, count) = match stage {
        Stage::Line { i, count } => (i, count),
        Stage::Gap { next, in_ms } => {
            gap(buf, r, lines, next, in_ms, pos, th, hits);
            return None;
        }
    };
    let line = &lines[i];
    let credit = r.height >= 7;
    let w = r.width.saturating_sub(2);
    let tall = narrow_tall(r.width, r.height);
    let p = if tall {
        narrow_plan(&line.words, w, r.height - credit as u16, big.as_ref().map(|b| (b.cpx, b.px)), scale)
    } else {
        big.as_ref().and_then(|b| plan_px(&line.words, w, r.height - credit as u16, b.cpx, b.px)).unwrap_or_else(|| plan(&line.words, w, r.height - credit as u16, scale))
    };
    // big text: the lines around it in smaller big text when that is still bigger than the cells
    let side = match (p.font, big.as_ref()) {
        (Font::Px { px, .. }, Some(b)) => Some(side(px, b.cpx.1)).filter(|(_, sp)| *sp >= 2),
        _ => None,
    };
    let block = p.height().min(r.height);
    // the line a little above the middle; the next lines under it, the last one above when there's room
    let avail = r.height - credit as u16;
    let top = r.y + avail.saturating_sub(block) * 2 / 5;
    let x0 = r.x + 1;
    let row_top = |row: u16| top + row * (p.lane + p.font.rows()) + p.lane;
    match (p.font, big.as_mut()) {
        (Font::Px { px, pitch, .. }, Some(b)) => px_line(b.kitty, r, x0, &p, line, pos, &row_top, th, b.cpx, px, pitch),
        _ => draw_line(buf, r, x0, &p, line, pos, &row_top, th),
    }
    hits.push((Rect::new(r.x, top, r.width, block), Hit::Lyric(line.start)));
    let bottom = r.bottom() - credit as u16;
    if tall {
        narrow_context(buf, r, lines, i, (top, top + block, bottom), th, hits);
    } else {
        // the rows a line around it takes: big text's own, or one of the cells
        let side_rows = side.map_or(1, |(_, sp)| sp);
        let mut aside = |y: u16, s: &str, c: Rgb, big: &mut Option<Big>| match (side, big.as_mut()) {
            (Some((sp, rows)), Some(b)) => side_line(b.kitty, r, y, s, c, sp, rows, b.cpx),
            _ => centre(buf, r, y, s, th.s(c)),
        };
        // the last line, fading, above
        if i > 0 && top > r.y + side_rows {
            let y = top - side_rows - 1;
            aside(y, &plain_text(&lines[i - 1].words), th.border, &mut big);
            hits.push((Rect::new(r.x, y, r.width, side_rows), Hit::Lyric(lines[i - 1].start)));
        }
        // the next ones below, dimmer the further they are
        let mut y = top + block + 1;
        for (k, nx) in lines.iter().enumerate().skip(i + 1).take(3) {
            if y + side_rows > bottom {
                break;
            }
            let c = if k == i + 1 { th.dim } else { th.border };
            aside(y, &plain_text(&nx.words), c, &mut big);
            hits.push((Rect::new(r.x, y, r.width, side_rows), Hit::Lyric(nx.start)));
            y += match p.font {
                Font::Plain => 1,
                Font::Big(_) => 2,
                Font::Px { .. } if side.is_some() => side_rows,
                Font::Px { .. } => 2,
            };
        }
    }
    if credit {
        let hint = if line.exact { "" } else { " · word timing estimated" };
        let credits = [format!("lyrics: {source}{hint}"), format!("{source}{hint}"), source.to_string()];
        let c = credits.iter().find(|c| c.width() < r.width as usize).unwrap_or(&credits[2]);
        let cw = c.width() as u16;
        text(buf, r.right().saturating_sub(cw + 1), r.bottom() - 1, cw, c, th.s(th.border));
    }
    if p.lane == 0 || line.words.is_empty() {
        return None;
    }
    // the count-in: a dot a second over the first word, in the ball's colour, spaced like the line's letters
    if count > 0 {
        let s0 = p.spots[0];
        let y = row_top(s0.row) - 1;
        let gap = p.font.char_w().max(2);
        for d in 0..count as u16 {
            let x = x0 + s0.x + d * gap;
            if x < r.right() {
                text(buf, x, y, 1, "●", th.s(th.accent2));
            }
        }
    }
    let (wx, h) = timing::ball(line, pos, timing::on_stage(lines, i));
    // during the count-in the ball waits out of sight; it shows for its hop onto the first word
    if count > 1 || (count == 1 && wx <= -1.0) {
        return None;
    }
    let cw = big.as_ref().map_or(1.0, |b| b.cpx.0) as f32;
    let n = line.words.len();
    // where the ball touches down on word `k`: over its first letter (`end`: over its last), in columns (a column's
    // middle is its index + 0.5)
    let touch = |k: usize, end: bool| -> f32 {
        match p.font {
            Font::Px { px, .. } => {
                let (left, w) = p.px[k];
                let half = (px * 0.3).min(w / 2.0);
                x0 as f32 + (if end { left + w - half } else { left + half }) / cw - 0.5
            }
            Font::Big(sc) => {
                let s = p.spots[k];
                let half = (3 * sc) as f32 / 2.0 - 0.5;
                let x = (x0 + s.x) as f32;
                if end { x + s.w.saturating_sub(sc) as f32 - 1.0 - half } else { x + half }
            }
            Font::Plain => (x0 + p.spots[k].x) as f32 + if end { p.spots[k].w.saturating_sub(1) as f32 } else { 0.0 },
        }
    };
    let base_of = |k: usize| (row_top(p.spots[k].row) - 1) as f32;
    let (x, base) = if wx < 0.0 {
        let from = (touch(0, false) - 3.0 * p.font.char_w().min(4) as f32).max(r.x as f32);
        let t = wx + 1.0;
        (from + (touch(0, false) - from) * t, base_of(0))
    } else {
        let a = (wx.floor() as usize).min(n - 1);
        let t = wx - a as f32;
        let lerp = |p: f32, q: f32, t: f32| p + (q - p) * t;
        if a + 1 >= n {
            // along the last word as it fills
            (lerp(touch(a, false), touch(a, true), t), base_of(a))
        } else if p.spots[a + 1].row == p.spots[a].row {
            (lerp(touch(a, false), touch(a + 1, false), t), base_of(a))
        } else {
            // the next word starts a row further down: along this one to its end, then the hop down to it
            let span = line.words[a + 1].start.saturating_sub(line.words[a].start).max(1);
            let up = 1.0 - timing::HOP_MS.min(span) as f32 / span as f32;
            if up > 0.0 && t < up {
                (lerp(touch(a, false), touch(a, true), t / up), base_of(a))
            } else {
                let u = if up > 0.0 { (t - up) / (1.0 - up) } else { t };
                let from = if up > 0.0 { touch(a, true) } else { touch(a, false) };
                (lerp(from, touch(a + 1, false), u), lerp(base_of(a), base_of(a + 1), u))
            }
        }
    };
    let rad = match p.font {
        Font::Px { px, .. } => Some((px * 0.16) as f64),
        _ => None,
    };
    Some(Ball { x, base, height: h, lane: p.lane, rad, moving: wx > -1.0 && wx < n as f32 })
}

/// The line on stage in big text: per row, the sung part (accent) and the rest (waiting), two images of the same
/// row split at the column the fill has reached.
#[allow(clippy::too_many_arguments)]
fn px_line(kitty: &mut crate::gfx::Kitty, r: Rect, x0: u16, p: &Plan, line: &Line, pos: u64, row_top: &dyn Fn(u16) -> u16, th: &Theme, (cw, ch): (f64, f64), px: f32, pitch: u16) {
    use crate::bigtext::{Span, key, render};
    let img_h = (pitch as f64 * ch).round() as u32;
    let baseline = ((img_h as f32 - px * crate::bigtext::line()) / 2.0 + px * crate::bigtext::ascent()).round();
    for (ri, (text, col, cols)) in p.row_px.iter().enumerate() {
        let y = row_top(ri as u16);
        if y + pitch > r.bottom() {
            break;
        }
        let left = *col as f32 * cw as f32;
        // how far the fill has got along this row, in pixels from its left
        let mut fill = 0.0f32;
        for (w, &(wx, ww)) in line.words.iter().zip(&p.px).zip(&p.spots).filter(|(_, s)| s.row == ri as u16).map(|(a, _)| a) {
            let f = timing::fill(w, pos);
            if f > 0.0 {
                fill = fill.max(wx - left + f * ww);
            }
        }
        let split = ((fill as f64 / cw).round() as u16).min(*cols);
        let img_w = (*cols as f64 * cw).round() as u32;
        let cut = ((split as f64 * cw).round() as u32).min(img_w);
        let x = x0 + col;
        for (sung, from, to) in [(true, 0u16, split), (false, split, *cols)] {
            if to <= from {
                continue;
            }
            let c = if sung { th.accent } else { th.fg };
            let spans = [Span { x: 0.0, text, rgb: c, px, bold: true }];
            let k = key(&spans, img_w, img_h, baseline);
            let (a, b) = if sung { (0, cut) } else { (cut, img_w) };
            kitty.place_rgba(k, Rect::new(x + from, y, to - from, pitch), Some([a, 0, b - a, img_h]), || (img_w, img_h, render(&spans, img_w, img_h, baseline)));
        }
    }
}

/// A line beside the one on stage in big text of `px` (`rows` cells tall), centred, cut with an ellipsis when too
/// wide.
#[allow(clippy::too_many_arguments)]
fn side_line(kitty: &mut crate::gfx::Kitty, r: Rect, y: u16, s: &str, c: Rgb, px: f32, rows: u16, (cw, ch): (f64, f64)) {
    use crate::bigtext::{Span, key, render, width};
    if y + rows > r.bottom() || r.width < 3 {
        return;
    }
    let max = ((r.width - 2) as f64 * cw) as f32;
    let mut text = s.trim().to_string();
    if width(&text, px, false) > max {
        let room = max - width("…", px, false);
        let mut cut = String::new();
        let mut w = 0.0;
        for ch in text.chars() {
            let a = crate::bigtext::advance(ch, px, false);
            if w + a > room {
                break;
            }
            w += a;
            cut.push(ch);
        }
        text = format!("{}…", cut.trim_end());
    }
    let cols = (((width(&text, px, false) + px * 0.15) as f64 / cw).ceil() as u16).clamp(1, r.width);
    let img_w = (cols as f64 * cw).round() as u32;
    let img_h = (rows as f64 * ch).round() as u32;
    let baseline = ((img_h as f32 - px * crate::bigtext::line()) / 2.0 + px * crate::bigtext::ascent()).round();
    let spans = [Span { x: 0.0, text: &text, rgb: c, px, bold: false }];
    let k = key(&spans, img_w, img_h, baseline);
    kitty.place_rgba(k, Rect::new(r.x + (r.width - cols) / 2, y, cols, rows), None, || (img_w, img_h, render(&spans, img_w, img_h, baseline)));
}

/// The line on stage, its words filling as they are sung.
#[allow(clippy::too_many_arguments)]
fn draw_line(buf: &mut Buffer, r: Rect, x0: u16, p: &Plan, line: &Line, pos: u64, row_top: &dyn Fn(u16) -> u16, th: &Theme) {
    let (sung, waiting) = (th.accent, th.fg);
    for (w, s) in line.words.iter().zip(&p.spots) {
        let y = row_top(s.row);
        if y >= r.bottom() {
            continue;
        }
        let f = timing::fill(w, pos);
        match p.font {
            Font::Plain => {
                // by grapheme cluster: a mark (Thai, Hindi, Vietnamese) goes into its base's cell (#146)
                let n = w.text.graphemes(true).count().max(1) as f32;
                let mut x = x0 + s.x;
                for (j, g) in w.text.graphemes(true).enumerate() {
                    let a = (f * n - j as f32).clamp(0.0, 1.0);
                    let mut st = th.s(waiting.lerp(sung, a));
                    if a >= 1.0 {
                        st = st.add_modifier(Modifier::BOLD);
                    }
                    let cw = g.width() as u16;
                    if x + cw > r.right() {
                        break;
                    }
                    text(buf, x, y, cw.max(1), g, st);
                    x += cw;
                }
            }
            Font::Big(sc) => big_word(buf, r, x0 + s.x, y, &w.text, sc, f, waiting, sung),
            // drawn as images (`px_line`)
            Font::Px { .. } => {}
        }
    }
}

/// A word in the pixel font at scale `sc`, filled to `f` from the left.
#[allow(clippy::too_many_arguments)]
fn big_word(buf: &mut Buffer, r: Rect, x: u16, y: u16, word: &str, sc: u16, f: f32, waiting: Rgb, sung: Rgb) {
    let chars: Vec<char> = word.chars().filter_map(big_char).collect();
    let cols = chars.len() as u16 * 4 * sc;
    let lit_cols = f * cols.saturating_sub(sc) as f32;
    let rows = Font::Big(sc).rows();
    let on = |col: u16, half: u16| -> bool {
        let (ci, cx) = ((col / (4 * sc)) as usize, (col % (4 * sc)) / sc);
        let py = (half / sc) as usize;
        cx < 3 && py < 5 && chars.get(ci).is_some_and(|&c| crate::ambient::glyph(c)[py].as_bytes()[cx as usize] == b'#')
    };
    for col in 0..cols {
        let cx = x + col;
        if cx >= r.right() {
            break;
        }
        let a = (lit_cols - col as f32).clamp(0.0, 1.0);
        let c = waiting.lerp(sung, a).color();
        for row in 0..rows {
            let cy = y + row;
            if cy >= r.bottom() {
                break;
            }
            let ch = match (on(col, row * 2), on(col, row * 2 + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                _ => continue,
            };
            if let Some(cell) = buf.cell_mut((cx, cy)) {
                cell.set_char(ch).set_fg(c);
            }
        }
    }
}

/// An instrumental (or the intro, or the end): what comes next and when, the notes drifting.
#[allow(clippy::too_many_arguments)]
fn gap(buf: &mut Buffer, r: Rect, lines: &[Line], next: Option<usize>, in_ms: u64, pos: u64, th: &Theme, hits: &mut Vec<(Rect, Hit)>) {
    let secs = in_ms.div_ceil(1000);
    let fig = format!("{secs}s");
    let fw = crate::tile::figure_width(&fig);
    // notes, a blank, the seconds (a big figure and a caption when there's room), a blank, the next line
    let big = r.height >= 10 && fw + 2 <= r.width && next.is_some();
    let block = if big { 8 } else { 5 };
    let mid = r.y + r.height.saturating_sub(block) * 2 / 5;
    // three notes, the lit one stepping along twice a second
    let step = (pos / 500 % 3) as usize;
    let notes: String = (0..3).map(|k| if k == step { "♪" } else { "·" }).collect::<Vec<_>>().join("   ");
    centre(buf, r, mid, &notes, th.s(th.accent));
    let Some(n) = next else {
        return centre(buf, r, mid + 2, "that's the last line", th.s(th.dim));
    };
    let mut y = mid + 2;
    if big {
        crate::tile::figure(buf, r.x + (r.width - fw) / 2, y, fw + 1, &fig, th.s(th.fg).add_modifier(Modifier::BOLD), th.s(th.dim));
        y += 4;
        centre(buf, r, y, "until the next line", th.s(th.dim));
    } else {
        centre(buf, r, y, &format!("next line in {secs} s"), th.s(th.dim));
    }
    y += 2;
    if y < r.bottom() {
        centre(buf, r, y, &plain_text(&lines[n].words), th.s(th.fg));
        hits.push((Rect::new(r.x, y, r.width, 1), Hit::Lyric(lines[n].start)));
    }
}

/// One or two rows: the line on stage (scrolled so the word being sung shows), the next under it.
fn tiny(buf: &mut Buffer, r: Rect, lines: &[Line], stage: Stage, pos: u64, th: &Theme) {
    let i = match stage {
        Stage::Line { i, .. } => i,
        Stage::Gap { next: Some(n), in_ms } => {
            let s = format!("♪ {} s · ", in_ms.div_ceil(1000));
            text(buf, r.x, r.y, r.width, &s, th.s(th.accent));
            let sw = s.width() as u16;
            text(buf, r.x + sw.min(r.width), r.y, r.width.saturating_sub(sw), &fit(&plain_text(&lines[n].words), r.width.saturating_sub(sw) as usize), th.s(th.dim));
            return;
        }
        Stage::Gap { next: None, .. } => return centre(buf, r, r.y, "♪", th.s(th.accent)),
    };
    let line = &lines[i];
    // the whole line as one row of grapheme clusters (a mark stays with its base, #146), each with its fill
    let mut cells: Vec<(&str, f32)> = vec![];
    // the column the word being sung starts at
    let mut now = 0usize;
    let mut width = 0usize;
    for (k, w) in line.words.iter().enumerate() {
        if k > 0 && !w.glued {
            cells.push((" ", if w.start <= pos { 1.0 } else { 0.0 }));
            width += 1;
        }
        let f = timing::fill(w, pos);
        let n = w.text.graphemes(true).count().max(1) as f32;
        if w.start <= pos {
            now = width;
        }
        for (j, g) in w.text.graphemes(true).enumerate() {
            cells.push((g, (f * n - j as f32).clamp(0.0, 1.0)));
            width += g.width();
        }
    }
    let w = r.width as usize;
    // scrolled (in columns: a CJK character takes two) so the word being sung sits a third of the way in
    let skip = if width <= w { 0 } else { now.saturating_sub(w / 3).min(width - w) };
    let mut x = r.x + if width <= w { ((w - width) / 2) as u16 } else { 0 };
    let mut col = 0usize;
    for (g, a) in &cells {
        let cw = g.width();
        col += cw;
        // whole clusters only: one cut by the left edge is skipped too
        if col - cw < skip {
            continue;
        }
        let cw = cw as u16;
        if x + cw > r.right() {
            break;
        }
        let mut st = th.s(th.fg.lerp(th.accent, *a));
        if *a >= 1.0 {
            st = st.add_modifier(Modifier::BOLD);
        }
        text(buf, x, r.y, cw.max(1), g, st);
        x += cw;
    }
    if r.height >= 2
        && let Some(nx) = lines.get(i + 1)
    {
        centre(buf, r, r.y + 1, &plain_text(&nx.words), th.s(th.dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::{LyricLine, LyricWord, Lyrics};

    fn words(s: &str) -> Vec<Word> {
        s.split(' ').enumerate().map(|(i, t)| Word { text: t.into(), start: i as u64 * 500, end: i as u64 * 500 + 400, glued: false }).collect()
    }

    #[test]
    fn a_line_lays_out_centred_and_wraps_whole_words() {
        let w = words("turn the lights down");
        let (s, rows) = lay_out(&w, Font::Plain, 40);
        assert_eq!(rows, 1);
        // 20 columns centred in 40
        assert_eq!((s[0].x, s[3].x + s[3].w), (10, 30));
        let (s, rows) = lay_out(&w, Font::Plain, 11);
        assert_eq!(rows, 2);
        assert!(s.iter().all(|p| p.x + p.w <= 11), "{s:?}");
        // glued pieces stay on one row
        let mut g = words("beau ti ful day");
        g[1].glued = true;
        g[2].glued = true;
        let (s, _) = lay_out(&g, Font::Plain, 9);
        assert_eq!((s[0].row, s[1].row, s[2].row, s[3].row), (0, 0, 0, 1));
        assert_eq!((s[1].x, s[2].x), (s[0].x + 4, s[0].x + 6), "no space inside a word");
    }

    #[test]
    fn the_plan_grows_with_the_pane() {
        let w = words("turn the lights down");
        assert_eq!(plan(&w, 30, 2, 2).lane, 0, "two rows: no room for the ball");
        assert_eq!((plan(&w, 30, 4, 2).font, plan(&w, 30, 4, 2).lane), (Font::Plain, 1));
        assert_eq!(plan(&w, 30, 8, 2).lane, 2);
        assert_eq!(plan(&w, 100, 14, 2).font, Font::Big(1));
        assert_eq!(plan(&w, 200, 20, 2).font, Font::Big(2));
        // a script the pixel font doesn't have stays plain
        let cjk = words("光が 曲がる");
        assert_eq!(plan(&cjk, 200, 40, 2).font, Font::Plain);
        // accents map to the plain letters
        assert_eq!(plan(&words("żółw tańczy"), 200, 40, 2).font, Font::Big(2));
    }

    /// An App playing a song whose lyrics have both timed and untimed lines, at `pos`.
    #[test]
    fn a_long_line_keeps_the_songs_font() {
        let (short, long) = (words("Turn the lights down"), words("Here comes the chorus hold on to the note and let the whole room carry it"));
        let line = |w: &[Word]| Line { start: 0, end: 9_000, words: w.to_vec(), exact: true, index: 0 };
        let song = [line(&short), line(&long)];
        // the cells: one scale for the whole song, the long line wrapped further at it
        for (w, h) in [(78u16, 20u16), (78, 30), (120, 30), (158, 40)] {
            let sc = song_scale(&song, w, h);
            let (a, b) = (plan(&short, w, h, sc), plan(&long, w, h, sc));
            assert_eq!(a.font, b.font, "{w}×{h}: {:?} vs {:?}", a.font, b.font);
            assert!(b.spots.iter().all(|s| s.x + s.w <= w + 2) && b.height() <= h, "{w}×{h}: {b:?}");
        }
        // big text: the long line stays big text in a Pane its words are too many for in three rows
        let cpx = (10.0, 20.0);
        let px = 60.0;
        assert!(plan_px(&short, 100, 28, cpx, px).is_some());
        let p = plan_px(&long, 100, 28, cpx, px).expect("a long line stays big text");
        assert!(matches!(p.font, Font::Px { .. }) && p.row_px.iter().all(|(_, c, w)| c + w <= 100), "{p:?}");
    }

    fn app_at(pos: u64, synced: bool) -> App {
        let mut app = App::new(None);
        app.playback.item = Some(singularity_proto::Item { uri: "spotify:track:k".into(), name: "Song".into(), duration_ms: 120_000, ..Default::default() });
        app.playback.status = singularity_proto::PlayStatus::Paused;
        app.playback.position_ms = pos;
        let timed = vec![
            LyricWord { start_ms: 3_000, end_ms: None, text: "Turn ".into() },
            LyricWord { start_ms: 3_500, end_ms: None, text: "the ".into() },
            LyricWord { start_ms: 4_000, end_ms: None, text: "lights ".into() },
            LyricWord { start_ms: 4_600, end_ms: Some(5_400), text: "down".into() },
        ];
        let lines = vec![
            LyricLine { start_ms: 3_000, text: "Turn the lights down".into(), words: timed },
            LyricLine { start_ms: 6_500, text: "Sing it like the night is young and the room is wide awake tonight".into(), ..Default::default() },
            LyricLine { start_ms: 12_000, text: "".into(), ..Default::default() },
            LyricLine { start_ms: 30_000, text: "Ünïcödé and 光が曲がる".into(), ..Default::default() },
        ];
        app.lyrics_event(Lyrics { uri: "spotify:track:k".into(), synced, lines, source: "fake".into(), ..Default::default() });
        app
    }

    /// The song's fourth line in letters every font has, arriving as new lyrics.
    fn ever_after(app: &mut App) {
        let mut l = app.lyrics.current().unwrap().clone();
        l.lines[3].text = "Ever after".into();
        app.lyrics_event(l);
    }

    /// An App paused at `pos` in a song of one timed line: `words` a second apart from 1 s.
    fn app_singing(words: &[&str], pos: u64) -> App {
        let mut app = App::new(None);
        app.playback.item = Some(singularity_proto::Item { uri: "spotify:track:m".into(), name: "Song".into(), duration_ms: 60_000, ..Default::default() });
        app.playback.status = singularity_proto::PlayStatus::Paused;
        app.playback.position_ms = pos;
        let timed = words.iter().enumerate().map(|(k, w)| LyricWord { start_ms: 1_000 + k as u64 * 1_000, end_ms: None, text: (*w).into() }).collect();
        let lines = vec![LyricLine { start_ms: 1_000, text: words.concat(), words: timed }];
        app.lyrics_event(Lyrics { uri: "spotify:track:m".into(), synced: true, lines, source: "fake".into(), ..Default::default() });
        app
    }

    /// The cells of `app` drawn into a `w`×`h` Pane at (0, 0), row by row; the cell a wide character covers is "".
    fn cells(app: &mut App, w: u16, h: u16) -> Vec<Vec<String>> {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        draw(&mut buf, Rect::new(0, 0, w, h), app, &Theme::singularity());
        (0..h)
            .map(|y| {
                let mut covered = 0;
                (0..w)
                    .map(|x| {
                        if covered > 0 {
                            covered -= 1;
                            return String::new();
                        }
                        let s = buf[(x, y)].symbol();
                        covered = s.width().saturating_sub(1);
                        s.to_string()
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn marks_stay_with_their_letters() {
        // Thai and Hindi with vowel marks that are characters of their own, in a normal and a tiny Pane
        for (line, marks) in [(["สวัสดี ", "ครับ"], ['\u{e31}', '\u{e35}']), (["नमस्ते ", "दुनिया"], ['\u{94d}', '\u{947}'])] {
            let whole: String = line.concat();
            for (w, h) in [(40u16, 9u16), (30, 1)] {
                let mut app = app_singing(&line, 1_500);
                let rows = cells(&mut app, w, h);
                let all: Vec<String> = rows.iter().map(|r| r.concat()).collect();
                assert!(all.iter().any(|r| r.contains(&whole)), "{w}×{h}: {all:#?}");
                for m in marks {
                    assert!(rows.iter().flatten().any(|s| s.contains(m) && s.chars().count() > 1), "{m:?} shares a cell with its letter in {w}×{h}");
                }
            }
        }
    }

    #[test]
    fn a_long_cjk_line_in_a_tiny_pane_scrolls_to_the_word_being_sung() {
        // 20 characters, 40 columns, in a 30-column row: ten words of two
        let line = ["夜空", "星屑", "光る", "風鈴", "海辺", "雨音", "花火", "月影", "雪国", "桜色"];
        // singing the fifth word (column 16): it sits a third of the way in, at column 10
        let mut app = app_singing(&line, 5_500);
        let row = cells(&mut app, 30, 1).remove(0);
        assert!(row.concat().starts_with("屑光る風鈴海辺"), "{}", row.concat());
        assert_eq!(row[10], "海");
        // singing the last one: scrolled as far as the line goes, the word in view
        let mut app = app_singing(&line, 10_500);
        let row = cells(&mut app, 30, 1).remove(0).concat();
        assert!(row.starts_with("る風鈴") && row.ends_with("桜色"), "{row}");
        // at the start nothing is skipped
        let mut app = app_singing(&line, 1_500);
        assert!(cells(&mut app, 30, 1).remove(0).concat().starts_with("夜空"));
    }

    #[test]
    fn a_latin_line_in_a_tiny_pane_scrolls_as_before() {
        // one cell per letter, the word being sung a third of the way in
        let line = ["Every ", "echo ", "finds ", "its ", "way ", "back ", "home ", "again"];
        let mut app = app_singing(&line, 5_500);
        let row = cells(&mut app, 20, 1).remove(0);
        assert!(row.iter().all(|s| s.chars().count() == 1));
        // "way" starts at column 21 of the line; 21 - 20/3 = 15 skipped
        assert!(row.concat().starts_with("s its way back home"), "{:?}", row.concat());
    }

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let th = Theme::singularity();
        let mut buf = Buffer::empty(Rect::new(0, 0, w + 2, h + 2));
        draw(&mut buf, Rect::new(1, 1, w, h), app, &th);
        (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn the_lyrics_delay_holds_the_words_back() {
        // paused at 4.3 s with the words 300 ms behind: drawn as at 4.0 s without a delay
        let mut late = app_at(4_300, true);
        late.config.lyrics.delay_ms = 300;
        assert_eq!(late.lyrics_position(singularity_proto::now_ms()), 4_000);
        let mut plain = app_at(4_000, true);
        assert_eq!(render(&mut late, 60, 12), render(&mut plain, 60, 12));
        // never before the song's start; a negative delay brings them forward
        late.config.lyrics.delay_ms = 9_000;
        assert_eq!(late.lyrics_position(singularity_proto::now_ms()), 0);
        late.config.lyrics.delay_ms = -200;
        assert_eq!(late.lyrics_position(singularity_proto::now_ms()), 4_500);
    }

    #[test]
    fn every_size_and_moment_draws_without_panicking() {
        let moments = [0, 1_000, 2_900, 3_000, 3_700, 4_300, 5_000, 6_000, 7_000, 9_000, 13_000, 20_000, 27_000, 31_000, 200_000];
        let mut sizes = vec![];
        for w in [1u16, 2, 3, 5, 8, 15, 16, 17, 30, 60, 120, 300] {
            for h in [1u16, 2, 3, 4, 5, 7, 9, 14, 20, 40, 100] {
                sizes.push((w, h));
            }
        }
        for synced in [true, false] {
            for &pos in &moments {
                let mut app = app_at(pos, synced);
                for &(w, h) in &sizes {
                    let _ = render(&mut app, w, h);
                }
            }
        }
        // and with nothing playing, nothing found, still looking
        let mut app = App::new(None);
        let _ = render(&mut app, 40, 10);
        app.playback.item = Some(singularity_proto::Item { uri: "spotify:track:x".into(), ..Default::default() });
        assert!(render(&mut app, 40, 10).contains("looking for lyrics"));
        app.lyrics_event(Lyrics { uri: "spotify:track:x".into(), missing: "no lyrics on Spotify or LRCLIB".into(), ..Default::default() });
        let s = render(&mut app, 60, 10);
        assert!(s.contains("nothing to sing along to") && s.contains("no lyrics on Spotify or LRCLIB"), "{s}");
    }

    #[test]
    fn the_line_fills_as_it_is_sung_and_the_ball_rides_above() {
        // between "the" (3.5 s) and "lights" (4 s): "Turn" and "the" sung, the ball over them
        let th = Theme::singularity();
        let mut app = app_at(3_700, true);
        let (w, h) = (60, 9);
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        draw(&mut buf, Rect::new(0, 0, w, h), &mut app, &th);
        let rows: Vec<String> = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
        let ly = rows.iter().position(|r| r.contains("Turn the lights down")).expect("the line");
        let lx = rows[ly].find("Turn").unwrap() as u16;
        assert_eq!(buf[(lx, ly as u16)].fg, th.accent.color(), "sung: accent");
        let lights = rows[ly].find("lights").unwrap() as u16;
        assert_eq!(buf[(lights, ly as u16)].fg, th.fg.color(), "not yet sung");
        assert!(rows[..ly].iter().any(|r| r.contains('●')), "the ball is in its lane: {rows:#?}");
        assert!(rows[ly + 1..].iter().any(|r| r.contains("Sing it like")), "the next line waits below");
        assert!(!rows.iter().any(|r| r.contains("estimated")), "this line's words are timed");
        // the next line's words are estimated, and the credit says so
        let mut app = app_at(8_000, true);
        assert!(render(&mut app, 70, 12).contains("word timing estimated"));
    }

    #[test]
    fn a_gap_says_what_comes_next_and_dots_count_it_in() {
        let mut app = app_at(18_000, true);
        let s = render(&mut app, 60, 12);
        assert!(s.contains("next line") && s.contains("Ünïcödé"), "{s}");
        // 2.5 s before the line after the gap: three dots and no ball yet
        let first = timing::timed(&app.lyrics.current().unwrap().lines, 120_000)[2].first();
        let mut app = app_at(first - 2_500, true);
        let s = render(&mut app, 60, 12);
        assert_eq!(s.matches('●').count(), 3, "{s}");
    }

    #[test]
    fn big_panes_use_the_pixel_font_and_unsynced_lyrics_just_show() {
        let mut app = app_at(3_700, true);
        let s = render(&mut app, 160, 30);
        assert!(s.contains('█') || s.contains('▀'), "big letters: {s}");
        let mut app = app_at(3_700, false);
        let s = render(&mut app, 80, 12);
        assert!(s.contains("Turn the lights down") && s.contains("not time-synced") && !s.contains('●'), "{s}");
    }

    #[test]
    fn the_pixel_tier_sends_a_smooth_ball() {
        let mut app = app_at(3_700, true);
        app.playback.status = singularity_proto::PlayStatus::Playing;
        app.playback.timestamp_ms = singularity_proto::now_ms();
        app.cell_px = Some((10.0, 20.0));
        app.kitty = Some(crate::gfx::Kitty::new(false));
        let _ = render(&mut app, 60, 12);
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        let apc = String::from_utf8_lossy(&out);
        assert!(apc.contains(&format!("i={},", BALL_ID * 2)) && apc.contains("f=32"), "{apc:.200}");
    }

    /// `app` on the pixel tier (cells of 10×20 pixels), drawn into a `w`×`h` Pane at (1, 1): the cells and what went to
    /// the terminal.
    fn pixel_frame(app: &mut App, w: u16, h: u16) -> (String, String) {
        if app.kitty.is_none() {
            app.cell_px = Some((10.0, 20.0));
            app.kitty = Some(crate::gfx::Kitty::new(false));
        }
        let s = render(app, w, h);
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        (s, String::from_utf8_lossy(&out).into_owned())
    }

    #[test]
    fn big_text_words_sit_where_the_font_puts_them() {
        let w = words("Every echo finds its way back home");
        let px = 60.0;
        let p = plan_px(&w, 200, 40, (10.0, 20.0), px).expect("fits");
        assert_eq!(p.rows, 1);
        let (text, col, cols) = &p.row_px[0];
        assert_eq!(text, "Every echo finds its way back home");
        // each word's left is the row's left and the width of the text before it, as the font measures
        let mut before = String::new();
        for (k, word) in w.iter().enumerate() {
            let x = *col as f32 * 10.0 + crate::bigtext::width(&before, px, true);
            assert!((p.px[k].0 - x).abs() < 0.01 && (p.px[k].1 - crate::bigtext::width(&word.text, px, true)).abs() < 0.01, "{k}");
            before.push_str(&word.text);
            before.push(' ');
        }
        // centred, inside the Pane
        assert!(col + cols <= 200 && (*col as i32 - (200 - col - cols) as i32).abs() <= 1, "{col} {cols}");
        // a long line wraps into up to three rows and shrinks only when even that doesn't fit
        let long = words("Here comes the chorus, hold on to the note and let the whole room carry it all night long");
        let p = plan_px(&long, 120, 30, (10.0, 20.0), 90.0).expect("fits");
        assert!(p.rows <= 3 && p.row_px.iter().all(|(_, c, n)| c + n <= 120), "{:?}", p.row_px);
        assert!(plan_px(&long, 20, 6, (10.0, 20.0), 90.0).is_none(), "too small: the cells' plans");
    }

    #[test]
    fn a_big_pane_sings_in_big_text_split_where_the_fill_is() {
        // between "the" and "lights": "Turn the" sung
        let mut app = app_at(3_700, true);
        ever_after(&mut app);
        app.playback.status = singularity_proto::PlayStatus::Playing;
        app.playback.timestamp_ms = singularity_proto::now_ms();
        let (s, out) = pixel_frame(&mut app, 160, 30);
        assert!(!s.contains("Turn the lights") && !s.contains('█'), "the line is big text, not cells: {s}");
        assert!(out.contains("f=32"), "{out:.300}");
        // the line's row: its sung part from the left (x=0), the rest after it (x>0), same height
        assert!(out.contains(",x=0,y=0,") && out.split(",x=").skip(1).any(|p| !p.starts_with("0,")), "{out:.400}");
        // and the ball rides above it
        assert!(out.contains(&format!("i={},", BALL_ID * 2)));
        // steady frames send no new text
        app.playback.status = singularity_proto::PlayStatus::Paused;
        app.playback.position_ms = 3_700;
        let _ = pixel_frame(&mut app, 160, 30);
        let (_, out) = pixel_frame(&mut app, 160, 30);
        assert!(!out.contains("a=t,"), "{out:.300}");
        // a CJK line: no big text for the song; the cells draw it
        let mut app = app_at(31_000, true);
        let (s, out) = pixel_frame(&mut app, 160, 30);
        assert!(s.contains("Ünïcödé") && !out.contains("a=t,"), "{s}");
    }

    /// An App paused at `pos` in a song of twelve lines, one every four seconds (the fake Catalogue's verse three
    /// times over).
    fn verses_at(pos: u64) -> App {
        let mut app = App::new(None);
        app.playback.item = Some(singularity_proto::Item { uri: "spotify:track:v".into(), name: "Song".into(), duration_ms: 120_000, ..Default::default() });
        app.playback.status = singularity_proto::PlayStatus::Paused;
        app.playback.position_ms = pos;
        let verse = ["Turn the lights down low", "Sing it like the night is young", "Every echo finds its way back home", "Here comes the chorus, hold on to the note and let the whole room carry it"];
        let lines = (0..12).map(|k| LyricLine { start_ms: 2_000 + k as u64 * 4_000, text: verse[k % 4].into(), ..Default::default() }).collect();
        app.lyrics_event(Lyrics { uri: "spotify:track:v".into(), synced: true, lines, source: "fake".into(), ..Default::default() });
        app
    }

    #[test]
    fn only_narrow_tall_panes_run_the_song_around_the_line() {
        // the contact sheet's narrow-tall Pane (24×30 with its frame) and the sizes scenario's 16×30
        assert!(narrow_tall(22, 28) && narrow_tall(16, 28));
        // tiny, wide-short, medium, big, full and the sizes scenario's others keep their look
        for (w, h) in [(18, 4), (98, 6), (58, 18), (118, 38), (180, 47), (14, 28), (44, 14), (88, 32), (254, 62), (108, 2)] {
            assert!(!narrow_tall(w, h), "{w}×{h}");
        }
    }

    #[test]
    fn a_narrow_tall_pane_is_full_of_the_song() {
        // the seventh line ("Every echo finds its way back home") being sung
        let mut app = verses_at(26_600);
        let s = render(&mut app, 22, 28);
        let rows: Vec<&str> = s.lines().collect();
        // whole lines, wrapped, never cut with an ellipsis
        assert!(!s.contains('…'), "{s}");
        assert!(s.contains("Here comes the") && s.contains("whole room carry it"), "{s}");
        // lines well before and after it, the Pane full from its top row to the credit: no run of empty rows longer
        // than the ball's lane
        let body = &rows[1..rows.len() - 2];
        assert!(body[0].trim().len() > 2 && body[body.len() - 1].trim().len() > 2, "{s}");
        let mut run = 0;
        for r in body {
            run = if r.trim().is_empty() { run + 1 } else { 0 };
            assert!(run <= 2, "{s}");
        }
    }

    #[test]
    fn a_narrow_tall_pane_sings_in_big_text_with_the_song_around_it() {
        let mut app = verses_at(26_600);
        let (s, out) = pixel_frame(&mut app, 22, 28);
        assert!(!s.contains("Every echo") && out.contains("a=t,"), "the line on stage is big text: {s}");
        assert!(s.contains("Here comes the") && s.contains("Sing it like the") && !s.contains('…'), "{s}");
        // its size worked out once for the song and this Pane
        assert!(app.lyrics.karaoke().size.is_some_and(|(k, px)| k.0 == 22 && px.is_some()));
    }

    /// What big text costs while a line is sung (release: `cargo test --release -p singularity -- --ignored
    /// karaoke_big_text_cost --nocapture`): every frame moves the fill and the ball; a new line now and then.
    #[test]
    #[ignore]
    fn karaoke_big_text_cost() {
        for (w, h, px) in [(240u16, 65u16, true), (240, 65, false)] {
            let mut app = app_at(3_000, true);
            ever_after(&mut app);
            if px {
                let _ = pixel_frame(&mut app, w, h);
            }
            let n = 3_000u64;
            let t = Instant::now();
            for i in 0..n {
                // 4 ms a frame (240 a second) through the first two lines
                app.playback.position_ms = 3_000 + i * 4 % 9_000;
                if px {
                    let _ = pixel_frame(&mut app, w, h);
                } else {
                    let _ = render(&mut app, w, h);
                }
            }
            println!("karaoke {w}x{h} {}: {:.1} µs a frame", if px { "big text" } else { "cells" }, t.elapsed().as_secs_f64() * 1e6 / n as f64);
        }
    }

    /// A song timed word by word the way LRCLIB's Lyricsfile times it (each word's start only, the line's end), invented
    /// words: three lines in a row, an instrumental, a fourth line.
    fn timed_song() -> Vec<LyricLine> {
        let line = |start: u64, words: &[(&str, u64)], end: u64| {
            let mut w: Vec<LyricWord> = words.iter().map(|(t, s)| LyricWord { start_ms: *s, end_ms: None, text: (*t).into() }).collect();
            if let Some(last) = w.last_mut() {
                last.end_ms = Some(end);
            }
            LyricLine { start_ms: start, text: words.iter().map(|w| w.0).collect(), words: w }
        };
        vec![
            line(8_120, &[("Paper ", 8_120), ("boats ", 8_790), ("drift ", 9_210), ("past ", 9_760), ("noon", 10_660)], 11_620),
            line(12_080, &[("Gulls ", 12_080), ("never ", 12_540), ("count ", 12_790), ("the ", 13_050), ("tides", 13_290)], 14_940),
            line(15_650, &[("Slow, ", 15_650), ("slow, ", 16_010), ("row ", 16_410), ("home", 17_460)], 18_420),
            line(40_000, &[("Lanterns ", 40_000), ("on ", 40_400), ("water", 40_700)], 42_000),
        ]
    }

    fn timed_app(pos: u64) -> App {
        let mut app = App::new(None);
        app.playback.item = Some(singularity_proto::Item { uri: "spotify:track:t".into(), name: "Song".into(), duration_ms: 60_000, ..Default::default() });
        app.playback.status = singularity_proto::PlayStatus::Paused;
        app.playback.position_ms = pos;
        app.lyrics_event(Lyrics { uri: "spotify:track:t".into(), synced: true, lines: timed_song(), source: "fake".into(), ..Default::default() });
        app
    }

    /// What a 60×9 Karaoke Pane shows being sung: the line on stage (the row drawn in the waiting or the sung colour,
    /// not the dimmed lines around it) and how many of its words have begun to fill.
    fn on_stage_now(app: &mut App) -> Option<(String, usize)> {
        let th = Theme::singularity();
        let (w, h) = (60u16, 9u16);
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        draw(&mut buf, Rect::new(0, 0, w, h), app, &th);
        let quiet = [th.dim.color(), th.border.color()];
        for y in 0..h - 1 {
            let mut words: Vec<(String, bool)> = vec![];
            let mut prev_space = true;
            for x in 0..w {
                let c = &buf[(x, y)];
                let letter = c.symbol().chars().any(char::is_alphanumeric);
                if !letter || quiet.contains(&c.fg) {
                    prev_space = true;
                    continue;
                }
                if prev_space {
                    words.push((String::new(), c.fg != th.fg.color()));
                }
                words.last_mut().expect("pushed").0.push_str(c.symbol());
                prev_space = false;
            }
            if !words.is_empty() {
                let text = words.iter().map(|w| w.0.as_str()).collect::<Vec<_>>().join(" ");
                return Some((text, words.iter().filter(|w| w.1).count()));
            }
        }
        None
    }

    /// The sync check: the song played through 10 ms at a time, the word the Pane highlights against the word being
    /// sung. Every word lights up within a step of its start, the line on stage is the one being sung, and the delay
    /// setting shifts it all by exactly its amount.
    #[test]
    fn the_highlighted_word_is_the_one_being_sung() {
        // how soon a begun word shows it: its first letter's tint is visible within this
        const TINT_MS: u64 = 40;
        let song = timed_song();
        let mut worst = 0;
        for delay in [0i32, 250, -150] {
            let mut app = timed_app(0);
            app.config.lyrics.delay_ms = delay;
            let mut lit_at: Vec<Vec<Option<u64>>> = song.iter().map(|l| vec![None; l.words.len()]).collect();
            for pos in (0..45_000u64).step_by(10) {
                app.playback.position_ms = pos;
                let at = (pos as i64 - delay as i64).max(0) as u64;
                let Some((text, lit)) = on_stage_now(&mut app) else { continue };
                let bare = |t: &str| t.chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect::<String>();
                let Some(i) = song.iter().position(|l| bare(&l.text).trim() == text) else { continue };
                // the line being sung is on stage while it is sung
                if let Some(now) = song.iter().position(|l| l.words[0].start_ms <= at && at < l.words.last().unwrap().end_ms.unwrap()) {
                    assert_eq!(i, now, "at {at} ms (delay {delay}): {text:?} on stage");
                }
                // and exactly the words begun are lit (a word's first letter shows its tint a few ms into the word)
                let begun = song[i].words.iter().filter(|w| w.start_ms <= at).count();
                let just = begun > 0 && at - song[i].words[begun - 1].start_ms < TINT_MS;
                assert!(lit == begun || (just && lit + 1 == begun), "at {at} ms (delay {delay}) in {text:?}: {lit} lit, {begun} begun");
                for seen in lit_at[i].iter_mut().take(lit) {
                    seen.get_or_insert(pos);
                }
            }
            for (i, l) in song.iter().enumerate() {
                for (k, w) in l.words.iter().enumerate() {
                    let seen = lit_at[i][k].unwrap_or_else(|| panic!("{:?} never lit (delay {delay})", w.text));
                    let lag = seen as i64 - delay as i64 - w.start_ms as i64;
                    assert!((0..TINT_MS as i64).contains(&lag), "{:?} lit {lag} ms after it is sung (delay {delay})", w.text);
                    worst = worst.max(lag);
                }
            }
        }
        println!("karaoke sync: every word lit at most {worst} ms after it is sung");
    }

    /// The position the Pane sings from moves on between the player's updates, stops on a pause, jumps with a seek,
    /// and a new song's words wait for its own lyrics.
    #[test]
    fn the_words_follow_play_pause_seek_and_a_new_song() {
        use singularity_proto::PlayStatus::*;
        let now = singularity_proto::now_ms();
        // playing: the last update was 1.6 s ago at 8.5 s, so "past" (9.76 s) is being sung, not "boats"
        let mut app = timed_app(8_500);
        (app.playback.status, app.playback.speed, app.playback.timestamp_ms) = (Playing, 1.0, now - 1_600);
        assert_eq!(on_stage_now(&mut app).map(|s| s.1), Some(4), "{:?}", on_stage_now(&mut app));
        // paused there: it stays
        (app.playback.status, app.playback.speed, app.playback.position_ms, app.playback.timestamp_ms) = (Paused, 0.0, 10_100, now - 5_000);
        assert_eq!(on_stage_now(&mut app).map(|s| s.1), Some(4));
        // a seek into the next line
        app.seek_to(13_100);
        app.playback.speed = 0.0;
        assert_eq!(on_stage_now(&mut app), Some(("Gulls never count the tides".into(), 4)));
        // another song starts: nothing of the old song's lines shows while its lyrics are on the way
        app.playback.item = Some(singularity_proto::Item { uri: "spotify:track:u".into(), name: "Next".into(), duration_ms: 60_000, ..Default::default() });
        app.playback.position_ms = 0;
        assert_eq!(on_stage_now(&mut app), None);
    }

    #[test]
    fn every_size_and_moment_draws_inside_the_pane_on_the_pixel_tier() {
        let moments = [0, 2_900, 3_000, 3_700, 4_300, 7_000, 9_000, 13_000, 20_000, 200_000];
        for synced in [true, false] {
            let mut app = app_at(0, synced);
            ever_after(&mut app);
            for &pos in &moments {
                app.playback.position_ms = pos;
                for w in [16u16, 30, 60, 90, 120, 160, 240, 300] {
                    for h in [3u16, 7, 9, 14, 20, 30, 40, 65, 100] {
                        let (_, out) = pixel_frame(&mut app, w, h);
                        for &(x, y, c, r) in &crate::lyrics::tests::placements(&out) {
                            assert!(x >= 1 && x + c <= w + 1 && y >= 1 && y + r <= h + 1, "{w}x{h} at {pos}: ({x},{y},{c},{r})");
                        }
                    }
                }
            }
        }
    }
}
