//! The Stats page (#36): a live "Wrapped" from the local listening log, in the main Pane. Period tabs (←/→ or a
//! click), minutes / plays / streak, a 24-hour listening clock, and the top artists, tracks and albums. Rows act like
//! everywhere else: a track plays (double-click / Enter), an artist or album opens its page.

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::{Stats, StatsPeriod, StatsRow};
use unicode_width::UnicodeWidthStr;

/// Rows shown per top list.
pub const TOP: usize = 10;
/// From this inner width the three top lists stand side by side.
const COLUMNS_FROM: u16 = 108;
/// Below this many rows the listening clock steps aside for the summary and the lists.
const CLOCK_FROM: u16 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Artists,
    Tracks,
    Albums,
}

impl Section {
    const ALL: [Section; 3] = [Section::Artists, Section::Tracks, Section::Albums];

    fn title(self) -> &'static str {
        match self {
            Section::Artists => "Top artists",
            Section::Tracks => "Top tracks",
            Section::Albums => "Top albums",
        }
    }

    pub fn rows(self, s: &Stats) -> &[StatsRow] {
        let r = match self {
            Section::Artists => &s.top_artists,
            Section::Tracks => &s.top_tracks,
            Section::Albums => &s.top_albums,
        };
        &r[..r.len().min(TOP)]
    }
}

#[derive(Clone, Debug, Default)]
pub struct StatsView {
    pub period: StatsPeriod,
    /// the selected row, counting through artists, then tracks, then albums
    pub sel: usize,
    /// first line shown
    pub top: usize,
    pub visible: usize,
}

/// The selectable rows in order: (section, index in it).
pub fn entries(s: &Stats) -> Vec<(Section, usize)> {
    Section::ALL.iter().flat_map(|&sec| (0..sec.rows(s).len()).map(move |i| (sec, i))).collect()
}

/// "3 h 12 min", "42 min"
pub fn fmt_minutes(m: u32) -> String {
    if m >= 60 { format!("{} h {:02} min", m / 60, m % 60) } else { format!("{m} min") }
}

/// The short form for narrow columns: "3h12", "42m".
fn short_minutes(m: u32) -> String {
    if m >= 60 { format!("{}h{:02}", m / 60, m % 60) } else { format!("{m}m") }
}

