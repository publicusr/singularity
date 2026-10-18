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
        "stream" => ("stream", ""),
        "home" => ("home", ""),
        "queue" => ("queue", ""),
        "well" => ("gravity well", "the black hole arrives with #16"),
        "lyrics" => ("tail -f lyrics.log", ""),
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
        // the wizard shows the code itself
        Some(CatalogueStatus::LoginPending { .. }) if !app.login_hidden && !crate::setup::wanted(app) => Some(centered(area, 64, 9)),
        _ => None,
    };
    let overlays = [
        login_r,
        app.palette.is_some().then(|| palette_rect(area, app)),
        app.help.then(|| help_rect(area, help_rows(&app.keymap).len())),
        crate::setup::wanted(app).then(|| crate::setup::rect(area)),
        app.health_open.then(|| crate::setup::health_rect(area)),
        app.devices_open.then(|| devices_rect(area, app)),
        app.menu.as_ref().map(|m| menu_rect(area, m)),
        app.toast.as_ref().map(|(msg, _)| toast_rect(area, msg)),
    ];
    app.overlays = overlays.into_iter().flatten().collect();
    top_bar(f, top, app, &th);
    panes(f, body, app, &th);
    player_bar(f, player, app, &th);
    hint_bar(f.buffer_mut(), hints, &th, &app.keymap);
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
        help(f, area, &th, &app.keymap);
    }
    if app.devices_open {
        devices(f.buffer_mut(), area, app, &th);
    }
    if app.health_open {
        crate::setup::draw_health(f.buffer_mut(), crate::setup::health_rect(area), app, &th);
    }
    if crate::setup::wanted(app) {
        crate::setup::draw_wizard(f.buffer_mut(), crate::setup::rect(area), app, &th);
    }
    if let Some((msg, _)) = app.toast.clone() {
        let r = toast_rect(area, &msg);
        text(f.buffer_mut(), r.x, r.y, r.width, &format!("  {msg}  "), th.chip(th.accent));
    }
}

fn devices_rect(area: Rect, app: &App) -> Rect {
    centered(area, 56, app.devices.len().max(1) as u16 + 5)
}

/// The device picker: where music plays. Click (or Enter) moves it there.
fn devices(buf: &mut Buffer, area: Rect, app: &mut App, th: &Theme) {
    let r = devices_rect(area, app);
    let inner = popup(buf, r, th, "Play on");
    if app.devices.is_empty() {
        text(buf, inner.x + 2, inner.y + 1, inner.width.saturating_sub(4), "looking for devices…", th.s(th.dim));
        return;
    }
    let list = app.devices.clone();
    for (i, d) in list.iter().enumerate() {
        let y = inner.y + 1 + i as u16;
        let sel = i == app.devices_sel;
        let st = if d.active { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.fg) };
        let bg = if sel { th.selection.color() } else { th.bg.color() };
        text(buf, inner.x, y, inner.width, &" ".repeat(inner.width as usize), th.base().bg(bg));
        let icon = match d.kind.as_str() {
            "smartphone" | "tablet" => "▯",
            "computer" => "▭",
            "tv" => "▢",
            _ => "◉",
        };
        text(buf, inner.x + 2, y, 2, icon, th.s(th.dim).bg(bg));
        let name = if d.is_engine { format!("{} (this one)", d.name) } else { d.name.clone() };
        text(buf, inner.x + 4, y, inner.width.saturating_sub(18), &name, st.bg(bg));
        let tag = if d.active { "♪ playing".to_string() } else { d.volume.map(|v| format!("vol {v}")).unwrap_or_default() };
        let tw = tag.chars().count() as u16;
        text(buf, inner.right().saturating_sub(tw + 2), y, tw, &tag, th.s(if d.active { th.accent } else { th.dim }).bg(bg));
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::DeviceRow(i));
    }
    text(buf, inner.x + 2, inner.bottom().saturating_sub(1), inner.width.saturating_sub(4), "Enter or click: play there · Esc: close", th.s(th.dim));
}

fn toast_rect(area: Rect, msg: &str) -> Rect {
    let w = (msg.chars().count() as u16 + 4).min(area.width);
    Rect::new(area.right().saturating_sub(w + 1), area.bottom().saturating_sub(5), w, 1)
}

const PALETTE_ROWS: usize = 16;

