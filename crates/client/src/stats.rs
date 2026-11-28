//! The Stats page (#36): a live "Wrapped" from the local listening log, in the main Pane. Period tabs (←/→ or a
//! click), minutes / plays / streak, a 24-hour listening clock, and the top artists, tracks and albums. Rows act like
//! everywhere else: a track plays (double-click / Enter), an artist or album opens its page.
//! Also the two side Widgets (#37): **today** (minutes, plays, streak, the last 7 days) and **top this week** (top 5
//! artists or tracks; a click on its title switches).

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::{text, blank};
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
    /// whether the wheel has the view (D86)
    pub wheeled: crate::scroll::Wheeled,
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
pub fn civil(day: i64) -> (i64, u32, u32) {
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
    let Some(mut view) = app.page.stats().cloned() else { return };
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
        if let Some(v) = app.page.stats_mut() {
            *v = view;
        }
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
    let end = lines.len().saturating_sub(view.visible);
    if view.wheeled.follows(view.sel, end)
        && let Some(&(sec, i)) = all.get(view.sel)
        && let Some(at) = lines.iter().position(|l| *l == Line::Row(sec, i) || (*l == Line::Columns(i) && w >= COLUMNS_FROM))
    {
        if at < view.top {
            view.top = at.saturating_sub(1);
        } else if at >= view.top + view.visible {
            view.top = at + 1 - view.visible;
        }
    }
    view.top = view.top.min(lines.len().saturating_sub(view.visible));
    let focused = app.is_focused(crate::layout::Widget::Home);
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
    if let Some(v) = app.page.stats_mut() {
        *v = view;
    }
    for (rect, h) in hits {
        app.hit(rect, h);
    }
}

const WEEKDAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
const WEEKDAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// The "today" Widget: minutes today as a headline, plays and the streak, and the last days as a column chart.
/// Small Panes get lines of text and a 7-day sparkline; tall ones a big figure, a comparison with the week's average
/// and a chart of 7, 14 or 28 days (by width) with the minutes over the columns when they fit. The words sit in the
/// middle of the width (#58). A narrow, tall Pane lays the days sideways, a bar a row (#76).
pub fn today(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    if r.height == 0 || w < 6 {
        return;
    }
    let Some(sum) = app.listening.as_ref() else {
        text(buf, x, r.y, w, "counting…", th.s(th.dim));
        return;
    };
    let mins = sum.minutes.last().copied().unwrap_or(0);
    let plays = if sum.plays_today == 1 { "1 play".to_string() } else { format!("{} plays", sum.plays_today) };
    let streak = match sum.streak_days {
        0 => "no streak yet".to_string(),
        1 => "1-day streak".to_string(),
        n => format!("{n}-day streak"),
    };
    let streak_st = th.s(if sum.streak_days > 1 { th.accent2 } else { th.dim });
    // small: lines of text (a narrow Pane tall enough still gets the figure and the chart)
    if r.height < 9 || w < 12 || (w < 22 && r.height < 16) {
        return today_compact(buf, r, sum, mins, &plays, &streak, streak_st, th);
    }
    // narrow and tall (#76): the last days as bars lying sideways, one a row
    if w <= 34 && r.height >= 24 {
        return today_bars(buf, r, sum, mins, &plays, &streak, streak_st, th);
    }
    let mut y = r.y;
    // a line in the middle of the Pane's width (#58)
    let centred = |buf: &mut Buffer, y: u16, s: &str, st| {
        let s = clip(s, w as usize);
        let sw = s.width() as u16;
        text(buf, crate::balance::centre(x, w, sw), y, sw, &s, st);
    };
    // the headline in the middle of the width (#58): minutes as a figure, plays and streak beside it (or under it,
    // when there's no room to set the two side by side in the middle)
    let fig = if mins >= 60 { format!("{}h{:02}m", mins / 60, mins % 60) } else { format!("{mins}m") };
    let big = th.s(th.accent).add_modifier(Modifier::BOLD);
    let fw = crate::tile::figure_width(&fig).min(w);
    let words_w = ["today", plays.as_str(), streak.as_str()].iter().map(|s| s.width() as u16).max().unwrap_or(0);
    let head_w = fw + 3 + words_w;
    if head_w + 6 <= w {
        let hx = crate::balance::centre(x, w, head_w);
        crate::tile::figure(buf, hx, y, w, &fig, big, th.s(th.dim));
        let side = hx + fw + 3;
        let sw = x + w - side;
        text(buf, side, y, sw, "today", th.s(th.dim));
        text(buf, side, y + 1, sw, &plays, th.s(th.fg).add_modifier(Modifier::BOLD));
        text(buf, side, y + 2, sw, &streak, streak_st);
        y += 4;
    } else {
        crate::tile::figure(buf, crate::balance::centre(x, w, fw), y, w, &fig, big, th.s(th.dim));
        y += 3;
        let both = format!("{plays} · {streak}");
        if both.width() as u16 <= w {
            centred(buf, y, &both, th.s(th.fg));
            y += 2;
        } else {
            centred(buf, y, &plays, th.s(th.fg));
            centred(buf, y + 1, &streak, streak_st);
            y += 3;
        }
    }
    // how today compares with the week before it
    let prev = &sum.minutes[sum.minutes.len().saturating_sub(8)..sum.minutes.len().saturating_sub(1)];
    let avg = if prev.is_empty() { 0.0 } else { prev.iter().sum::<u32>() as f32 / prev.len() as f32 };
    if y + 5 < r.bottom() && avg > 0.0 {
        let pct = ((mins as f32 / avg - 1.0) * 100.0).round() as i32;
        let (arrow, st) = if pct >= 0 { ('▲', th.s(th.accent2)) } else { ('▼', th.s(th.dim)) };
        let line = format!("{arrow} {}% vs the 7-day average ({}/day)", pct.abs(), short_minutes(avg.round() as u32));
        centred(buf, y, &line, st);
        y += 2;
    }
    // the last week in a line, when the chart has rows to give
    if r.bottom().saturating_sub(y) >= 14 {
        let week = &sum.minutes[sum.minutes.len().saturating_sub(7)..];
        let total: u32 = week.iter().sum();
        let (best_i, best) = week.iter().enumerate().max_by_key(|(_, m)| **m).map(|(i, m)| (i, *m)).unwrap_or((0, 0));
        let wd = (sum.today_weekday as usize + 7 * 2 - (week.len() - 1 - best_i)) % 7;
        // a narrow Pane: the short label and figure, so the total isn't cut to its first digit
        let (label, tw) = if 12 + fmt_minutes(total).width() as u16 <= w { ("last 7 days ", fmt_minutes(total)) } else { ("7 days ", short_minutes(total)) };
        let lw = label.width() as u16;
        let b = format!("{} ({})", WEEKDAY_NAMES[wd], short_minutes(best));
        let best_shown = best > 0 && lw + tw.width() as u16 + 24 <= w;
        // the line in the middle of the width (#58)
        let line_w = (lw + tw.width() as u16 + if best_shown { 12 + b.width() as u16 } else { 0 }).min(w);
        let lx = crate::balance::centre(x, w, line_w);
        text(buf, lx, y, lw, label, th.s(th.dim));
        text(buf, lx + lw, y, (x + w).saturating_sub(lx + lw), &tw, th.s(th.fg).add_modifier(Modifier::BOLD));
        let bx = lx + lw + tw.width() as u16;
        if best_shown {
            text(buf, bx, y, 13, " · best day ", th.s(th.dim));
            text(buf, bx + 12, y, (x + w).saturating_sub(bx + 12), &b, th.s(th.fg));
        }
        y += 2;
    }
    // the last days as columns, today the accent; weekday initials under them
    let days = if w >= 84 { 28 } else if w >= 48 { 14 } else { 7 }.min(sum.minutes.len());
    let rows = r.bottom().saturating_sub(y + 1);
    if rows < 2 || days == 0 {
        return;
    }
    let shown = &sum.minutes[sum.minutes.len() - days..];
    let max = shown.iter().copied().max().unwrap_or(0).max(1);
    let pitch = (w / days as u16).max(1);
    let gap = u16::from(pitch >= 3);
    let cw = pitch - gap;
    let left = x + (w - pitch * days as u16) / 2;
    let base = y + rows - 1;
    // the values sit over the columns when there's a row free above the tallest and the columns are wide enough
    let labels = cw >= 4 && rows >= 4;
    let col_rows = if labels { rows - 1 } else { rows };
    let faint = th.bg.lerp(th.border, 0.7);
    for (i, m) in shown.iter().enumerate() {
        let cx = left + i as u16 * pitch;
        let is_today = i == days - 1;
        let t = *m as f32 / max as f32;
        if *m == 0 {
            for c in 0..cw {
                text(buf, cx + c, base, 1, "▁", th.s(faint));
            }
        } else {
            let col = if is_today { th.accent } else { th.ramp_at(0.3 + 0.6 * t) };
            crate::tile::column(buf, cx, base, cw, col_rows, t, col, th);
        }
        if labels && *m > 0 {
            let v = short_minutes(*m);
            let top = base.saturating_sub(((t * col_rows as f32).ceil() as u16).max(1));
            if v.width() as u16 <= cw + gap {
                text(buf, cx, top, cw + gap, &v, th.s(if is_today { th.accent } else { th.dim }));
            }
        }
        let back = days - 1 - i;
        let wd = (sum.today_weekday as usize + 7 * 5 - back) % 7;
        if days <= 14 || wd == 0 || is_today {
            let st = if is_today { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.border) };
            if cw >= 3 && days <= 7 {
                text(buf, cx, base + 1, cw, WEEKDAY_NAMES[wd], st);
            } else {
                text(buf, cx, base + 1, 1, WEEKDAYS[wd], st);
            }
        }
    }
}

