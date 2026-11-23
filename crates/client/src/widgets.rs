//! The Widgets the other designs bring (#18): Classic+'s Now playing, Cockpit's levels (and its listening Pane, filled
//! by #23), Terminal's rain, scope and `top`. Colours come from the theme; motion from the Daemon's spectrum frames.

use crate::app::App;
use crate::theme::{Rgb, Theme};
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use std::collections::HashMap;
use std::time::Instant;

/// Widget state that lives across frames.
#[derive(Default)]
pub struct State {
    rain: Rain,
    scope: Vec<f32>,
    /// when the scope was last drawn: its persistence is per second, not per frame
    scope_at: Option<Instant>,
    top: Top,
}

fn ramp(th: &Theme, t: f32) -> Rgb {
    th.ramp_at(t)
}

/// bass, mid and treble (0..=1) at the real output level (they follow the volume).
fn bands3(app: &App) -> [f32; 3] {
    app.meters.frame.as_ref().map(|f| f.out_bands.map(|v| v as f32 / 255.0)).unwrap_or([0.0; 3])
}

// ── Now playing (Classic+) ────────────────────────────────────────────

/// Cover, title and artists, and what's next (only while no queue Pane is on screen: the layout is checked every
/// frame, so opening or closing one takes effect at once). No meters of its own: a Pane holds one thing. A Pane too
/// small for a cover gets the words; a wide one puts the cover at its left and the rest beside it; a tall one stacks
/// them; a big one spells the title in the pixel font.
pub fn nowplaying(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let m = u16::from(r.width >= 12);
    let (x, w) = (r.x + m, r.width.saturating_sub(2 * m));
    if w == 0 || r.height == 0 {
        return;
    }
    let Some(item) = app.playback.item.clone() else {
        text(buf, x, r.y + r.height / 2, w, "nothing playing", th.s(th.dim));
        return;
    };
    let queue = np_queue(app);
    let url = item.cover_url.clone().unwrap_or_default();
    // wide: a cover as tall as the Pane (twice as wide in cells: about square), the rest beside it
    if r.height >= 4 && w >= r.height * 2 + 24 {
        let cw = r.height * 2;
        crate::home::cover(buf, Rect::new(x, r.y, cw, r.height), &url, "♪", app, th);
        let ix = x + cw + 2;
        return np_info(buf, Rect::new(ix, r.y, (x + w).saturating_sub(ix), r.height), &item, app, queue, th);
    }
    // tall: a square cover on top, as big as leaves the words (and, taller, what's next) their rows
    if r.height >= 8 && w >= 8 {
        let q = if queue && r.height >= 14 { (app.playback.queue.len() as u16 + 2).min(9).min(r.height / 3) } else { 0 };
        let ch = (w / 2).min(r.height.saturating_sub(4 + q)).max(2);
        let cw = (ch * 2).min(w);
        crate::home::cover(buf, Rect::new(x + (w - cw) / 2, r.y, cw, ch), &url, "♪", app, th);
        return np_info(buf, Rect::new(x, r.y + ch + 1, w, r.height - ch - 1), &item, app, queue, th);
    }
    np_info(buf, Rect::new(x, r.y, w, r.height), &item, app, queue, th);
}

/// Whether Now playing lists what's next: there is something queued and no queue Pane already shows it.
fn np_queue(app: &App) -> bool {
    !app.playback.queue.is_empty() && !app.layout.has(crate::layout::Widget::Queue)
}

/// Now playing's words in `r`: the title (in the pixel font when there's room for it big), artists, where it plays
/// from, then what's next (if `queue`) in the rows left.
fn np_info(buf: &mut Buffer, r: Rect, item: &singularity_proto::Item, app: &App, queue: bool, th: &Theme) {
    if r.width == 0 || r.height == 0 {
        return;
    }
    let bottom = r.bottom();
    let mut y = r.y;
    let big = if r.height >= 24 && r.width >= 40 { (2..=4u16).rev().find_map(|s| crate::ambient::pixel_fit(&item.name, r.width, r.height / 3, s, 2).map(|l| (s, l))) } else { None };
    match big {
        Some((s, lines)) => {
            for line in &lines {
                crate::ambient::pixel_line(buf, r.x, y, r.width, line, s, 0, None, 0.0, th);
                y += crate::ambient::pixel_rows(s) + 1;
            }
        }
        None => {
            text(buf, r.x, y, r.width, &item.name, th.s(th.fg).add_modifier(Modifier::BOLD));
            y += 1;
        }
    }
    let mut line = |buf: &mut Buffer, s: &str| {
        if y < bottom {
            text(buf, r.x, y, r.width, s, th.s(th.dim));
            y += 1;
        }
    };
    line(buf, &item.artists.join(", "));
    if let Some(c) = &app.playback.context_name {
        line(buf, &format!("from {c}"));
    }
    y += 1;
    if queue && y + 1 < bottom {
        text(buf, r.x, y, r.width, "Next in queue", th.s(th.fg).add_modifier(Modifier::BOLD));
        y += 1;
        for q in app.playback.queue.iter().take(bottom.saturating_sub(y) as usize) {
            text(buf, r.x, y, 2.min(r.width), "■", th.s(th.accent));
            text(buf, r.x + 2, y, r.width.saturating_sub(2), &format!("{} · {}", q.item.name, q.item.artists.join(", ")), th.s(th.dim));
            y += 1;
        }
    }
}

// ── levels (Cockpit) ──────────────────────────────────────────────────

/// Left/right peak meters, then bass, mid and treble. Taller Panes make the bars thicker; a wide, short one puts the
/// two groups side by side.
pub fn levels(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) {
    if r.width == 0 || r.height == 0 {
        return;
    }
    let vu = app.meters.frame.as_ref().map(|f| f.out_vu).unwrap_or([0, 0]);
    let [b, m, t] = bands3(app);
    let bars = [("L", vu[0] as f32 / 255.0), ("R", vu[1] as f32 / 255.0), ("bass", b), ("mid", m), ("high", t)];
    if r.height < 6 && r.width >= 48 {
        let half = r.width / 2;
        level_bars(buf, Rect::new(r.x, r.y, half - 1, r.height), &bars[..2], 2, false, th);
        level_bars(buf, Rect::new(r.x + half + 1, r.y, r.width - half - 1, r.height), &bars[2..], 3, false, th);
        return;
    }
    level_bars(buf, r, &bars, 2, true, th);
}

