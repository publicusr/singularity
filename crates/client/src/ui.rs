//! Rendering. Pure function of `App` state, plus hit-box registration for the mouse.

use crate::app::{Action, App, Hit, Link, Menu};
use crate::scrub::{self, Bar};
use crate::theme::Theme;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear};
use singularity_proto::{CatalogueStatus, EngineStatus, PlayStatus};

/// Pane titles, and which ticket brings each Pane's real Widget.
fn slot_info(slot: &str) -> (&'static str, &'static str) {
    match slot {
        "library" => ("~/library", "library sidebar arrives with #9"),
        "stream" => ("stream", "stream stats arrive with #5"),
        "home" => ("home", ""),
        "queue" => ("queue", "queue arrives with #12"),
        "well" => ("gravity well", "the black hole arrives with #16"),
        "lyrics" => ("tail -f lyrics.log", "synced lyrics arrive with #14"),
        "loudness" => ("loudness", "arrives with #15"),
        "fft" => ("fft", "arrives with #15"),
        _ => ("?", ""),
    }
}

pub fn text(buf: &mut Buffer, x: u16, y: u16, max: u16, s: &str, st: Style) {
    if max > 0 {
        buf.set_stringn(x, y, s, max as usize, st);
    }
}

fn fill(buf: &mut Buffer, r: Rect, th: &Theme) {
    for y in r.top()..r.bottom() {
        for x in r.left()..r.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_char(' ').set_style(th.base());
            }
        }
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.begin_frame();
    let area = f.area();
    let th = app.theme.clone();
    fill(f.buffer_mut(), area, &th);
    if area.width < 20 || area.height < 8 {
        text(f.buffer_mut(), area.x, area.y, area.width, "◉ terminal too small", th.s(th.dim));
        return;
    }
    let [top, body, player, hints] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(3), Constraint::Length(1)]).areas(area);
    // popups are known up front, so pixel images under them can step down to cells this frame
    let login_r = match &app.catalogue {
        Some(CatalogueStatus::LoginPending { .. }) if !app.login_hidden => Some(centered(area, 64, 9)),
        _ => None,
    };
    let overlays = [
        login_r,
        app.palette.is_some().then(|| palette_rect(area, app)),
        app.help.then(|| help_rect(area)),
        app.menu.as_ref().map(|m| menu_rect(area, m)),
        app.toast.as_ref().map(|(msg, _)| toast_rect(area, msg)),
    ];
    app.overlays = overlays.into_iter().flatten().collect();
    top_bar(f, top, app, &th);
    panes(f, body, app, &th);
    player_bar(f, player, app, &th);
    hint_bar(f.buffer_mut(), hints, &th);
    if let (Some(r), Some(CatalogueStatus::LoginPending { url, code })) = (login_r, app.catalogue.clone()) {
        login(f, r, &th, &url, &code);
    }
    if app.menu.is_some() {
        menu(f, area, app, &th);
    }
    if app.palette.is_some() {
        palette(f, area, app, &th);
    }
    if app.help {
        help(f, area, &th);
    }
    if let Some((msg, _)) = app.toast.clone() {
        let r = toast_rect(area, &msg);
        text(f.buffer_mut(), r.x, r.y, r.width, &format!("  {msg}  "), th.chip(th.accent));
    }
}

fn toast_rect(area: Rect, msg: &str) -> Rect {
    let w = (msg.chars().count() as u16 + 4).min(area.width);
    Rect::new(area.right().saturating_sub(w + 1), area.bottom().saturating_sub(5), w, 1)
}

fn palette_rect(area: Rect, app: &App) -> Rect {
    centered(area, 60, (app.palette_items().len() as u16 + 4).max(6))
}

fn menu_rect(area: Rect, m: &Menu) -> Rect {
    let w = m.items.iter().map(|(l, _)| l.chars().count() as u16).max().unwrap_or(0) + 6;
    let h = m.items.len() as u16 + 2;
    let x = m.at.0.min(area.right().saturating_sub(w));
    let y = if m.at.1 + h > area.bottom() { m.at.1.saturating_sub(h) } else { m.at.1 };
    Rect::new(x, y, w.min(area.width), h.min(area.height))
}

fn menu(f: &mut Frame, area: Rect, app: &mut App, th: &Theme) {
    let Some(m) = app.menu.as_ref() else { return };
    let r = menu_rect(area, m);
    let items = m.items.clone();
    let selected = m.selected;
    f.render_widget(Clear, r);
    let block = Block::default().borders(Borders::ALL).border_set(ratatui::symbols::border::ROUNDED).border_style(th.s(th.accent)).style(th.base());
    let inner = block.inner(r);
    f.render_widget(block, r);
    for (i, (label, _)) in items.iter().enumerate() {
        let y = inner.y + i as u16;
        let st = if i == selected { th.s(th.accent).bg(th.selection.color()) } else { th.s(th.fg) };
        text(f.buffer_mut(), inner.x, y, inner.width, &format!(" {label:<w$}", w = inner.width.saturating_sub(1) as usize), st);
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::MenuItem(i));
    }
}