/// Days since 1970-01-01 → (year, month, day) (H. Hinnant's algorithm, as the Daemon's calendar).
fn civil(day: i64) -> (i64, u32, u32) {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "12 – 18 Oct 2026", "1 Jan – 18 Oct 2026"
pub fn range(from: i64, to: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let ((fy, fm, fd), (ty, tm, td)) = (civil(from), civil(to));
    let mon = |m: u32| MONTHS[(m - 1) as usize];
    if from == to {
        format!("{td} {} {ty}", mon(tm))
    } else if (fy, fm) == (ty, tm) {
        format!("{fd} – {td} {} {ty}", mon(tm))
    } else if fy == ty {
        format!("{fd} {} – {td} {} {ty}", mon(fm), mon(tm))
    } else {
        format!("{fd} {} {fy} – {td} {} {ty}", mon(fm), mon(tm))
    }
}

/// `s` cut to `w` display columns, with an ellipsis when it doesn't fit.
fn clip(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.to_string().width() + 1 > w {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// A bar of `cells` cells filled to `t` (eighths), in the ramp's colour for `t`.
fn bar(buf: &mut Buffer, x: u16, y: u16, cells: u16, t: f32, th: &Theme) {
    const EIGHTHS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    let eighths = (t.clamp(0.0, 1.0) * cells as f32 * 8.0).round() as u16;
    let st = th.s(th.ramp_at(0.35 + 0.65 * t));
    for i in 0..cells {
        let e = eighths.saturating_sub(i * 8).min(8) as usize;
        if let Some(c) = buf.cell_mut((x + i, y)) {
            c.set_char(EIGHTHS[e]).set_style(st);
        }
    }
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let Some(mut view) = app.stats_view.clone() else { return };
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    if w < 16 || r.height < 3 {
        return;
    }
    let mut hits: Vec<(Rect, Hit)> = vec![];
    // tabs: long, short, or short without padding, whichever fits; and the dates they cover
    let fits = |name: fn(StatsPeriod) -> &'static str, pad: u16| StatsPeriod::ALL.iter().map(|p| name(*p).width() as u16 + 2 * pad + 1).sum::<u16>() <= w + 1;
    let (name, pad): (fn(StatsPeriod) -> &'static str, u16) = if fits(StatsPeriod::label, 1) {
        (StatsPeriod::label, 1)
    } else if fits(StatsPeriod::short, 1) {
        (StatsPeriod::short, 1)
    } else {
        (StatsPeriod::short, 0)
    };
    let mut tx = x;
    for p in StatsPeriod::ALL {
        let pad_s = " ".repeat(pad as usize);
        let label = format!("{pad_s}{}{pad_s}", name(p));
        let lw = label.width() as u16;
        if tx + lw > x + w {
            break;
        }
        let st = if p == view.period { th.chip(th.accent) } else { th.s(th.dim) };
        text(buf, tx, r.y, lw, &label, st);
        hits.push((Rect::new(tx, r.y, lw, 1), Hit::StatsPeriod(p)));
        tx += lw + 1;
    }
    let Some(s) = app.stats.get(&view.period).cloned() else {
        text(buf, x, r.y + 2, w, "counting…", th.s(th.dim));
        app.stats_view = Some(view);
        for (rect, h) in hits {
            app.hit(rect, h);
        }
        return;
    };
    let dates = range(s.from_day, s.to_day);
    if tx + dates.width() as u16 + 1 < x + w {
        let dw = dates.width() as u16;
        text(buf, x + w - dw, r.y, dw, &dates, th.s(th.border));
    }

    // everything below the tabs is one scrolling column of lines, built first, then drawn from `view.top`
    let body = Rect::new(r.x, r.y + 2, r.width, r.height.saturating_sub(2));
    let mut lines: Vec<Line> = vec![];
    if s.plays == 0 {
        lines.push(Line::Text("nothing logged yet in this period: the log fills as you listen (a Play is 30 s, or half a short track)".into(), Style::Dim));
    } else {
        let streak = if s.streak_days > 0 { format!(" · {}-day streak (longest {} here)", s.streak_days, s.longest_streak_days) } else { String::new() };
        lines.push(Line::Text(format!("{} · {} plays{streak}", fmt_minutes(s.minutes), s.plays), Style::Strong));
        lines.push(Line::Blank);
        if r.height >= CLOCK_FROM {
            lines.push(Line::Text("when you listen".into(), Style::Heading));
            lines.extend([Line::Clock(0), Line::Clock(1), Line::Clock(2), Line::ClockLabels]);
            lines.push(Line::Blank);
        }
        if w >= COLUMNS_FROM {
            lines.push(Line::Headings);
            for i in 0..TOP {
                if Section::ALL.iter().any(|sec| i < sec.rows(&s).len()) {
                    lines.push(Line::Columns(i));
                }
            }
        } else {
            for sec in Section::ALL {
                if sec.rows(&s).is_empty() {
                    continue;
                }
                lines.push(Line::Text(sec.title().into(), Style::Heading));
                lines.extend((0..sec.rows(&s).len()).map(|i| Line::Row(sec, i)));
                lines.push(Line::Blank);
            }
        }
    }

    // text wraps rather than being cut (the summary on a narrow Pane, the empty log's note)
    let lines: Vec<Line> = lines
        .into_iter()
        .flat_map(|l| match l {
            Line::Text(t, st) => crate::lyrics::wrap(&t, w as usize).into_iter().map(|row| Line::Text(row, st)).collect(),
            other => vec![other],
        })
        .collect();
    // keep the selection in view
    let all = entries(&s);
    view.sel = view.sel.min(all.len().saturating_sub(1));
    view.visible = body.height as usize;
    if let Some(&(sec, i)) = all.get(view.sel)
        && let Some(at) = lines.iter().position(|l| *l == Line::Row(sec, i) || (*l == Line::Columns(i) && w >= COLUMNS_FROM))
    {
        if at < view.top {
            view.top = at.saturating_sub(1);
        } else if at >= view.top + view.visible {
            view.top = at + 1 - view.visible;
        }
    }
    view.top = view.top.min(lines.len().saturating_sub(view.visible));
    let focused = app.focus == "home";
    let selected = all.get(view.sel).copied();
    let peak = |sec: Section| sec.rows(&s).first().map(|r| r.minutes.max(1)).unwrap_or(1);
    let clock_max = s.by_hour.iter().copied().max().unwrap_or(0).max(1);
    for (k, line) in lines.iter().skip(view.top).take(view.visible).enumerate() {
        let y = body.y + k as u16;
        match line {
            Line::Blank => {}
            Line::Text(t, style) => {
                let st = match style {
                    Style::Dim => th.s(th.dim),
                    Style::Strong => th.s(th.fg).add_modifier(Modifier::BOLD),
                    Style::Heading => th.s(th.accent).add_modifier(Modifier::BOLD),
                };
                text(buf, x, y, w, &clip(t, w as usize), st);
            }
            Line::Clock(row) => {
                // three rows of eighths: row 0 is the top
                const EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
                let cw = (w / 24).clamp(1, 3);
                for (h, m) in s.by_hour.iter().enumerate().take(24) {
                    let t = *m as f32 / clock_max as f32;
                    let eighths = (t * 24.0).round() as i32 - (2 - *row as i32) * 8;
                    let ch = EIGHTHS[eighths.clamp(0, 8) as usize];
                    let st = th.s(th.ramp_at(0.3 + 0.7 * t));
                    for c in 0..cw.saturating_sub(u16::from(cw > 1)) {
                        if let Some(cell) = buf.cell_mut((x + h as u16 * cw + c, y)) {
                            cell.set_char(ch).set_style(st);
                        }
                    }
                }
            }
            Line::ClockLabels => {
                let cw = (w / 24).clamp(1, 3);
                for h in [0u16, 6, 12, 18] {
                    let label = format!("{h:02}");
                    text(buf, x + h * cw, y, 2, &label, th.s(th.border));
                }
            }
            Line::Headings => {
                let colw = w / 3;
                for (c, sec) in Section::ALL.iter().enumerate() {
                    text(buf, x + c as u16 * colw, y, colw.saturating_sub(2), sec.title(), th.s(th.accent).add_modifier(Modifier::BOLD));
                }
            }
            Line::Row(sec, i) => {
                let rect = Rect::new(x, y, w, 1);
                row(buf, rect, *sec, *i, &s, peak(*sec), focused && selected == Some((*sec, *i)), th);
                hits.push((rect, Hit::StatsRow(all.iter().position(|e| *e == (*sec, *i)).unwrap_or(0))));
            }
            Line::Columns(i) => {
                let colw = w / 3;
                for (c, sec) in Section::ALL.iter().enumerate() {
                    if *i >= sec.rows(&s).len() {
                        continue;
                    }
                    let rect = Rect::new(x + c as u16 * colw, y, colw.saturating_sub(2), 1);
                    row(buf, rect, *sec, *i, &s, peak(*sec), focused && selected == Some((*sec, *i)), th);
                    hits.push((rect, Hit::StatsRow(all.iter().position(|e| *e == (*sec, *i)).unwrap_or(0))));
                }
            }
        }
    }
    if view.top + view.visible < lines.len() {
        text(buf, r.right().saturating_sub(1), r.bottom().saturating_sub(1), 1, "▾", th.s(th.dim));
    }
    app.stats_view = Some(view);
    for (rect, h) in hits {
        app.hit(rect, h);
    }
}

/// One ranked row: rank, name (and artist), then minutes, a bar and plays as room allows.
#[allow(clippy::too_many_arguments)]
fn row(buf: &mut Buffer, r: Rect, sec: Section, i: usize, s: &Stats, peak: u32, selected: bool, th: &Theme) {
    let it = &sec.rows(s)[i];
    let bg = if selected { th.selection.color() } else { th.bg.color() };
    text(buf, r.x, r.y, r.width, &" ".repeat(r.width as usize), th.base().bg(bg));
    let w = r.width as usize;
    // the right side, as wide as there's room for
    let plays = if sec == Section::Tracks && s.period != StatsPeriod::All && it.all_time_plays > it.plays {
        format!("{}× ({} ever)", it.plays, it.all_time_plays)
    } else {
        format!("{}×", it.plays)
    };
    let (mins, bar_w, plays) = if w >= 56 {
        (fmt_minutes(it.minutes), 8u16, plays)
    } else if w >= 34 {
        (short_minutes(it.minutes), 5, format!("{}×", it.plays))
    } else {
        (short_minutes(it.minutes), 0, String::new())
    };
    let right_w = mins.width() + if bar_w > 0 { bar_w as usize + 1 } else { 0 } + if plays.is_empty() { 0 } else { plays.width() + 1 };
    let rank = format!("{:>2} ", i + 1);
    let left_w = w.saturating_sub(right_w + rank.width() + 1);
    text(buf, r.x, r.y, 3, &rank, th.s(th.border).bg(bg));
    let name_st = th.s(if selected { th.accent } else { th.fg }).bg(bg);
    let name = clip(&it.name, left_w);
    text(buf, r.x + 3, r.y, left_w as u16, &name, name_st);
    if !it.subtitle.is_empty() && name.width() + 3 < left_w {
        let sub = clip(&format!(" · {}", it.subtitle), left_w - name.width());
        text(buf, r.x + 3 + name.width() as u16, r.y, (left_w - name.width()) as u16, &sub, th.s(th.dim).bg(bg));
    }
    let mut rx = r.right() - right_w as u16;
    let mw = mins.width() as u16;
    text(buf, rx, r.y, mw, &mins, th.s(th.dim).bg(bg));
    rx += mw + 1;
    if bar_w > 0 {
        bar(buf, rx, r.y, bar_w, it.minutes as f32 / peak as f32, th);
        rx += bar_w + 1;
    }
    if !plays.is_empty() {
        text(buf, rx, r.y, plays.width() as u16, &plays, th.s(th.border).bg(bg));
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Line {
    Blank,
    Text(String, Style),
    /// a row of the 24-hour clock (0 = top)
    Clock(u8),
    ClockLabels,
    /// the three lists' titles, side by side
    Headings,
    /// row `i` of all three lists, side by side
    Columns(usize),
    /// row `i` of one list
    Row(Section, usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Style {
    Dim,
    Strong,
    Heading,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_minutes_read_naturally() {
        // 2026-10-12 is day 20738
        assert_eq!(range(20738, 20744), "12 – 18 Oct 2026");
        assert_eq!(range(20727, 20744), "1 – 18 Oct 2026");
        assert_eq!(range(20454, 20744), "1 Jan – 18 Oct 2026");
        assert_eq!(range(20744, 20744), "18 Oct 2026");
        assert_eq!(fmt_minutes(42), "42 min");
        assert_eq!(fmt_minutes(192), "3 h 12 min");
        assert_eq!(short_minutes(192), "3h12");
    }

    #[test]
    fn clipping_counts_display_columns() {
        assert_eq!(clip("Nova Drift", 20), "Nova Drift");
        assert_eq!(clip("Nova Drift", 6), "Nova …");
        assert!(clip("光が曲がる光が曲がる", 7).width() <= 7);
    }
}