/// "today" in a small Pane: three lines and a 7-day sparkline, as many as fit.
#[allow(clippy::too_many_arguments)]
fn today_compact(buf: &mut Buffer, r: Rect, sum: &singularity_proto::ListeningSummary, mins: u32, plays: &str, streak: &str, streak_st: ratatui::style::Style, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    let headline = if mins == 0 && sum.plays_today > 0 { "under a minute today".to_string() } else { format!("{} today", fmt_minutes(mins)) };
    let lines = [(headline.as_str(), th.s(th.accent).add_modifier(Modifier::BOLD)), (plays, th.s(th.fg)), (streak, streak_st)];
    // the lines alone (no room for the days under them or beside them): each in the middle of the width (#58)
    let alone = r.height <= 4 && !(r.height >= 2 && w >= 48);
    let mut y = r.y;
    for (t, st) in lines {
        if y >= r.bottom() {
            return;
        }
        let t = clip(t, w as usize);
        let lx = if alone { crate::balance::centre(x, w, t.width() as u16) } else { x };
        text(buf, lx, y, (x + w).saturating_sub(lx), &t, st);
        y += 1;
    }
    // wide and short: the last 7 days as columns at the right, beside the lines
    let week = &sum.minutes[sum.minutes.len().saturating_sub(7)..];
    let max = week.iter().copied().max().unwrap_or(0).max(1);
    if y + 1 >= r.bottom() && r.height >= 2 && w >= 48 {
        let lines_w = [headline.width(), plays.width(), streak.width()].into_iter().max().unwrap_or(0) as u16;
        let x0 = x + w - 7 * 3;
        if x0 > x + lines_w + 2 {
            let base = r.bottom() - 2;
            for (i, m) in week.iter().enumerate() {
                let is_today = i == week.len() - 1;
                let t = *m as f32 / max as f32;
                let col = if is_today { th.accent } else { th.ramp_at(0.3 + 0.7 * t) };
                let cx = x0 + i as u16 * 3;
                if *m == 0 {
                    text(buf, cx, base, 2, "▁▁", th.s(th.border));
                } else {
                    crate::tile::column(buf, cx, base, 2, r.height - 1, t, col, th);
                }
                let day = (sum.today_weekday as usize + 7 - (week.len() - 1 - i)) % 7;
                text(buf, cx, base + 1, 1, WEEKDAYS[day], th.s(if is_today { th.accent } else { th.border }));
            }
        }
        return;
    }
    // the last 7 days, oldest first, with weekday initials under them when there's a row for them
    if y + 1 < r.bottom() {
        y += 1;
        const EIGHTHS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
        let cw = (w / 7).clamp(1, 3);
        for (i, m) in week.iter().enumerate() {
            let t = *m as f32 / max as f32;
            let ch = if *m == 0 { "·" } else { EIGHTHS[((t * 8.0).round() as usize).clamp(1, 8)] };
            let is_today = i == week.len() - 1;
            let st = th.s(if *m == 0 { th.border } else if is_today { th.accent } else { th.ramp_at(0.3 + 0.7 * t) });
            let cx = x + i as u16 * cw;
            for c in 0..cw.saturating_sub(u16::from(cw > 1)).max(1) {
                text(buf, cx + c, y, 1, ch, st);
            }
            if y + 1 < r.bottom() {
                let day = (sum.today_weekday as usize + 7 - (week.len() - 1 - i)) % 7;
                text(buf, cx, y + 1, 1, WEEKDAYS[day], th.s(if is_today { th.accent } else { th.border }));
            }
        }
    }
}