/// `bars` stacked in `r` with a blank row after the first `group` of them (when there's room), each as thick as the
/// rows allow (up to 6 rows, a blank row between thick ones), the whole centred top to bottom if `centre`.
fn level_bars(buf: &mut Buffer, r: Rect, bars: &[(&str, f32)], group: usize, centre: bool, th: &Theme) {
    let n = bars.len() as u16;
    let split = u16::from(group < bars.len());
    // the thickest bars that fit: n bars, a row between thick ones, and the gap between the groups
    let (thick, between, gap) = (1..=6u16)
        .rev()
        .map(|t| (t, u16::from(t >= 2), split * t.div_ceil(2)))
        .find(|&(t, sp, g)| n * t + (n - 1) * sp + g <= r.height)
        .unwrap_or((1, 0, u16::from(r.height > n)));
    let need = n * thick + (n - 1) * between + gap;
    let mut y = r.y + if centre { r.height.saturating_sub(need) / 2 } else { 0 };
    // labels: words when there's room, initials when narrow, none when tiny
    let label_w: u16 = if r.width >= 16 { 6 } else if r.width >= 6 { 2 } else { 0 };
    let w = r.width - label_w;
    for (i, (label, v)) in bars.iter().enumerate() {
        if i == group && i > 0 {
            y += gap;
        }
        if y >= r.bottom() {
            break;
        }
        let rows = thick.min(r.bottom() - y);
        let lit = (v.clamp(0.0, 1.0) * w as f32 * 8.0).round() as u16;
        if label_w > 0 {
            let l = if label_w == 2 { &label[..1] } else { label };
            text(buf, r.x, y + (rows - 1) / 2, label_w, l, th.s(th.dim));
        }
        for row in 0..rows {
            for k in 0..w {
                let e = lit.saturating_sub(k * 8).min(8) as usize;
                let col = ramp(th, k as f32 / w.max(1) as f32);
                let (ch, st) = match (thick, e) {
                    (1, 0) => ('·', th.s(th.border)),
                    (1, _) => ('■', th.s(col)),
                    (_, 0) => ('█', th.s(th.bg.lerp(th.border, 0.25))),
                    (_, 8) => ('█', th.s(col)),
                    // the bar's tip in eighths, over the track
                    (_, e) => (crate::tile::H_EIGHTHS[e], th.s(col).bg(th.bg.lerp(th.border, 0.25).color())),
                };
                if let Some(c) = buf.cell_mut((r.x + label_w + k, y + row)) {
                    c.set_char(ch).set_style(st);
                }
            }
        }
        y += rows + between;
    }
}

/// Cockpit's listening Pane: minutes per day from the local listening log (#23), a column per week (Monday on top,
/// this week on the right), and a line of totals under it. Short Panes stack two days per cell (half-blocks).
pub fn listening(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) {
    if r.width < 4 || r.height < 2 {
        return;
    }
    if let Some(s) = app.listening.as_ref().filter(|s| s.plays > 0 && r.width >= 40 && r.height >= 12) {
        return listening_large(buf, r, s, th);
    }
    let sum = app.listening.as_ref();
    let weeks = (r.width / 2).min(26) as i64;
    let today_wd = sum.map(|s| s.today_weekday as i64).unwrap_or(6);
    let minutes = sum.map(|s| s.minutes.as_slice()).unwrap_or(&[]);
    let max = minutes.iter().copied().max().unwrap_or(0).max(1) as f32;
    // a day's colour: None later this week, the quiet colour for a day without listening, else the ramp
    let quiet = th.bg.lerp(th.dim, 0.3);
    let day = |c: i64, d: i64| -> Option<Rgb> {
        let back = (weeks - 1 - c) * 7 + today_wd - d;
        if back < 0 || d > 6 {
            return None;
        }
        let m = usize::try_from(minutes.len() as i64 - 1 - back).ok().and_then(|i| minutes.get(i)).copied().unwrap_or(0);
        Some(if m == 0 { quiet } else { ramp(th, 0.15 + 0.85 * (m as f32 / max).sqrt()) })
    };
    let half = r.height < 8;
    let rows = if half { 4.min(r.height - 1) } else { 7 };
    for c in 0..weeks {
        let x = r.x + 1 + c as u16 * 2;
        for row in 0..rows as i64 {
            let y = r.y + row as u16;
            if half {
                let (top, bottom) = (day(c, row * 2), day(c, row * 2 + 1));
                if let Some(cell) = buf.cell_mut((x, y))
                    && (top.is_some() || bottom.is_some())
                {
                    cell.set_char('▀').set_fg(top.unwrap_or(th.bg).color()).set_bg(bottom.unwrap_or(th.bg).color());
                }
            } else if let Some(col) = day(c, row) {
                text(buf, x, y, 1, "■", th.s(col));
            }
        }
    }
    let parts: Vec<String> = match sum {
        None => vec!["the listening log comes from singularityd".to_string()],
        Some(s) if s.plays == 0 => vec!["nothing logged yet".into(), "a track counts after 30 s".into()],
        Some(s) => {
            let h = if s.hours_this_year < 10.0 { format!("{:.1}", s.hours_this_year) } else { format!("{:.0}", s.hours_this_year) };
            let mut parts = vec![format!("{h} h this year")];
            parts.extend(s.top_artist.as_ref().map(|a| format!("top: {a}")));
            parts.push(format!("streak {} day{}", s.streak_days, if s.streak_days == 1 { "" } else { "s" }));
            parts
        }
    };
    // the totals on one line, or (a narrow Pane with rows to spare) as many as they need
    let tw = r.width.saturating_sub(2) as usize;
    let mut lines: Vec<String> = vec![];
    for p in parts {
        match lines.last_mut() {
            Some(l) if l.chars().count() + 3 + p.chars().count() <= tw => *l = format!("{l} · {p}"),
            _ => lines.push(p),
        }
    }
    let ty = r.y + rows + (r.height > 8) as u16;
    let room = r.bottom().saturating_sub(ty) as usize;
    if lines.len() > room {
        lines = vec![lines.join(" · ")];
    }
    for (i, l) in lines.iter().enumerate().take(room) {
        text(buf, r.x + 1, ty + i as u16, tw as u16, l, th.s(th.dim));
    }
}

