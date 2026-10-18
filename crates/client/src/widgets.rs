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

/// Cover, title and artists, a small spectrum, and what's next.
pub fn nowplaying(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let Some(item) = app.playback.item.clone() else {
        text(buf, r.x + 1, r.y + r.height / 2, r.width.saturating_sub(2), "nothing playing", th.s(th.dim));
        return;
    };
    let w = r.width.saturating_sub(2);
    // a square cover (cells are about twice as tall as wide), leaving room for the rest
    let ch = (w / 2).min(r.height.saturating_sub(10)).max(3);
    let cw = (ch * 2).min(w);
    let cover = Rect::new(r.x + 1 + (w - cw) / 2, r.y, cw, ch);
    crate::home::cover(buf, cover, item.cover_url.as_deref().unwrap_or(""), "♪", app, th);
    let mut y = cover.bottom() + 1;
    text(buf, r.x + 1, y, w, &item.name, th.s(th.fg).add_modifier(Modifier::BOLD));
    text(buf, r.x + 1, y + 1, w, &item.artists.join(", "), th.s(th.dim));
    if let Some(c) = &app.playback.context_name {
        text(buf, r.x + 1, y + 2, w, &format!("from {c}"), th.s(th.dim));
    }
    y += 4;
    let sh = 5.min(r.bottom().saturating_sub(y));
    if sh > 1 {
        crate::meters::spectrum(buf, Rect::new(r.x + 1, y, w, sh), &app.meters, th);
        y += sh + 1;
    }
    if y + 1 < r.bottom() {
        text(buf, r.x + 1, y, w, "Next in queue", th.s(th.fg).add_modifier(Modifier::BOLD));
        y += 1;
        for q in app.playback.queue.iter().take(r.bottom().saturating_sub(y) as usize) {
            text(buf, r.x + 1, y, 2, "■", th.s(th.accent));
            text(buf, r.x + 3, y, w.saturating_sub(2), &format!("{} · {}", q.item.name, q.item.artists.join(", ")), th.s(th.dim));
            y += 1;
        }
    }
}

// ── levels (Cockpit) ──────────────────────────────────────────────────

/// Left/right peak meters, then bass, mid and treble.
pub fn levels(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) {
    let vu = app.meters.frame.as_ref().map(|f| f.out_vu).unwrap_or([0, 0]);
    let bar = |buf: &mut Buffer, y: u16, label: &str, v: f32| {
        if y >= r.bottom() {
            return;
        }
        let w = r.width.saturating_sub(7);
        let lit = (v.clamp(0.0, 1.0) * w as f32).round() as u16;
        text(buf, r.x, y, 6, label, th.s(th.dim));
        for x in 0..w {
            let (ch, c) = if x < lit { ("■", ramp(th, x as f32 / w.max(1) as f32)) } else { ("·", th.border) };
            text(buf, r.x + 6 + x, y, 1, ch, th.s(c));
        }
    };
    bar(buf, r.y, "L", vu[0] as f32 / 255.0);
    bar(buf, r.y + 1, "R", vu[1] as f32 / 255.0);
    let [b, m, t] = bands3(app);
    // a blank row between the two groups when there's room for it
    let gap = if r.height >= 6 { 3 } else { 2 };
    for (i, (name, v)) in [("bass", b), ("mid", m), ("high", t)].into_iter().enumerate() {
        bar(buf, r.y + gap + i as u16, name, v);
    }
}