/// The "top this week" Widget: the week's top artists or tracks (`tracks`), each a clickable row. Small Panes get
/// one line per entry; tall ones a summary line and two lines per entry (the name, then a bar of its minutes), the
/// leader in the accent, as many entries as fit.
pub fn top_week(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    if r.height == 0 || w < 6 {
        return;
    }
    let Some(s) = app.stats.get(&StatsPeriod::Week).cloned() else {
        text(buf, x, r.y, w, "counting…", th.s(th.dim));
        return;
    };
    let rows = if app.top_week_tracks { &s.top_tracks } else { &s.top_artists };
    if rows.is_empty() {
        text(buf, x, r.y, w, &clip("nothing yet this week", w as usize), th.s(th.dim));
        return;
    }
    let sec = if app.top_week_tracks { Section::Tracks } else { Section::Artists };
    let peak = rows[0].minutes.max(1);
    let mut hits = vec![];
    // two lines an entry, once at least four entries fit that way
    let tall = w >= 26 && r.height >= 2 + 4 * 2;
    // more entries than fit: the wheel scrolls them
    if !tall {
        let len = sec.rows(&s).len();
        let first = app.top_week_scroll.window(len, r.height as usize);
        // fewer entries than rows: the list in the middle of the Pane's height (#58)
        let top = crate::balance::centre(r.y, r.height, len.saturating_sub(first).min(r.height as usize) as u16);
        for i in first..len.min(first + r.height as usize) {
            let rect = Rect::new(x, top + (i - first) as u16, w, 1);
            row(buf, rect, sec, i, &s, peak, false, th);
            hits.push((rect, Hit::TopWeek(i)));
        }
    } else {
        let n = rows.len().min(((r.height - 2) / 2) as usize).min(25);
        // when in the day this week's listening happened, under the entries when there are rows for it
        let list_h = 2 + n as u16 * 2 + 1;
        let spare = r.height.saturating_sub(list_h);
        let hours_shown = spare >= 7 && w >= 30 && s.by_hour.iter().any(|m| *m > 0);
        // more rows than the chart usually takes (12): it steps up, and the whole block sits in the middle (#58)
        let roomy = spare > 12;
        let hours_h = match (hours_shown, roomy) {
            (true, true) => crate::balance::step(&[12, 16, 20, 24], |h| list_h + h + 4 <= r.height),
            (true, false) => spare,
            (false, _) => 0,
        };
        // (the list's last row is the gap over the chart)
        let block = if hours_h > 0 { list_h + hours_h } else { list_h - 1 };
        let top = if roomy { crate::balance::centre(r.y, r.height, block) } else { r.y };
        // the week in one line: minutes and plays, the dates at the right
        let total = format!("{} · {} plays", fmt_minutes(s.minutes), s.plays);
        text(buf, x, top, w, &clip(&total, w as usize), th.s(th.fg).add_modifier(Modifier::BOLD));
        let dates = range(s.from_day, s.to_day);
        if total.width() + dates.width() + 3 <= w as usize {
            text(buf, x + w - dates.width() as u16, top, dates.width() as u16, &dates, th.s(th.border));
        }
        crate::tile::rule(buf, x, top + 1, w, th);
        let first = app.top_week_scroll.window(rows.len(), n);
        let rank_w: u16 = if first + n >= 10 { 3 } else { 2 };
        for (i, it) in rows.iter().enumerate().skip(first).take(n) {
            let y = top + 2 + (i - first) as u16 * 2;
            let lead = i == 0;
            let rank_st = if i < 3 { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.border) };
            text(buf, x, y, rank_w, &format!("{:>w$}", i + 1, w = rank_w as usize - 1), rank_st);
            let nx = x + rank_w + 1;
            let nw = (x + w).saturating_sub(nx);
            // the right side: plays (and all-time plays for a track, when there's room)
            let plays = if sec == Section::Tracks && it.all_time_plays > it.plays && w >= 48 { format!("{}× · {} ever", it.plays, it.all_time_plays) } else { format!("{}×", it.plays) };
            let pw = plays.width() as u16;
            let room = nw.saturating_sub(pw + 2) as usize;
            let name = clip(&it.name, room);
            let name_st = if lead { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.fg).add_modifier(Modifier::BOLD) };
            text(buf, nx, y, room as u16, &name, name_st);
            if !it.subtitle.is_empty() && name.width() + 4 < room {
                let sub = clip(&format!(" · {}", it.subtitle), room - name.width());
                text(buf, nx + name.width() as u16, y, (room - name.width()) as u16, &sub, th.s(th.dim));
            }
            if pw + 1 < nw {
                text(buf, x + w - pw, y, pw, &plays, th.s(th.dim));
            }
            // the bar, its length the minutes against the leader's, the minutes after it
            let mins = short_minutes(it.minutes);
            let bw = nw.saturating_sub(mins.width() as u16 + 1);
            let t = it.minutes as f32 / peak as f32;
            let fill = if lead { th.accent } else { th.ramp_at(0.35 + 0.55 * t) };
            let cells = ((t * bw as f32).ceil() as u16).clamp(1, bw.max(1));
            crate::tile::hbar(buf, nx, y + 1, cells, (t * bw as f32) / cells as f32, fill, None, th);
            text(buf, nx + cells + 1, y + 1, mins.width() as u16, &mins, th.s(if lead { th.accent } else { th.dim }));
            hits.push((Rect::new(x, y, w, 2), Hit::TopWeek(i)));
        }
        if hours_h > 0 {
            hours(buf, Rect::new(x, top + list_h, w, hours_h), &s.by_hour, th);
        }
    }
    for (rect, h) in hits {
        app.hit(rect, h);
    }
}