/// The listening Pane when it has room: month names over the weeks, weekday names beside them, square-ish cells as
/// big as the Pane allows, a less → more legend, and the year's numbers under the grid (the hours as a figure when
/// there are rows for it). The parts stack with a row between them, the whole block centred in the Pane's height (the
/// owner, 2026-11-08: the numbers pinned to the bottom left "a ton of dead space" under the grid).
fn listening_large(buf: &mut Buffer, r: Rect, s: &singularity_proto::ListeningSummary, th: &Theme) {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const DAYS: [&str; 7] = ["Mon", "", "Wed", "", "Fri", "", "Sun"];
    let figure_footer = r.height >= 22 && r.width >= 56;
    let footer = if figure_footer { 4 } else { 3 };
    let (x0, w) = (r.x + 1, r.width.saturating_sub(2));
    let label_w = 4;
    let avail_h = r.height.saturating_sub(1 + footer);
    let weeks: u16 = 26;
    // a cell's pitch: square cells (a column is half a row in pixels), as big as the Pane allows; each cell's last
    // row is an upper half-block, so cells keep a half-row gap under them and a column's gap beside them
    let px_max = ((w - label_w) / weeks).max(2);
    let py = (avail_h / 7).min(px_max / 2).max(1);
    let px = (2 * py).min(px_max).max(2);
    let weeks = ((w - label_w) / px).min(weeks).max(1) as i64;
    let grid_w = weeks as u16 * px;
    let gx = x0 + label_w + (w - label_w).saturating_sub(grid_w) / 2;
    // the block's rows: months, the grid, the legend; the breakdown (a title and 7 rows) after a row's gap when it
    // fits; then the numbers (the cell footer starts with a blank row of its own, the figure footer gets one)
    const BREAKDOWN: u16 = 8;
    let grid_h = 1 + 7 * py + 1;
    let footer_gap = figure_footer as u16;
    let breakdown = r.height.saturating_sub(grid_h + footer_gap + footer) > BREAKDOWN; // a gap row and the breakdown
    let block = grid_h + if breakdown { 1 + BREAKDOWN } else { 0 } + footer_gap + footer;
    let top = r.y + r.height.saturating_sub(block) / 2;
    let gy = top + 1;
    let cw = px - 1;
    let today_wd = s.today_weekday as i64;
    let max = s.minutes.iter().copied().max().unwrap_or(0).max(1) as f32;
    let quiet = th.bg.lerp(th.dim, 0.25);
    let today_day = (singularity_proto::now_ms() / 86_400_000) as i64;
    let mut last_month = 0;
    let mut label_end = 0u16;
    for c in 0..weeks {
        // the column's Monday, days before today
        let monday_back = (weeks - 1 - c) * 7 + today_wd;
        let (_, month, _) = crate::stats::civil(today_day - monday_back);
        let mx = gx + c as u16 * px;
        if month != last_month && c + 3 < weeks && mx >= label_end {
            text(buf, mx, top, 3, MONTHS[(month - 1) as usize], th.s(th.dim));
            label_end = mx + 4;
        }
        last_month = month;
        for d in 0..7i64 {
            let back = monday_back - d;
            if back < 0 {
                continue;
            }
            let m = usize::try_from(s.minutes.len() as i64 - 1 - back).ok().and_then(|i| s.minutes.get(i)).copied().unwrap_or(0);
            let col = if m == 0 { quiet } else { th.ramp_at(0.15 + 0.85 * (m as f32 / max).sqrt()) };
            let (cx, cy) = (gx + c as u16 * px, gy + d as u16 * py);
            for yy in 0..py {
                let glyph = if yy + 1 == py { "▀" } else { "█" };
                for xx in 0..cw {
                    text(buf, cx + xx, cy + yy, 1, glyph, th.s(col));
                }
            }
            // today: marked in its cell
            if back == 0
                && cw >= 3
                && py >= 2
                && let Some(cell) = buf.cell_mut((cx + cw / 2, cy + (py - 1) / 2))
            {
                cell.set_char('•').set_fg(th.bg.color()).set_bg(col.color());
            }
        }
    }
    for (d, name) in DAYS.iter().enumerate() {
        text(buf, x0, gy + d as u16 * py + (py - 1) / 2, label_w - 1, name, th.s(th.border));
    }
    // the legend, right-aligned under the grid
    let ly = gy + 7 * py;
    let legend_w = 4 + 1 + 5 * 2 + 4;
    if grid_w >= legend_w + 10 && ly < r.bottom() {
        let lx = gx + grid_w - 1 - legend_w;
        text(buf, lx, ly, 4, "less", th.s(th.border));
        for i in 0..5u16 {
            let col = if i == 0 { quiet } else { th.ramp_at(0.15 + 0.85 * (i as f32 / 4.0)) };
            text(buf, lx + 5 + i * 2, ly, 1, "■", th.s(col));
        }
        text(buf, lx + 5 + 10, ly, 4, "more", th.s(th.border));
    }
    // rows to spare between the grid and the numbers: the average day by weekday, and the months, side by side
    let mut end = ly + 1;
    if breakdown {
        listening_breakdown(buf, Rect::new(x0, end + 1, w, BREAKDOWN), s, today_day, th);
        end += 1 + BREAKDOWN;
    }
    // the numbers: hours this year (as a figure when tall), the streak, the last 7 days, all plays, the top artist
    let fy = (end + footer_gap).min(r.bottom() - footer);
    let hours = if s.hours_this_year < 10.0 { format!("{:.1}", s.hours_this_year) } else { format!("{:.0}", s.hours_this_year) };
    let week: u32 = s.minutes.iter().rev().take(7).sum();
    let week_s = if week >= 60 { format!("{} h {:02} min", week / 60, week % 60) } else { format!("{week} min") };
    let streak = format!("{} day{}", s.streak_days, if s.streak_days == 1 { "" } else { "s" });
    let plays = s.plays.to_string();
    let mut tx = x0;
    let ty = if figure_footer {
        let fig = format!("{hours}h");
        let fw = crate::tile::figure(buf, x0, fy + 1, w, &fig, th.s(th.accent).add_modifier(Modifier::BOLD), th.s(th.dim));
        text(buf, x0, fy, fw.max(10), "this year", th.s(th.dim));
        tx += fw + 3;
        fy + 1
    } else {
        fy + 1
    };
    let year = format!("{hours} h");
    let mut tiles: Vec<(&str, &str, Rgb)> = vec![];
    if !figure_footer {
        tiles.push(("this year", &year, th.accent));
    }
    tiles.push(("last 7 days", &week_s, th.fg));
    tiles.push(("streak", &streak, if s.streak_days > 1 { th.accent2 } else { th.fg }));
    tiles.push(("plays", &plays, th.fg));
    if let Some(a) = &s.top_artist {
        tiles.push(("top artist", a, th.fg));
    }
    for (label, value, col) in tiles {
        let tw = label.chars().count().max(value.chars().count()) as u16 + 3;
        if tx + tw - 2 > x0 + w {
            break;
        }
        crate::tile::stat(buf, tx, ty, tw - 2, label, value, th.s(col), th);
        tx += tw;
    }
}