fn top_bar(f: &mut Frame, r: Rect, app: &mut App, th: &Theme) {
    let buf = f.buffer_mut();
    text(buf, r.x + 1, r.y, 14, "◉ SINGULARITY", th.title());
    let (link, col) = match &app.link {
        Link::Connecting => ("connecting…".to_string(), th.dim),
        Link::Connected { version } => (format!("singularityd {version}"), th.dim),
        Link::Incompatible(_) => ("daemon incompatible".to_string(), th.error),
    };
    let engine = match &app.engine {
        None => String::new(),
        Some(EngineStatus::NotConfigured { .. }) => "engine: not configured".into(),
        Some(EngineStatus::Starting) => "engine: starting".into(),
        Some(EngineStatus::NeedsPairing { device_name }) => format!("pair \"{device_name}\" from your phone"),
        Some(EngineStatus::Ready { device_name, .. }) => format!("▶ {device_name}"),
        Some(EngineStatus::Failed { .. }) => "engine: failed".into(),
    };
    let account = match &app.catalogue {
        Some(CatalogueStatus::Ready { username, premium, .. }) => format!("{username}{}", if *premium { " · Premium" } else { "" }),
        Some(CatalogueStatus::LoginPending { code, .. }) => format!("log in: code {code}"),
        Some(CatalogueStatus::Connecting) => "connecting to Spotify…".into(),
        Some(CatalogueStatus::LoggedOut) => "not logged in (^K → Log in)".into(),
        Some(CatalogueStatus::Unavailable { .. }) => "Spotify browsing unavailable".into(),
        None => String::new(),
    };
    let right = [engine, account, link].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("  ·  ");
    let w = right.chars().count() as u16;
    text(buf, r.right().saturating_sub(w + 1), r.y, w, &right, th.s(col));
}

fn panes(f: &mut Frame, body: Rect, app: &mut App, th: &Theme) {
    let computed = app.layout.compute(body);
    for (slot, rect) in &computed.slots {
        let (title, todo) = slot_info(slot);
        // the main Pane is titled after what it shows
        let title = match (slot.as_str(), &app.view) {
            ("home", Some(v)) => v.title.clone(),
            _ => title.to_string(),
        };
        let focused = *slot == app.focus;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(th.borders)
            .border_style(th.s(if focused { th.border_focus } else { th.border }))
            .title(Span::styled(format!(" {title} "), if focused { th.title() } else { th.s(th.accent) }))
            .style(th.base());
        let inner = block.inner(*rect);
        // the Pane's own hit first, so the things inside it win
        app.hit(*rect, Hit::Focus(slot.clone()));
        if slot == "well" {
            // the well draws its own (sagging) frame
            crate::visual::well::draw(f.buffer_mut(), *rect, app, th);
            continue;
        }
        f.render_widget(block, *rect);
        if slot == "library" {
            crate::sidebar::draw(f.buffer_mut(), inner, app, th);
            continue;
        }
        if slot == "fft" {
            crate::meters::spectrum(f.buffer_mut(), inner, &app.meters, th);
            continue;
        }
        if slot == "loudness" {
            crate::meters::loudness(f.buffer_mut(), inner, &app.meters, th);
            continue;
        }
        if slot == "home" && app.view.is_some() {
            crate::tracks::draw(f.buffer_mut(), inner, app, th);
            continue;
        }
        if slot == "home" {
            if !home_status(f.buffer_mut(), inner, app, th) {
                crate::home::draw(f.buffer_mut(), inner, app, th);
            }
            continue;
        }
        if inner.height > 0 {
            let y = inner.y + inner.height / 2;
            let w = todo.chars().count() as u16;
            let x = inner.x + inner.width.saturating_sub(w) / 2;
            text(f.buffer_mut(), x, y, inner.width, todo, th.s(th.dim));
        }
    }
    // space bends around the hole: neighbouring border lines curve toward it
    if let Some(hole) = app.hole_at {
        crate::visual::well::bend_borders(f.buffer_mut(), body, hole, app.hole.bass());
    }
    // dividers last, so their grips win over the Panes underneath
    for (i, d) in computed.dividers.iter().enumerate() {
        app.hit(d.grip, Hit::Divider(i));
    }
    app.dividers = computed.dividers;
}

