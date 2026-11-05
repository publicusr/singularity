//! The Karaoke Widget: the line being sung big and centred, its words filling with the accent as they are sung (left
//! to right, a character at a time), and a ball hopping from word to word above them, landing on each as it starts.
//! The next lines wait dimmed below, the last one fades above. Before a line that follows an instrumental, dots
//! count it in; during the instrumental the Pane says what comes next and when.
//!
//! It scales with its Pane: one or two rows give the line alone (scrolled to the word being sung); a few more add the
//! ball's lane; a big Pane draws the line in the marquee's pixel font (two sizes), and on the pixel tier the ball is
//! a smooth image of its own instead of a `●` stepping through cells. Word times come with the lyrics when the source
//! has them, else they're estimated (`timing.rs`); the credit line says which.

pub mod timing;

use crate::app::{App, Hit};
use crate::theme::{Rgb, Theme};
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use std::time::{Duration, Instant};
use timing::{Line, Stage, Word};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The ball's Visual id on the pixel tier (the hole is 1, the idle takeover 2, glow 3).
const BALL_ID: u32 = 4;

/// What the Widget keeps between frames: the song's timed lines (worked out once per song).
#[derive(Default)]
pub struct State {
    /// (song, lines, its length)
    key: (String, usize, u64),
    lines: Vec<Line>,
    /// the ball is moving on the pixel tier until then: frames come quicker meanwhile
    smooth_until: Option<Instant>,
    /// unsynced lyrics follow the song's progress; the wheel takes them for a while
    pub(crate) follow: crate::scroll::Follow,
}

impl State {
    /// Whether the ball needs frames faster than the usual 30 a second right now.
    pub fn smooth(&self) -> bool {
        self.smooth_until.is_some_and(|t| Instant::now() < t)
    }
}

/// How the line on stage is drawn: plain text, or the pixel font at a scale (a font pixel is `s` columns by `s`
/// half-rows: square on a terminal's 1:2 cells).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Font {
    Plain,
    Big(u16),
}

