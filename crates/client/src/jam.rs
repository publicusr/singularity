//! Jam (D64) in the device picker: start one on this device, show its invite (link + QR) and guests, end it.
//! The Jam row is the picker's row after the last device (`Hit::DeviceRow(devices.len())`).

use qrcodegen::{QrCode, QrCodeEcc};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use singularity_proto::Jam;

use crate::app::{App, Hit};
use crate::theme::Theme;
use crate::ui::{text, blank};

/// Guests listed under the QR; more are counted.
const GUESTS_SHOWN: usize = 4;
/// Quiet zone around the QR, in modules (2 scans fine on phones and saves rows).
const BORDER: i32 = 2;

/// The QR code for `link` as rows of half blocks, two modules a row; dark modules are `█`/`▀`/`▄`.
pub fn qr_rows(link: &str) -> Vec<String> {
    let Ok(code) = QrCode::encode_text(link, QrCodeEcc::Low) else { return vec![] };
    let size = code.size();
    let dark = |x: i32, y: i32| x >= 0 && y >= 0 && x < size && y < size && code.get_module(x, y);
    (-BORDER..size + BORDER)
        .step_by(2)
        .map(|y| {
            (-BORDER..size + BORDER)
                .map(|x| match (dark(x, y), dark(x, y + 1)) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                })
                .collect()
        })
        .collect()
}

/// The people in a Jam besides the one who runs it.
pub fn guests(jam: &Jam) -> usize {
    jam.members.iter().filter(|m| !m.owner).count()
}

/// Columns the picker needs for the Jam section: the whole invite link fits on its line.
pub fn width(app: &App) -> u16 {
    app.jam.as_ref().and_then(|j| j.invite.as_deref()).map(|l| l.chars().count() as u16 + 6).unwrap_or(0)
}

/// Rows the Jam section needs below the device list, with room `room` left on screen.
pub fn height(app: &App, room: u16) -> u16 {
    let base = 2; // a gap and the Jam row
    let Some(jam) = &app.jam else { return base };
    let listed = guests(jam).min(GUESTS_SHOWN) as u16 + u16::from(guests(jam) > GUESTS_SHOWN);
    let link = u16::from(jam.invite.is_some());
    let qr = jam.invite.as_deref().map(|l| qr_rows(l).len() as u16).unwrap_or(0);
    let without_qr = base + link + listed;
    if qr > 0 && without_qr + qr <= room { without_qr + qr } else { without_qr }
}

/// Draw the section from row `y` of the picker's inside `inner`; `row` is the Jam row's index in the picker.
pub fn draw(buf: &mut Buffer, inner: Rect, mut y: u16, row: usize, app: &mut App, th: &Theme) {
    y += 1;
    let sel = app.devices_sel == row;
    let bg = if sel { th.selection.color() } else { th.bg.color() };
    blank(buf, inner.x, y, inner.width, th.base().bg(bg));
    let engine = app.devices.iter().find(|d| d.is_engine).map(|d| d.name.clone()).unwrap_or_else(|| "this device".into());
    let (label, tag) = match &app.jam {
        None => (format!("✚ Start a Jam on {engine}"), "friends join with a link".to_string()),
        Some(j) => {
            let n = guests(j);
            let on = j.host.clone().unwrap_or_else(|| "another device".into());
            let who = match n {
                0 => "no guests yet".to_string(),
                1 => "1 guest".to_string(),
                n => format!("{n} guests"),
            };
            (format!("◎ Jam on {on} · {who}"), if j.owner { "Enter: end it".into() } else { "someone else's".into() })
        }
    };
    let st = if app.jam.is_some() { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.fg) };
    text(buf, inner.x + 2, y, inner.width.saturating_sub(4), &label, st.bg(bg));
    let tw = tag.chars().count() as u16;
    if 2 + label.chars().count() as u16 + 2 + tw <= inner.width {
        text(buf, inner.right().saturating_sub(tw + 2), y, tw, &tag, th.s(th.dim).bg(bg));
    }
    app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::DeviceRow(row));
    let Some(jam) = app.jam.clone() else { return };
    y += 1;
    if let Some(link) = &jam.invite {
        text(buf, inner.x + 2, y, inner.width.saturating_sub(4), link, th.s(th.dim));
        y += 1;
        let qr = qr_rows(link);
        let room = inner.bottom().saturating_sub(y + 1);
        let listed = guests(&jam).min(GUESTS_SHOWN) as u16 + u16::from(guests(&jam) > GUESTS_SHOWN);
        if qr.len() as u16 + listed <= room {
            // black on white whatever the Theme: phone cameras need the contrast
            let w = qr.first().map(|r| r.chars().count() as u16).unwrap_or(0);
            let x = inner.x + inner.width.saturating_sub(w) / 2;
            for line in &qr {
                text(buf, x, y, w, line, Style::default().fg(Color::Black).bg(Color::White));
                y += 1;
            }
        }
    }
    let shown: Vec<_> = jam.members.iter().filter(|m| !m.owner).collect();
    for m in shown.iter().take(GUESTS_SHOWN) {
        let mark = if m.listening { "♪" } else { "·" };
        text(buf, inner.x + 4, y, inner.width.saturating_sub(6), &format!("{mark} {}", m.name), th.s(th.fg));
        y += 1;
    }
    if shown.len() > GUESTS_SHOWN {
        text(buf, inner.x + 4, y, inner.width.saturating_sub(6), &format!("and {} more", shown.len() - GUESTS_SHOWN), th.s(th.dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invite_link_is_a_square_qr_of_half_blocks() {
        let rows = qr_rows("https://open.spotify.com/socialsession/FakeJamToken123");
        let w = rows[0].chars().count();
        assert!(rows.iter().all(|r| r.chars().count() == w), "every row as wide");
        assert_eq!(rows.len(), w.div_ceil(2), "two modules a row");
        assert!(w <= 40, "fits the 56-wide picker: {w}");
        assert!(rows[0].chars().all(|c| c == ' '), "a quiet zone on top");
        assert!(rows[1].contains('█') || rows[1].contains('▀') || rows[1].contains('▄'));
    }

    #[test]
    fn the_half_blocks_are_exactly_the_codes_modules() {
        let link = "https://open.spotify.com/socialsession/FakeJamToken123";
        let code = QrCode::encode_text(link, QrCodeEcc::Low).unwrap();
        let rows = qr_rows(link);
        for (r, line) in rows.iter().enumerate() {
            for (c, ch) in line.chars().enumerate() {
                let (x, y) = (c as i32 - BORDER, 2 * r as i32 - BORDER);
                let dark = |y: i32| x >= 0 && y >= 0 && x < code.size() && y < code.size() && code.get_module(x, y);
                assert_eq!((ch == '█' || ch == '▀', ch == '█' || ch == '▄'), (dark(y), dark(y + 1)), "row {r} col {c}");
            }
        }
    }
}