fn player_bar(f: &mut Frame, r: Rect, app: &mut App, th: &Theme) {
    let pb = app.playback.clone();
    let buf = f.buffer_mut();
    let now = pb.item.as_ref().map(|i| format!("{} — {}", i.name, i.artists.join(", "))).unwrap_or_else(|| "nothing playing".into());
    text(buf, r.x + 1, r.y, r.width, "singularity:~$ ", th.title());
    text(buf, r.x + 16, r.y, r.width.saturating_sub(17), &format!("now-playing --follow  # {now}"), th.s(th.fg));

    let y = r.y + 1;
    let (title, artists) = pb.item.as_ref().map(|i| (i.name.clone(), i.artists.join(", "))).unwrap_or_default();
    let tw = (r.width / 4).max(16);
    text(buf, r.x + 1, y, tw, &title, th.s(th.fg).add_modifier(Modifier::BOLD));
    text(buf, r.x + 1, y + 1, tw, &artists, th.s(th.dim));

    let mid_w = (r.width / 2).max(30).min(r.width.saturating_sub(4));
    let mx = r.x + (r.width - mid_w) / 2;
    let playing = pb.status == PlayStatus::Playing;
    let repeat = match pb.repeat {
        singularity_proto::RepeatMode::Off => " ↻",
        singularity_proto::RepeatMode::Context => " ↻",
        singularity_proto::RepeatMode::Track => "↻1",
    };
    let buttons = [
        (" ⇄", Action::Shuffle, pb.shuffle),
        ("|◀", Action::Previous, false),
        (if playing { "❚❚" } else { " ▶" }, Action::TogglePlay, false),
        ("▶|", Action::Next, false),
        (repeat, Action::Repeat, pb.repeat != singularity_proto::RepeatMode::Off),
    ];
    let bx = mx + mid_w.saturating_sub(buttons.len() as u16 * 5) / 2;
    for (i, (label, act, on)) in buttons.iter().enumerate() {
        let x = bx + i as u16 * 5;
        let st = if *act == Action::TogglePlay {
            th.chip(th.accent)
        } else if *on {
            th.s(th.accent).add_modifier(Modifier::BOLD)
        } else {
            th.s(if matches!(act, Action::Shuffle | Action::Repeat) { th.dim } else { th.fg })
        };
        text(buf, x, y, 4, &format!(" {label} "), st);
        app.hit(Rect::new(x, y, 4, 1), Hit::Action(*act));
    }
    // progress line
    let now_ms = singularity_proto::now_ms();
    let pos = app.display_position(now_ms);
    let dur = pb.item.as_ref().map(|i| i.duration_ms).unwrap_or(0);
    let scrubbing = app.scrub.as_ref().map(|s| s.bar);
    let bar = Rect::new(mx + 6, y + 1, mid_w.saturating_sub(12), 1);
    let seeking = scrubbing == Some(Bar::Seek);
    let buf = f.buffer_mut();
    text(buf, mx, y + 1, 6, &fmt_ms(pos), th.s(if seeking { th.accent } else { th.dim }));
    text(buf, bar.right() + 1, y + 1, 6, &fmt_ms(dur), th.s(th.dim));
    let (full, tip) = scrub::fill(pos, dur, bar.width);
    for x in 0..bar.width {
        let (ch, col) = if x < full { ("━", th.accent) } else if x == full && tip { ("╸", th.accent) } else { ("─", th.border) };
        text(buf, bar.x + x, bar.y, 1, ch, th.s(col));
    }
    if seeking {
        // the target time floats above the knob while dragging
        let label = format!(" {} ", fmt_ms(pos));
        let w = label.chars().count() as u16;
        let kx = (bar.x + full.min(bar.width.saturating_sub(1))).saturating_sub(w / 2).clamp(bar.x, bar.right().saturating_sub(w));
        text(buf, kx, y, w, &label, th.chip(th.accent));
    }
    app.hit(bar, Hit::Bar(Bar::Seek, bar));
    // volume
    let vw = 12u16;
    let vx = r.right().saturating_sub(vw + 2);
    if vx > bar.right() + 8 {
        let vol = app.display_volume();
        let vbar = Rect::new(vx, y + 1, vw, 1);
        let buf = f.buffer_mut();
        if scrubbing == Some(Bar::Volume) {
            text(buf, vx.saturating_sub(5), y + 1, 4, &format!("{vol:>3}%"), th.s(th.accent));
        } else {
            text(buf, vx.saturating_sub(4), y + 1, 3, "vol", th.s(th.dim));
        }
        let lit = (vol as f32 / 100.0 * vw as f32).round() as u16;
        for x in 0..vw {
            text(buf, vx + x, y + 1, 1, if x < lit { "▬" } else { "─" }, th.s(if x < lit { th.fg } else { th.border }));
        }
        app.hit(vbar, Hit::Bar(Bar::Volume, vbar));
    }
}

fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

const HINTS: [(&str, &str); 7] = [("␣", "play/pause"), ("⇧←→", "seek"), ("n/b", "next/prev"), ("Tab", "focus"), ("^K", "palette"), ("?", "help"), ("q", "quit")];

/// Shows as many hints as fit; when space runs out, the least important go first
/// (play/pause, palette, help and quit always stay).
fn hint_bar(buf: &mut Buffer, r: Rect, th: &Theme) {
    const DROP_ORDER: [&str; 3] = ["n/b", "Tab", "⇧←→"];
    let width = |hs: &[(&str, &str)]| hs.iter().map(|(k, d)| k.chars().count() as u16 + d.chars().count() as u16 + 5).sum::<u16>() + 1;
    let mut hints: Vec<(&str, &str)> = HINTS.to_vec();
    for drop in DROP_ORDER {
        if width(&hints) <= r.width {
            break;
        }
        hints.retain(|(k, _)| *k != drop);
    }
    let mut x = r.x + 1;
    for (k, d) in hints {
        let kw = k.chars().count() as u16 + 2;
        let dw = d.chars().count() as u16 + 3;
        if x + kw + dw > r.right() {
            break;
        }
        text(buf, x, r.y, kw, &format!(" {k} "), th.chip(th.dim));
        text(buf, x + kw, r.y, dw, &format!(" {d}"), th.s(th.dim));
        x += kw + dw;
    }
}

fn centered(r: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(r.width), h.min(r.height));
    Rect::new(r.x + (r.width - w) / 2, r.y + (r.height - h) / 3, w, h)
}

fn overlay_block<'a>(th: &Theme, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_set(ratatui::symbols::border::ROUNDED)
        .border_style(th.s(th.accent))
        .title(Span::styled(format!(" {title} "), th.title()))
        .style(th.base())
}