/// Under a big listening grid: the average day for each weekday, and the minutes of each month, as bars.
fn listening_breakdown(buf: &mut Buffer, r: Rect, s: &singularity_proto::ListeningSummary, today_day: i64, th: &Theme) {
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let n = s.minutes.len();
    // by weekday: the average over the days logged
    let mut sums = [0u32; 7];
    let mut counts = [0u32; 7];
    for (i, m) in s.minutes.iter().enumerate() {
        let back = (n - 1 - i) as i64;
        let wd = (s.today_weekday as i64 - back).rem_euclid(7) as usize;
        sums[wd] += m;
        counts[wd] += 1;
    }
    // by month: the last six months, the oldest first (the oldest may hold only its logged days)
    let mut months: [(u32, u32); 6] = [(0, 0); 6];
    let (ty, tm, _) = crate::stats::civil(today_day);
    let month_index = |y: i64, m: u32| y * 12 + m as i64 - 1;
    let now_idx = month_index(ty, tm);
    for (i, m) in s.minutes.iter().enumerate() {
        let (y, mo, _) = crate::stats::civil(today_day - (n - 1 - i) as i64);
        let k = now_idx - month_index(y, mo);
        if (0..6).contains(&k) {
            let slot = &mut months[5 - k as usize];
            *slot = (mo, slot.1 + m);
        }
    }
    let two = r.width >= 70;
    let colw = if two { r.width / 2 - 2 } else { r.width };
    let short = |m: u32| if m >= 60 { format!("{}h{:02}", m / 60, m % 60) } else { format!("{m}m") };
    let bars = |buf: &mut Buffer, x: u16, title: &str, rows: &mut dyn Iterator<Item = (&str, u32, String)>, peak: u32| {
        text(buf, x, r.y, colw, title, th.s(th.dim));
        let peak = peak.max(1);
        for (i, (name, v, val)) in rows.enumerate() {
            let y = r.y + 1 + i as u16;
            if y >= r.bottom() {
                break;
            }
            text(buf, x, y, 4, name, th.s(th.border));
            let bw = colw.saturating_sub(5 + 7);
            let t = v as f32 / peak as f32;
            let fill = if v == peak { th.accent } else { th.ramp_at(0.3 + 0.6 * t) };
            crate::tile::hbar(buf, x + 4, y, bw, t, fill, Some(th.bg.lerp(th.border, 0.4)), th);
            text(buf, x + 5 + bw, y, 7, &val, th.s(if v == peak { th.accent } else { th.dim }));
        }
    };
    let avg = |d: usize| sums[d] / counts[d].max(1);
    let peak = (0..7).map(avg).max().unwrap_or(0);
    bars(buf, r.x, "the average day", &mut (0..7).map(|d| (DAYS[d], avg(d), short(avg(d)))), peak);
    if two {
        let peak = months.iter().map(|m| m.1).max().unwrap_or(0);
        bars(buf, r.x + colw + 4, "by month", &mut months.iter().filter(|m| m.0 > 0).map(|(mo, m)| (MONTHS[(*mo - 1) as usize], *m, format!("{} h", m / 60))), peak);
    }
}

// ── rain (Terminal) ───────────────────────────────────────────────────

fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xff_ffff) as f32 / 16_777_215.0
}

/// Falling glyph columns that speed up with the music.
#[derive(Default)]
struct Rain {
    /// per column: head row, speed (rows/s), trail length
    cols: Vec<(f32, f32, u16)>,
    width: u16,
    last: Option<Instant>,
    t: f32,
    /// how much the music speeds it up now, eased (no jumps on beats)
    boost: f32,
}

const GLYPHS: &str = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789Z:.=*+<>";

pub fn rain(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let energy = app.meters.frame.as_ref().map(|f| f.level as f32 / 255.0).unwrap_or(0.0);
    let beat = app.meters.frame.as_ref().is_some_and(|f| f.beat);
    let s = &mut app.widgets.rain;
    let dt = s.last.map(|l| l.elapsed().as_secs_f32().min(0.1)).unwrap_or(0.0);
    s.last = Some(Instant::now());
    s.t += dt;
    if s.width != r.width {
        s.width = r.width;
        s.cols = (0..r.width as i32).map(|i| (-(hash(i, 1) * r.height as f32 * 2.0), 3.0 + hash(i, 2) * 6.0, 4 + (hash(i, 3) * 18.0) as u16)).collect();
    }
    // calm: loud passages ease it up to about twice as fast, a beat nudges it; it settles back over a second or two.
    // Without music it runs a little faster than a quiet song: slower, it stepped visibly (owner).
    let playing = matches!(app.playback.status, singularity_proto::PlayStatus::Playing | singularity_proto::PlayStatus::Buffering);
    let target = if playing { 1.0 + energy * 1.0 + if beat { 0.3 } else { 0.0 } } else { 1.6 };
    s.boost = if s.boost == 0.0 { 1.0 } else { s.boost + (target - s.boost) * (dt * 1.5).min(1.0) };
    let boost = s.boost;
    for (i, c) in s.cols.iter_mut().enumerate() {
        c.0 += c.1 * dt * boost;
        if c.0 - c.2 as f32 > r.height as f32 {
            let k = (i as i32).wrapping_add((c.0 * 977.0) as i32);
            *c = (-(hash(k, 4) * 10.0), 3.0 + hash(k, 5) * 6.0, 4 + (hash(k, 6) * 18.0) as u16);
        }
    }
    let glyphs: Vec<char> = GLYPHS.chars().collect();
    let t = s.t;
    for (x, c) in s.cols.iter().enumerate() {
        for k in 0..c.2 as i32 {
            let y = c.0 as i32 - k;
            if y < 0 || y >= r.height as i32 {
                continue;
            }
            let seed = hash(x as i32, y + (t * 6.0) as i32 * i32::from(k == 0));
            let ch = glyphs[(seed * glyphs.len() as f32) as usize % glyphs.len()];
            let fade = 1.0 - k as f32 / c.2 as f32;
            let col = if k == 0 { th.accent2 } else { th.accent.scale(fade.powf(1.5)) };
            text(buf, r.x + x as u16, r.y + y as u16, 1, &ch.to_string(), th.s(col));
        }
    }
}

