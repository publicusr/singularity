//! The mini-player (#47): in a terminal too small for any layout, Singularity is just a player. One row: controls,
//! title — artist, time. Two rows: title (and ♥) over controls, progress and time. More: a centred block with the
//! artist and a progress bar of its own. Everything is clickable, and keys work as usual.

use crate::app::{Action, App, Hit};
use crate::scrub::Bar;
use crate::scrub;
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::PlayStatus;
use unicode_width::UnicodeWidthStr;

/// Below either of these, the full layout gives way to the mini-player.
pub const MIN_W: u16 = 50;
pub const MIN_H: u16 = 14;

pub fn wanted(area: Rect) -> bool {
    area.width < MIN_W || area.height < MIN_H
}

fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

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

/// ◀ ▶ ▶|: drawn at (x, y), each with its hit; returns the width used.
fn controls(buf: &mut Buffer, x: u16, y: u16, app: &mut App, th: &Theme) -> u16 {
    let playing = app.playback.status == PlayStatus::Playing;
    let buttons = [("|◀", Action::Previous), (if playing { "❚❚" } else { " ▶" }, Action::TogglePlay), ("▶|", Action::Next)];
    for (i, (label, act)) in buttons.iter().enumerate() {
        let bx = x + i as u16 * 4;
        let st = if *act == Action::TogglePlay { th.chip(th.accent) } else { th.s(th.fg) };
        text(buf, bx, y, 3, &format!("{label} "), st);
        app.hit(Rect::new(bx, y, 3, 1), Hit::Action(*act));
    }
    12
}

/// A progress bar over `bar` (seek by click or drag, as the player bar's).
fn progress(buf: &mut Buffer, bar: Rect, app: &mut App, th: &Theme) {
    let pos = app.display_position(singularity_proto::now_ms());
    let dur = app.playback.item.as_ref().map(|i| i.duration_ms).unwrap_or(0);
    let (full, tip) = scrub::fill(pos, dur, bar.width);
    for x in 0..bar.width {
        let (ch, col) = if x < full { ("━", th.accent) } else if x == full && tip { ("╸", th.accent) } else { ("─", th.border) };
        text(buf, bar.x + x, bar.y, 1, ch, th.s(col));
    }
    app.hit(bar, Hit::Bar(Bar::Seek, bar));
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    if r.width < 8 || r.height == 0 {
        return;
    }
    let item = app.playback.item.clone();
    let (title, artists) = item.as_ref().map(|i| (i.name.clone(), i.artists.join(", "))).unwrap_or_else(|| ("nothing playing".into(), String::new()));
    let pos = app.display_position(singularity_proto::now_ms());
    let dur = item.as_ref().map(|i| i.duration_ms).unwrap_or(0);
    let time = format!("{} / {}", fmt_ms(pos), fmt_ms(dur));
    let heart = item.as_ref().filter(|i| crate::saved::savable(&i.uri)).map(|i| (i.uri.clone(), i.name.clone(), app.is_saved(&i.uri)));
    let title_st = th.s(th.fg).add_modifier(Modifier::BOLD);
    match r.height {
        1 => {
            // |◀ ❚❚ ▶|  Title — Artist ……… 1:23 / 3:20
            let used = if r.width >= 24 { controls(buf, r.x, r.y, app, th) } else { 0 };
            let tw = time.width() as u16;
            let room = r.width.saturating_sub(used + if r.width >= used + tw + 12 { tw + 1 } else { 0 });
            let line = if artists.is_empty() { title } else { format!("{title} — {artists}") };
            text(buf, r.x + used, r.y, room, &clip(&line, room as usize), title_st);
            if r.width >= used + tw + 12 {
                text(buf, r.right() - tw, r.y, tw, &time, th.s(th.dim));
            }
        }
        2 => {
            let hw = if heart.is_some() { 2 } else { 0 };
            let tw = r.width.saturating_sub(hw);
            let line = if artists.is_empty() { title } else { format!("{title} — {artists}") };
            text(buf, r.x, r.y, tw, &clip(&line, tw as usize), title_st);
            if let Some((uri, name, liked)) = &heart {
                let (hr, h) = crate::saved::heart(buf, r.right() - 1, r.y, uri, name, *liked, th.bg.color(), th);
                app.hit(hr, h);
            }
            let used = controls(buf, r.x, r.y + 1, app, th);
            let t = fmt_ms(pos);
            let bar_w = r.width.saturating_sub(used + t.width() as u16 + 2);
            if bar_w >= 4 {
                progress(buf, Rect::new(r.x + used, r.y + 1, bar_w, 1), app, th);
                text(buf, r.x + used + bar_w + 1, r.y + 1, t.width() as u16, &t, th.s(th.dim));
            }
        }
        h => {
            // a centred block: title (♥), artist, progress with times, controls
            let rows = 4u16.min(h);
            let y0 = r.y + (h - rows) / 2;
            let w = r.width.min(60);
            let x0 = r.x + (r.width - w) / 2;
            let centre = |buf: &mut Buffer, y: u16, s: &str, st| {
                let s = clip(s, w as usize);
                let sw = s.width() as u16;
                text(buf, x0 + (w - sw) / 2, y, sw, &s, st);
                x0 + (w - sw) / 2 + sw
            };
            let end = centre(buf, y0, &title, title_st);
            if let Some((uri, name, liked)) = &heart
                && end + 2 <= r.right()
            {
                let (hr, hit) = crate::saved::heart(buf, end + 1, y0, uri, name, *liked, th.bg.color(), th);
                app.hit(hr, hit);
            }
            if rows >= 2 {
                centre(buf, y0 + 1, &artists, th.s(th.dim));
            }
            if rows >= 3 {
                let (a, b) = (fmt_ms(pos), fmt_ms(dur));
                let bar = Rect::new(x0 + a.width() as u16 + 1, y0 + 2, w.saturating_sub(a.width() as u16 + b.width() as u16 + 2), 1);
                text(buf, x0, y0 + 2, a.width() as u16, &a, th.s(th.dim));
                progress(buf, bar, app, th);
                text(buf, bar.right() + 1, y0 + 2, b.width() as u16, &b, th.s(th.dim));
            }
            if rows >= 4 {
                controls(buf, x0 + (w.saturating_sub(12)) / 2, y0 + 3, app, th);
            }
        }
    }
}