/// A 24-hour column chart of `by_hour` (minutes per hour of the day) in `r`: a title, the columns, hour labels.
fn hours(buf: &mut Buffer, r: Rect, by_hour: &[u32], th: &Theme) {
    crate::tile::rule(buf, r.x, r.y, r.width, th);
    text(buf, r.x, r.y + 1, r.width, "when you listened", th.s(th.dim));
    let pitch = (r.width / 24).clamp(1, 4);
    let gap = u16::from(pitch >= 2);
    let x0 = r.x + (r.width - pitch * 24) / 2;
    let rows = r.height.saturating_sub(3);
    let base = r.y + 2 + rows - 1;
    let max = by_hour.iter().copied().max().unwrap_or(0).max(1);
    let peak_h = by_hour.iter().enumerate().max_by_key(|(_, m)| **m).map(|(h, _)| h).unwrap_or(0);
    for (h, m) in by_hour.iter().take(24).enumerate() {
        let t = *m as f32 / max as f32;
        let cx = x0 + h as u16 * pitch;
        let col = if h == peak_h { th.accent } else { th.ramp_at(0.3 + 0.6 * t) };
        if *m == 0 {
            for c in 0..pitch - gap {
                text(buf, cx + c, base, 1, "▁", th.s(th.bg.lerp(th.border, 0.7)));
            }
        } else {
            crate::tile::column(buf, cx, base, pitch - gap, rows, t, col, th);
        }
    }
    for h in [0u16, 6, 12, 18] {
        text(buf, x0 + h * pitch, base + 1, 2, ["00", "06", "12", "18"][h as usize / 6], th.s(th.border));
    }
    let peak = format!("most at {peak_h:02}:00");
    let pw = peak.width() as u16;
    if pw + 18 < r.width {
        text(buf, r.right() - pw, r.y + 1, pw, &peak, th.s(th.dim));
    }
}