/// Cockpit's listening Pane: minutes per day from the local listening log (#23), a column per week (Monday on top,
/// this week on the right), and a line of totals under it. Short Panes stack two days per cell (half-blocks).
pub fn listening(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) {
    if r.width < 4 || r.height < 2 {
        return;
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
    let line = match sum {
        None => "the listening log comes from singularityd".to_string(),
        Some(s) if s.plays == 0 => "nothing logged yet · a track counts after 30 s".into(),
        Some(s) => {
            let h = if s.hours_this_year < 10.0 { format!("{:.1}", s.hours_this_year) } else { format!("{:.0}", s.hours_this_year) };
            let mut parts = vec![format!("{h} h this year")];
            parts.extend(s.top_artist.as_ref().map(|a| format!("top: {a}")));
            parts.push(format!("streak {} day{}", s.streak_days, if s.streak_days == 1 { "" } else { "s" }));
            parts.join(" · ")
        }
    };
    text(buf, r.x + 1, r.y + rows + (r.height > 8) as u16, r.width.saturating_sub(2), &line, th.s(th.dim));
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
        s.cols = (0..r.width as i32).map(|i| (-(hash(i, 1) * r.height as f32 * 2.0), 6.0 + hash(i, 2) * 14.0, 4 + (hash(i, 3) * 18.0) as u16)).collect();
    }
    let boost = 1.0 + energy * 2.5 + if beat { 1.5 } else { 0.0 };
    for (i, c) in s.cols.iter_mut().enumerate() {
        c.0 += c.1 * dt * boost;
        if c.0 - c.2 as f32 > r.height as f32 {
            let k = (i as i32).wrapping_add((c.0 * 977.0) as i32);
            *c = (-(hash(k, 4) * 10.0), 6.0 + hash(k, 5) * 14.0, 4 + (hash(k, 6) * 18.0) as u16);
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
    let bands: Vec<f32> = app.meters.frame.as_ref().map(|f| f.bands.iter().map(|b| *b as f32 / 255.0).collect()).unwrap_or_default();
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
        // phosphor persistence
        *d = *d * 0.7 + s / total * 0.9 * 0.3;
    }
    let mut dots = vec![0u8; w * r.height as usize * 4];
    let mut prev: Option<f32> = None;
    for (i, s) in disp.iter().enumerate() {
        let y = h * 0.5 - s.clamp(-1.0, 1.0) * h * 0.45;
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

/// The processes behind the music, as `top` would show them (read from /proc once a second).
#[derive(Default)]
struct Top {
    rows: Vec<(u32, f32, u64, String)>,
    /// pid → CPU ticks at the last sample
    ticks: HashMap<u32, u64>,
    at: Option<Instant>,
}

const WATCHED: [&str; 4] = ["soloist", "singularityd", "singularity", "pipewire"];

impl Top {
    fn sample(&mut self) {
        if self.at.is_some_and(|a| a.elapsed().as_secs_f32() < 1.0) {
            return;
        }
        let dt = self.at.map(|a| a.elapsed().as_secs_f32()).unwrap_or(1.0);
        self.at = Some(Instant::now());
        let page = 4096u64;
        let mut rows = vec![];
        let mut ticks = HashMap::new();
        let Ok(dir) = std::fs::read_dir("/proc") else { return };
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
            let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else { continue };
            let comm = comm.trim();
            if !WATCHED.contains(&comm) {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
            // fields after the ")" of the command name: utime and stime are 12th and 13th
            let f: Vec<&str> = stat.rsplit_once(')').map(|(_, r)| r.split_whitespace().collect()).unwrap_or_default();
            let t = f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            let rss = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * page;
            let cpu = self.ticks.get(&pid).map(|p| t.saturating_sub(*p) as f32 / 100.0 / dt * 100.0).unwrap_or(0.0);
            ticks.insert(pid, t);
            rows.push((pid, cpu, rss, comm.to_string()));
        }
        rows.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        self.rows = rows;
        self.ticks = ticks;
    }
}

pub fn top(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let s = &mut app.widgets.top;
    s.sample();
    text(buf, r.x, r.y, r.width, &format!("{:<7}{:>6} {:>6}  CMD", "PID", "CPU%", "MEM"), th.s(th.dim).add_modifier(Modifier::BOLD));
    for (i, (pid, cpu, rss, cmd)) in s.rows.iter().enumerate() {
        let y = r.y + 1 + i as u16;
        if y >= r.bottom() {
            break;
        }
        let mem = if *rss >= 1 << 30 { format!("{:.1}G", *rss as f32 / (1u64 << 30) as f32) } else { format!("{}M", rss >> 20) };
        text(buf, r.x, y, r.width, &format!("{pid:<7}{cpu:>6.1} {mem:>6}  {cmd}"), th.s(if *cpu > 25.0 { th.accent } else { th.fg }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_finds_this_process() {
        // the test binary isn't called "singularity", so look for nothing in particular: it must simply not panic and
        // produce rows only for the watched names
        let mut t = Top::default();
        t.sample();
        assert!(t.rows.iter().all(|r| WATCHED.contains(&r.3.as_str())));
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
        assert!(row(5).starts_with("L") && row(5).contains('■') && row(10).starts_with("high"));
    }
}