fn palette(f: &mut Frame, area: Rect, app: &mut App, th: &Theme) {
    let items = app.palette_items();
    let (query, selected) = app.palette.as_ref().map(|p| (p.query.clone(), p.selected)).unwrap_or_default();
    let r = palette_rect(area, app);
    f.render_widget(Clear, r);
    let block = overlay_block(th, "Command palette");
    let inner = block.inner(r);
    f.render_widget(block, r);
    let buf = f.buffer_mut();
    text(buf, inner.x + 1, inner.y, inner.width.saturating_sub(2), &format!("› {query}▏"), th.s(th.fg).add_modifier(Modifier::BOLD));
    for (i, a) in items.iter().enumerate() {
        let y = inner.y + 1 + i as u16;
        if y >= inner.bottom() {
            break;
        }
        let row = Rect::new(inner.x, y, inner.width, 1);
        let st = if i == selected { th.s(th.accent).bg(th.selection.color()) } else { th.s(th.fg) };
        let buf = f.buffer_mut();
        text(buf, inner.x, y, inner.width, &" ".repeat(inner.width as usize), st);
        text(buf, inner.x + 2, y, inner.width.saturating_sub(12), a.label(), st);
        let k = a.key();
        text(buf, inner.right().saturating_sub(k.chars().count() as u16 + 2), y, 10, k, th.s(th.dim));
        app.hit(row, Hit::PaletteItem(i));
    }
}

/// Home's status line while there are no Shelves to show. Returns false once the feed can be drawn.
fn home_status(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) -> bool {
    let m = match (&app.catalogue, &app.home) {
        (_, Some(h)) if !h.shelves.is_empty() => return false,
        (Some(CatalogueStatus::LoginPending { .. }), _) => "waiting for the Spotify login…",
        (Some(CatalogueStatus::LoggedOut), _) => "not logged in · Ctrl+K → \"Log in to Spotify\"",
        (Some(CatalogueStatus::Connecting), _) | (Some(CatalogueStatus::Ready { .. }), None) => "loading your Home…",
        (Some(CatalogueStatus::Unavailable { .. }), _) => "Spotify browsing is unavailable right now; playback still works",
        (Some(CatalogueStatus::Ready { .. }), Some(_)) => "your Home is empty right now",
        (None, _) => "Home appears once the Catalogue is connected",
    };
    let w = m.chars().count() as u16;
    text(buf, r.x + r.width.saturating_sub(w) / 2, r.y + r.height / 2, r.width, m, th.s(th.dim));
    true
}

fn login(f: &mut Frame, r: Rect, th: &Theme, url: &str, code: &str) {
    f.render_widget(Clear, r);
    let block = overlay_block(th, "Log in to Spotify");
    let inner = block.inner(r);
    f.render_widget(block, r);
    let buf = f.buffer_mut();
    text(buf, inner.x + 2, inner.y + 1, inner.width.saturating_sub(4), "Your browser should have opened. If not, go to:", th.s(th.fg));
    text(buf, inner.x + 2, inner.y + 2, inner.width.saturating_sub(4), url, th.s(th.accent2));
    text(buf, inner.x + 2, inner.y + 4, 12, "and enter", th.s(th.fg));
    text(buf, inner.x + 12, inner.y + 4, 20, &format!(" {code} "), th.chip(th.accent));
    text(buf, inner.x + 2, inner.y + 6, inner.width.saturating_sub(4), "Singularity never sees your password.  Esc hides this.", th.s(th.dim));
}

const HELP_ROWS: [(&str, &str); 8] = [
    ("Mouse", "click anything · drag pane borders · drag seek/volume · scroll"),
    ("Ctrl+K  :", "command palette: find any action by typing"),
    ("Space", "play / pause"),
    ("⇧← ⇧→", "seek 5 s    , .  seek 1 s    n / b  next / previous"),
    ("+ -", "volume           s / r   shuffle / repeat"),
    ("↑↓←→ ⏎", "move through Home, play · right-click or Menu key: more"),
    ("Tab", "focus the next pane"),
    ("q / Esc", "quit the Client (music keeps playing)"),
];

fn help_rect(area: Rect) -> Rect {
    centered(area, 76, HELP_ROWS.len() as u16 + 4)
}