// ── scope (Terminal) ──────────────────────────────────────────────────

/// An oscilloscope trace drawn in braille. The Daemon sends spectra, not samples, so the wave is put back together
/// from the bands (each band a sine at its own frequency), which moves with the music like the real thing.
pub fn scope(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.width < 2 || r.height < 2 {
        return;
    }
    let (w, h) = (r.width as usize * 2, r.height as f32 * 4.0);
    let t = app.hole.time();
    // paused: the trace goes flat at once. The bands take a second or two to fall, and the wave is scaled by their
    // own sum, so it would otherwise hold its full height until they're nearly gone.
    let playing = matches!(app.playback.status, singularity_proto::PlayStatus::Playing | singularity_proto::PlayStatus::Buffering);
    let bands: Vec<f32> = app.meters.frame.as_ref().filter(|_| playing).map(|f| f.bands.iter().map(|b| *b as f32 / 255.0).collect()).unwrap_or_default();
    // phosphor persistence: ~90 ms whatever the frame rate (per frame, it trailed the music by ~0.3 s at an
    // unfocused 10 fps)
    let dt = app.widgets.scope_at.map_or(1.0 / 30.0, |t| t.elapsed().as_secs_f32().min(0.5));
    app.widgets.scope_at = Some(Instant::now());
    let keep = 0.7f32.powf(dt * 30.0);
    let disp = &mut app.widgets.scope;
    if disp.len() != w {
        *disp = vec![0.0; w];
    }
    let groups = 8.min(bands.len());
    let total: f32 = (0..groups).map(|g| bands[g * bands.len() / groups.max(1)]).sum::<f32>().max(0.3);
    for (i, d) in disp.iter_mut().enumerate() {
        let x = i as f32 / w as f32;
        let mut s = 0.0;
        for g in 0..groups {
            let a = bands[g * bands.len() / groups];
            let f = 1.5 + g as f32 * 1.7;
            s += a * (std::f32::consts::TAU * f * x + t * (2.0 + g as f32)).sin();
        }
        *d = *d * keep + s / total * 0.9 * (1.0 - keep);
    }
    let mut dots = vec![0u8; w * r.height as usize * 4];
    let mut prev: Option<f32> = None;
    for (i, s) in disp.iter().enumerate() {
        // rounded: a trace a hair off the middle (the persistence never quite reaches 0) stays on one row
        let y = (h * 0.5 - s.clamp(-1.0, 1.0) * h * 0.45).round().min(h - 1.0);
        let from = prev.unwrap_or(y);
        let (a, b) = if from < y { (from, y) } else { (y, from) };
        for yy in a as usize..=(b as usize).min(h as usize - 1) {
            dots[yy * w + i] = 1;
        }
        prev = Some(y);
    }
    const BITS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    for cy in 0..r.height as usize {
        for cx in 0..r.width as usize {
            let mut bits = 0;
            for (dy, row) in BITS.iter().enumerate() {
                for (dx, bit) in row.iter().enumerate() {
                    if dots[(cy * 4 + dy) * w + cx * 2 + dx] != 0 {
                        bits |= bit;
                    }
                }
            }
            if bits != 0 {
                let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                let mid = 1.0 - ((cy as f32 + 0.5) / r.height as f32 - 0.5).abs() * 2.0;
                text(buf, r.x + cx as u16, r.y + cy as u16, 1, &ch.to_string(), th.s(ramp(th, 0.35 + mid * 0.6)));
            }
        }
    }
}

// ── top (Terminal) ────────────────────────────────────────────────────

/// The processes behind the music, as `top` would show them (sampled once a second): the Daemon, the Client, and
/// whichever player runs (Soloist, the Spotify app and its helpers), plus PipeWire on Linux.
#[derive(Default)]
struct Top {
    rows: Vec<(u32, f32, u64, String)>,
    /// pid → CPU time (ns) at the last sample
    cpu_ns: HashMap<u32, u64>,
    at: Option<Instant>,
    /// CPU % of all the watched processes together, a sample a second, the newest last
    total: std::collections::VecDeque<f32>,
    /// pid → its CPU % history, the same way
    hist: HashMap<u32, std::collections::VecDeque<f32>>,
}

/// Seconds of CPU history kept for the graphs.
const HISTORY: usize = 240;

fn push_capped(q: &mut std::collections::VecDeque<f32>, v: f32) {
    if q.len() == HISTORY {
        q.pop_front();
    }
    q.push_back(v);
}

const WATCHED: [&str; 5] = ["soloist", "singularityd", "singularity", "pipewire", "spotify"];

/// Is `name` one of the music's processes? (the Spotify app is "Spotify" and "Spotify Helper (…)" on macOS)
fn watched(name: &str) -> bool {
    WATCHED.contains(&name) || name == "Spotify" || name.starts_with("Spotify Helper")
}

impl Top {
    fn sample(&mut self) {
        if self.at.is_some_and(|a| a.elapsed().as_secs_f32() < 1.0) {
            return;
        }
        let dt = self.at.map(|a| a.elapsed().as_secs_f32()).unwrap_or(1.0);
        let had_one = self.at.is_some();
        self.at = Some(Instant::now());
        let mut rows = vec![];
        let mut cpu_ns = HashMap::new();
        for (pid, name, ns, rss) in processes() {
            let cpu = self.cpu_ns.get(&pid).map(|p| ns.saturating_sub(*p) as f32 / 1e9 / dt * 100.0).unwrap_or(0.0);
            cpu_ns.insert(pid, ns);
            rows.push((pid, cpu, rss, name));
        }
        rows.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        // (the first sample has no rates yet)
        if had_one {
            push_capped(&mut self.total, rows.iter().map(|r| r.1).sum());
            for (pid, cpu, ..) in &rows {
                push_capped(self.hist.entry(*pid).or_default(), *cpu);
            }
        }
        self.hist.retain(|pid, _| cpu_ns.contains_key(pid));
        self.rows = rows;
        self.cpu_ns = cpu_ns;
    }
}

