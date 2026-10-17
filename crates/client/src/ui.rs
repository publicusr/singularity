//! Rendering. Pure function of `App` state, plus hit-box registration for the mouse.

use crate::app::{Action, App, Hit, Link};
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
        "home" => ("home", "your Home feed arrives with #8"),
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
    top_bar(f, top, app, &th);
    panes(f, body, app, &th);
    player_bar(f, player, app, &th);
    hint_bar(f.buffer_mut(), hints, &th);
    if let Some(CatalogueStatus::LoginPending { url, code }) = app.catalogue.clone() {
        if !app.login_hidden {
            login(f, area, &th, &url, &code);
        }
    }
    if app.palette.is_some() {
        palette(f, area, app, &th);
    }
    if app.help {
        help(f, area, &th);
    }
    if let Some((msg, _)) = app.toast.clone() {
        let w = (msg.chars().count() as u16 + 4).min(area.width);
        text(f.buffer_mut(), area.right().saturating_sub(w + 1), area.bottom().saturating_sub(5), w, &format!("  {msg}  "), th.chip(th.accent));
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
        let focused = *slot == app.focus;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(th.borders)
            .border_style(th.s(if focused { th.border_focus } else { th.border }))
            .title(Span::styled(format!(" {title} "), if focused { th.title() } else { th.s(th.accent) }))
            .style(th.base());
        let inner = block.inner(*rect);
        f.render_widget(block, *rect);
        if slot == "home" && home_preview(f.buffer_mut(), inner, app, th) {
            app.hit(*rect, Hit::Focus(slot.clone()));
            continue;
        }
        if inner.height > 0 {
            let y = inner.y + inner.height / 2;
            let w = todo.chars().count() as u16;
            let x = inner.x + inner.width.saturating_sub(w) / 2;
            text(f.buffer_mut(), x, y, inner.width, todo, th.s(th.dim));
        }
        app.hit(*rect, Hit::Focus(slot.clone()));
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

const HINTS: [(&str, &str); 7] = [("␣", "play/pause"), ("←→", "seek"), ("n/b", "next/prev"), ("Tab", "focus"), ("^K", "palette"), ("?", "help"), ("q", "quit")];

/// Shows as many hints as fit; when space runs out, the least important go first
/// (play/pause, palette, help and quit always stay).
fn hint_bar(buf: &mut Buffer, r: Rect, th: &Theme) {
    const DROP_ORDER: [&str; 3] = ["n/b", "Tab", "←→"];
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
    let r = centered(area, 60, (items.len() as u16 + 4).max(6));
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

/// Text preview of the Home feed (cover cards replace this in #8). Returns false when there's nothing to show.
fn home_preview(buf: &mut Buffer, r: Rect, app: &App, th: &Theme) -> bool {
    let msg = match (&app.catalogue, &app.home) {
        (_, Some(h)) if !h.shelves.is_empty() => None,
        (Some(CatalogueStatus::LoggedOut), _) => Some("not logged in · Ctrl+K → \"Log in to Spotify\""),
        (Some(CatalogueStatus::Connecting), _) | (Some(CatalogueStatus::Ready { .. }), None) => Some("loading your Home…"),
        (Some(CatalogueStatus::Unavailable { .. }), _) => Some("Spotify browsing is unavailable right now; playback still works"),
        _ => return false,
    };
    if let Some(m) = msg {
        let w = m.chars().count() as u16;
        text(buf, r.x + r.width.saturating_sub(w) / 2, r.y + r.height / 2, r.width, m, th.s(th.dim));
        return true;
    }
    let Some(h) = &app.home else { return false };
    let mut y = r.y;
    for shelf in h.shelves.iter().filter(|s| s.kind != "feed") {
        if y + 1 >= r.bottom() {
            break;
        }
        let title = if shelf.title.is_empty() { "Jump back in" } else { shelf.title.as_str() };
        text(buf, r.x + 1, y, r.width.saturating_sub(2), title, th.s(th.accent).add_modifier(Modifier::BOLD));
        y += 1;
        let names: Vec<&str> = shelf.cards.iter().map(|c| c.name.as_str()).collect();
        text(buf, r.x + 2, y, r.width.saturating_sub(3), &names.join("  ·  "), th.s(th.fg));
        y += 2;
    }
    true
}

fn login(f: &mut Frame, area: Rect, th: &Theme, url: &str, code: &str) {
    let r = centered(area, 64, 9);
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

fn help(f: &mut Frame, area: Rect, th: &Theme) {
    let rows = [
        ("Mouse", "click anything · drag pane borders · drag seek/volume · scroll"),
        ("Ctrl+K  :", "command palette: find any action by typing"),
        ("Space", "play / pause"),
        ("← →", "seek 5 s          n / b   next / previous"),
        ("+ -", "volume           s / r   shuffle / repeat"),
        ("Tab", "focus the next pane"),
        ("q / Esc", "quit the Client (music keeps playing)"),
    ];
    let r = centered(area, 72, rows.len() as u16 + 4);
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
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
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
}