fn help(f: &mut Frame, area: Rect, th: &Theme) {
    let rows = HELP_ROWS;
    let r = help_rect(area);
    f.render_widget(Clear, r);
    let block = overlay_block(th, "You only need these");
    let inner = block.inner(r);
    f.render_widget(block, r);
    let buf = f.buffer_mut();
    for (i, (k, d)) in rows.iter().enumerate() {
        let y = inner.y + 1 + i as u16;
        text(buf, inner.x + 2, y, 12, k, th.title());
        text(buf, inner.x + 14, y, inner.width.saturating_sub(15), d, th.s(th.fg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn click(app: &mut App, x: u16, y: u16) {
        app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE });
        app.mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE });
    }

    #[test]
    fn renders_at_many_sizes_without_panicking() {
        for (w, h) in [(80, 24), (120, 40), (300, 80), (20, 8), (10, 3)] {
            let mut app = App::new(None);
            let s = render(&mut app, w, h);
            if w >= 80 {
                assert!(s.contains("SINGULARITY"), "{w}x{h}");
                assert!(s.contains("home") && s.contains("gravity well"), "{w}x{h}");
            }
        }
    }

    #[test]
    fn snapshot_80x24_has_shell_parts() {
        let mut app = App::new(None);
        let s = render(&mut app, 80, 24);
        for part in ["◉ SINGULARITY", "~/library", "now-playing", "palette", "quit"] {
            assert!(s.contains(part), "missing {part:?} in\n{s}");
        }
    }

    #[test]
    fn clicking_a_pane_focuses_it() {
        let mut app = App::new(None);
        render(&mut app, 160, 48);
        let well = app.layout.compute(Rect::new(0, 1, 160, 48 - 5)).slots.into_iter().find(|(s, _)| s == "well").unwrap().1;
        click(&mut app, well.x + 5, well.y + 3);
        assert_eq!(app.focus, "well");
    }

    #[test]
    fn dragging_a_divider_resizes_panes() {
        let mut app = App::new(None);
        let body = Rect::new(0, 1, 160, 48 - 5);
        render(&mut app, 160, 48);
        let before = app.layout.compute(body).slots[0].1.width;
        let grip = app.dividers[0].grip;
        app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: grip.x, row: grip.y + 5, modifiers: KeyModifiers::NONE });
        app.mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Left), column: grip.x + 10, row: grip.y + 5, modifiers: KeyModifiers::NONE });
        app.mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: grip.x + 10, row: grip.y + 5, modifiers: KeyModifiers::NONE });
        let after = app.layout.compute(body).slots[0].1.width;
        assert_eq!(after, before + 10);
    }

    #[test]
    fn palette_filters_and_runs_actions() {
        let mut app = App::new(None);
        app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert!(app.palette.is_some());
        for c in "help".chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.palette_items(), vec![Action::Help]);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.palette.is_none() && app.help);
        // and by mouse
        app.help = false;
        app.run(Action::Palette);
        render(&mut app, 120, 40);
        let quit_idx = app.palette_items().iter().position(|a| *a == Action::Quit).unwrap();
        // palette rows start one line below the query line inside the box
        let s = render(&mut app, 120, 40);
        let row = s.lines().position(|l| l.contains("Quit (music keeps playing)")).unwrap() as u16;
        let col = s.lines().nth(row as usize).unwrap().chars().position(|c| c == 'Q').unwrap() as u16;
        let _ = quit_idx;
        click(&mut app, col, row);
        assert!(app.quit);
    }

    fn playing_app(dur_ms: u64) -> (App, tokio::sync::mpsc::Receiver<singularity_proto::Command>) {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let mut app = App::new(Some(tx));
        app.link = Link::Connected { version: "test".into() };
        app.playback.item = Some(singularity_proto::Item { name: "Song".into(), duration_ms: dur_ms, ..Default::default() });
        app.playback.status = PlayStatus::Paused;
        (app, rx)
    }

    fn sent(rx: &mut tokio::sync::mpsc::Receiver<singularity_proto::Command>) -> Vec<singularity_proto::Command> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    fn seek_bar(app: &App) -> Rect {
        app.hits.iter().find_map(|(_, h)| match h { Hit::Bar(Bar::Seek, r) => Some(*r), _ => None }).unwrap()
    }

    fn mouse(app: &mut App, kind: MouseEventKind, fx: f64, row: u16, precise: bool) {
        app.pointer(MouseEvent { kind, column: fx as u16, row, modifiers: KeyModifiers::NONE }, fx, precise);
    }

    #[test]
    fn dragging_the_seek_bar_shows_a_tooltip_and_seeks_once_on_release() {
        use singularity_proto::Command;
        let (mut app, mut rx) = playing_app(240_000);
        render(&mut app, 120, 40);
        let bar = seek_bar(&app);
        let row = bar.y;
        let x0 = bar.x as f64 + 10.5;
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x0, row, false);
        let start = app.scrub.as_ref().unwrap().value;
        for d in 1..=7 {
            mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x0 + d as f64, row, false);
        }
        assert!(sent(&mut rx).is_empty(), "no seeks while dragging");
        let s = render(&mut app, 120, 40);
        let target = fmt_ms(start + 7000);
        assert!(s.lines().nth(row as usize - 1).unwrap().contains(&format!(" {target} ")), "tooltip with {target} above the bar:\n{s}");
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x0 + 7.0, row, false);
        assert_eq!(sent(&mut rx), vec![Command::Seek { position_ms: start + 7000 }]);
        assert!(app.scrub.is_none());
        assert_eq!(app.display_position(app.playback.timestamp_ms), start + 7000, "bar stays at the target");
    }

    #[test]
    fn pixel_pointer_lands_on_the_exact_second() {
        use singularity_proto::Command;
        let (mut app, mut rx) = playing_app(240_000);
        render(&mut app, 120, 40);
        let bar = seek_bar(&app);
        // aim for 1:37 with a sub-cell pointer
        let f = 97_000.0 / 240_000.0;
        let fx = bar.x as f64 + 0.5 + f * (bar.width - 1) as f64;
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), fx, bar.y, true);
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), fx, bar.y, true);
        assert_eq!(sent(&mut rx), vec![Command::Seek { position_ms: 97_000 }]);
    }

    #[test]
    fn wheel_and_shift_arrows_nudge_one_second_and_coalesce() {
        use singularity_proto::Command;
        let (mut app, mut rx) = playing_app(240_000);
        app.playback.position_ms = 60_000;
        app.playback.timestamp_ms = singularity_proto::now_ms();
        render(&mut app, 120, 40);
        let bar = seek_bar(&app);
        for _ in 0..3 {
            mouse(&mut app, MouseEventKind::ScrollUp, bar.x as f64 + 3.5, bar.y, false);
        }
        app.key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE));
        assert_eq!(app.display_position(singularity_proto::now_ms()), 62_000);
        app.tick();
        assert!(sent(&mut rx).is_empty(), "waits for the nudging to settle");
        std::thread::sleep(crate::app::NUDGE_SETTLE + std::time::Duration::from_millis(20));
        app.tick();
        assert_eq!(sent(&mut rx), vec![Command::Seek { position_ms: 62_000 }]);
    }

    #[test]
    fn volume_wheel_moves_one_percent() {
        use singularity_proto::Command;
        let (mut app, mut rx) = playing_app(240_000);
        app.playback.volume = 50;
        render(&mut app, 160, 40);
        let vbar = app.hits.iter().find_map(|(_, h)| match h { Hit::Bar(Bar::Volume, r) => Some(*r), _ => None }).unwrap();
        mouse(&mut app, MouseEventKind::ScrollDown, vbar.x as f64 + 1.5, vbar.y, false);
        assert_eq!(app.display_volume(), 49);
        std::thread::sleep(crate::app::NUDGE_SETTLE + std::time::Duration::from_millis(20));
        app.tick();
        assert_eq!(sent(&mut rx), vec![Command::SetVolume { percent: 49 }]);
    }

    // ── Home (#8): recorded, anonymised feed in, state and screen out ──

    fn home_app() -> (App, tokio::sync::mpsc::Receiver<singularity_proto::Command>) {
        let (mut app, rx) = playing_app(200_000);
        let feed: singularity_proto::HomeFeed = serde_json::from_str(include_str!("../tests/fixtures/home.json")).unwrap();
        // a few covers have arrived: solid colours, so the tier is visible in the buffer
        for (i, url) in feed.shelves.iter().flat_map(|s| &s.cards).filter_map(|c| c.image_url.clone()).take(40).enumerate() {
            let c = [(i * 40 % 255) as u8, 90, 160];
            app.covers.insert(&url, image::RgbImage::from_pixel(300, 300, image::Rgb(c)));
        }
        app.catalogue = Some(CatalogueStatus::Ready { username: "u".into(), country: "SE".into(), premium: true });
        app.home = Some(feed);
        (app, rx)
    }

    fn card_rect(app: &App, shelf: usize, card: usize) -> Rect {
        try_card_rect(app, shelf, card).unwrap()
    }

    fn try_card_rect(app: &App, shelf: usize, card: usize) -> Option<Rect> {
        app.hits.iter().find_map(|(r, h)| (*h == Hit::Card(crate::home::CardRef { shelf, card })).then_some(*r))
    }

    /// Wheel Home down until the Shelf's first Card is on screen.
    fn scroll_to(app: &mut App, shelf: usize) -> Rect {
        let home = app.layout.compute(Rect::new(0, 1, 200, 64 - 5)).slots.into_iter().find(|(s, _)| s == "home").unwrap().1;
        for _ in 0..40 {
            render(app, 200, 64);
            if let Some(r) = try_card_rect(app, shelf, 0) {
                return r;
            }
            mouse(app, MouseEventKind::ScrollDown, home.x as f64 + 3.5, home.y + 2, false);
        }
        panic!("Shelf {shelf} never came into view");
    }

    #[test]
    fn home_snapshot_shows_shortcuts_shelves_and_covers() {
        let (mut app, _rx) = home_app();
        let s = render(&mut app, 200, 64);
        for part in ["Liked Songs", "Daily Mix 1", "Made For You"] {
            assert!(s.contains(part), "missing {part:?}:\n{s}");
        }
        assert!(s.contains('▀'), "covers drawn as half-blocks in the cell tier");
        assert!(s.contains('›'), "long Shelves can scroll sideways");
        // never a Card cut in half at the bottom
        let home = app.layout.compute(Rect::new(0, 1, 200, 64 - 5)).slots.into_iter().find(|(s, _)| s == "home").unwrap().1;
        assert!(app.hits.iter().all(|(r, h)| !matches!(h, Hit::Card(_)) || r.bottom() < home.bottom()));
    }

    #[test]
    fn home_scrolls_by_rows_to_the_feed_tail() {
        let (mut app, _rx) = home_app();
        render(&mut app, 200, 64);
        scroll_to(&mut app, 30);
        let s = render(&mut app, 200, 64);
        assert!(s.contains("Feed pick 20"), "the feed tail is reachable:\n{s}");
        assert!(s.contains('♪'), "covers still loading show a placeholder");
        assert!(s.contains("Because you"), "feed Cards carry their section title");
        assert!(!s.contains("Liked Songs"));
    }

    #[test]
    fn clicking_a_card_opens_its_list_and_play_plays_it() {
        use singularity_proto::Command;
        let (mut app, mut rx) = home_app();
        render(&mut app, 200, 64);
        let r = card_rect(&app, 1, 0);
        click(&mut app, r.x + 2, r.y + 1);
        // never plays on a click: the list opens in the main Pane
        assert_eq!(sent(&mut rx), vec![Command::GetTracks { uri: "spotify:playlist:mfy1".into() }]);
        assert_eq!(app.focus, "home");
        let s = render(&mut app, 200, 64);
        assert!(s.contains("Daily Mix 1") && s.contains("▶ Play"), "{s}");
        let play = app.hits.iter().find_map(|(r, h)| (*h == Hit::PlayView).then_some(*r)).unwrap();
        click(&mut app, play.x + 2, play.y);
        assert_eq!(sent(&mut rx), vec![Command::Play { uri: Some("spotify:playlist:mfy1".into()) }]);
        // Esc is back to Home, where it was
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.view.is_none());
    }

    #[test]
    fn home_shelves_scroll_sideways_with_shift_wheel() {
        let (mut app, _rx) = home_app();
        render(&mut app, 200, 64);
        let rad = scroll_to(&mut app, 2);
        app.pointer(MouseEvent { kind: MouseEventKind::ScrollDown, column: rad.x + 1, row: rad.y + 1, modifiers: KeyModifiers::SHIFT }, rad.x as f64 + 1.5, false);
        render(&mut app, 200, 64);
        assert_eq!(card_rect(&app, 2, 1).x, rad.x, "the Shelf moved one Card left");
    }

    #[test]
    fn arrows_browse_home_and_enter_opens() {
        use singularity_proto::Command;
        let (mut app, mut rx) = home_app();
        app.focus = "home".into();
        render(&mut app, 200, 64);
        let key = |app: &mut App, c| app.key(KeyEvent::new(c, KeyModifiers::NONE));
        key(&mut app, KeyCode::Down); // selects the first Card
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert_eq!(sent(&mut rx), vec![Command::GetTracks { uri: "spotify:playlist:mix1".into() }]);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        // Shift+arrows still seek while Home has focus
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        assert_eq!(app.display_position(singularity_proto::now_ms()), 5000);
    }

    #[test]
    fn right_click_menu_offers_play_go_to_and_copy_link() {
        let (mut app, mut rx) = home_app();
        render(&mut app, 200, 64);
        let r = scroll_to(&mut app, 3);
        app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: r.x + 1, row: r.y + 1, modifiers: KeyModifiers::NONE });
        let s = render(&mut app, 200, 64);
        assert!(s.contains("Play") && s.contains("Go to album") && s.contains("Copy link"), "{s}");
        let link = s.lines().position(|l| l.contains("Copy link")).unwrap() as u16;
        let col = s.lines().nth(link as usize).unwrap().chars().position(|c| c == 'C').unwrap() as u16;
        click(&mut app, col, link);
        assert!(app.menu.is_none());
        assert_eq!(app.clipboard.as_deref(), Some("https://open.spotify.com/album/rp0"));
        assert!(sent(&mut rx).is_empty());
    }

    #[test]
    fn pixel_tier_places_covers_and_steps_down_under_popups() {
        let (mut app, _rx) = home_app();
        app.kitty = Some(crate::gfx::Kitty::new(false));
        app.cell_px = Some((10.0, 20.0));
        let s = render(&mut app, 200, 64);
        assert!(!s.contains('▀'), "no half-blocks when kitty draws the covers");
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.matches("a=t,").count() >= 10 && out.contains("a=p,"), "covers transmitted and placed");
        // the palette covers the middle of Home: those covers become half-blocks, the rest stay pixels
        app.run(Action::Palette);
        let s = render(&mut app, 200, 64);
        assert!(s.contains('▀'));
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("a=d,d=i") && !out.contains("a=t,"), "hidden placements removed, nothing re-sent");
    }

    // ── Library (#9) ──

    fn library_app() -> (App, tokio::sync::mpsc::Receiver<singularity_proto::Command>) {
        use singularity_proto::{Library, LibraryItem};
        let (mut app, rx) = playing_app(200_000);
        let item = |uri: &str, name: &str, kind: &str, depth| LibraryItem { uri: uri.into(), name: name.into(), subtitle: String::new(), kind: kind.into(), image_url: None, depth };
        app.library = Some(Library {
            items: vec![
                item("spotify:collection", "Liked Songs", "liked", 0),
                item("spotify:user:u:folder:f", "Moods", "folder", 0),
                item("spotify:playlist:p", "Night Drive", "playlist", 1),
                item("spotify:album:a", "Quiet Orbit", "album", 0),
            ],
        });
        (app, rx)
    }

    fn page(offset: u32, n: u32, total: u32) -> singularity_proto::TrackPage {
        let tracks = (offset..offset + n)
            .map(|i| singularity_proto::TrackRow {
                uri: format!("spotify:track:t{i}"),
                name: format!("Song {:04}", total - i), // reverse alphabetical, so sorting visibly reorders
                artists: vec![if i % 2 == 0 { "Nova Drift".into() } else { "Mira Vale".into() }],
                album: "Album".into(),
                added_at: Some("2025-01-01T00:00:00Z".into()),
                duration_ms: 180_000 + i as u64,
                playable: true,
            })
            .collect();
        singularity_proto::TrackPage { uri: "spotify:collection".into(), offset, total, tracks }
    }

    fn side_rect(app: &App, row: usize) -> Rect {
        app.hits.iter().find_map(|(r, h)| (*h == Hit::Side(row)).then_some(*r)).unwrap()
    }

    #[test]
    fn sidebar_lists_the_library_with_folders_indented() {
        let (mut app, _rx) = library_app();
        let s = render(&mut app, 160, 48);
        for part in ["⌂ Home", "♥ Liked Songs", "▸ Moods", "≡ Night Drive", "◎ Quiet Orbit"] {
            assert!(s.contains(part), "missing {part:?}\n{s}");
        }
        let moods = side_rect(&app, 2);
        let line = s.lines().nth(moods.y as usize + 1).unwrap();
        assert!(line.find('≡').unwrap() > s.lines().nth(moods.y as usize).unwrap().find('▸').unwrap(), "the playlist sits inside the folder");
    }

    #[test]
    fn liked_songs_load_progressively_and_play_from_the_clicked_track() {
        use singularity_proto::Command;
        let (mut app, mut rx) = library_app();
        render(&mut app, 160, 48);
        let r = side_rect(&app, 1);
        click(&mut app, r.x + 3, r.y);
        assert_eq!(sent(&mut rx), vec![Command::GetTracks { uri: "spotify:collection".into() }]);
        app.track_page(page(0, 100, 3000));
        let s = render(&mut app, 160, 48);
        assert!(s.contains("Liked Songs") && s.contains("loading 100/3000"), "{s}");
        // the rest streams in; the list stays usable meanwhile
        for off in (100..3000).step_by(100) {
            app.track_page(page(off, 100, 3000));
        }
        let s = render(&mut app, 160, 48);
        assert!(s.contains("3000 songs") && !s.contains("loading"));
        // double-click the third row
        let row = app.hits.iter().find_map(|(r, h)| (*h == Hit::Track(2)).then_some(*r)).unwrap();
        click(&mut app, row.x + 5, row.y);
        click(&mut app, row.x + 5, row.y);
        assert_eq!(sent(&mut rx), vec![Command::PlayFrom { context_uri: "spotify:collection".into(), index: 2, track_uri: Some("spotify:track:t2".into()) }]);
    }

    #[test]
    fn sorted_and_filtered_rows_still_play_from_their_place_in_the_list() {
        use singularity_proto::Command;
        let (mut app, mut rx) = library_app();
        render(&mut app, 160, 48);
        let r = side_rect(&app, 1);
        click(&mut app, r.x + 3, r.y);
        sent(&mut rx);
        app.track_page(page(0, 50, 50));
        render(&mut app, 160, 48);
        // sort by title via the header
        let title = app.hits.iter().find_map(|(r, h)| (*h == Hit::SortCol(crate::tracks::Col::Title)).then_some(*r)).unwrap();
        click(&mut app, title.x, title.y);
        let s = render(&mut app, 160, 48);
        assert!(s.contains("Title ↑"));
        // type-to-filter: "/" then text
        let key = |app: &mut App, c| app.key(KeyEvent::new(c, KeyModifiers::NONE));
        key(&mut app, KeyCode::Char('/'));
        for c in "mira".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Enter);
        let s = render(&mut app, 160, 48);
        assert!(s.contains("filter: mira") && s.contains("(25 shown)"), "{s}");
        assert!(!s.contains("Nova Drift"));
        // the first row now is the alphabetically first Mira Vale song: Song 0001 = t49
        key(&mut app, KeyCode::Enter);
        assert_eq!(sent(&mut rx), vec![Command::PlayFrom { context_uri: "spotify:collection".into(), index: 49, track_uri: Some("spotify:track:t49".into()) }]);
        // Backspace goes back to Home
        key(&mut app, KeyCode::Backspace);
        assert!(app.view.is_none());
    }

    #[test]
    fn spectrum_frames_light_the_fft_and_loudness_panes() {
        let mut app = App::new(None);
        for i in 0..60u32 {
            let bands = (0..64).map(|b| ((b * 4 + i as usize) % 255) as u8).collect();
            app.meters.push(singularity_proto::SpectrumFrame { bands, level: 180, vu: [220, 200], beat: i % 15 == 0 });
        }
        let s = render(&mut app, 160, 48);
        let slots = app.layout.compute(Rect::new(0, 1, 160, 48 - 5)).slots;
        let pane = |name: &str| slots.iter().find(|(s, _)| s == name).unwrap().1;
        let region = |r: Rect| s.lines().skip(r.y as usize + 1).take(r.height as usize - 2).map(|l| l.chars().skip(r.x as usize + 1).take(r.width as usize - 2).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert!(region(pane("fft")).contains('█'), "{}", region(pane("fft")));
        let loud = region(pane("loudness"));
        assert!(loud.starts_with("L ▬") && loud.contains('█'), "{loud}");
        assert!(!s.contains("arrives with #15"));
    }

    #[test]
    fn the_well_cycles_looks_and_sends_animated_frames_in_the_pixel_tier() {
        let mut app = App::new(None);
        let s = render(&mut app, 160, 48);
        assert!(s.contains("gravity well") && s.contains("◂ Inferno ▸"));
        app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        assert!(render(&mut app, 160, 48).contains("◂ Gargantua ▸"));
        let well = app.layout.compute(Rect::new(0, 1, 160, 48 - 5)).slots.into_iter().find(|(s, _)| s == "well").unwrap().1;
        click(&mut app, well.x + well.width / 2, well.y + well.height / 2);
        assert!(render(&mut app, 160, 48).contains("◂ M87* ▸") && app.focus == "well");
        // pixel tier: one animated frame per draw, covering the well's inside
        app.kitty = Some(crate::gfx::Kitty::new(false));
        app.cell_px = Some((10.0, 20.0));
        render(&mut app, 160, 48);
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        // (the CPU renderer caps the resolution; the terminal scales it into the cells)
        assert!(out.contains("a=T,f=32,o=z,i=2,") && out.contains(&format!("p=1,c={},r={},C=1,z=-1", well.width - 4, well.height - 2)), "{}", &out[..out.len().min(200)]);
    }

    #[test]
    fn bending_never_moves_text() {
        // every letter on screen sits where it would without the hole
        let mut app = App::new(None);
        let with = render(&mut app, 160, 48);
        let hole = app.hole_at.expect("the well is on screen");
        let mut plain = App::new(None);
        let mut t = Terminal::new(TestBackend::new(160, 48)).unwrap();
        t.draw(|f| {
            draw(f, &mut plain);
        })
        .unwrap();
        let without: Vec<Vec<char>> = (0..48u16).map(|y| (0..160u16).map(|x| t.backend().buffer()[(x, y)].symbol().chars().next().unwrap_or(' ')).collect()).collect();
        let structural = |c: char| c == ' ' || ('\u{2500}'..='\u{257f}').contains(&c) || ('\u{2800}'..='\u{28ff}').contains(&c);
        for (y, line) in with.lines().enumerate() {
            for (x, c) in line.chars().enumerate() {
                let inside_well = x as u16 >= hole.well.x && (x as u16) < hole.well.right() && y as u16 >= hole.well.y && (y as u16) < hole.well.bottom();
                if !inside_well && (!structural(c) || !structural(without[y][x])) {
                    assert_eq!(c, without[y][x], "cell ({x},{y}) changed");
                }
            }
        }
    }
}