/// One ranked row: rank, name (and artist), then minutes, a bar and plays as room allows.
#[allow(clippy::too_many_arguments)]
fn row(buf: &mut Buffer, r: Rect, sec: Section, i: usize, s: &Stats, peak: u32, selected: bool, th: &Theme) {
    let it = &sec.rows(s)[i];
    let bg = if selected { th.selection.color() } else { th.bg.color() };
    blank(buf, r.x, r.y, r.width, th.base().bg(bg));
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
    let mut rx = r.right().saturating_sub(right_w as u16).max(r.x);
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

/// "today" in a narrow, tall Pane (#76, the owner's option B): the minutes as a figure, plays and the streak under
/// it, then the last days as bars across, one a row (two rows apart where there's room; 14 days where the Pane is tall
/// enough, else 7), the weekday at the left and the minutes at the right, today in the accent; the whole in the middle
/// of the Pane.
#[allow(clippy::too_many_arguments)]
fn today_bars(buf: &mut Buffer, r: Rect, sum: &singularity_proto::ListeningSummary, mins: u32, plays: &str, streak: &str, streak_st: ratatui::style::Style, th: &Theme) {
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    let fig = if mins >= 60 { format!("{}h{:02}m", mins / 60, mins % 60) } else { format!("{mins}m") };
    let fw = crate::tile::figure_width(&fig).min(w);
    // the figure and a blank row, plays, the streak and a blank row
    let head = 3 + 1 + 2 + 1;
    let days = if r.height >= head + 2 * 14 + 2 { 14 } else { 7 }.min(sum.minutes.len());
    if days == 0 {
        return;
    }
    let pitch = if r.height > head + 2 * days as u16 { 2 } else { 1 };
    let block = head + days as u16 * pitch - (pitch - 1);
    let mut y = r.y + r.height.saturating_sub(block) / 2;
    crate::tile::figure(buf, crate::balance::centre(x, w, fw), y, w, &fig, th.s(th.accent).add_modifier(Modifier::BOLD), th.s(th.dim));
    y += 4;
    for (s, st) in [(plays, th.s(th.fg)), (streak, streak_st)] {
        let s = clip(s, w as usize);
        text(buf, crate::balance::centre(x, w, s.width() as u16), y, w, &s, st);
        y += 1;
    }
    y += 1;
    let shown = &sum.minutes[sum.minutes.len() - days..];
    let max = shown.iter().copied().max().unwrap_or(0).max(1);
    // the weekday (3 columns and a gap), the bar, the minutes (up to 5 columns)
    let bw = w.saturating_sub(4 + 5);
    for (i, m) in shown.iter().enumerate() {
        let is_today = i == days - 1;
        let wd = (sum.today_weekday as usize + 7 * 5 - (days - 1 - i)) % 7;
        text(buf, x, y, 3, WEEKDAY_NAMES[wd], th.s(if is_today { th.accent } else { th.border }));
        let t = *m as f32 / max as f32;
        let col = if is_today { th.accent } else { th.ramp_at(0.3 + 0.6 * t) };
        let eighths = (t * bw as f32 * 8.0).round() as u16;
        for k in 0..bw {
            let e = eighths.saturating_sub(k * 8).min(8) as usize;
            let (ch, st) = if e == 0 { (if k == 0 { '▏' } else { ' ' }, th.s(th.border)) } else { (crate::tile::H_EIGHTHS[e], th.s(col)) };
            if let Some(c) = buf.cell_mut((x + 4 + k, y)) {
                c.set_char(ch).set_style(st);
            }
        }
        let v = if *m == 0 { "·".to_string() } else { short_minutes(*m) };
        let vw = v.width() as u16;
        text(buf, (x + w).saturating_sub(vw), y, vw, &v, th.s(if is_today { th.accent } else { th.dim }));
        y += pitch;
    }
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

    fn summary() -> singularity_proto::ListeningSummary {
        let mut minutes: Vec<u32> = (0..singularity_proto::LISTENING_DAYS as u32).map(|i| i * 7 % 40).collect();
        *minutes.last_mut().unwrap() = 0;
        singularity_proto::ListeningSummary { minutes, today_weekday: 1, hours_this_year: 19.0, top_artist: Some("Mira Vale".into()), streak_days: 10, plays: 534, plays_today: 0, backup_short: None }
    }

    fn week() -> Stats {
        let row = |name: &str, minutes: u32, plays: u32| StatsRow { name: name.into(), minutes, plays, ..Default::default() };
        Stats {
            period: StatsPeriod::Week,
            from_day: 20780,
            to_day: 20781,
            minutes: 42,
            plays: 18,
            by_hour: (0..24).map(|h| if (7..10).contains(&h) { 10 + h } else { 0 }).collect(),
            top_artists: vec![row("Glass Harbour", 13, 5), row("Saffron Echo", 11, 5), row("The Quiet Orbit", 7, 3), row("Nova Drift", 7, 3), row("Mira Vale", 4, 2)],
            ..Default::default()
        }
    }

    /// The rows of `r` in `buf`, as text.
    fn rows(buf: &Buffer, r: Rect) -> Vec<String> {
        (r.top()..r.bottom()).map(|y| (r.left()..r.right()).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect()
    }

    #[test]
    fn today_puts_its_count_in_the_middle() {
        // #58: the count clung to the top-left corner
        let mut app = App::new(None);
        app.listening = Some(summary());
        let th = app.theme.clone();
        let draw = |w: u16, h: u16| {
            let r = Rect::new(2, 1, w, h);
            let mut buf = Buffer::empty(Rect::new(0, 0, w + 4, h + 2));
            today(&mut buf, r, &app, &th);
            (buf, r)
        };
        // tiny: each line in the middle
        let (buf, r) = draw(18, 4);
        for (i, line) in rows(&buf, r).iter().enumerate().take(3) {
            let (left, right) = (line.len() - line.trim_start().len(), line.len() - line.trim_end().len());
            assert!(left.abs_diff(right) <= 1, "tiny, line {i}: {line:?}");
        }
        // with the figure: the headline (the figure and its words) in the middle of the Pane's width
        // (a narrow Pane under 24 rows: the columns still; taller, the bars below)
        for (w, h) in [(22, 20), (58, 18), (118, 38)] {
            let (buf, r) = draw(w, h);
            let head = Rect::new(r.x, r.y, r.width, 3);
            let (_, _, left, right) = crate::balance::margins(&buf, head);
            assert!(left >= 3 && left.abs_diff(right) <= 2, "{w}x{h}: {left} columns left of the headline, {right} right\n{}", rows(&buf, r).join("\n"));
        }
    }

    #[test]
    fn a_narrow_tall_today_lays_its_days_sideways_in_the_middle() {
        // #76 (option B): a bar a row, the weekday at the left, the minutes at the right, today last
        let mut app = App::new(None);
        app.listening = Some(summary());
        let th = app.theme.clone();
        for (w, h, days) in [(16u16, 30u16, 7usize), (24, 30, 7), (36, 30, 7), (30, 40, 14)] {
            let r = Rect::new(2, 1, w, h);
            let mut buf = Buffer::empty(Rect::new(0, 0, w + 4, h + 2));
            today(&mut buf, r, &app, &th);
            let lines = rows(&buf, r);
            let all = lines.join("\n");
            let bars: Vec<&String> = lines.iter().filter(|l| WEEKDAY_NAMES.iter().any(|d| l.trim_start().starts_with(d))).collect();
            assert_eq!(bars.len(), days, "{w}x{h}: a row a day\n{all}");
            assert!(bars.last().unwrap().trim_start().starts_with("Tue") && bars.last().unwrap().trim_end().ends_with('·'), "{w}x{h}: today last, nothing yet\n{all}");
            assert!(bars.iter().any(|l| l.contains('█')) && bars.iter().any(|l| l.trim_end().ends_with('m')), "{w}x{h}: bars and minutes\n{all}");
            assert!(lines.iter().any(|l| l.trim() == "0 plays") && lines.iter().any(|l| l.trim() == "10-day streak"), "{w}x{h}\n{all}");
            let (over, under, ..) = crate::balance::margins(&buf, r);
            assert!(over.abs_diff(under) <= 1, "{w}x{h}: {over} rows over, {under} under\n{all}");
        }
        // a row short or a column wide of it: the columns as before
        for (w, h) in [(24u16, 23u16), (37, 30)] {
            let r = Rect::new(2, 1, w, h);
            let mut buf = Buffer::empty(Rect::new(0, 0, w + 4, h + 2));
            today(&mut buf, r, &app, &th);
            assert!(!rows(&buf, r).iter().any(|l| l.trim_start().starts_with("Tue")), "{w}x{h}");
        }
    }

    #[test]
    fn top_this_week_sits_balanced_and_grows_in_a_big_pane() {
        // #58: a narrow, tall Pane left the space under the list empty; big ones stopped growing and left the bottom empty
        let mut app = App::new(None);
        app.stats.insert(StatsPeriod::Week, week());
        let th = app.theme.clone();
        let mut draw = |w: u16, h: u16| {
            let r = Rect::new(2, 1, w, h);
            let mut buf = Buffer::empty(Rect::new(0, 0, w + 4, h + 2));
            app.hits.clear();
            top_week(&mut buf, r, &mut app, &th);
            let first = app.hits.iter().find_map(|(rect, h)| (*h == Hit::TopWeek(0)).then_some(rect.y));
            (crate::balance::margins(&buf, r), first, buf, r)
        };
        let ((over, under, ..), first, ..) = draw(22, 28);
        assert!(over >= 8 && over.abs_diff(under) <= 1, "narrow-tall: {over} rows over, {under} under");
        assert_eq!(first, Some(1 + over), "the click follows the row");
        for (w, h) in [(118, 38), (178, 47)] {
            let ((over, under, ..), _, buf, r) = draw(w, h);
            assert!(over.abs_diff(under) <= 1 && over + under <= h / 3, "{w}x{h}: {over} rows over, {under} under\n{}", rows(&buf, r).join("\n"));
        }
        // medium, tiny and wide-short: as before, from the top
        for (w, h) in [(58, 18), (18, 4), (98, 6)] {
            assert_eq!(draw(w, h).0 .0, 0, "{w}x{h}");
        }
    }

    #[test]
    fn clipping_counts_display_columns() {
        assert_eq!(clip("Nova Drift", 20), "Nova Drift");
        assert_eq!(clip("Nova Drift", 6), "Nova …");
        assert!(clip("光が曲がる光が曲がる", 7).width() <= 7);
    }
}