/// The watched processes: pid, name, CPU time used so far (ns), resident memory (bytes). Linux: `/proc`.
#[cfg(not(target_os = "macos"))]
fn processes() -> Vec<(u32, String, u64, u64)> {
    let page = 4096u64;
    // clock ticks are 10 ms on every Linux this runs on (USER_HZ = 100)
    let tick_ns = 10_000_000u64;
    let Ok(dir) = std::fs::read_dir("/proc") else { return vec![] };
    let mut out = vec![];
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else { continue };
        let comm = comm.trim();
        if !watched(comm) {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        // fields after the ")" of the command name: utime and stime are 12th and 13th
        let f: Vec<&str> = stat.rsplit_once(')').map(|(_, r)| r.split_whitespace().collect()).unwrap_or_default();
        let t = f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let rss = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * page;
        out.push((pid, comm.to_string(), t * tick_ns, rss));
    }
    out
}

/// The watched processes: pid, name, CPU time used so far (ns), resident memory (bytes). macOS: libproc.
#[cfg(target_os = "macos")]
#[allow(deprecated)] // libc's Mach timebase (it points at the `mach2` crate, not worth a dependency)
fn processes() -> Vec<(u32, String, u64, u64)> {
    // SAFETY: libproc fills buffers of the sizes passed; every count it returns is bounds-checked before use
    unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return vec![];
        }
        let mut pids = vec![0i32; n as usize + 64];
        let n = libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * std::mem::size_of::<i32>()) as i32);
        pids.truncate(n.max(0) as usize);
        // task times are in Mach absolute time units (not ns on Apple silicon)
        let mut tb = libc::mach_timebase_info { numer: 1, denom: 1 };
        libc::mach_timebase_info(&mut tb);
        let mut out = vec![];
        for pid in pids.into_iter().filter(|p| *p > 0) {
            let mut buf = [0u8; 64];
            let len = libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32);
            if len <= 0 {
                continue;
            }
            let name = String::from_utf8_lossy(&buf[..len as usize]).into_owned();
            if !watched(&name) {
                continue;
            }
            let mut ti: libc::proc_taskinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
            if libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&mut ti as *mut libc::proc_taskinfo).cast(), size) != size {
                continue;
            }
            let ns = (ti.pti_total_user + ti.pti_total_system) as u128 * tb.numer as u128 / tb.denom.max(1) as u128;
            out.push((pid as u32, name, ns as u64, ti.pti_resident_size));
        }
        out
    }
}

/// "1.2G", "245M"
fn mem(rss: u64) -> String {
    if rss >= 1 << 30 { format!("{:.1}G", rss as f32 / (1u64 << 30) as f32) } else { format!("{}M", rss >> 20) }
}