impl Font {
    /// Columns per character (the pixel font's include the gap after it).
    fn char_w(self) -> u16 {
        match self {
            Font::Plain => 1,
            Font::Big(s) => 4 * s,
        }
    }
    /// Extra columns between two words.
    fn space(self) -> u16 {
        match self {
            Font::Plain => 1,
            Font::Big(s) => 2 * s,
        }
    }
    /// Text rows a line of it takes.
    fn rows(self) -> u16 {
        match self {
            Font::Plain => 1,
            Font::Big(s) => (5 * s).div_ceil(2),
        }
    }
    fn width(self, s: &str) -> u16 {
        match self {
            Font::Plain => s.width() as u16,
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
}

impl Plan {
    /// Rows the whole line takes, lanes included.
    fn height(&self) -> u16 {
        self.rows.saturating_mul(self.lane + self.font.rows())
    }
}

/// The biggest way the line fits `w` × `h` with room left for the next line and the credit.
fn plan(words: &[Word], w: u16, h: u16) -> Plan {
    let big_ok = !words.is_empty() && words.iter().all(|x| x.text.chars().all(|c| big_char(c).is_some()));
    if big_ok {
        // (scale, lane, most rows, rows to keep free: the big size wants two next lines, the small one)
        for (s, lane, most, spare) in [(2u16, 4u16, 2u16, 4u16), (1, 3, 3, 2)] {
            let font = Font::Big(s);
            let (spots, rows) = lay_out(words, font, w);
            let fits = spots.iter().all(|p| p.x + p.w <= w + s);
            let p = Plan { font, lane, spots, rows };
            if fits && rows <= most && p.height() + spare <= h {
                return p;
            }
        }
    }
    let (spots, rows) = lay_out(words, Font::Plain, w);
    let lane = if rows * 3 + 3 <= h {
        2
    } else if rows * 2 < h {
        1
    } else {
        0
    };
    Plan { font: Font::Plain, lane, spots, rows }
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
    let Some(lyr) = app.lyrics.as_ref().filter(|l| l.uri == item.uri) else {
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
    let pos = app.display_position(singularity_proto::now_ms());
    if !lyr.synced {
        return unsynced(buf, r, app, pos, th);
    }
    let duration = item.duration_ms;
    let st = &mut app.karaoke;
    if st.key.0 != lyr.uri || (st.key.1, st.key.2) != (lyr.lines.len(), duration) {
        st.key = (lyr.uri.clone(), lyr.lines.len(), duration);
        st.lines = timing::timed(&lyr.lines, duration);
    }
    let source = lyr.source.clone();
    let lines = std::mem::take(&mut app.karaoke.lines);
    let playing = app.playback.status == singularity_proto::PlayStatus::Playing;
    let mut hits = vec![];
    let ball = synced(buf, r, &lines, pos, &source, th, &mut hits);
    // the pixel tier's ball, or the cells' `●`
    let mut smooth = false;
    if let Some(b) = ball {
        smooth = playing && b.height > 0.0;
        match app.pixel_tier_for(r) {
            Some(px) if b.lane >= 2 => pixel_ball(app, r, b, px, th),
            _ => cell_ball(buf, r, b, th),
        }
    }
    if smooth && app.pixel_tier_for(r).is_some() {
        app.karaoke.smooth_until = Some(Instant::now() + Duration::from_millis(200));
    }
    app.karaoke.lines = lines;
    for (rect, h) in hits {
        app.hit(rect, h);
    }
}

/// Where to draw the ball: a column and the bottom row of its lane (fractions: between words, between rows), how
/// high in the lane (0..1), and the lane's height.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ball {
    x: f32,
    base: f32,
    height: f32,
    lane: u16,
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
    let rad = (ch * [0.28, 0.28, 0.28, 0.36, 0.42][b.lane.min(4) as usize]).max(2.0);
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
    let Some(lyr) = app.lyrics.as_ref() else { return };
    let body = r.height.saturating_sub(1);
    let rows: Vec<String> = lyr.lines.iter().flat_map(|l| crate::lyrics::wrap(if l.text.is_empty() { "♪" } else { &l.text }, r.width.saturating_sub(2) as usize)).collect();
    let spare = rows.len().saturating_sub(body as usize);
    let dur = app.duration_ms().max(1);
    let first = (spare as f64 * (pos as f64 / dur as f64).clamp(0.0, 1.0)) as usize;
    let first = app.karaoke.follow.first(first, spare);
    for (k, row) in rows.iter().skip(first).take(body as usize).enumerate() {
        centre(buf, r, r.y + k as u16, row.trim(), th.s(th.fg));
    }
    if r.height >= 2 {
        centre(buf, r, r.bottom() - 1, &format!("{} · not time-synced: no ball", lyr.source), th.s(th.border));
    }
}

/// The synced view; returns where the ball goes (if anywhere). Seek targets go to `hits`.
fn synced(buf: &mut Buffer, r: Rect, lines: &[Line], pos: u64, source: &str, th: &Theme, hits: &mut Vec<(Rect, Hit)>) -> Option<Ball> {
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
    let p = plan(&line.words, w, r.height - credit as u16);
    let block = p.height().min(r.height);
    // the line a little above the middle; the next lines under it, the last one above when there's room
    let avail = r.height - credit as u16;
    let top = r.y + avail.saturating_sub(block) * 2 / 5;
    let x0 = r.x + 1;
    let row_top = |row: u16| top + row * (p.lane + p.font.rows()) + p.lane;
    draw_line(buf, r, x0, &p, line, pos, &row_top, th);
    hits.push((Rect::new(r.x, top, r.width, block), Hit::Lyric(line.start)));
    // the last line, fading, above
    if i > 0 && top >= r.y + 2 {
        let prev = plain_text(&lines[i - 1].words);
        centre(buf, r, top - 2, &prev, th.s(th.border));
        hits.push((Rect::new(r.x, top - 2, r.width, 1), Hit::Lyric(lines[i - 1].start)));
    }
    // the next ones below, dimmer the further they are
    let mut y = top + block + 1;
    let bottom = r.bottom() - credit as u16;
    for (k, nx) in lines.iter().enumerate().skip(i + 1).take(3) {
        if y >= bottom {
            break;
        }
        let st = if k == i + 1 { th.s(th.dim) } else { th.s(th.border) };
        centre(buf, r, y, &plain_text(&nx.words), st);
        hits.push((Rect::new(r.x, y, r.width, 1), Hit::Lyric(nx.start)));
        y += if p.font == Font::Plain { 1 } else { 2 };
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
    let centre_of = |k: usize| {
        let s = p.spots[k];
        let trail = if let Font::Big(sc) = p.font { sc } else { 0 };
        (x0 + s.x) as f32 + (s.w.saturating_sub(trail) as f32 - 1.0) / 2.0
    };
    let base_of = |k: usize| (row_top(p.spots[k].row) - 1) as f32;
    let (x, base) = if wx < 0.0 {
        let from = (centre_of(0) - 3.0 * p.font.char_w().min(4) as f32).max(r.x as f32);
        let t = wx + 1.0;
        (from + (centre_of(0) - from) * t, base_of(0))
    } else {
        let a = (wx.floor() as usize).min(line.words.len() - 1);
        let b = (a + 1).min(line.words.len() - 1);
        let t = wx - a as f32;
        (centre_of(a) + (centre_of(b) - centre_of(a)) * t, base_of(a) + (base_of(b) - base_of(a)) * t)
    };
    Some(Ball { x, base, height: h, lane: p.lane })
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
                let n = w.text.chars().count().max(1) as f32;
                let mut x = x0 + s.x;
                for (j, c) in w.text.chars().enumerate() {
                    let a = (f * n - j as f32).clamp(0.0, 1.0);
                    let mut st = th.s(waiting.lerp(sung, a));
                    if a >= 1.0 {
                        st = st.add_modifier(Modifier::BOLD);
                    }
                    let cw = c.width().unwrap_or(0) as u16;
                    if x + cw > r.right() {
                        break;
                    }
                    let mut tmp = [0u8; 4];
                    text(buf, x, y, cw.max(1), c.encode_utf8(&mut tmp), st);
                    x += cw;
                }
            }
            Font::Big(sc) => big_word(buf, r, x0 + s.x, y, &w.text, sc, f, waiting, sung),
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
    // the whole line as one row of characters, each with its fill
    let mut cells: Vec<(char, f32)> = vec![];
    let mut now = 0usize;
    for (k, w) in line.words.iter().enumerate() {
        if k > 0 && !w.glued {
            cells.push((' ', if w.start <= pos { 1.0 } else { 0.0 }));
        }
        let f = timing::fill(w, pos);
        let n = w.text.chars().count().max(1) as f32;
        if w.start <= pos {
            now = cells.len();
        }
        for (j, c) in w.text.chars().enumerate() {
            cells.push((c, (f * n - j as f32).clamp(0.0, 1.0)));
        }
    }
    let width: usize = cells.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
    let w = r.width as usize;
    // scrolled so the word being sung sits a third of the way in
    let skip = if width <= w { 0 } else { now.saturating_sub(w / 3).min(cells.len().saturating_sub(w)) };
    let mut x = r.x + if width <= w { ((w - width) / 2) as u16 } else { 0 };
    for (c, a) in cells.iter().skip(skip) {
        let cw = c.width().unwrap_or(0) as u16;
        if x + cw > r.right() {
            break;
        }
        let mut st = th.s(th.fg.lerp(th.accent, *a));
        if *a >= 1.0 {
            st = st.add_modifier(Modifier::BOLD);
        }
        let mut tmp = [0u8; 4];
        text(buf, x, r.y, cw.max(1), c.encode_utf8(&mut tmp), st);
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
        assert_eq!(plan(&w, 30, 2).lane, 0, "two rows: no room for the ball");
        assert_eq!((plan(&w, 30, 4).font, plan(&w, 30, 4).lane), (Font::Plain, 1));
        assert_eq!(plan(&w, 30, 8).lane, 2);
        assert_eq!(plan(&w, 100, 14).font, Font::Big(1));
        assert_eq!(plan(&w, 200, 20).font, Font::Big(2));
        // a script the pixel font doesn't have stays plain
        let cjk = words("光が 曲がる");
        assert_eq!(plan(&cjk, 200, 40).font, Font::Plain);
        // accents map to the plain letters
        assert_eq!(plan(&words("żółw tańczy"), 200, 40).font, Font::Big(2));
    }

    /// An App playing a song whose lyrics have both timed and untimed lines, at `pos`.
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
        app.lyrics = Some(Lyrics { uri: "spotify:track:k".into(), synced, lines, source: "fake".into(), ..Default::default() });
        app
    }

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let th = Theme::singularity();
        let mut buf = Buffer::empty(Rect::new(0, 0, w + 2, h + 2));
        draw(&mut buf, Rect::new(1, 1, w, h), app, &th);
        (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
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
        app.lyrics = Some(Lyrics { uri: "spotify:track:x".into(), missing: "no lyrics on Spotify or LRCLIB".into(), ..Default::default() });
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
        let first = timing::timed(&app.lyrics.as_ref().unwrap().lines, 120_000)[2].first();
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
}