fn palette_rect(area: Rect, app: &App) -> Rect {
    // at most 16 rows of actions; the list scrolls with the selection
    centered(area, 60, (app.palette_items().len().min(PALETTE_ROWS) as u16 + 4).max(6))
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
        let title = match (slot.as_str(), &app.view, &app.artist) {
            ("home", _, _) if app.search.is_some() => "search".to_string(),
            ("home", _, Some(a)) => a.name.clone(),
            ("home", Some(v), _) => v.title.clone(),
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
        if slot == "lyrics" {
            crate::lyrics::draw(f.buffer_mut(), inner, app, th);
            continue;
        }
        if slot == "queue" {
            crate::queue::draw(f.buffer_mut(), inner, app, th);
            continue;
        }
        if slot == "stream" {
            crate::setup::draw_stream(f.buffer_mut(), inner, app, th);
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
        if slot == "home" && app.search.is_some() {
            crate::search::draw(f.buffer_mut(), inner, app, th);
            continue;
        }
        if slot == "home" && app.artist.is_some() {
            crate::artist::draw(f.buffer_mut(), inner, app, th);
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
    // which device plays: a click opens the picker
    let device = match app.remote_device() {
        Some(d) => format!(" ⧉ playing on {} ", d.name),
        None if app.devices.len() > 1 => " ⧉ devices ".to_string(),
        None => String::new(),
    };
    if !device.is_empty() {
        let dw = device.chars().count() as u16;
        let dx = r.right().saturating_sub(dw + 1);
        let st = if app.remote_device().is_some() { th.chip(th.accent2) } else { th.s(th.dim) };
        text(buf, dx, r.y, dw, &device, st);
        app.hit(Rect::new(dx, r.y, dw, 1), Hit::Action(Action::Devices));
    }
    let buf = f.buffer_mut();

    let y = r.y + 1;
    let (title, artists) = pb.item.as_ref().map(|i| (i.name.clone(), i.artists.join(", "))).unwrap_or_default();
    let tw = (r.width / 4).max(16);
    // ♥ after the title: liked or not, a click toggles it
    let heart = pb.item.as_ref().filter(|i| crate::saved::savable(&i.uri)).map(|i| (i.uri.clone(), app.is_saved(&i.uri) == Some(true)));
    let title_w = (title.chars().count() as u16).min(tw.saturating_sub(if heart.is_some() { 3 } else { 0 }));
    text(buf, r.x + 1, y, title_w, &title, th.s(th.fg).add_modifier(Modifier::BOLD));
    if let Some((uri, liked)) = heart {
        let hx = r.x + 1 + title_w + 1;
        text(buf, hx, y, 1, if liked { "♥" } else { "♡" }, th.s(if liked { th.accent } else { th.dim }));
        app.hit(Rect::new(hx, y, 1, 1), Hit::Save(singularity_proto::Link { name: title.clone(), uri }));
    }
    let buf = f.buffer_mut();
    text(buf, r.x + 1, y + 1, tw, &artists, th.s(th.dim));
    // the title opens its album, each artist their page
    if let Some(i) = &pb.item {
        if let Some(au) = &i.album_uri {
            let w = (i.name.chars().count() as u16).min(tw);
            app.hit(Rect::new(r.x + 1, y, w, 1), Hit::Link(singularity_proto::Link { name: i.album.clone().unwrap_or_default(), uri: au.clone() }));
        }
        let mut ax = r.x + 1;
        for (k, (name, uri)) in i.artists.iter().zip(&i.artist_uris).enumerate() {
            if k > 0 {
                ax += 2;
            }
            let w = (name.chars().count() as u16).min((r.x + 1 + tw).saturating_sub(ax));
            if w == 0 {
                break;
            }
            app.hit(Rect::new(ax, y + 1, w, 1), Hit::Link(singularity_proto::Link { name: name.clone(), uri: uri.clone() }));
            ax += w;
        }
    }
    let buf = f.buffer_mut();

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


/// Shows as many hints as fit; when space runs out, the least important go first
/// (play/pause, palette, help and quit always stay).
fn hint_bar(buf: &mut Buffer, r: Rect, th: &Theme, m: &crate::keymap::Keymap) {
    use Action as A;
    const DROP_ORDER: [&str; 3] = ["next/prev", "focus", "seek"];
    let space = |k: String| if k == "Space" { "␣".to_string() } else { k };
    let all = [
        (space(m.label(A::TogglePlay)), "play/pause"),
        (m.pair(A::SeekBack, A::SeekForward), "seek"),
        (m.pair(A::Next, A::Previous), "next/prev"),
        (m.label(A::FocusNext), "focus"),
        (m.label(A::Palette), "palette"),
        (m.label(A::Help), "help"),
        (m.label(A::Quit), "quit"),
    ];
    // an action without a key has no hint
    let mut hints: Vec<(String, &str)> = all.into_iter().filter(|(k, _)| !k.is_empty()).collect();
    let width = |hs: &[(String, &str)]| hs.iter().map(|(k, d)| k.chars().count() as u16 + d.chars().count() as u16 + 5).sum::<u16>() + 1;
    for drop in DROP_ORDER {
        if width(&hints) <= r.width {
            break;
        }
        hints.retain(|(_, d)| *d != drop);
    }
    let mut x = r.x + 1;
    for (k, d) in hints {
        let kw = k.chars().count() as u16 + 2;
        let dw = d.chars().count() as u16 + 3;
        if x + kw + dw > r.right() {
            break;
        }
        text(buf, x, r.y, kw, &format!(" {k} "), th.chip(th.dim));
        let d: &str = d;
        text(buf, x + kw, r.y, dw, &format!(" {d}"), th.s(th.dim));
        x += kw + dw;
    }
}

pub fn centered(r: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(r.width), h.min(r.height));
    Rect::new(r.x + (r.width - w) / 2, r.y + (r.height - h) / 3, w, h)
}

/// Draw a popup frame over `r` (cleared first); returns the inside.
pub fn popup(buf: &mut Buffer, r: Rect, th: &Theme, title: &str) -> Rect {
    use ratatui::widgets::Widget;
    Clear.render(r, buf);
    let block = overlay_block(th, title);
    let inner = block.inner(r);
    block.render(r, buf);
    inner
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
    let rows = (inner.height.saturating_sub(1) as usize).min(PALETTE_ROWS).max(1);
    let first = selected.saturating_sub(rows - 1);
    for (i, a) in items.iter().enumerate().skip(first).take(rows) {
        let y = inner.y + 1 + (i - first) as u16;
        if y >= inner.bottom() {
            break;
        }
        let row = Rect::new(inner.x, y, inner.width, 1);
        let st = if i == selected { th.s(th.accent).bg(th.selection.color()) } else { th.s(th.fg) };
        let buf = f.buffer_mut();
        text(buf, inner.x, y, inner.width, &" ".repeat(inner.width as usize), st);
        text(buf, inner.x + 2, y, inner.width.saturating_sub(12), a.label(), st);
        let k = app.keymap.label(*a);
        text(buf, inner.right().saturating_sub(k.chars().count() as u16 + 2), y, 10, &k, th.s(th.dim));
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

/// The help screen, from the keymap in use.
fn help_rows(m: &crate::keymap::Keymap) -> Vec<(String, String)> {
    use Action as A;
    vec![
        ("Mouse".into(), "click anything · drag pane borders · drag seek/volume · scroll".into()),
        (m.label(A::Palette), "command palette: find any action by typing".into()),
        (m.label(A::TogglePlay), "play / pause".into()),
        (m.pair(A::SeekBack, A::SeekForward), format!("seek 5 s    {}  seek 1 s    {}  next / previous", m.pair(A::SeekBackFine, A::SeekForwardFine), m.pair(A::Next, A::Previous))),
        (m.pair(A::VolumeUp, A::VolumeDown), format!("volume           {}   shuffle / repeat", m.pair(A::Shuffle, A::Repeat))),
        (m.label(A::Like), "like / unlike what's playing (or click its ♡)".into()),
        (m.nav_label().into(), "move through Home, play · right-click or Menu key: more".into()),
        (m.pair(A::Back, A::Forward), "back / forward (Esc and Backspace go back too)".into()),
        (m.label(A::FocusNext), "focus the next pane".into()),
        (m.label(A::Quit), "quit the Client (music keeps playing)".into()),
        ("keys.toml".into(), format!("{} keys · change them in ~/.config/singularity/keys.toml", m.preset)),
    ]
}

fn help_rect(area: Rect, rows: usize) -> Rect {
    centered(area, 80, rows as u16 + 4)
}

fn help(f: &mut Frame, area: Rect, th: &Theme, m: &crate::keymap::Keymap) {
    let rows = help_rows(m);
    let r = help_rect(area, rows.len());
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
        for c in "quit".chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
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

    /// What the App sent, without the background `CheckSaved` questions (`sent_all` keeps them).
    fn sent(rx: &mut tokio::sync::mpsc::Receiver<singularity_proto::Command>) -> Vec<singularity_proto::Command> {
        sent_all(rx).into_iter().filter(|c| !matches!(c, singularity_proto::Command::CheckSaved { .. })).collect()
    }

    fn sent_all(rx: &mut tokio::sync::mpsc::Receiver<singularity_proto::Command>) -> Vec<singularity_proto::Command> {
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
                ..Default::default()
            })
            .collect();
        singularity_proto::TrackPage { uri: "spotify:collection".into(), offset, total, tracks, header: None }
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
        let moods = side_rect(&app, 3);
        let line = s.lines().nth(moods.y as usize + 1).unwrap();
        assert!(line.find('≡').unwrap() > s.lines().nth(moods.y as usize).unwrap().find('▸').unwrap(), "the playlist sits inside the folder");
    }

    #[test]
    fn liked_songs_load_progressively_and_play_from_the_clicked_track() {
        use singularity_proto::Command;
        let (mut app, mut rx) = library_app();
        render(&mut app, 160, 48);
        let r = side_rect(&app, 2);
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
        let r = side_rect(&app, 2);
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

    // ── Setup, health, stream (#5) ──

    #[test]
    fn first_run_wizard_takes_the_key_without_showing_it() {
        use singularity_proto::{Command, EngineStatus};
        let (mut app, mut rx) = playing_app(1);
        app.engine = Some(EngineStatus::NotConfigured { reason: "no Soloist API key yet".into() });
        let s = render(&mut app, 120, 40);
        assert!(s.contains("Set up Singularity") && s.contains("Spotify Premium") && s.contains(crate::setup::KEY_URL), "{s}");
        app.paste("spak_secret123");
        let s = render(&mut app, 120, 40);
        assert!(!s.contains("secret123") && s.contains("••••"), "the key never shows");
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::SetSoloistKey { key: "spak_secret123".into() }]);
        // next steps follow what the Daemon reports
        app.engine = Some(EngineStatus::NeedsPairing { device_name: "Singularity".into() });
        assert!(render(&mut app, 120, 40).contains("device picker"));
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!render(&mut app, 120, 40).contains("Set up Singularity"), "Esc: later");
    }

    #[test]
    fn stream_pane_and_health_show_quality_and_the_fix() {
        use singularity_proto::{Command, StreamInfo};
        let (mut app, mut rx) = playing_app(1);
        app.stream = Some(StreamInfo { codec: "vorbis".into(), bitrate: 160_000, average_bitrate: None, sample_rate: 44_100, sample_format: "32-bit-float-pcm".into() });
        let s = render(&mut app, 160, 48);
        assert!(s.contains("NOT LOSSLESS") && s.contains("VORBIS"), "{s}");
        app.stream = Some(StreamInfo { codec: "flac".into(), bitrate: 1_400_000, average_bitrate: Some(1_689_310), sample_rate: 44_100, sample_format: "32-bit-float-pcm".into() });
        let s = render(&mut app, 160, 48);
        assert!(s.contains("LOSSLESS") && !s.contains("NOT LOSSLESS") && s.contains("1.69 Mbit/s"));
        app.run(Action::Health);
        app.tick();
        assert_eq!(sent(&mut rx), vec![Command::GetHealth]);
        app.health = Some(singularity_proto::Health { soloist_key: "keyring".into(), spotify_login: "keyring".into(), audio_output: Some("DAC Pro".into()), engine_expires_in_days: Some(80), engine_build: Some("2026-10-16".into()) });
        let s = render(&mut app, 160, 48);
        assert!(s.contains("Health check") && s.contains("DAC Pro") && s.contains("FLAC · 1.40 Mbit/s"), "{s}");
    }

    // ── Queue (#12) ──

    #[test]
    fn queue_widget_marks_autoplay_and_edits_through_menus() {
        use singularity_proto::{Command, Item, QueueEntry};
        let (mut app, mut rx) = playing_app(200_000);
        app.playback.context_name = Some("Night Drive".into());
        let e = |n: &str, src: &str| QueueEntry { item: Item { uri: format!("spotify:track:{n}"), name: n.into(), artists: vec!["Nova Drift".into()], duration_ms: 200_000, ..Default::default() }, source: src.into(), uid: format!("uid-{n}") };
        app.playback.queue = vec![e("Mine", "queue"), e("Ctx", "context"), e("Auto", "autoplay")];
        let s = render(&mut app, 160, 48);
        for part in ["Next in queue", "Next from Night Drive", "Autoplay", "autoplay  3:20"] {
            assert!(s.contains(part), "missing {part:?}\n{s}");
        }
        // right-click the queued one → Remove
        let row = app.hits.iter().find_map(|(r, h)| (*h == Hit::QueueRow(0)).then_some(*r)).unwrap();
        app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: row.x + 2, row: row.y, modifiers: KeyModifiers::NONE });
        let s = render(&mut app, 160, 48);
        assert!(s.contains("Remove from queue") && s.contains("Clear queue"));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::RemoveFromQueue { uri: "spotify:track:Mine".into(), uid: Some("uid-Mine".into()) }]);
        // a context track can be played next, not removed
        app.focus = "queue".into();
        app.queue_view.sel = 1;
        app.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert!(sent(&mut rx).is_empty());
        app.run(Action::ClearQueue);
        assert_eq!(sent(&mut rx), vec![Command::ClearQueue]);
    }

    #[test]
    fn track_rows_can_play_next_or_be_queued() {
        use singularity_proto::Command;
        let (mut app, mut rx) = library_app();
        render(&mut app, 160, 48);
        let r = side_rect(&app, 2);
        click(&mut app, r.x + 3, r.y);
        sent(&mut rx);
        app.track_page(page(0, 10, 10));
        render(&mut app, 160, 48);
        let row = app.hits.iter().find_map(|(r, h)| (*h == Hit::Track(3)).then_some(*r)).unwrap();
        app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: row.x + 5, row: row.y, modifiers: KeyModifiers::NONE });
        let s = render(&mut app, 160, 48);
        assert!(s.contains("Play from here") && s.contains("Play next") && s.contains("Add to queue"));
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::PlayNext { uri: "spotify:track:t3".into() }]);
    }

    // ── Lyrics (#14) ──

    #[test]
    fn lyrics_follow_the_song_and_a_click_seeks() {
        use singularity_proto::{Command, LyricLine, Lyrics};
        let (mut app, mut rx) = playing_app(200_000);
        app.playback.item.as_mut().unwrap().uri = "spotify:track:s".into();
        app.tick();
        assert_eq!(sent(&mut rx), vec![Command::GetLyrics { uri: "spotify:track:s".into() }], "asked once for the new track");
        app.tick();
        assert!(sent(&mut rx).is_empty());
        let lines = (0..20).map(|i| LyricLine { start_ms: i * 5_000, text: format!("line {i}") }).collect();
        app.lyrics = Some(Lyrics { uri: "spotify:track:s".into(), synced: true, lines, source: "Musixmatch".into() });
        app.playback.position_ms = 31_000; // line 6 plays
        app.playback.timestamp_ms = singularity_proto::now_ms();
        let s = render(&mut app, 160, 48);
        assert!(s.contains("[00:30] line 6") && s.contains("lyrics: Musixmatch"), "{s}");
        let l9 = app.hits.iter().find_map(|(r, h)| (*h == Hit::Lyric(45_000)).then_some(*r)).unwrap();
        click(&mut app, l9.x + 10, l9.y);
        assert_eq!(sent(&mut rx), vec![Command::Seek { position_ms: 45_000 }]);
        // none: a calm empty state
        app.lyrics = Some(Lyrics { uri: "spotify:track:s".into(), ..Default::default() });
        assert!(render(&mut app, 160, 48).contains("no lyrics for this one"));
    }

    // ── Album & artist pages, history (#11) ──

    fn fake_artist() -> singularity_proto::ArtistPage {
        use singularity_proto::{Card, Shelf, TrackRow};
        let top = (0..10).map(|i| TrackRow { uri: format!("spotify:track:hit{i}"), name: format!("Hit {i}"), artists: vec!["Nova Drift".into()], album: "Orbits".into(), album_uri: Some("spotify:album:orbits".into()), duration_ms: 200_000, playable: true, ..Default::default() }).collect();
        let card = |n: &str| Card { uri: format!("spotify:album:{n}"), name: n.into(), subtitle: "2024 · Album".into(), image_url: None, kind: "album".into() };
        singularity_proto::ArtistPage { uri: "spotify:artist:nova".into(), name: "Nova Drift".into(), monthly_listeners: Some(1_234_567), image_url: None, top_tracks: top, shelves: vec![Shelf { title: "Albums".into(), kind: "shelf".into(), cards: vec![card("orbits"), card("drift")] }] }
    }

    #[test]
    fn names_are_links_and_history_walks_back_and_forth() {
        use singularity_proto::Command;
        let (mut app, mut rx) = library_app();
        render(&mut app, 160, 48);
        let r = side_rect(&app, 2);
        click(&mut app, r.x + 3, r.y);
        sent(&mut rx);
        let mut p = page(0, 10, 10);
        for (i, t) in p.tracks.iter_mut().enumerate() {
            t.artist_uris = vec![format!("spotify:artist:a{i}")];
            t.album_uri = Some("spotify:album:orbits".into());
        }
        p.tracks[0].artist_uris = vec!["spotify:artist:nova".into()];
        app.track_page(p);
        render(&mut app, 200, 48);
        // sort the list, so we can see it's remembered later
        app.run(Action::SortTitle);
        let s = render(&mut app, 200, 48);
        assert!(s.contains("Title ↑"));
        // click an artist's name in a row: their page opens
        let link = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Link(l) if l.uri == "spotify:artist:nova").then_some(*r)).unwrap();
        click(&mut app, link.x, link.y);
        assert_eq!(sent(&mut rx), vec![Command::GetArtist { uri: "spotify:artist:nova".into() }]);
        app.artist_page(fake_artist());
        let s = render(&mut app, 200, 48);
        assert!(s.contains("Nova Drift") && s.contains("1 234 567 monthly listeners") && s.contains("Popular") && s.contains("Hit 0") && s.contains("Albums"), "{s}");
        // a release opens its track list
        let card = card_rect_any(&app);
        click(&mut app, card.x + 2, card.y + 1);
        assert_eq!(sent(&mut rx), vec![Command::GetTracks { uri: "spotify:album:orbits".into() }]);
        // back: the artist (from cache), back again: Liked Songs, still sorted
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert!(app.artist.is_some() && sent(&mut rx).is_empty(), "no refetch");
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.view.as_ref().map(|v| (v.uri.as_str(), v.sort.is_some())), Some(("spotify:collection", true)));
        // forward again
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
        assert!(app.artist.is_some());
        // double-click a popular track: the artist's context from there
        render(&mut app, 200, 48);
        let row = app.hits.iter().find_map(|(r, h)| (*h == Hit::TopTrack(2)).then_some(*r)).unwrap();
        click(&mut app, row.x + 6, row.y);
        click(&mut app, row.x + 6, row.y);
        assert_eq!(sent(&mut rx), vec![Command::PlayFrom { context_uri: "spotify:artist:nova".into(), index: 2, track_uri: Some("spotify:track:hit2".into()) }]);
    }

    fn card_rect_any(app: &App) -> Rect {
        app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Card(_)).then_some(*r)).unwrap()
    }

    // ── Search (#10) ──

    #[test]
    fn search_while_typing_with_chips_and_recents() {
        use singularity_proto::{Card, Command, SearchResults, TrackRow};
        let (mut app, mut rx) = playing_app(1);
        app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert!(app.search.is_some());
        for c in "nova".chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
            app.tick();
        }
        assert!(sent(&mut rx).is_empty(), "nothing while typing");
        std::thread::sleep(crate::search::DEBOUNCE + std::time::Duration::from_millis(20));
        app.tick();
        assert_eq!(sent(&mut rx), vec![Command::Search { query: "nova".into() }]);
        let card = |n: &str, k: &str| Card { uri: format!("spotify:{k}:{n}"), name: n.into(), subtitle: k.into(), image_url: None, kind: k.into() };
        let tracks = (0..8).map(|i| TrackRow { uri: format!("spotify:track:n{i}"), name: format!("Nova song {i}"), artists: vec!["Nova Drift".into()], artist_uris: vec!["spotify:artist:nova".into()], duration_ms: 200_000, playable: true, ..Default::default() }).collect();
        app.search.as_mut().unwrap().set_results(SearchResults { query: "nova".into(), top: vec![card("Nova Drift", "artist")], tracks, artists: vec![card("Nova Drift", "artist")], albums: vec![card("Event Horizon", "album")], playlists: vec![], podcasts: vec![] });
        let s = render(&mut app, 200, 64);
        assert!(s.contains("Songs") && s.contains("Nova song 4") && !s.contains("Nova song 5"), "All shows five songs:\n{s}");
        assert!(s.contains("Top results") && s.contains("Albums"));
        // the Songs chip by keyboard: all eight
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(render(&mut app, 200, 64).contains("Nova song 7"));
        // Albums chip by mouse; opening one remembers the search
        let chip = app.hits.iter().find_map(|(r, h)| (*h == Hit::Chip(3)).then_some(*r)).unwrap();
        click(&mut app, chip.x + 1, chip.y);
        render(&mut app, 200, 64);
        let c = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Card(_)).then_some(*r)).unwrap();
        click(&mut app, c.x + 2, c.y + 1);
        assert_eq!(sent(&mut rx), vec![Command::GetTracks { uri: "spotify:album:Event Horizon".into() }]);
        assert_eq!(app.recent, ["nova"]);
        // back to the search, results and chip intact
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.search.as_ref().map(|s| (s.query.as_str(), s.chip)), Some(("nova", 3)));
        // an empty query lists recent searches
        for _ in 0..4 {
            app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
        let s = render(&mut app, 200, 64);
        assert!(s.contains("Recent searches") && s.contains("  nova"), "{s}");
    }

    // ── Devices (#13) ──

    #[test]
    fn device_picker_moves_playback_and_the_bar_says_where() {
        use singularity_proto::{Command, Device};
        let (mut app, mut rx) = playing_app(1);
        let dev = |id: &str, name: &str, active: bool, engine: bool| Device { id: id.into(), name: name.into(), kind: "speaker".into(), active, volume: Some(50), is_engine: engine };
        app.devices = vec![dev("solo", "Singularity", true, true), dev("phone", "Pixel", false, false)];
        let s = render(&mut app, 160, 48);
        assert!(s.contains("⧉ devices"));
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::GetDevices]);
        let s = render(&mut app, 160, 48);
        assert!(s.contains("Play on") && s.contains("Singularity (this one)") && s.contains("♪ playing"), "{s}");
        let row = app.hits.iter().find_map(|(r, h)| (*h == Hit::DeviceRow(1)).then_some(*r)).unwrap();
        click(&mut app, row.x + 3, row.y);
        assert_eq!(sent(&mut rx), vec![Command::Transfer { device_id: "phone".into() }]);
        assert!(!app.devices_open);
        // now the phone plays: the bar says so
        app.devices = vec![dev("solo", "Singularity", false, true), dev("phone", "Pixel", true, false)];
        assert!(render(&mut app, 160, 48).contains("⧉ playing on Pixel"));
    }

    // ── Like/follow (#22) ──

    fn flag(uri: &str, saved: bool) -> singularity_proto::SavedFlag {
        singularity_proto::SavedFlag { uri: uri.into(), saved }
    }

    fn liked_app() -> (App, tokio::sync::mpsc::Receiver<singularity_proto::Command>) {
        let (mut app, mut rx) = library_app();
        app.playback.item = Some(singularity_proto::Item { uri: "spotify:track:new".into(), name: "Fresh".into(), artists: vec!["Nova Drift".into()], duration_ms: 200_000, ..Default::default() });
        render(&mut app, 160, 48);
        let r = side_rect(&app, 2);
        click(&mut app, r.x + 3, r.y);
        app.track_page(page(0, 5, 5));
        app.tick();
        sent(&mut rx);
        (app, rx)
    }

    fn liked_uris(app: &App) -> Vec<String> {
        app.lists["spotify:collection"].rows.iter().map(|r| r.uri.clone()).collect()
    }

    #[test]
    fn the_heart_likes_at_once_and_rolls_back_when_spotify_refuses() {
        use singularity_proto::Command;
        let (mut app, mut rx) = playing_app(200_000);
        app.playback.item.as_mut().unwrap().uri = "spotify:track:fail".into();
        app.tick();
        assert!(sent_all(&mut rx).contains(&Command::CheckSaved { uris: vec!["spotify:track:fail".into()] }), "the ♥ asks about the new track");
        app.tick();
        assert!(sent_all(&mut rx).is_empty(), "once");
        app.saved_event(vec![flag("spotify:track:fail", false)]);
        let s = render(&mut app, 160, 48);
        assert!(s.contains("Song ♡"), "{s}");
        // click the heart
        let h = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Save(_)).then_some(*r)).unwrap();
        click(&mut app, h.x, h.y);
        assert_eq!(sent(&mut rx), vec![Command::SetSaved { uri: "spotify:track:fail".into(), saved: true }]);
        assert!(render(&mut app, 160, 48).contains("Song ♥"), "shown before Spotify answers");
        // Spotify refuses: the old value comes back
        app.saved_event(vec![flag("spotify:track:fail", false)]);
        assert!(render(&mut app, 160, 48).contains("Song ♡"));
        // the key does the same
        app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::SetSaved { uri: "spotify:track:fail".into(), saved: true }]);
        assert_eq!(app.is_saved("spotify:track:fail"), Some(true));
    }

    #[test]
    fn quick_double_toggles_settle_on_the_last_answer() {
        let (mut app, mut rx) = playing_app(200_000);
        app.playback.item.as_mut().unwrap().uri = "spotify:track:x".into();
        app.run(Action::Like);
        app.run(Action::Like);
        assert_eq!(sent(&mut rx).len(), 2);
        assert_eq!(app.is_saved("spotify:track:x"), Some(false));
        // the first answer is stale: the second toggle is still out
        app.saved_event(vec![flag("spotify:track:x", true)]);
        assert_eq!(app.is_saved("spotify:track:x"), Some(false));
        app.saved_event(vec![flag("spotify:track:x", false)]);
        assert_eq!(app.is_saved("spotify:track:x"), Some(false));
    }

    #[test]
    fn liked_songs_follow_likes_without_a_reload() {
        use singularity_proto::Command;
        let (mut app, mut rx) = liked_app();
        assert_eq!(app.is_saved("spotify:track:t3"), Some(true), "everything in Liked Songs is liked");
        // like what's playing: it lands on top
        app.run(Action::Like);
        assert_eq!(sent(&mut rx), vec![Command::SetSaved { uri: "spotify:track:new".into(), saved: true }]);
        assert_eq!(liked_uris(&app)[0], "spotify:track:new");
        let s = render(&mut app, 160, 48);
        assert!(s.contains("6 songs") && s.contains("Fresh"), "{s}");
        assert_eq!(app.library.as_ref().unwrap().items[0].subtitle, "6 songs", "the sidebar counts along");
        // unlike row 3 (t2) from its menu
        let row = app.hits.iter().find_map(|(r, h)| (*h == Hit::Track(3)).then_some(*r)).unwrap();
        app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: row.x + 5, row: row.y, modifiers: KeyModifiers::NONE });
        let m = app.menu.as_ref().unwrap();
        let i = m.items.iter().position(|(l, _)| l == "Remove from Liked Songs").expect("menu offers removal");
        app.menu.as_mut().unwrap().selected = i;
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::SetSaved { uri: "spotify:track:t2".into(), saved: false }]);
        assert!(!liked_uris(&app).contains(&"spotify:track:t2".to_string()));
        assert!(!render(&mut app, 160, 48).contains("Song 0003"));
        // Spotify refuses: the row comes back where it was
        app.saved_event(vec![flag("spotify:track:t2", true)]);
        assert_eq!(liked_uris(&app)[3], "spotify:track:t2");
        assert_eq!(app.lists["spotify:collection"].total, 6);
        assert!(sent(&mut rx).is_empty(), "no reload");
    }

    #[test]
    fn menus_ask_and_reword_save_when_the_answer_arrives() {
        use singularity_proto::Command;
        let (mut app, mut rx) = playing_app(200_000);
        let item = singularity_proto::Item { uri: "spotify:track:q".into(), name: "Q".into(), ..Default::default() };
        app.playback.queue = vec![singularity_proto::QueueEntry { item, source: "queue".into(), uid: "u".into() }];
        app.open_queue_menu(0, Some((10, 10)));
        assert_eq!(sent_all(&mut rx), vec![Command::CheckSaved { uris: vec!["spotify:track:q".into()] }]);
        assert!(app.menu.as_ref().unwrap().items.iter().any(|(l, _)| l == "Save to Liked Songs"));
        app.saved_event(vec![flag("spotify:track:q", true)]);
        assert!(app.menu.as_ref().unwrap().items.iter().any(|(l, _)| l == "Remove from Liked Songs"));
    }

    #[test]
    fn artists_are_followed_and_playlists_need_a_second_click_to_go() {
        use singularity_proto::Command;
        let (mut app, mut rx) = library_app();
        app.open("spotify:artist:nova", "Nova Drift", "", "artist");
        assert!(sent_all(&mut rx).contains(&Command::CheckSaved { uris: vec!["spotify:artist:nova".into()] }));
        app.artist_page(fake_artist());
        let s = render(&mut app, 160, 48);
        assert!(s.contains("+ Follow"), "{s}");
        let b = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Save(l) if l.uri == "spotify:artist:nova").then_some(*r)).unwrap();
        click(&mut app, b.x + 1, b.y);
        assert_eq!(sent(&mut rx), vec![Command::SetSaved { uri: "spotify:artist:nova".into(), saved: true }]);
        assert!(render(&mut app, 160, 48).contains("✓ Following"));
        // a saved playlist: the first click only asks
        app.open("spotify:playlist:p", "Night Drive", "", "playlist");
        sent(&mut rx);
        app.saved_event(vec![flag("spotify:playlist:p", true)]);
        let s = render(&mut app, 160, 48);
        assert!(s.contains("♥ Saved"), "{s}");
        let b = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Save(l) if l.uri == "spotify:playlist:p").then_some(*r)).unwrap();
        click(&mut app, b.x + 1, b.y);
        assert!(sent(&mut rx).is_empty());
        click(&mut app, b.x + 1, b.y);
        assert_eq!(sent(&mut rx), vec![Command::SetSaved { uri: "spotify:playlist:p".into(), saved: false }]);
        assert!(render(&mut app, 160, 48).contains("♡ Save"));
    }
}