/// `top` for the music's processes. A row each: the name, a CPU bar, CPU %, memory and the pid as width allows. Tall
/// Panes put the processes' CPU together on top, a graph of the last minutes under it, and give each process a
/// sparkline of its own when there are rows for it.
pub fn top(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let s = &mut app.widgets.top;
    s.sample();
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    if w < 8 || r.height == 0 {
        return;
    }
    let n = s.rows.len() as u16;
    let mut y = r.y;
    if n == 0 {
        text(buf, x, y, w, "no music processes running", th.s(th.dim));
        return;
    }
    // the summary and graph when there's room for them and every process
    let summary = w >= 14 && r.height >= n + 9;
    if summary {
        let total: f32 = s.rows.iter().map(|r| r.1).sum();
        let rss: u64 = s.rows.iter().map(|r| r.2).sum();
        let cores = std::thread::available_parallelism().map(|c| c.get()).unwrap_or(1);
        let head = format!("{total:.1}%");
        text(buf, x, y, w, "cpu ", th.s(th.dim));
        text(buf, x + 4, y, w.saturating_sub(4), &head, th.s(th.accent).add_modifier(Modifier::BOLD));
        let hw = head.len() as u16 + 5;
        text(buf, x + hw, y, w.saturating_sub(hw), &format!("of {cores} cores"), th.s(th.border));
        let m = format!("{} resident", mem(rss));
        let mw = m.len() as u16;
        if hw + 12 + mw < w {
            text(buf, x + w - mw, y, mw, &m, th.s(th.dim));
        }
        y += 1;
        // the graph: the last minutes of the processes' CPU, scaled to their peak (at least 5 %)
        let gh = ((r.height - n - 4) / 2).clamp(2, 10);
        let peak = s.total.iter().copied().fold(5.0f32, f32::max);
        let gw = w.saturating_sub(6);
        crate::tile::area(buf, Rect::new(x, y, gw, gh), s.total.iter().copied(), peak, th, |v| th.ramp_at(0.3 + 0.7 * (v / peak)));
        let top_label = format!("{peak:.0}%");
        text(buf, x + gw + 1, y, 5, &top_label, th.s(th.border));
        text(buf, x + gw + 1, y + gh - 1, 5, "0%", th.s(th.border));
        // a baseline under the graph, so an idle minute still reads as a graph
        crate::tile::rule(buf, x, y + gh, gw, th);
        let secs = s.total.len().min(gw as usize);
        if secs >= 60 {
            text(buf, x, y + gh, 8, &format!("{}m ago", secs / 60), th.s(th.border));
        }
        y += gh + 2;
    }
    // the columns: name, bar, CPU %, memory, pid, as many as fit
    let pid_w: u16 = if w >= 52 { 8 } else { 0 };
    let name_w = (w / 4).clamp(8, 18).min(w);
    let fixed = name_w + 1 + 6 + 1 + 6 + pid_w;
    let bar_w = w.saturating_sub(fixed + 1);
    let bar_w = if bar_w >= 6 { bar_w } else { 0 };
    let cpu_x = x + name_w + 1 + if bar_w > 0 { bar_w + 1 } else { 0 };
    if y < r.bottom() && w >= 24 {
        let st = th.s(th.border).add_modifier(Modifier::BOLD);
        text(buf, x, y, name_w, "PROCESS", st);
        if bar_w > 0 {
            text(buf, x + name_w + 1, y, bar_w, "CPU", st);
        }
        text(buf, cpu_x, y, 6, "  CPU%", st);
        if cpu_x + 7 + 6 <= x + w {
            text(buf, cpu_x + 7, y, 6, "   MEM", st);
        }
        if pid_w > 0 {
            text(buf, x + w - pid_w, y, pid_w, "     PID", st);
        }
        y += 1;
    }
    // a sparkline row under each process, when every one fits that way
    let spark = summary && bar_w > 0 && y + 2 * n <= r.bottom();
    for (pid, cpu, rss, cmd) in &s.rows {
        if y >= r.bottom() {
            break;
        }
        let t = (*cpu / 100.0).clamp(0.0, 1.0);
        let hot = *cpu > 25.0;
        text(buf, x, y, name_w, cmd, th.s(if hot { th.accent } else { th.fg }).add_modifier(Modifier::BOLD));
        if w < 24 {
            let c = format!("{cpu:.0}%");
            let cw = c.len() as u16;
            if cw + 2 < w {
                text(buf, x + w - cw, y, cw, &c, th.s(th.dim));
            }
            y += 1;
            continue;
        }
        if bar_w > 0 {
            let fill = th.ramp_at(0.35 + 0.65 * t);
            crate::tile::hbar(buf, x + name_w + 1, y, bar_w, t, fill, Some(th.bg.lerp(th.border, 0.5)), th);
        }
        text(buf, cpu_x, y, 6, &format!("{cpu:>6.1}"), th.s(if hot { th.accent } else { th.fg }));
        if cpu_x + 7 + 6 <= x + w {
            text(buf, cpu_x + 7, y, 6, &format!("{:>6}", mem(*rss)), th.s(th.dim));
        }
        if pid_w > 0 {
            text(buf, x + w - pid_w, y, pid_w, &format!("{pid:>8}"), th.s(th.border));
        }
        y += 1;
        if spark {
            if let Some(h) = s.hist.get(pid) {
                let peak = h.iter().copied().fold(5.0f32, f32::max);
                crate::tile::sparkline(buf, Rect::new(x + name_w + 1, y, bar_w, 1), h.iter().copied(), peak, th);
            }
            y += 1;
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn rain_eases_with_the_music_instead_of_jumping() {
        let mut app = crate::app::App::new(None);
        let th = app.theme.clone();
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
        let loud = singularity_proto::SpectrumFrame { bands: vec![255; 64], level: 255, vu: [255, 255], beat: true, ..Default::default() };
        rain(&mut buf, Rect::new(0, 0, 40, 20), &mut app, &th);
        for _ in 0..3 {
            app.meters.push(loud.clone());
            std::thread::sleep(std::time::Duration::from_millis(50));
            rain(&mut buf, Rect::new(0, 0, 40, 20), &mut app, &th);
        }
        let b = app.widgets.rain.boost;
        assert!(b > 1.0 && b < 1.35, "a loud beat eases it up, a little at a time: {b}");
        assert!(app.widgets.rain.cols.iter().all(|c| c.1 <= 9.0), "slow columns");
    }
    use super::*;

    #[test]
    fn the_big_listening_pane_keeps_its_numbers_close_under_the_grid() {
        let th = crate::app::App::new(None).theme.clone();
        let minutes: Vec<u32> = (0..singularity_proto::LISTENING_DAYS as u32).map(|i| i * 7 % 50).collect();
        let s = singularity_proto::ListeningSummary { minutes, today_weekday: 3, hours_this_year: 19.0, top_artist: Some("Nova Drift".into()), streak_days: 10, plays: 534, plays_today: 3 };
        for (w, h) in [(40, 12), (46, 16), (60, 24), (90, 34), (120, 40), (120, 60), (150, 50), (200, 30), (256, 64), (80, 80)] {
            let r = Rect::new(3, 2, w, h);
            let mut buf = Buffer::empty(Rect::new(0, 0, w + 6, h + 4));
            listening_large(&mut buf, r, &s, &th);
            let row = |y: u16| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
            let inked: Vec<u16> = (0..buf.area.height).filter(|&y| !row(y).trim().is_empty()).collect();
            let (first, last) = (inked[0], *inked.last().unwrap());
            assert!(first >= r.y && last < r.bottom(), "{w}x{h}: inside the Pane");
            assert!(inked.windows(2).all(|p| p[1] - p[0] <= 3), "{w}x{h}: no more than two blank rows between parts: {inked:?}");
            assert!((last.saturating_sub(3)..=last).any(|y| row(y).contains("last 7 days")), "{w}x{h}: the numbers end the block");
            // the block sits in the middle: the space over it and under it differ by a row at most
            let (over, under) = (first - r.y, r.bottom() - 1 - last);
            assert!(over.abs_diff(under) <= 2, "{w}x{h}: centred, {over} over and {under} under");
        }
    }

    #[test]
    fn top_finds_this_process() {
        // the test binary isn't called "singularity", so look for nothing in particular: it must simply not panic and
        // produce rows only for the watched names
        let mut t = Top::default();
        t.sample();
        assert!(t.rows.iter().all(|r| watched(&r.3)));
        assert!(watched("Spotify Helper (Renderer)") && watched("singularityd") && !watched("Spotlight"));
    }

    #[test]
    fn the_scope_goes_flat_when_the_music_pauses() {
        let th = Theme::singularity();
        let mut app = App::new(None);
        let r = Rect::new(0, 0, 40, 20);
        let rows = |app: &mut App| {
            // a frame every 33 ms, as at 30 fps
            app.widgets.scope_at = Some(Instant::now() - std::time::Duration::from_millis(33));
            let mut buf = Buffer::empty(r);
            scope(&mut buf, r, app, &th);
            (0..20).filter(|y| (0..40).any(|x| buf[(x, *y)].symbol() != " ")).count()
        };
        app.playback.status = singularity_proto::PlayStatus::Playing;
        app.meters.push(singularity_proto::SpectrumFrame { bands: vec![200; 64], level: 200, ..Default::default() });
        let mut playing = 0;
        for _ in 0..10 {
            playing = rows(&mut app);
        }
        assert!(playing > 2, "a wave while it plays: {playing} rows");
        // paused, the last loud frame still there (the Daemon's bars are still falling): flat within ~0.7 s at 30 fps
        app.playback.status = singularity_proto::PlayStatus::Paused;
        let mut paused = 0;
        for _ in 0..20 {
            paused = rows(&mut app);
        }
        assert_eq!(paused, 1, "a flat line");
    }

    #[test]
    fn the_scope_keeps_up_with_the_music_at_any_frame_rate() {
        // the same 0.2 s of music drawn at 30 and at 5 fps ends at the same trace
        let th = Theme::singularity();
        let r = Rect::new(0, 0, 40, 20);
        let trace = |fps: u64| {
            let mut app = App::new(None);
            app.playback.status = singularity_proto::PlayStatus::Playing;
            app.meters.push(singularity_proto::SpectrumFrame { bands: vec![200; 64], level: 200, ..Default::default() });
            for _ in 0..(fps / 5) {
                app.widgets.scope_at = Some(Instant::now() - std::time::Duration::from_millis(1000 / fps));
                scope(&mut Buffer::empty(r), r, &mut app, &th);
            }
            app.widgets.scope.iter().map(|d| d.abs()).sum::<f32>()
        };
        let (fast, slow) = (trace(30), trace(5));
        assert!((fast - slow).abs() < fast * 0.05, "30 fps {fast}, 5 fps {slow}");
    }

    #[test]
    fn the_widgets_draw_inside_their_rect() {
        let th = Theme::singularity();
        let mut app = App::new(None);
        app.meters.push(singularity_proto::SpectrumFrame { bands: (0..64).map(|i| (i * 4) as u8).collect(), level: 200, vu: [200, 120], beat: true, out_level: 200, out_vu: [200, 120], out_bands: [60, 120, 200] });
        app.playback.item = Some(singularity_proto::Item { name: "Song".into(), artists: vec!["Artist".into()], ..Default::default() });
        let area = Rect::new(0, 0, 60, 30);
        let inner = Rect::new(5, 5, 40, 20);
        for f in [nowplaying as fn(&mut Buffer, Rect, &mut App, &Theme), rain, scope, top] {
            let mut buf = Buffer::empty(area);
            f(&mut buf, inner, &mut app, &th);
            for y in 0..30 {
                for x in 0..60 {
                    if !(inner.contains((x, y).into())) {
                        assert_eq!(buf[(x, y)].symbol(), " ", "drew outside at {x},{y}");
                    }
                }
            }
        }
        let mut buf = Buffer::empty(area);
        levels(&mut buf, inner, &app, &th);
        let row = |y| (5..45).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        // 20 rows: bars two rows thick, the five of them centred
        let labels: Vec<u16> = (5..25).filter(|&y| row(y).starts_with(|c: char| c.is_alphabetic())).collect();
        assert_eq!(labels.len(), 5, "{:?}", (5..25).map(row).collect::<Vec<_>>());
        assert!(row(labels[0]).starts_with("L ") && row(labels[0]).contains('█') && row(labels[4]).starts_with("high"));
        // a wide, short Pane: the two groups side by side
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 4));
        levels(&mut buf, Rect::new(0, 0, 80, 4), &app, &th);
        let row0: String = (0..80).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(row0.starts_with("L ") && row0.contains("bass"), "{row0}");
    }

    #[test]
    fn now_playing_has_no_meters_and_lists_the_queue_only_without_a_queue_pane() {
        let th = Theme::singularity();
        let mut app = App::new(None);
        app.playback.item = Some(singularity_proto::Item { name: "Song".into(), artists: vec!["Artist".into()], ..Default::default() });
        app.playback.queue = vec![singularity_proto::QueueEntry { item: singularity_proto::Item { name: "Next One".into(), ..Default::default() }, source: "queue".into(), uid: String::new() }];
        let screen = |app: &mut App, w: u16, h: u16| {
            let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
            nowplaying(&mut buf, Rect::new(0, 0, w, h), app, &th);
            (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
        };
        app.layout = crate::layout::Node::split(crate::layout::Axis::Horizontal, crate::layout::Size::Ratio(0.5), crate::layout::Node::pane(crate::layout::Widget::NowPlaying), crate::layout::Node::pane(crate::layout::Widget::Lyrics));
        for (w, h) in [(40, 40), (140, 8), (110, 20)] {
            let s = screen(&mut app, w, h);
            assert!(s.contains("Next One"), "no queue Pane: the queue shows ({w}×{h}): {s}");
            assert!(!s.contains('▁') && !s.contains('█'), "no spectrum ({w}×{h}): {s}");
        }
        // a queue Pane opens: the list goes, at once
        app.layout = crate::layout::Node::split(crate::layout::Axis::Horizontal, crate::layout::Size::Ratio(0.5), crate::layout::Node::pane(crate::layout::Widget::NowPlaying), crate::layout::Node::pane(crate::layout::Widget::Queue));
        assert!(!screen(&mut app, 40, 40).contains("Next"));
    }

    #[test]
    fn now_playing_fits_its_shape() {
        let th = Theme::singularity();
        let mut app = App::new(None);
        app.playback.item = Some(singularity_proto::Item { name: "Song".into(), artists: vec!["Artist".into()], ..Default::default() });
        let rows = |app: &mut App, w: u16, h: u16| {
            let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
            nowplaying(&mut buf, Rect::new(0, 0, w, h), app, &th);
            (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>()
        };
        // a Pane a row or two tall: the words, no cover
        let tiny = rows(&mut app, 12, 2);
        assert_eq!(tiny[0].trim(), "Song", "{tiny:?}");
        assert_eq!(tiny[1].trim(), "Artist");
        // wide and short: the cover at the left (its ♪ stand-in, no art loaded), the title beside it at the top
        let wide = rows(&mut app, 110, 6);
        let at = wide[0].find("Song").unwrap();
        assert!(at > 12 && at < 20, "{wide:#?}");
        // tall: the cover on top, the title under it
        let tall = rows(&mut app, 30, 30);
        let y = tall.iter().position(|r| r.contains("Song")).unwrap();
        assert!(y > 8, "{tall:#?}");
    }
}
