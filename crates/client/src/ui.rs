//! Rendering. Pure function of `App` state, plus hit-box registration for the mouse.

use crate::app::{Action, App, Hit, Link, Menu};
use crate::scrub::{self, Bar};
use crate::theme::{Rgb, Theme};
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
        "hole" => ("black hole", ""),
        "nowplaying" => ("Now playing", ""),
        "levels" => ("levels", ""),
        "listening" => ("listening · 26 weeks", ""),
        "rain" => ("rain", ""),
        "scope" => ("osc", ""),
        "top" => ("top · system", ""),
        "today" => ("today", ""),
        "topweek" => ("top this week", ""),
        "starfield" => ("starfield", ""),
        "marquee" => ("marquee", ""),
        "glow" => ("glow", ""),
        _ => ("?", ""),
    }
}

pub fn text(buf: &mut Buffer, x: u16, y: u16, max: u16, s: &str, st: Style) {
    if max > 0 {
        buf.set_stringn(x, y, s, max as usize, st);
    }
}

/// `w` blank cells in style `st` from (`x`, `y`): a row's background, without a string of spaces each frame (Q1).
pub fn blank(buf: &mut Buffer, x: u16, y: u16, w: u16, st: Style) {
    for x in x..x.saturating_add(w) {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.set_char(' ').set_style(st);
        }
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
    app.meters.expire();
    app.begin_frame();
    let area = f.area();
    let th = app.theme.clone();
    // covers and the hole follow the Theme's tint (a Theme switch, a theme file saved, `ui.tint_art` edited)
    let tint = th.art_tint(app.config.ui.tint_art);
    app.hole.set_tint(tint.as_ref().map(|t| t.stops4()));
    app.art_tint = tint;
    fill(f.buffer_mut(), area, &th);
    // too small for any layout: just a player (#47)
    if crate::mini::wanted(area) {
        app.overlays.clear();
        crate::mini::draw(f.buffer_mut(), area, app, &th);
        if let Some((msg, _)) = app.toast.clone()
            && area.height > 2
        {
            text(f.buffer_mut(), area.x, area.bottom() - 1, area.width, &format!(" {msg}"), th.s(th.accent));
        }
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
        app.picker.as_ref().map(|p| crate::looks::picker_rect(area, p)),
        app.naming.is_some().then(|| naming_rect(area)),
        app.playlists.details.is_some().then(|| crate::playlists::details_rect(area)),
        app.playlists.adding.is_some().then(|| crate::playlists::adding_rect(area, app.adding_rows().len())),
        app.cover_form.is_some().then(|| crate::cover::rect(area)),
        app.menu.as_ref().map(|m| menu_rect(area, m)),
        app.toast.as_ref().map(|(msg, _)| toast_rect(area, msg)),
    ];
    app.overlays = overlays.into_iter().flatten().collect();
    top_bar(f, top, app, &th);
    panes(f, body, app, &th);
    if th.scanlines {
        scanlines(f.buffer_mut(), body);
    }
    crate::visual::idle::draw(f.buffer_mut(), body, app, &th);
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
    if app.picker.is_some() {
        crate::looks::draw_picker(f.buffer_mut(), area, app, &th);
    }
    if let Some(n) = app.naming.clone() {
        let (title, hint) = match n.purpose {
            crate::app::NameFor::Layout => ("Save design as", "Enter save · Esc cancel · find it in Ctrl+K → Pick a design"),
            crate::app::NameFor::NewPlaylist => ("New playlist", "Enter create · Esc cancel"),
        };
        let inner = popup(f.buffer_mut(), naming_rect(area), &th, title);
        let buf = f.buffer_mut();
        // the end of a long name stays in view
        let w = inner.width.saturating_sub(5) as usize;
        let shown: String = n.text.chars().skip(n.text.chars().count().saturating_sub(w)).collect();
        text(buf, inner.x + 1, inner.y, inner.width.saturating_sub(2), &format!("› {shown}▏"), th.s(th.fg).add_modifier(Modifier::BOLD));
        text(buf, inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), hint, th.s(th.dim));
    }
    if app.playlists.details.is_some() {
        crate::playlists::draw_details(f.buffer_mut(), area, app, &th);
    }
    if app.playlists.adding.is_some() {
        crate::playlists::draw_adding(f.buffer_mut(), area, app, &th);
    }
    if app.cover_form.is_some() {
        crate::cover::draw(f.buffer_mut(), area, app, &th);
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
    if app.config.ui.transparent {
        transparent(f.buffer_mut(), area, &th);
    }
    if app.kitty.as_ref().is_some_and(|k| k.below_backgrounds) {
        opaque_overlays(f.buffer_mut(), &app.overlays.clone(), &th);
    }
}

/// kitty treats a cell whose background *equals* its own default (pure black, usually) as default, and lets an image
/// under backgrounds show through it. Popups must hide what's under them, so their black becomes a black kitty can't
/// mistake for its own (#41 follow-up: the owner saw covers through Ctrl+K).
fn opaque_overlays(buf: &mut Buffer, overlays: &[Rect], th: &Theme) {
    let black = th.bg.color();
    let nudged = Rgb(th.bg.0.saturating_add(1), th.bg.1.saturating_add(1), th.bg.2.saturating_add(1)).color();
    for r in overlays {
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                if let Some(c) = buf.cell_mut((x, y))
                    && (c.bg == black || c.bg == ratatui::style::Color::Reset)
                {
                    c.set_bg(nudged);
                }
            }
        }
    }
}

/// Every other row a little dimmer, like a CRT (Terminal's themes).
fn scanlines(buf: &mut Buffer, r: Rect) {
    for y in (r.y..r.bottom()).filter(|y| y % 2 == 1) {
        for x in r.x..r.right() {
            if let Some(c) = buf.cell_mut((x, y))
                && let ratatui::style::Color::Rgb(r, g, b) = c.fg
            {
                c.set_fg(ratatui::style::Color::Rgb((r as f32 * 0.75) as u8, (g as f32 * 0.75) as u8, (b as f32 * 0.75) as u8));
            }
        }
    }
}

/// `ui.transparent`: the theme's black becomes the terminal's own background (text on chips keeps its colour).
fn transparent(buf: &mut Buffer, r: Rect, th: &Theme) {
    let bg = th.bg.color();
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            if let Some(c) = buf.cell_mut((x, y))
                && c.bg == bg
            {
                c.set_bg(ratatui::style::Color::Reset);
            }
        }
    }
}

fn naming_rect(area: Rect) -> Rect {
    centered(area, 64, 4)
}

fn devices_rect(area: Rect, app: &App) -> Rect {
    let list = app.devices.len().max(1) as u16 + 5;
    let jam = if app.devices.is_empty() { 0 } else { crate::jam::height(app, area.height.saturating_sub(list)) };
    centered(area, crate::jam::width(app).max(56).min(area.width), list + jam)
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
        blank(buf, inner.x, y, inner.width, th.base().bg(bg));
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
    crate::jam::draw(buf, inner, inner.y + 1 + list.len() as u16, list.len(), app, th);
    // the vim preset makes j a ↓: then the Jam row is reached like the others
    let j_is_jam = app.keymap.nav(ratatui::crossterm::event::KeyEvent::new(ratatui::crossterm::event::KeyCode::Char('j'), ratatui::crossterm::event::KeyModifiers::NONE)).is_none();
    let hint = if j_is_jam { "Enter or click: play there · j: Jam · Esc: close" } else { "Enter or click: play there · Esc: close" };
    text(buf, inner.x + 2, inner.bottom().saturating_sub(1), inner.width.saturating_sub(4), hint, th.s(th.dim));
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

/// Superscript digits for numbered Pane titles (Cockpit).
fn superscript(n: usize) -> String {
    n.to_string().chars().map(|c| ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'][c as usize - '0' as usize]).collect()
}

fn panes(f: &mut Frame, body: Rect, app: &mut App, th: &Theme) {
    let computed = app.layout.compute_shaped(body, app.cell_px.map(|(w, h)| h / w).unwrap_or(2.0));
    // the ◂ ▸ swaps sit on a border a divider's grip may cover: registered after the grips, so they win
    let mut swaps: Vec<(Rect, Hit)> = vec![];
    for (i, (slot, rect)) in computed.slots.iter().enumerate() {
        let (title, todo) = slot_info(slot);
        let title = crate::design::title(app.design, slot, title);
        // the main Pane is titled after what it shows
        let title = match (slot.as_str(), &app.view, &app.artist) {
            ("home", _, _) if app.search.is_some() => "search".to_string(),
            ("home", _, _) if app.stats_view.is_some() => "stats".to_string(),
            ("home", _, _) if app.settings_view.is_some() => "settings".to_string(),
            ("home", _, Some(a)) => a.name.clone(),
            ("home", Some(v), _) => v.title.clone(),
            // its title is the switch between artists and tracks
            ("topweek", _, _) if app.top_week_tracks => "top tracks · week ⇄".to_string(),
            ("topweek", _, _) => "top artists · week ⇄".to_string(),
            _ => title.to_string(),
        };
        let number = if th.numbered { superscript(i + 1) } else { String::new() };
        let focused = *slot == app.focus;
        let title_st = if focused { th.title() } else { th.s(th.accent) };
        let mut spans = vec![Span::styled(format!(" {number}"), title_st)];
        // the main Pane: ← → through the pages seen, while there are any, at the right end of its title like every
        // Pane's arrows (D44); dim when there's nowhere to go that way
        let history = slot == "home" && (!app.back.is_empty() || !app.forward.is_empty()) && rect.width > 20;
        // a Slot that can show other Widgets always keeps its ◂ ▸ (7 cells at the right): a long title gives way
        let can_swap = (!crate::layouts::options(slot).is_empty() && slot != "well") || history;
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let room = (rect.width as usize).saturating_sub(2 + used + if can_swap { 8 } else { 0 });
        let title = if title.chars().count() + 1 > room && room > 1 { title.chars().take(room - 2).collect::<String>() + "…" } else { title };
        spans.push(Span::styled(format!("{title} "), title_st));
        if slot == "topweek" {
            let tw = spans.iter().map(|s| s.content.chars().count()).sum::<usize>() as u16;
            swaps.push((Rect::new(rect.x + 1, rect.y, tw.min(rect.width.saturating_sub(2)), 1), Hit::TopWeekToggle));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(th.borders)
            .border_style(th.s(if focused { th.border_focus } else { th.pane_border(i) }))
            .title(ratatui::text::Line::from(spans))
            .style(th.base());
        let inner = block.inner(*rect);
        // the Pane's own hit first, so the things inside it win
        app.hit(*rect, Hit::Focus(slot.clone()));
        // a Slot that can show other Widgets says so: ◂ ▸ in its title swap them (so does v)
        let swappable = can_swap && rect.width >= 12;
        if slot == "well" {
            // the well draws its own (sagging) frame
            crate::visual::well::draw(f.buffer_mut(), *rect, app, th);
            continue;
        }
        f.render_widget(block, *rect);
        if history {
            let (x, y) = (rect.right().saturating_sub(7), rect.y);
            let buf = f.buffer_mut();
            let on = |b: bool| th.s(if b { th.fg } else { th.border });
            text(buf, x, y, 1, " ", title_st);
            text(buf, x + 1, y, 1, "←", on(!app.back.is_empty()));
            text(buf, x + 2, y, 1, " ", title_st);
            text(buf, x + 3, y, 1, "→", on(!app.forward.is_empty()));
            text(buf, x + 4, y, 1, " ", title_st);
            swaps.push((Rect::new(x, y, 2, 1), Hit::History(-1)));
            swaps.push((Rect::new(x + 2, y, 3, 1), Hit::History(1)));
        } else if swappable {
            let (x, y) = (rect.right().saturating_sub(7), rect.y);
            let buf = f.buffer_mut();
            text(buf, x, y, 5, " ◂ ▸ ", th.s(if focused { th.accent } else { th.dim }));
            swaps.push((Rect::new(x, y, 2, 1), Hit::Swap(slot.clone(), -1)));
            swaps.push((Rect::new(x + 2, y, 3, 1), Hit::Swap(slot.clone(), 1)));
        }
        match slot.as_str() {
            "hole" => crate::visual::well::draw_plain(f.buffer_mut(), *rect, inner, app, th),
            "nowplaying" => crate::widgets::nowplaying(f.buffer_mut(), inner, app, th),
            "levels" => crate::widgets::levels(f.buffer_mut(), inner, app, th),
            "listening" => {
                crate::widgets::listening(f.buffer_mut(), inner, app, th);
                // a click opens the whole story
                app.hit(inner, Hit::Action(Action::Stats));
            }
            "rain" => crate::widgets::rain(f.buffer_mut(), inner, app, th),
            "scope" => crate::widgets::scope(f.buffer_mut(), inner, app, th),
            "top" => crate::widgets::top(f.buffer_mut(), inner, app, th),
            "today" => {
                crate::stats::today(f.buffer_mut(), inner, app, th);
                app.hit(inner, Hit::Action(Action::Stats));
            }
            "topweek" => crate::stats::top_week(f.buffer_mut(), inner, app, th),
            "starfield" => crate::ambient::starfield(f.buffer_mut(), inner, app, th),
            "marquee" => crate::ambient::marquee(f.buffer_mut(), inner, app, th),
            "glow" => crate::ambient::glow(f.buffer_mut(), inner, app, th),
            _ => {}
        }
        if matches!(slot.as_str(), "hole" | "nowplaying" | "levels" | "listening" | "rain" | "scope" | "top" | "today" | "topweek" | "starfield" | "marquee" | "glow") {
            continue;
        }
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
        if slot == "home" && app.stats_view.is_some() {
            crate::stats::draw(f.buffer_mut(), inner, app, th);
            continue;
        }
        if slot == "home" && app.settings_view.is_some() {
            crate::settings::draw(f.buffer_mut(), inner, app, th);
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
    for (r, h) in swaps {
        app.hit(r, h);
    }
    app.dividers = computed.dividers;
}

fn player_bar(f: &mut Frame, r: Rect, app: &mut App, th: &Theme) {
    // what the bar shows, without the queue it doesn't (the whole state, queue and all, was cloned every frame: Q1)
    let p = &app.playback;
    let pb = singularity_proto::PlaybackState { status: p.status, item: p.item.clone(), context_name: p.context_name.clone(), shuffle: p.shuffle, repeat: p.repeat, ..Default::default() };
    let buf = f.buffer_mut();
    let now = pb.item.as_ref().map(|i| format!("{} — {}", i.name, i.artists.join(", "))).unwrap_or_else(|| "nothing playing".into());
    match crate::design::DESIGNS[app.design].prompt {
        Some(host) => {
            let p = format!("{host}:~$ ");
            let pw = p.chars().count() as u16;
            text(buf, r.x + 1, r.y, r.width, &p, th.title());
            text(buf, r.x + 1 + pw, r.y, r.width.saturating_sub(pw + 1), &format!("now-playing --follow  # {now}"), th.s(th.fg));
        }
        // the designs without a shell say it plainly
        None => {
            let from = pb.context_name.as_deref().map(|c| format!("Playing from {c}")).unwrap_or_default();
            text(buf, r.x + 1, r.y, r.width.saturating_sub(2), &from, th.s(th.dim));
        }
    }
    // which device plays: a click opens the picker
    let device = match app.remote_device() {
        Some(d) => format!(" ⧉ playing on {} ", d.name),
        None if app.jam.is_some() => format!(" ◎ Jam · {} ", app.jam.as_ref().map(crate::jam::guests).unwrap_or(0)),
        None if app.devices.len() > 1 => " ⧉ devices ".to_string(),
        None => String::new(),
    };
    if !device.is_empty() {
        let dw = device.chars().count() as u16;
        let dx = r.right().saturating_sub(dw + 1);
        let st = if app.remote_device().is_some() || app.jam.is_some() { th.chip(th.accent2) } else { th.s(th.dim) };
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
    let rows = (inner.height.saturating_sub(1) as usize).clamp(1, PALETTE_ROWS);
    let first = selected.saturating_sub(rows - 1);
    for (i, a) in items.iter().enumerate().skip(first).take(rows) {
        let y = inner.y + 1 + (i - first) as u16;
        if y >= inner.bottom() {
            break;
        }
        let row = Rect::new(inner.x, y, inner.width, 1);
        let st = if i == selected { th.s(th.accent).bg(th.selection.color()) } else { th.s(th.fg) };
        let buf = f.buffer_mut();
        blank(buf, inner.x, y, inner.width, st);
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
        (m.label(A::ToggleIdle), "idle takeover on / off (any key wakes the screen)".into()),
        (m.label(A::Quit), "quit the Client (music keeps playing)".into()),
        ("keys.toml".into(), format!("{} keys · all of them in {} → Settings", m.preset, m.label(A::Palette))),
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
    fn every_design_draws_a_one_row_pane_body_without_panicking() {
        // `layout::MIN_H` is 3: a Pane squeezed to its minimum leaves one row inside its frame, and smaller
        // terminals give some Panes no body at all. Nothing in a Pane's drawing may subtract below zero there.
        use crate::layout::{Axis, Node, Size};
        use singularity_proto::{Item, QueueEntry};
        let queue = || vec![
            QueueEntry { item: Item { uri: "spotify:track:a".into(), name: "A".into(), duration_ms: 90_000, ..Default::default() }, source: "queue".into(), uid: "ua".into() },
            QueueEntry { item: Item { uri: "spotify:track:b".into(), name: "B".into(), duration_ms: 90_000, ..Default::default() }, source: "context".into(), uid: "ub".into() },
        ];
        let sizes = [(50u16, 14u16), (60, 14), (80, 14), (120, 14), (50, 15), (80, 16), (100, 18), (80, 20), (120, 24)];
        let mut one_row = false;
        for d in 0..crate::design::DESIGNS.len() {
            for (w, h) in sizes {
                let (mut app, _rx) = playing_app(200_000);
                app.set_design(d);
                app.playback.queue = queue();
                // the Panes this design leaves: are any of them down to one row of body (borders included)?
                let body = Rect::new(0, 1, w, h.saturating_sub(5));
                one_row |= app.layout.compute_shaped(body, 2.0).slots.iter().any(|(_, r)| r.height <= 3);
                let _ = render(&mut app, w, h);
            }
        }
        assert!(one_row, "the sizes above squeeze a Pane to its minimum");
        // and the worst layouts: every Widget in one Pane after another, nested halving, so most of them are one
        // row high or none at all
        let names = ["library", "home", "queue", "lyrics", "fft", "loudness", "well", "nowplaying", "levels", "listening", "rain", "scope", "top", "today", "topweek", "starfield", "marquee", "glow", "hole", "stream"];
        let mut stack = Node::Slot(names[0].into());
        for name in &names[1..] {
            stack = Node::Split { axis: Axis::Vertical, first: Size::Ratio(0.5), a: Box::new(stack), b: Box::new(Node::Slot((*name).into())) };
        }
        for (w, h) in [(60, 14), (80, 20), (120, 40)] {
            let (mut app, _rx) = playing_app(200_000);
            app.layout = stack.clone();
            app.playback.queue = queue();
            assert!(app.layout.compute(Rect::new(0, 1, w, h - 5)).slots.iter().any(|(_, r)| r.height <= 1), "{w}x{h}: the stack squeezes a Pane below two rows");
            let _ = render(&mut app, w, h);
        }
    }

    #[test]
    fn a_tiny_terminal_is_a_mini_player() {
        use singularity_proto::Command;
        let (mut app, mut rx) = playing_app(200_000);
        app.playback.item.as_mut().unwrap().artists = vec!["Nova Drift".into()];
        for (w, h) in [(20u16, 1u16), (40, 2), (60, 5), (49, 13), (120, 13)] {
            let s = render(&mut app, w, h);
            assert!(s.contains("Song"), "{w}x{h}: the title\n{s}");
            assert!(!s.contains("SINGULARITY") && !s.contains("too small"), "{w}x{h}: no layout");
            assert!(s.lines().all(|l| l.chars().count() <= w as usize));
            if w >= 24 {
                let play = app.hits.iter().find_map(|(r, h)| (*h == Hit::Action(Action::TogglePlay)).then_some(*r)).unwrap_or_else(|| panic!("{w}x{h}: no ▶"));
                sent(&mut rx);
                click(&mut app, play.x + 1, play.y);
                assert_eq!(sent(&mut rx), vec![Command::TogglePlay], "{w}x{h}");
            }
            if h >= 2 {
                assert!(app.hits.iter().any(|(_, h)| matches!(h, Hit::Bar(Bar::Seek, _))), "{w}x{h}: a progress bar to seek with");
            }
        }
        // big enough again: the full layout comes back
        assert!(render(&mut app, 80, 24).contains("SINGULARITY"));
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
        let well = app.layout.compute_shaped(Rect::new(0, 1, 160, 48 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "well").unwrap().1;
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
    fn palette_labels_are_short_and_old_words_still_find_things() {
        for a in Action::ALL {
            assert!(a.label().chars().count() <= 28, "{:?}: {:?} is too long", a, a.label());
            assert!(!a.label().contains('('), "{:?}: no explanations in the palette", a);
        }
        let (mut app, _rx) = home_app();
        let found = |app: &mut App, q: &str| {
            app.palette = Some(crate::app::Palette { query: q.into(), selected: 0 });
            app.palette_items()
        };
        assert_eq!(found(&mut app, "palette")[0], Action::NextVariant, "the old word still finds it");
        assert!(found(&mut app, "colours").contains(&Action::PickTheme));
        assert!(found(&mut app, "layout").contains(&Action::SaveLayout));
        assert!(found(&mut app, "wrapped").contains(&Action::Stats));
        assert!(found(&mut app, "rename").contains(&Action::EditPlaylist));
        // the help screen's lines fit 80 columns
        for (k, d) in help_rows(&app.keymap) {
            assert!(k.chars().count() <= 12 && d.chars().count() <= 62, "{k:?} {d:?}");
        }
    }

    #[test]
    fn palette_filters_and_runs_actions() {
        let mut app = App::new(None);
        app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert!(app.palette.is_some());
        for c in "help".chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.palette_items()[0], Action::Help);
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
        let row = s.lines().position(|l| l.contains("Quit")).unwrap() as u16;
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

    #[test]
    fn the_visuals_keep_their_rate_while_nothing_plays() {
        // D62 (D60 undone): paused or playing, 30 focused and the setting unfocused
        let (mut app, _rx) = playing_app(200_000);
        assert_eq!((app.frame_rate(true), app.frame_rate(false)), (30, 30));
        app.config.visuals.unfocused_fps = 2;
        app.playback.status = PlayStatus::Playing;
        assert_eq!((app.frame_rate(true), app.frame_rate(false)), (30, 2));
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

    #[test]
    fn a_volume_just_set_isnt_undone_by_a_late_update() {
        let (mut app, _rx) = playing_app(240_000);
        app.playback.volume = 50;
        render(&mut app, 160, 40);
        let vbar = app.hits.iter().find_map(|(_, h)| match h { Hit::Bar(Bar::Volume, r) => Some(*r), _ => None }).unwrap();
        mouse(&mut app, MouseEventKind::ScrollDown, vbar.x as f64 + 1.5, vbar.y, false);
        std::thread::sleep(crate::app::NUDGE_SETTLE + std::time::Duration::from_millis(20));
        app.tick();
        // the Connect device hasn't reported it yet: an update with the old volume keeps the new one
        let mut late = app.playback.clone();
        late.volume = 50;
        app.playback_event(late.clone());
        assert_eq!(app.display_volume(), 49);
        // confirmed: from then on the player's value is shown as it comes
        late.volume = 49;
        app.playback_event(late.clone());
        late.volume = 30;
        app.playback_event(late);
        assert_eq!(app.display_volume(), 30, "changed elsewhere (the phone): shown");
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
        let home = app.layout.compute_shaped(Rect::new(0, 1, 200, 64 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "home").unwrap().1;
        for _ in 0..400 {
            render(app, 200, 64);
            if let Some(r) = try_card_rect(app, shelf, 0).filter(|r| r.height > 2) {
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
        let home = app.layout.compute_shaped(Rect::new(0, 1, 200, 64 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "home").unwrap().1;
        assert!(app.hits.iter().all(|(r, h)| !matches!(h, Hit::Card(_)) || r.bottom() < home.bottom()));
    }

    #[test]
    fn switching_theme_retints_covers_at_once() {
        let (mut app, _rx) = home_app();
        let card = |app: &mut App| {
            render(app, 200, 64);
            let r = card_rect(app, 1, 0);
            let mut t = Terminal::new(TestBackend::new(200, 64)).unwrap();
            t.draw(|f| draw(f, app)).unwrap();
            match t.backend().buffer()[(r.x + r.width / 2, r.y + 2)].fg {
                ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
                c => panic!("{c:?}"),
            }
        };
        let real = card(&mut app);
        assert!(app.art_tint.is_none(), "Singularity keeps real colours");
        assert!(app.set_theme("terminal-amber"));
        let amber = card(&mut app);
        assert!(app.art_tint.is_some() && amber != real && amber.0 >= amber.1 && amber.1 >= amber.2, "amber: {amber:?}");
        // and back off with `ui.tint_art = false`
        app.config.ui.tint_art = false;
        assert_eq!(card(&mut app), real);
    }

    #[test]
    fn home_scrolls_by_lines_to_the_feed_tail() {
        let (mut app, _rx) = home_app();
        render(&mut app, 200, 64);
        scroll_to(&mut app, 30);
        app.feed_view().scroll_lines(1000);
        let s = render(&mut app, 200, 64);
        assert!(s.contains("Feed pick 20"), "the feed tail is reachable:\n{s}");
        assert!(s.contains('♪'), "covers still loading show a placeholder");
        assert!(s.contains("Because you"), "feed Cards carry their section title");
        assert!(!s.contains("Liked Songs"));
    }

    #[test]
    fn the_main_panes_arrows_go_back_and_forward() {
        let (mut app, _rx) = home_app();
        render(&mut app, 200, 64);
        assert!(!app.hits.iter().any(|(_, h)| matches!(h, Hit::History(_))), "no history yet: no arrows");
        let r = card_rect(&app, 1, 0);
        click(&mut app, r.x + 2, r.y + 1);
        render(&mut app, 200, 64);
        let back = app.hits.iter().find_map(|(r, h)| (*h == Hit::History(-1)).then_some(*r)).expect("← shown with history");
        // at the right end of the title, where every Pane keeps its arrows (D44)
        let home = app.layout.compute_shaped(Rect::new(0, 1, 200, 64 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "home").unwrap().1;
        assert!(back.x > home.x + home.width / 2, "{back:?} in {home:?}");
        assert!(render(&mut app, 200, 64).contains("← →"));
        assert!(app.view.is_some());
        click(&mut app, back.x + 1, back.y);
        assert!(app.view.is_none() && app.forward.len() == 1, "back on Home");
        render(&mut app, 200, 64);
        let fwd = app.hits.iter().find_map(|(r, h)| (*h == Hit::History(1)).then_some(*r)).expect("▸ shown");
        click(&mut app, fwd.x, fwd.y);
        assert!(app.view.as_ref().is_some_and(|v| v.uri == "spotify:playlist:mfy1"), "forward again");
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

    #[test]
    fn in_kitty_popups_cover_images_instead_of_turning_them_into_blocks() {
        let (mut app, _rx) = home_app();
        let mut k = crate::gfx::Kitty::new(false);
        k.below_backgrounds = true;
        app.kitty = Some(k);
        app.cell_px = Some((10.0, 20.0));
        let flush = |app: &mut App| {
            let mut out = vec![];
            app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
            String::from_utf8(out).unwrap()
        };
        render(&mut app, 200, 64);
        let first = flush(&mut app);
        assert!(first.contains("a=p,") && first.contains("z=-1073741825"), "placed under cell backgrounds");
        assert!(!first.contains(",z=-1,"));
        // the cells under a cover let it show: the terminal's default background
        let card = card_rect(&app, 1, 0);
        let mut t = Terminal::new(TestBackend::new(200, 64)).unwrap();
        t.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(t.backend().buffer()[(card.x + 1, card.y + 1)].bg, ratatui::style::Color::Reset);
        // the palette opens over Home: no half-blocks, nothing removed or re-sent; the palette's cells hide them
        app.run(Action::Palette);
        let s = render(&mut app, 200, 64);
        assert!(!s.contains('▀'), "no step-down to half-blocks");
        let out = flush(&mut app);
        assert!(!out.contains("a=d,d=i") && !out.contains("a=t,"), "{out}");
        let mut t = Terminal::new(TestBackend::new(200, 64)).unwrap();
        t.draw(|f| draw(f, &mut app)).unwrap();
        let pal = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::PaletteItem(0)).then_some(*r)).unwrap();
        // …and not one kitty takes for its default: pure black (its usual background) would let the covers through
        let bg = t.backend().buffer()[(pal.x + 2, pal.y + 1)].bg;
        assert!(bg != ratatui::style::Color::Reset && bg != app.theme.bg.color(), "{bg:?}");
    }

    #[test]
    fn part_visible_rows_are_drawn_cropped_in_both_tiers() {
        // cell tier: scrolled 3 lines into the first Shelf, its covers' lower rows still show
        let (mut app, _rx) = home_app();
        render(&mut app, 200, 64);
        let first_shelf = app.home_view.rows.iter().position(|r| r.kind == crate::home::RowKind::Shelf).unwrap();
        let shelf_y: u16 = app.home_view.rows.iter().take(first_shelf).map(|r| r.height).sum();
        app.home_view.offset = shelf_y + 3;
        render(&mut app, 200, 64);
        let r = card_rect(&app, 1, 0);
        let home = app.layout.compute_shaped(Rect::new(0, 1, 200, 64 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "home").unwrap().1;
        assert_eq!(r.y, home.y + 1, "the Card is clipped at the Pane's top, not dropped");
        // pixel tier: the same cover placed with a crop of its lower part
        app.kitty = Some(crate::gfx::Kitty::new(false));
        app.cell_px = Some((10.0, 20.0));
        render(&mut app, 200, 64);
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        // the Shelf's title line plus 3 cover lines are above: 2 cover lines (40 px) cut off the top
        assert!(out.contains(",x=0,y=40,"), "a cropped placement: {}", &out[..out.len().min(300)]);
    }

    // ── Library (#9) ──

    fn library_app() -> (App, tokio::sync::mpsc::Receiver<singularity_proto::Command>) {
        use singularity_proto::{Library, LibraryItem};
        let (mut app, rx) = playing_app(200_000);
        let item = |uri: &str, name: &str, kind: &str, depth| LibraryItem { uri: uri.into(), name: name.into(), subtitle: String::new(), kind: kind.into(), image_url: None, depth, own: false };
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
        for part in ["⌂  Home", "♥  Liked Songs", "▸  Moods", "≡  Night Drive", "◎  Quiet Orbit"] {
            assert!(s.contains(part), "missing {part:?}\n{s}");
        }
        let moods = side_rect(&app, 3);
        let line = s.lines().nth(moods.y as usize + 1).unwrap();
        assert!(line.find('≡').unwrap() > s.lines().nth(moods.y as usize).unwrap().find('▸').unwrap(), "the playlist sits inside the folder");
    }

    #[test]
    fn sidebar_rows_show_a_miniature_cover_once_it_has_arrived() {
        let (mut app, _rx) = library_app();
        let url = "https://i.scdn.co/image/night";
        app.library.as_mut().unwrap().items[2].image_url = Some(url.into());
        let s = render(&mut app, 160, 48);
        assert!(s.contains("≡  Night Drive"), "the glyph while the cover loads\n{s}");
        app.covers.insert(url, image::RgbImage::from_pixel(64, 64, image::Rgb([200, 40, 90])));
        let mut t = Terminal::new(TestBackend::new(160, 48)).unwrap();
        t.draw(|f| draw(f, &mut app)).unwrap();
        let row = side_rect(&app, 4);
        let buf = t.backend().buffer().clone();
        let x = (row.x..row.right()).find(|&x| buf[(x, row.y)].symbol() == "▀").expect("a half-block cover");
        assert_eq!(buf[(x, row.y)].fg, ratatui::style::Color::Rgb(200, 40, 90));
        assert_eq!(buf[(x + 1, row.y)].symbol(), "▀", "two cells wide");
        let line: String = (row.x..row.right()).map(|x| buf[(x, row.y)].symbol().to_string()).collect();
        assert!(line.contains("▀▀ Night Drive") && !line.contains('≡'), "{line}");
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
            app.meters.push(singularity_proto::SpectrumFrame { bands, level: 180, vu: [220, 200], beat: i % 15 == 0, out_level: 180, out_vu: [220, 200], out_bands: [200; 3] });
        }
        let s = render(&mut app, 160, 48);
        let slots = app.layout.compute_shaped(Rect::new(0, 1, 160, 48 - 5), 2.0).slots;
        let pane = |name: &str| slots.iter().find(|(s, _)| s == name).unwrap().1;
        let region = |r: Rect| s.lines().skip(r.y as usize + 1).take(r.height as usize - 2).map(|l| l.chars().skip(r.x as usize + 1).take(r.width as usize - 2).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert!(region(pane("fft")).contains('█'), "{}", region(pane("fft")));
        let loud = region(pane("loudness"));
        assert!(loud.starts_with("L ▬") && loud.contains('█'), "{loud}");
        assert!(!s.contains("arrives with #15"));
        // the loudness Pane follows the real output level, not the gained one
        app.meters = Default::default();
        app.meters.push(singularity_proto::SpectrumFrame { bands: vec![200; 64], level: 230, vu: [230, 230], out_vu: [20, 20], ..Default::default() });
        let s = render(&mut app, 160, 48);
        let loud = s.lines().nth(pane("loudness").y as usize + 1).unwrap().chars().skip(pane("loudness").x as usize + 1).take(pane("loudness").width as usize - 2).collect::<String>();
        assert!(loud.matches('▬').count() <= 3, "a quiet output shows a short meter: {loud}");
    }

    #[test]
    fn the_well_cycles_looks_and_sends_animated_frames_in_the_pixel_tier() {
        let mut app = App::new(None);
        let s = render(&mut app, 160, 48);
        assert!(s.contains("gravity well") && s.contains("◂ Inferno ▸"));
        app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        assert!(render(&mut app, 160, 48).contains("◂ Gargantua ▸"));
        let well = app.layout.compute_shaped(Rect::new(0, 1, 160, 48 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "well").unwrap().1;
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
    fn the_wheel_moves_the_highlight_anywhere_over_a_list_and_the_library() {
        let mut app = App::new(None);
        app.view = Some(crate::tracks::TrackView::new("spotify:playlist:p", "P"));
        let rows = (0..80)
            .map(|i| singularity_proto::TrackRow {
                uri: format!("spotify:track:{i}"),
                name: format!("Song {i}"),
                artists: vec!["Nova Drift".into()],
                artist_uris: vec!["spotify:artist:n".into()],
                album: "Album".into(),
                album_uri: Some("spotify:album:a".into()),
                playable: true,
                ..Default::default()
            })
            .collect();
        app.track_page(singularity_proto::TrackPage { uri: "spotify:playlist:p".into(), offset: 0, total: 80, tracks: rows, header: None });
        app.library = Some(singularity_proto::Library {
            items: (0..60).map(|i| singularity_proto::LibraryItem { uri: format!("spotify:playlist:{i}"), name: format!("List {i}"), subtitle: String::new(), kind: "playlist".into(), image_url: None, depth: 0, own: false }).collect(),
        });
        render(&mut app, 160, 48);
        // over an artist link, where the wheel used to do nothing
        let link = app.hits.iter().find(|(_, h)| matches!(h, Hit::Link(l) if l.uri == "spotify:artist:n")).unwrap().0;
        for _ in 0..3 {
            mouse(&mut app, MouseEventKind::ScrollDown, link.x as f64 + 0.5, link.y, false);
        }
        assert_eq!(app.view.as_ref().unwrap().sel, 3);
        mouse(&mut app, MouseEventKind::ScrollUp, link.x as f64 + 0.5, link.y, false);
        assert_eq!(app.view.as_ref().unwrap().sel, 2);
        // Shift: a page; the list follows the highlight
        let vis = app.view.as_ref().unwrap().visible;
        for _ in 0..3 {
            app.pointer(MouseEvent { kind: MouseEventKind::ScrollDown, column: link.x, row: link.y, modifiers: KeyModifiers::SHIFT }, link.x as f64 + 0.5, false);
        }
        let v = app.view.as_ref().unwrap();
        assert_eq!(v.sel, 2 + 3 * vis);
        assert!(v.sel >= v.top && v.sel < v.top + v.visible, "the highlight stays in view");
        // the library sidebar
        let lib = app.layout.compute_shaped(Rect::new(0, 1, 160, 48 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "library").unwrap().1;
        for _ in 0..50 {
            mouse(&mut app, MouseEventKind::ScrollDown, lib.x as f64 + 3.5, lib.y + 3, false);
        }
        render(&mut app, 160, 48);
        assert_eq!(app.sidebar.sel, 50);
        assert!(app.sidebar.sel >= app.sidebar.top && app.sidebar.sel < app.sidebar.top + app.sidebar.visible);
    }

    #[test]
    fn cockpits_listening_pane_shows_the_log_as_weeks() {
        let mut app = App::new(None);
        app.set_design(crate::design::by_id("cockpit").unwrap());
        assert!(render(&mut app, 160, 48).contains("the listening log comes from singularityd"));
        let mut minutes = vec![0; singularity_proto::LISTENING_DAYS];
        let n = minutes.len();
        (minutes[n - 1], minutes[n - 2], minutes[n - 8]) = (60, 30, 15);
        app.listening = Some(singularity_proto::ListeningSummary { minutes, today_weekday: 3, hours_this_year: 1.75, top_artist: Some("Nova Drift".into()), streak_days: 2, plays: 9, plays_today: 3 });
        let s = render(&mut app, 160, 48);
        assert!(s.contains("listening · 26 weeks") && s.contains("1.8 h this year · top: Nova Drift · streak 2 days"), "{s}");
        // this week is the last column, Monday on top, two days a cell (Cockpit's Pane is short): Wednesday over
        // Thursday (today, the brightest) in the second row; a week ago too; Friday on hasn't come
        let r = app.layout.compute_shaped(Rect::new(0, 1, 160, 48 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "listening").unwrap().1;
        let (x0, y0) = (r.x + 2, r.y + 1);
        let last = ((r.width - 2) / 2).min(26) - 1;
        let buf = {
            let mut t = Terminal::new(TestBackend::new(160, 48)).unwrap();
            t.draw(|f| draw(f, &mut app)).unwrap();
            t.backend().buffer().clone()
        };
        let cell = |c: u16, row: u16| &buf[(x0 + c * 2, y0 + row)];
        let th = &app.theme;
        let quiet = th.bg.lerp(th.dim, 0.3).color();
        let brightest = th.ramp.last().unwrap().color();
        assert_eq!((cell(last, 1).symbol(), cell(last, 1).bg), ("▀", brightest), "today");
        assert_ne!(cell(last, 1).fg, quiet, "yesterday");
        assert_eq!((cell(last, 0).fg, cell(last, 0).bg), (quiet, quiet), "Monday, Tuesday: nothing");
        assert_ne!(cell(last - 1, 1).bg, quiet, "a week ago");
        assert_eq!(cell(last, 2).symbol(), " ", "Friday and Saturday are still to come");
    }

    #[test]
    fn the_well_is_empty_while_its_hole_is_out() {
        let mut app = App::new(None);
        assert!(!render(&mut app, 160, 48).contains("the well is empty"));
        app.run(crate::app::Action::IdlePreview);
        app.tick();
        // (at first the hole still sits over its well; once it has drifted off, the message reads)
        app.idle_for(10.0);
        let s = render(&mut app, 160, 48);
        assert!(s.contains("· the well is empty ·"), "{s}");
        app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.tick();
        assert!(!render(&mut app, 160, 48).contains("the well is empty"));
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
    fn a_starving_stream_is_explained_once() {
        use singularity_proto::{Delivery, StreamInfo};
        let mut app = App::new(None);
        app.stream = Some(StreamInfo { codec: "flac".into(), bitrate: 1_400_000, average_bitrate: Some(1_643_334), sample_rate: 44_100, sample_format: "32-bit-float-pcm".into() });
        assert_eq!(app.need_kbps(), Some(205));
        let now = singularity_proto::now_ms();
        // one stall: nothing said yet
        app.delivery_event(Delivery { bandwidth_kbps: Some(44), measured_ms: now, stalls_ms: vec![now] });
        assert!(app.toast.is_none());
        // they keep coming on a slow connection: said once, with the numbers
        app.delivery_event(Delivery { bandwidth_kbps: Some(44), measured_ms: now, stalls_ms: vec![now - 15_000, now] });
        let msg = app.toast.take().map(|(m, _)| m).unwrap_or_default();
        assert!(msg.contains("too slow for lossless") && msg.contains("44 kB/s") && msg.contains("205"), "{msg}");
        app.delivery_event(Delivery { bandwidth_kbps: Some(40), measured_ms: now, stalls_ms: vec![now - 30_000, now - 15_000, now] });
        assert!(app.toast.is_none(), "not once per stall");
        let s = render(&mut app, 160, 48);
        assert!(s.contains("40 kB/s, need 205") && s.contains("3 in the last 60s"), "{s}");
        // a healthy stream: no stall line, no warning colour
        app.delivery_event(Delivery { bandwidth_kbps: Some(900), measured_ms: now, stalls_ms: vec![] });
        let s = render(&mut app, 160, 48);
        assert!(s.contains("900 kB/s") && !s.contains(", need") && !s.contains("in the last 60s"));
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
        app.health = Some(singularity_proto::Health { soloist_key: "keyring".into(), spotify_login: "keyring".into(), audio_output: Some("DAC Pro".into()), engine_expires_in_days: Some(80), engine_build: Some("2026-10-16".into()), ..Default::default() });
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
    fn the_wheel_moves_the_queues_highlight_and_the_list_follows() {
        use singularity_proto::{Item, QueueEntry};
        let (mut app, _rx) = playing_app(200_000);
        let e = |i: usize| QueueEntry { item: Item { uri: format!("spotify:track:{i}"), name: format!("Song {i}"), ..Default::default() }, source: if i < 5 { "queue" } else { "context" }.into(), uid: format!("u{i}") };
        app.playback.queue = (0..120).map(e).collect();
        render(&mut app, 160, 48);
        let pane = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Focus(s) if s == "queue").then_some(*r)).unwrap();
        // over a heading, a row or "now playing": anywhere in the Pane
        for _ in 0..7 {
            mouse(&mut app, MouseEventKind::ScrollDown, pane.x as f64 + 3.5, pane.y + 1, false);
        }
        assert_eq!((app.queue_view.sel, app.focus.as_str()), (7, "queue"));
        mouse(&mut app, MouseEventKind::ScrollUp, pane.x as f64 + 3.5, pane.y + 1, false);
        assert_eq!(app.queue_view.sel, 6);
        // Shift: a page; the list follows the highlight, which stays on screen
        for _ in 0..4 {
            app.pointer(MouseEvent { kind: MouseEventKind::ScrollDown, column: pane.x + 3, row: pane.y + 1, modifiers: KeyModifiers::SHIFT }, pane.x as f64 + 3.5, false);
            render(&mut app, 160, 48);
        }
        let sel = app.queue_view.sel;
        assert!(sel > 6 + 3 * app.queue_view.visible, "{sel}");
        assert!(app.hits.iter().any(|(_, h)| *h == Hit::QueueRow(sel)), "the highlighted row is drawn");
    }

    #[test]
    fn a_show_lists_its_episodes_with_progress_and_fetches_older_ones_near_the_end() {
        use singularity_proto::{Command, Progress, TrackPage, TrackRow};
        let (mut app, mut rx) = playing_app(200_000);
        let show = "spotify:show:s1";
        let eps = |from: u32, to: u32| TrackPage {
            uri: show.into(),
            offset: from,
            total: 130,
            tracks: (from..to)
                .map(|i| TrackRow {
                    uri: format!("spotify:episode:e{i}"),
                    name: format!("Episode {}", 130 - i),
                    added_at: Some("2026-10-16T06:00:00Z".into()),
                    duration_ms: 3_725_000,
                    playable: true,
                    progress: Some(match i {
                        0 => Progress { position_ms: 600_000, done: false },
                        1 => Progress { position_ms: 0, done: true },
                        _ => Progress::default(),
                    }),
                    ..Default::default()
                })
                .collect(),
            header: (from == 0).then(|| singularity_proto::ListHeader { subtitle: "Podcast · Pub".into(), ..Default::default() }),
        };
        app.open(show, "A Show", "", "show");
        assert_eq!(sent(&mut rx), vec![Command::GetTracks { uri: show.into() }]);
        app.track_page(eps(0, 50));
        app.track_page(eps(50, 100));
        let s = render(&mut app, 160, 48);
        for part in ["130 episodes", "Podcast · Pub", "Progress", "Released", "52 min left", "✓ played", "2026-10-16", "1:02:05"] {
            assert!(s.contains(part), "missing {part:?}\n{s}");
        }
        assert!(sent(&mut rx).is_empty(), "the top of a show asks for nothing more");
        // Follow (#60): the chip says so, and the change goes to the Daemon
        assert!(s.contains("♡ Follow"));
        let chip = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Save(l) if l.uri == show).then_some(*r)).unwrap();
        click(&mut app, chip.x + 1, chip.y);
        assert!(render(&mut app, 160, 48).contains("♥ Following"));
        assert_eq!(sent(&mut rx), vec![Command::SetSaved { uri: show.into(), saved: true }]);
        // near the end of what's here: the next episodes, once
        app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        render(&mut app, 160, 48);
        render(&mut app, 160, 48);
        assert_eq!(sent(&mut rx), vec![Command::MoreTracks { uri: show.into(), offset: 100 }]);
        // an episode card opens its show at that episode, fetching on until it's there
        app.run(Action::GoHome);
        app.open("spotify:episode:e120", "Episode 10", "", "episode");
        assert_eq!(sent(&mut rx), vec![Command::FindShow { uri: "spotify:episode:e120".into() }]);
        app.episode_show("spotify:episode:e120".into(), singularity_proto::Link { name: "A Show".into(), uri: show.into() });
        assert_eq!(sent(&mut rx), vec![Command::GetTracks { uri: show.into() }]);
        app.track_page(eps(0, 50));
        app.track_page(eps(50, 100));
        render(&mut app, 160, 48);
        assert!(sent(&mut rx).iter().any(|c| matches!(c, Command::MoreTracks { offset: 100, .. })));
        app.track_page(eps(100, 130));
        render(&mut app, 160, 48);
        let v = app.view.as_ref().unwrap();
        assert_eq!((v.sel, v.reveal_uri.is_none()), (120, true));
        assert!(render(&mut app, 160, 48).contains("Episode 10"));
    }

    #[test]
    fn a_book_lists_its_chapters_and_librespot_says_what_plays_them() {
        use singularity_proto::{Card, Command, EngineKind, Progress, SearchResults, TrackPage, TrackRow};
        let (mut app, mut rx) = playing_app(200_000);
        let book = "spotify:show:b1";
        // in search: the Audiobooks chip, with Spotify's access line (D58)
        app.run(Action::Search);
        app.search.as_mut().unwrap().query = "orbit".into();
        let card = Card { uri: book.into(), name: "The Long Orbit".into(), subtitle: "Listen free until Oct 31 · Ada Quill".into(), image_url: None, kind: "show".into() };
        app.search.as_mut().unwrap().set_results(SearchResults { query: "orbit".into(), audiobooks: vec![card], ..Default::default() });
        app.search.as_mut().unwrap().set_chip(6);
        let s = render(&mut app, 160, 48);
        for part in ["Audiobooks", "The Long Or", "Listen free"] {
            assert!(s.contains(part), "missing {part:?}\n{s}");
        }
        // its page: chapters with progress, Save (not Follow), no release dates
        app.open(book, "The Long Orbit", "", "show");
        sent(&mut rx);
        let chapters = (0..24u32)
            .map(|i| TrackRow {
                uri: format!("spotify:episode:c{i}"),
                name: format!("Chapter {i}"),
                duration_ms: 1_500_000,
                playable: true,
                progress: Some(if i == 0 { Progress { position_ms: 0, done: true } } else { Progress::default() }),
                ..Default::default()
            })
            .collect();
        let header = singularity_proto::ListHeader { subtitle: "Audiobook · Ada Quill · Listen free until Oct 31 · narrated by Rowan Tide".into(), book: true, ..Default::default() };
        app.track_page(TrackPage { uri: book.into(), offset: 0, total: 24, tracks: chapters, header: Some(header) });
        let s = render(&mut app, 160, 48);
        for part in ["24 chapters", "♡ Save", "Audiobook · Ada Quill · Listen free until Oct 31", "✓ played", "Chapter 3"] {
            assert!(s.contains(part), "missing {part:?}\n{s}");
        }
        assert!(!s.contains("Released") && !s.contains("Follow"), "{s}");
        // Soloist plays a chapter in its book
        app.engine_kind = EngineKind::Soloist;
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::PlayFrom { context_uri: book.into(), index: 1, track_uri: Some("spotify:episode:c1".into()) }]);
        // librespot can't (D57): nothing is sent, and the Client says what can
        app.engine_kind = EngineKind::Librespot;
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(sent(&mut rx).is_empty());
        assert!(render(&mut app, 160, 48).contains("Audiobooks need Soloist or the Spotify app"));
    }

    #[test]
    fn podcasts_in_search_list_episodes_that_play_and_lead_to_their_show() {
        use singularity_proto::{Command, Progress, SearchResults, TrackRow};
        let (mut app, mut rx) = playing_app(200_000);
        app.run(Action::Search);
        app.search.as_mut().unwrap().query = "orbits".into();
        let ep = |i: u32, p: Progress| TrackRow { uri: format!("spotify:episode:e{i}"), name: format!("Orbit talk {i}"), artists: vec!["A Show".into()], artist_uris: vec!["spotify:show:s".into()], duration_ms: 3_000_000, playable: true, progress: Some(p), ..Default::default() };
        let episodes = vec![ep(1, Progress { position_ms: 1_200_000, done: false }), ep(2, Progress { position_ms: 0, done: true })];
        app.search.as_mut().unwrap().set_results(SearchResults { query: "orbits".into(), episodes, ..Default::default() });
        app.search.as_mut().unwrap().set_chip(5);
        let s = render(&mut app, 160, 48);
        for part in ["Episodes", "Orbit talk 1", "A Show", "30 min left", "✓ played", "50 min"] {
            assert!(s.contains(part), "missing {part:?}\n{s}");
        }
        sent(&mut rx);
        // a double-click plays it
        let row = app.hits.iter().find_map(|(r, h)| (*h == Hit::SearchTrack(0)).then_some(*r)).unwrap();
        click(&mut app, row.x + 2, row.y);
        click(&mut app, row.x + 2, row.y);
        assert_eq!(sent(&mut rx), vec![Command::Play { uri: Some("spotify:episode:e1".into()) }]);
        // its menu leads to the show, at that episode
        app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: row.x + 2, row: row.y, modifiers: KeyModifiers::NONE });
        let at = app.menu.as_ref().unwrap().items.iter().position(|(l, _)| l == "Go to show").expect("Go to show");
        for _ in 0..at {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::FindShow { uri: "spotify:episode:e1".into() }]);
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

    // ── Queue drag (#32) ──

    #[test]
    fn dragging_reorders_the_queue_and_lifts_album_songs_into_it() {
        use singularity_proto::{Command, Item, QueueEntry};
        let (mut app, mut rx) = playing_app(200_000);
        let e = |n: &str, source: &str| QueueEntry { item: Item { uri: format!("spotify:track:{n}"), name: format!("Song {n}"), ..Default::default() }, source: source.into(), uid: format!("u{n}") };
        app.playback.queue = vec![e("a", "queue"), e("b", "queue"), e("c", "queue"), e("x", "context"), e("y", "context")];
        render(&mut app, 160, 48);
        let row = |app: &App, i: usize| app.hits.iter().find_map(|(r, h)| (*h == Hit::QueueRow(i)).then_some(*r)).unwrap();
        let (c, a) = (row(&app, 2), row(&app, 0));
        let mouse = |app: &mut App, kind, r: Rect| app.mouse(MouseEvent { kind, column: r.x + 3, row: r.y, modifiers: KeyModifiers::NONE });
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), c);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), a);
        // while dragging, the list shows where it lands
        let s = render(&mut app, 160, 48);
        let (pc, pa) = (s.find("Song c").unwrap(), s.find("Song a").unwrap());
        assert!(pc < pa, "c shown above a while dragging");
        assert!(sent(&mut rx).is_empty(), "nothing sent before the drop");
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), a);
        assert_eq!(sent(&mut rx), vec![Command::MoveInQueue { uri: "spotify:track:c".into(), uid: Some("uc".into()), to: 0, into_queue: false }]);
        assert_eq!(app.playback.queue.iter().map(|q| q.item.uri.rsplit(':').next().unwrap()).collect::<String>(), "cabxy", "shown at once");
        // dragged far down, it stays among the queued songs
        render(&mut app, 160, 48);
        let (c, y) = (row(&app, 0), row(&app, 4));
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), c);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), y);
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), y);
        assert_eq!(sent(&mut rx), vec![Command::MoveInQueue { uri: "spotify:track:c".into(), uid: Some("uc".into()), to: 2, into_queue: false }]);
        let order = |app: &App| app.playback.queue.iter().map(|q| format!("{}{}", q.item.uri.rsplit(':').next().unwrap(), if q.source == "queue" { "" } else { "'" })).collect::<String>();
        // a context song moves among the context's
        render(&mut app, 160, 48);
        let (y, x) = (row(&app, 4), row(&app, 3));
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), y);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x);
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x);
        assert_eq!(sent(&mut rx), vec![Command::MoveInQueue { uri: "spotify:track:y".into(), uid: Some("uy".into()), to: 0, into_queue: false }], "first of the context");
        assert_eq!(order(&app), "abcy'x'");
        // …or up into your queue (#50): onto a queued row it takes that place, and the preview shows it there
        render(&mut app, 160, 48);
        let (x, b) = (row(&app, 4), row(&app, 1));
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), b);
        let s = render(&mut app, 160, 48);
        let (px, pb, pnext) = (s.find("Song x").unwrap(), s.find("Song b").unwrap(), s.find("Next from").or(s.find("Next up")).unwrap());
        assert!(px < pb && pb < pnext, "x shown among the queued while dragging");
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), b);
        assert_eq!(sent(&mut rx), vec![Command::MoveInQueue { uri: "spotify:track:x".into(), uid: Some("ux".into()), to: 1, into_queue: true }]);
        assert_eq!(order(&app), "axbcy'");
        // onto the context's heading: last of the queued
        render(&mut app, 160, 48);
        let y = row(&app, 4);
        let heading = Rect { y: y.y - 1, ..y };
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), y);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), heading);
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), heading);
        assert_eq!(sent(&mut rx), vec![Command::MoveInQueue { uri: "spotify:track:y".into(), uid: Some("uy".into()), to: 4, into_queue: true }]);
        assert_eq!(order(&app), "axbcy");
        // a queued song never goes down into the context
        app.playback.queue.push(e("w", "context"));
        render(&mut app, 160, 48);
        let (a, w) = (row(&app, 0), row(&app, 5));
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), a);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), w);
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), w);
        assert_eq!(sent(&mut rx), vec![Command::MoveInQueue { uri: "spotify:track:a".into(), uid: Some("ua".into()), to: 4, into_queue: false }], "last of the queued");
        app.playback.queue.pop();
        // autoplay doesn't move
        app.playback.queue.push(e("z", "autoplay"));
        render(&mut app, 160, 48);
        let (z, top) = (row(&app, 5), row(&app, 3));
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), z);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), top);
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), top);
        assert!(sent(&mut rx).is_empty());
    }

    #[test]
    fn a_queue_drag_survives_the_queue_changing_under_it() {
        use singularity_proto::{Command, Item, QueueEntry};
        let (mut app, mut rx) = playing_app(200_000);
        let e = |n: &str, source: &str| QueueEntry { item: Item { uri: format!("spotify:track:{n}"), name: format!("Song {n}"), ..Default::default() }, source: source.into(), uid: format!("u{n}") };
        app.playback.queue = vec![e("a", "queue"), e("x", "context"), e("y", "context")];
        render(&mut app, 160, 48);
        let row = |app: &App, i: usize| app.hits.iter().find_map(|(r, h)| (*h == Hit::QueueRow(i)).then_some(*r)).unwrap();
        let mouse = |app: &mut App, kind, r: Rect| app.mouse(MouseEvent { kind, column: r.x + 3, row: r.y, modifiers: KeyModifiers::NONE });
        let (y, x) = (row(&app, 2), row(&app, 1));
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), y);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x);
        // a Playback event lands mid-drag: two queued songs appear above, so the index the drag started from is a
        // different song now and the context's section starts after the place the drop would have used
        app.playback.queue = vec![e("a", "queue"), e("b", "queue"), e("c", "queue"), e("x", "context"), e("y", "context")];
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x);
        assert_eq!(sent(&mut rx), vec![Command::MoveInQueue { uri: "spotify:track:y".into(), uid: Some("uy".into()), to: 0, into_queue: false }], "the dragged song moves, found by its uid");
        let order: String = app.playback.queue.iter().map(|q| q.item.uri.rsplit(':').next().unwrap()).collect();
        assert_eq!(order, "abcyx", "shown at once, before x");
    }

    #[test]
    fn every_row_has_a_heart_asked_about_in_batches() {
        use singularity_proto::{Command, SavedFlag, TrackPage, TrackRow};
        let (mut app, mut rx) = playing_app(200_000);
        app.open("spotify:playlist:big", "Big", "", "playlist");
        let rows: Vec<TrackRow> = (0..500).map(|i| TrackRow { uri: format!("spotify:track:r{i}"), name: format!("Row {i}"), artists: vec!["A".into()], duration_ms: 180_000, playable: true, ..Default::default() }).collect();
        app.track_page(TrackPage { uri: "spotify:playlist:big".into(), offset: 0, total: 500, tracks: rows, header: None });
        sent_all(&mut rx);
        let checks = |rx: &mut tokio::sync::mpsc::Receiver<Command>| sent_all(rx).into_iter().filter_map(|c| if let Command::CheckSaved { uris } = c { Some(uris) } else { None }).collect::<Vec<_>>();
        render(&mut app, 160, 48);
        let first = checks(&mut rx);
        assert_eq!(first.len(), 1, "one batch for the rows in view and ahead");
        assert_eq!(first[0].len(), 50);
        assert_eq!(first[0][0], "spotify:track:r0");
        // scrolling within that window asks nothing; past it, one more batch
        for _ in 0..10 {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            render(&mut app, 160, 48);
        }
        assert!(checks(&mut rx).is_empty());
        for _ in 0..60 {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            render(&mut app, 160, 48);
        }
        let more = checks(&mut rx);
        assert!(!more.is_empty() && more.iter().all(|b| b.len() <= 50) && more.len() <= 2, "{} batches", more.len());
        // Spotify's answer shows; a click on a ♥ toggles it
        app.saved_event(vec![SavedFlag { uri: "spotify:track:r68".into(), saved: true }, SavedFlag { uri: "spotify:track:r69".into(), saved: false }]);
        let s = render(&mut app, 160, 48);
        let line = |s: &str, name: &str| s.lines().find(|l| l.contains(name)).unwrap().to_string();
        assert!(line(&s, "Row 68 ").contains('♥') && line(&s, "Row 69 ").contains('♡'), "{s}");
        let h = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Save(l) if l.uri == "spotify:track:r69").then_some(*r)).unwrap();
        click(&mut app, h.x, h.y);
        assert!(sent_all(&mut rx).contains(&Command::SetSaved { uri: "spotify:track:r69".into(), saved: true }));
        assert!(line(&render(&mut app, 160, 48), "Row 69 ").contains('♥'), "shown at once");
    }

    // ── Stats page (#36) ──

    fn seeded_stats(period: singularity_proto::StatsPeriod) -> singularity_proto::Stats {
        use singularity_proto::StatsRow;
        let row = |kind: &str, i: usize, name: &str, sub: &str| StatsRow {
            name: name.into(),
            uri: Some(format!("spotify:{kind}:{i}")),
            track_uri: format!("spotify:track:t{i}"),
            subtitle: sub.into(),
            minutes: 300 - i as u32 * 25,
            plays: 40 - i as u32 * 3,
            all_time_plays: 90,
        };
        let artists = ["Nova Drift", "Mira Vale", "Glass Harbour With A Name Long Enough To Need Clipping", "Saffron Echo"];
        singularity_proto::Stats {
            period,
            from_day: 20738,
            to_day: 20744,
            minutes: 1234,
            plays: 321,
            streak_days: 5,
            longest_streak_days: 9,
            by_hour: (0..24).map(|h| (h * 7 % 24) as u32 * 10).collect(),
            top_artists: artists.iter().enumerate().map(|(i, a)| row("artist", i, a, "")).collect(),
            top_tracks: (0..12).map(|i| row("track", i, &format!("Track number {i} with a rather long title"), artists[i % 4])).collect(),
            top_albums: (0..3).map(|i| row("album", i, &format!("Album {i}"), artists[i])).collect(),
        }
    }

    fn stats_app() -> (App, tokio::sync::mpsc::Receiver<singularity_proto::Command>) {
        use singularity_proto::{Command, StatsPeriod};
        let (mut app, mut rx) = playing_app(200_000);
        app.run(Action::Stats);
        assert_eq!(sent(&mut rx), vec![Command::GetStats { period: StatsPeriod::Week }]);
        for p in StatsPeriod::ALL {
            app.stats.insert(p, seeded_stats(p));
        }
        (app, rx)
    }

    #[test]
    fn the_stats_page_fits_every_size_without_spilling() {
        use singularity_proto::StatsPeriod;
        let (mut app, _rx) = stats_app();
        for (w, h) in [(80u16, 24u16), (140, 40), (256, 68)] {
            for (i, _) in StatsPeriod::ALL.iter().enumerate() {
                if i > 0 {
                    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
                }
                let s = render(&mut app, w, h);
                let home = app.layout.compute_shaped(Rect::new(0, 1, w, h - 5), 2.0).slots.into_iter().find(|(s, _)| s == "home").unwrap().1;
                assert!(s.contains("stats"), "{w}x{h}: titled");
                assert!(s.contains("This week") || (s.contains("Week") && s.contains("All")), "{w}x{h}: every tab: {s}");
                assert!(s.contains("plays"), "{w}x{h}: the summary is in view");
                assert!(s.contains("Nova Drift"), "{w}x{h}: the top artist\n{s}");
                // nothing written over the Pane's right border
                let lines: Vec<&str> = s.lines().collect();
                for y in home.y + 1..home.bottom() - 1 {
                    let c = lines[y as usize].chars().nth((home.right() - 1) as usize).unwrap();
                    assert!(!c.is_alphanumeric(), "{w}x{h}: text over the border at row {y}: {:?}", lines[y as usize]);
                }
            }
            // back to the first tab for the next size; ← stops there and doesn't seek
            for _ in 0..5 {
                app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
            }
            assert_eq!(app.stats_view.as_ref().unwrap().period, StatsPeriod::Week);
        }
        let s = render(&mut app, 256, 68);
        assert!(s.contains("Top artists") && s.contains("Top tracks") && s.contains("Top albums"));
        assert!(s.contains("20 h 34 min · 321 plays · 5-day streak"), "{s}");
        assert!(s.contains("12 – 18 Oct 2026"));
    }

    #[test]
    fn stats_rows_open_artists_and_play_tracks() {
        use singularity_proto::{Command, StatsPeriod};
        let (mut app, mut rx) = stats_app();
        render(&mut app, 140, 40);
        // a click on the top artist opens its page, and Back comes back to Stats
        let r = app.hits.iter().find_map(|(r, h)| (*h == Hit::StatsRow(0)).then_some(*r)).unwrap();
        click(&mut app, r.x + 4, r.y);
        assert_eq!(app.artist.as_ref().map(|a| a.uri.as_str()), Some("spotify:artist:0"));
        sent(&mut rx);
        app.run(Action::Back);
        assert!(app.stats_view.is_some());
        assert_eq!(sent(&mut rx), vec![Command::GetStats { period: StatsPeriod::Week }], "asked again on return");
        // Enter on the first track plays it (rows: 4 artists, then tracks)
        render(&mut app, 140, 40);
        for _ in 0..4 {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::Play { uri: Some("spotify:track:t0".into()) }]);
        // a tab click asks for that period; a logged Play refreshes what's shown
        render(&mut app, 140, 40);
        let tab = app.hits.iter().find_map(|(r, h)| (*h == Hit::StatsPeriod(StatsPeriod::Year)).then_some(*r)).unwrap();
        click(&mut app, tab.x + 1, tab.y);
        assert_eq!(sent(&mut rx), vec![Command::GetStats { period: StatsPeriod::Year }]);
        app.refresh_stats();
        assert_eq!(sent(&mut rx), vec![Command::GetStats { period: StatsPeriod::Year }]);
        // a row without a link says so instead of doing nothing
        app.stats.get_mut(&StatsPeriod::Year).unwrap().top_artists[0].uri = None;
        render(&mut app, 140, 40);
        let r = app.hits.iter().find_map(|(r, h)| (*h == Hit::StatsRow(0)).then_some(*r)).unwrap();
        click(&mut app, r.x + 4, r.y);
        assert!(app.toast.as_ref().is_some_and(|(m, _)| m.contains("Finding its link")));
    }

    #[test]
    fn an_empty_log_says_it_fills_as_you_listen() {
        let (mut app, _rx) = playing_app(200_000);
        app.run(Action::Stats);
        assert!(render(&mut app, 140, 40).contains("counting…"));
        app.stats.insert(Default::default(), singularity_proto::Stats { by_hour: vec![0; 24], ..Default::default() });
        let s = render(&mut app, 140, 40);
        // it wraps rather than being cut
        assert!(s.contains("nothing logged yet in this period") && s.contains("half a short track)"), "{s}");
        let home = app.layout.compute_shaped(Rect::new(0, 1, 140, 40 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "home").unwrap().1;
        let pane: String = s.lines().map(|l| l.chars().skip(home.x as usize).take(home.width as usize).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert!(!pane.contains('…'), "nothing cut: {pane}");
    }

    // ── Stats Widgets (#37) ──

    #[test]
    fn stats_widgets_swap_into_every_side_slot_and_fit_their_minimums() {
        use singularity_proto::{Command, ListeningSummary, StatsPeriod};
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let mut app = App::new(Some(tx));
        app.link = Link::Connected { version: "test".into() };
        let mut minutes = vec![0u32; singularity_proto::LISTENING_DAYS];
        let n = minutes.len();
        minutes[n - 3..].copy_from_slice(&[30, 0, 42]);
        app.listening = Some(ListeningSummary { minutes, today_weekday: 2, streak_days: 1, plays_today: 9, ..Default::default() });
        // every side Slot of every design can show both (a Slot is followed by its place in the layout)
        for d in 0..crate::design::DESIGNS.len() {
            app.set_design(d);
            let sides: Vec<usize> = app.layout.slot_names().iter().enumerate().filter(|(_, s)| crate::layouts::options(s).contains(&"today")).map(|(i, _)| i).collect();
            for at in sides {
                for want in ["today", "topweek"] {
                    for _ in 0..12 {
                        let now = app.layout.slot_names()[at].clone();
                        if now == want {
                            break;
                        }
                        app.swap_widget(&now, 1);
                    }
                    assert_eq!(app.layout.slot_names()[at], want, "{}: Slot {at}", crate::design::DESIGNS[d].id);
                    let s = render(&mut app, 160, 48);
                    let title = if want == "today" { "today" } else { "top artists" };
                    assert!(s.contains(title), "{}: {want} in Slot {at}\n{s}", crate::design::DESIGNS[d].id);
                }
                app.set_design(d);
            }
        }
        // swapping "top this week" in asks for the week's stats
        app.set_design(0);
        sent_all(&mut rx);
        while !app.layout.slot_names().contains(&"topweek".to_string()) {
            let s = app.layout.slot_names().into_iter().find(|s| crate::layouts::options(s).contains(&"topweek")).unwrap();
            app.swap_widget(&s, 1);
        }
        assert!(sent_all(&mut rx).contains(&Command::GetStats { period: StatsPeriod::Week }));
        // and again after a logged Play
        app.refresh_stats();
        assert_eq!(sent_all(&mut rx), vec![Command::GetStats { period: StatsPeriod::Week }]);

        // the smallest Panes: nothing spills, nothing panics
        app.stats.insert(StatsPeriod::Week, seeded_stats(StatsPeriod::Week));
        for (w, h) in [(12u16, 3u16), (14, 4), (20, 6), (30, 10), (60, 12)] {
            let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, w + 4, h + 2));
            let th = app.theme.clone();
            crate::stats::today(&mut buf, Rect::new(1, 1, w, h), &app, &th);
            crate::stats::top_week(&mut buf, Rect::new(1, 1, w, h), &mut app, &th);
            for y in 0..h + 2 {
                for x in [0, w + 1, w + 2, w + 3] {
                    assert_eq!(buf[(x, y)].symbol(), " ", "{w}x{h}: drawn outside at ({x},{y})");
                }
            }
        }
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 30, 8));
        let th = app.theme.clone();
        crate::stats::today(&mut buf, Rect::new(0, 0, 30, 8), &app, &th);
        let rows: Vec<String> = (0..8).map(|y| (0..30).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
        assert!(rows[0].contains("42 min today") && rows[1].contains("9 plays") && rows[2].contains("1-day streak"), "{rows:#?}");
        assert!(rows[5].contains('M') || rows[5].contains('W'), "weekday initials under the week: {rows:#?}");
    }

    #[test]
    fn top_this_week_switches_between_artists_and_tracks() {
        use singularity_proto::{Command, StatsPeriod};
        let (mut app, mut rx) = playing_app(200_000);
        while !app.layout.slot_names().contains(&"topweek".to_string()) {
            let s = app.layout.slot_names().into_iter().find(|s| crate::layouts::options(s).contains(&"topweek")).unwrap();
            app.swap_widget(&s, 1);
        }
        app.stats.insert(StatsPeriod::Week, seeded_stats(StatsPeriod::Week));
        let s = render(&mut app, 160, 48);
        assert!(s.contains("top artists") && s.contains("Nova Drift"), "{s}");
        let toggle = app.hits.iter().find_map(|(r, h)| (*h == Hit::TopWeekToggle).then_some(*r)).unwrap();
        click(&mut app, toggle.x + 3, toggle.y);
        let s = render(&mut app, 160, 48);
        assert!(s.contains("top tracks") && s.contains("Track number 0"), "{s}");
        sent(&mut rx);
        // a track plays on a second click
        let r = app.hits.iter().find_map(|(r, h)| (*h == Hit::TopWeek(1)).then_some(*r)).unwrap();
        click(&mut app, r.x + 4, r.y);
        assert!(sent(&mut rx).is_empty());
        click(&mut app, r.x + 4, r.y);
        assert_eq!(sent(&mut rx), vec![Command::Play { uri: Some("spotify:track:t1".into()) }]);
        // back to artists: a click opens one
        let toggle = app.hits.iter().find_map(|(r, h)| (*h == Hit::TopWeekToggle).then_some(*r)).unwrap();
        click(&mut app, toggle.x + 3, toggle.y);
        render(&mut app, 160, 48);
        let r = app.hits.iter().find_map(|(r, h)| (*h == Hit::TopWeek(0)).then_some(*r)).unwrap();
        click(&mut app, r.x + 4, r.y);
        assert_eq!(app.artist.as_ref().map(|a| a.uri.as_str()), Some("spotify:artist:0"));
    }

    #[test]
    fn every_swappable_pane_keeps_its_arrows_whatever_it_shows() {
        // the owner's case: cycling a narrow Pane onto a long title ("listening · 26 weeks", "starfield · demo") hid
        // the ◂ ▸ and left the Slot stuck there
        for d in 0..crate::design::DESIGNS.len() {
            let mut app = App::new(None);
            app.set_design(d);
            for (w, h) in [(80u16, 24u16), (140, 40)] {
                let slots: Vec<(String, Rect)> = app.layout.compute_shaped(Rect::new(0, 1, w, h - 5), 2.0).slots.into_iter().filter(|(s, _)| !crate::layouts::options(s).is_empty() && s != "well").collect();
                for (k, (_, rect)) in slots.iter().enumerate() {
                    // walk this Slot through its whole cycle; the arrows are there every step
                    for _ in 0..14 {
                        render(&mut app, w, h);
                        let now = app.layout.compute_shaped(Rect::new(0, 1, w, h - 5), 2.0).slots.into_iter().filter(|(s, _)| !crate::layouts::options(s).is_empty() && s != "well").nth(k).unwrap().0;
                        if rect.width >= 12 {
                            assert!(app.hits.iter().any(|(r, h)| *h == Hit::Swap(now.clone(), 1) && r.y == rect.y), "{} {w}x{h}: no ▸ on {now}", crate::design::DESIGNS[d].id);
                        }
                        app.swap_widget(&now, 1);
                    }
                }
            }
        }
    }

    #[test]
    fn the_wheel_scrolls_an_artist_page_wherever_the_pointer_is() {
        let (mut app, _rx) = playing_app(200_000);
        let mut page = fake_artist();
        let more = page.shelves[0].clone();
        page.shelves.extend((0..6).map(|i| singularity_proto::Shelf { title: format!("Shelf {i}"), ..more.clone() }));
        app.open("spotify:artist:nova", "Nova Drift", "", "artist");
        app.artist_page(page);
        render(&mut app, 200, 48);
        let wheel = |app: &mut App, x: u16, y: u16| mouse(app, MouseEventKind::ScrollDown, x as f64 + 0.5, y, false);
        // over a popular track's album name (a link of its own): the highlight moves
        let row = app.hits.iter().find_map(|(r, h)| (*h == Hit::TopTrack(2)).then_some(*r)).unwrap();
        let link = app.hits.iter().find_map(|(r, h)| (matches!(h, Hit::Link(_)) && r.y == row.y).then_some(*r)).expect("a name link on the row");
        wheel(&mut app, link.x + 1, link.y);
        assert_eq!(app.artist.as_ref().unwrap().top_sel, 1);
        // below the songs, over the Shelves (Cards or not): the page scrolls by lines
        let home = app.layout.compute_shaped(Rect::new(0, 1, 200, 48 - 5), 2.0).slots.into_iter().find(|(s, _)| s == "home").unwrap().1;
        wheel(&mut app, home.right() - 3, home.bottom() - 3);
        render(&mut app, 200, 48);
        assert!(app.artist.as_ref().unwrap().shelves.offset > 0, "the Shelves scrolled");
    }

    // ── Ambient Widgets (#44) ──

    #[test]
    fn every_ambient_widget_stays_inside_its_pane_at_any_size() {
        use singularity_proto::{Item, QueueEntry};
        let (mut app, _rx) = playing_app(200_000);
        app.playback.item.as_mut().unwrap().name = "Łódź à la carte, vol. 2!".into();
        app.playback.queue = (0..6).map(|i| QueueEntry { item: Item { uri: format!("spotify:track:q{i}"), name: format!("Song {i}"), duration_ms: 150_000 + i * 30_000, ..Default::default() }, source: if i < 2 { "queue" } else { "context" }.into(), uid: String::new() }).collect();
        app.covers.insert("https://i.scdn.co/image/c", image::RgbImage::from_pixel(64, 64, image::Rgb([200, 60, 40])));
        app.playback.item.as_mut().unwrap().cover_url = Some("https://i.scdn.co/image/c".into());
        let th = app.theme.clone();
        type Draw = fn(&mut ratatui::buffer::Buffer, Rect, &mut App, &crate::theme::Theme);
        let demos: [(&str, Draw); 3] = [("starfield", crate::ambient::starfield), ("marquee", crate::ambient::marquee), ("glow", crate::ambient::glow)];
        for (name, draw) in demos {
            for (w, h) in [(12u16, 3u16), (20, 5), (40, 8), (80, 14), (160, 30)] {
                let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, w + 2, h + 2));
                for _ in 0..3 {
                    draw(&mut buf, Rect::new(1, 1, w, h), &mut app, &th);
                }
                for y in 0..h + 2 {
                    for x in 0..w + 2 {
                        if x == 0 || y == 0 || x == w + 1 || y == h + 1 {
                            assert_eq!(buf[(x, y)].symbol(), " ", "{name} {w}x{h}: drew outside at ({x},{y})");
                        }
                    }
                }
            }
        }
        // the starfield costs next to nothing: 600 stars at 160×30, well under a millisecond a frame in release
        // (a debug build is far slower; this only catches a mistake that makes it heavy)
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 160, 30));
        let t0 = std::time::Instant::now();
        for _ in 0..100 {
            crate::ambient::starfield(&mut buf, Rect::new(0, 0, 160, 30), &mut app, &th);
        }
        let per = t0.elapsed().as_secs_f64() * 1000.0 / 100.0;
        assert!(per < 8.0, "starfield {per:.2} ms a frame");
        // the marquee spells the title in big pixels (accents fall back)
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 160, 10));
        crate::ambient::marquee(&mut buf, Rect::new(0, 0, 160, 10), &mut app, &th);
        let s: String = (0..10).map(|y| (0..160).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect();
        assert!(s.contains('█') || s.contains('▀'), "{s}");
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
        app.lyrics = Some(Lyrics { uri: "spotify:track:s".into(), synced: true, lines, source: "Musixmatch".into(), ..Default::default() });
        app.playback.position_ms = 31_000; // line 6 plays
        app.playback.timestamp_ms = singularity_proto::now_ms();
        let s = render(&mut app, 160, 48);
        assert!(s.contains("[00:30] line 6") && s.contains("lyrics: Musixmatch"), "{s}");
        let l9 = app.hits.iter().find_map(|(r, h)| (*h == Hit::Lyric(45_000)).then_some(*r)).unwrap();
        click(&mut app, l9.x + 10, l9.y);
        assert_eq!(sent(&mut rx), vec![Command::Seek { position_ms: 45_000 }]);
        // none: a calm empty state that says why
        app.lyrics = Some(Lyrics { uri: "spotify:track:s".into(), ..Default::default() });
        assert!(render(&mut app, 160, 48).contains("no lyrics for this one"));
        app.lyrics = Some(Lyrics { uri: "spotify:track:s".into(), missing: "no lyrics on Spotify or LRCLIB".into(), ..Default::default() });
        assert!(render(&mut app, 160, 48).contains("no lyrics on Spotify or LRCLIB"));
    }

    /// The lyrics Pane alone, `w` columns wide: its rows as text.
    fn lyrics_pane(app: &mut App, w: u16, h: u16) -> Vec<String> {
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, w, h));
        app.hits.clear();
        let th = app.theme.clone();
        crate::lyrics::draw(&mut buf, Rect::new(0, 0, w, h), app, &th);
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn long_lyric_lines_wrap_and_the_current_one_stays_whole() {
        use singularity_proto::{LyricLine, Lyrics};
        let (mut app, _rx) = playing_app(400_000);
        app.playback.item.as_mut().unwrap().uri = "spotify:track:s".into();
        let long = "and the long ones fold over onto the next row when the Pane is narrow";
        let lines = (0..30).map(|i| LyricLine { start_ms: i * 5_000, text: if i % 3 == 1 { format!("{i} {long}") } else { format!("line {i}") } }).collect();
        app.lyrics = Some(Lyrics { uri: "spotify:track:s".into(), synced: true, lines, source: "LRCLIB".into(), ..Default::default() });
        let at = |app: &mut App, ms: u64| {
            app.playback.position_ms = ms;
            app.playback.timestamp_ms = singularity_proto::now_ms();
        };
        for w in [20u16, 26, 34, 60] {
            for line in 0..30u64 {
                at(&mut app, line * 5_000 + 100);
                let rows = lyrics_pane(&mut app, w, 12);
                // the current line's rows are all in view and, joined, are the whole line: nothing cut
                let mut ys: Vec<u16> = app.hits.iter().filter(|(_, h)| *h == Hit::Lyric(line * 5_000)).map(|(r, _)| r.y).collect();
                ys.sort();
                // text starts after the stamp column when there is one
                let col = if w >= 32 { 9 } else { 1 };
                let joined = ys.iter().map(|&y| rows[y as usize].chars().skip(col).collect::<String>().trim().to_string()).collect::<Vec<_>>().join(" ");
                let whole = if line % 3 == 1 { format!("{line} {long}") } else { format!("line {line}") };
                assert_eq!(joined, whole, "{w} at line {line}: {rows:#?}");
                assert!(ys.windows(2).all(|p| p[1] == p[0] + 1) && ys.last() < Some(&11), "{w} at line {line}: {rows:#?}");
            }
        }
        // the timestamp only on a line's first row, continuations indented
        at(&mut app, 5_100);
        let rows = lyrics_pane(&mut app, 40, 12);
        let i = rows.iter().position(|r| r.contains("[00:05] 1 and")).unwrap();
        assert!(rows[i + 1].starts_with(&" ".repeat(11)) && !rows[i + 1].contains('['), "{rows:#?}");
        // the rows are kept between frames: another song with as many lines, at the same width, is wrapped afresh
        app.playback.item.as_mut().unwrap().uri = "spotify:track:t".into();
        let lines = (0..30).map(|i| LyricLine { start_ms: i * 5_000, text: format!("another song {i}") }).collect();
        app.lyrics = Some(Lyrics { uri: "spotify:track:t".into(), synced: true, lines, source: "LRCLIB".into(), ..Default::default() });
        let rows = lyrics_pane(&mut app, 40, 12);
        assert!(rows.iter().any(|r| r.contains("another song 1")) && !rows.iter().any(|r| r.contains("long")), "{rows:#?}");
    }

    #[test]
    fn unsynced_lyrics_scroll_by_rows_and_stop_at_the_end() {
        use singularity_proto::{LyricLine, Lyrics};
        let (mut app, _rx) = playing_app(400_000);
        app.playback.item.as_mut().unwrap().uri = "spotify:track:s".into();
        let lines = (0..10).map(|i| LyricLine { start_ms: 0, text: format!("line {i} with words enough to wrap") }).collect();
        app.lyrics = Some(Lyrics { uri: "spotify:track:s".into(), synced: false, lines, source: "LRCLIB".into(), ..Default::default() });
        app.lyrics_scroll = 1_000;
        let rows = lyrics_pane(&mut app, 20, 8);
        assert!(rows[6].contains("wrap") && rows[7].trim() == "LRCLIB · unsynced", "the last row of the last line ends the view: {rows:#?}");
        assert!(app.lyrics_scroll < 1_000, "clamped, so the wheel turns back at once");
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
        app.search.as_mut().unwrap().set_results(SearchResults { query: "nova".into(), top: vec![card("Nova Drift", "artist")], tracks, artists: vec![card("Nova Drift", "artist")], albums: vec![card("Event Horizon", "album")], playlists: vec![], podcasts: vec![], episodes: vec![], audiobooks: vec![] });
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

    // ── popups own the mouse ──

    /// What a stray event could move behind a popup.
    fn behind(app: &App) -> String {
        format!("{:?} {:?} {} {} {} {:?}", app.sidebar.sel, app.queue_view.sel, app.focus, app.playback.volume, app.home_view.offset, app.playback.position_ms)
    }

    /// Every mouse event the terminal can send, at points all over the screen: nothing behind the popup moves.
    fn sweep(app: &mut App, w: u16, h: u16, what: &str) {
        // Settings is a page, not a popup: its Panes beside it still take clicks, but the wheel stays on the page
        let wheel_only = what == "settings";
        let before = behind(app);
        for y in (0..h).step_by(2) {
            for x in (0..w).step_by(3) {
                for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
                    for kind in [MouseEventKind::ScrollDown, MouseEventKind::ScrollUp, MouseEventKind::ScrollLeft, MouseEventKind::ScrollRight, MouseEventKind::Drag(MouseButton::Left), MouseEventKind::Down(MouseButton::Right), MouseEventKind::Moved].into_iter().filter(|k| !wheel_only || matches!(k, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight)) {
                        app.pointer(MouseEvent { kind, column: x, row: y, modifiers }, x as f64 + 0.5, false);
                        assert_eq!(behind(app), before, "{what}: {kind:?} at ({x},{y}) moved something behind the popup");
                    }
                }
            }
        }
    }

    #[test]
    fn the_wheel_and_the_mouse_never_reach_the_panes_behind_a_popup() {
        use singularity_proto::{Item, QueueEntry};
        let feed: singularity_proto::HomeFeed = serde_json::from_str(include_str!("../tests/fixtures/home.json")).unwrap();
        let opens: [(&str, fn(&mut App)); 6] = [
            ("devices", |a| a.run(Action::Devices)),
            ("palette", |a| a.run(Action::Palette)),
            ("help", |a| a.run(Action::Help)),
            ("health", |a| a.run(Action::Health)),
            ("menu", |a| a.open_queue_menu(0, Some((20, 10)))),
            ("settings", |a| a.run(Action::Settings)),
        ];
        for (name, open) in opens {
            for (w, h) in [(200u16, 60u16), (90, 30)] {
                let (mut app, _rx) = library_app();
                app.catalogue = Some(CatalogueStatus::Ready { username: "u".into(), country: "SE".into(), premium: true });
                app.home = Some(feed.clone());
                app.playback.queue = (0..8).map(|i| QueueEntry { item: Item { uri: format!("spotify:track:q{i}"), name: format!("Song {i}"), duration_ms: 150_000, ..Default::default() }, source: "queue".into(), uid: format!("u{i}") }).collect();
                app.devices = vec![singularity_proto::Device { id: "a".into(), name: "Speaker".into(), kind: "speaker".into(), active: true, volume: Some(50), is_engine: true }];
                render(&mut app, w, h);
                open(&mut app);
                render(&mut app, w, h);
                sweep(&mut app, w, h, name);
                // and after another frame of the popup, still
                render(&mut app, w, h);
                sweep(&mut app, w, h, name);
            }
        }
    }

    #[test]
    fn the_wheel_drives_the_popup_even_off_its_edge() {
        let (mut app, _rx) = library_app();
        app.devices = (0..3).map(|i| singularity_proto::Device { id: format!("d{i}"), name: format!("Dev {i}"), kind: "speaker".into(), active: i == 0, volume: Some(50), is_engine: false }).collect();
        render(&mut app, 160, 48);
        app.run(Action::Devices);
        render(&mut app, 160, 48);
        assert_eq!(app.devices_sel, 0);
        // far outside the popup, on the Library, the wheel still moves the popup's highlight and nothing else
        mouse(&mut app, MouseEventKind::ScrollDown, 1.5, 10, false);
        mouse(&mut app, MouseEventKind::ScrollDown, 1.5, 10, false);
        assert_eq!((app.devices_sel, app.sidebar.sel), (2, 0));
        // a click outside only closes it
        click(&mut app, 1, 10);
        assert!(!app.devices_open);
        assert_eq!(app.sidebar.sel, 0, "the click that closed the popup didn't open a library row");
        // the Settings page keeps the wheel when the pointer drifts onto the Library
        app.run(Action::Settings);
        render(&mut app, 160, 48);
        mouse(&mut app, MouseEventKind::ScrollDown, 1.5, 10, false);
        assert_eq!(app.sidebar.sel, 0);
        let v = app.settings_view.as_ref().unwrap();
        assert!(v.sel > 0 || v.on_tabs, "the Settings page scrolled");
    }

    // ── Jam (D64) ──

    fn jam(owner: bool, guests: &[&str]) -> singularity_proto::Jam {
        use singularity_proto::JamMember;
        let mut members = vec![JamMember { id: "m0".into(), name: "me".into(), owner: true, listening: true }];
        members.extend(guests.iter().enumerate().map(|(i, g)| JamMember { id: format!("m{}", i + 1), name: g.to_string(), owner: false, listening: true }));
        singularity_proto::Jam { session_id: "s".into(), owner, host: Some("Singularity".into()), host_is_engine: true, invite: owner.then(|| "https://open.spotify.com/socialsession/FakeJamToken123".into()), queue_only: false, members }
    }

    #[test]
    fn a_jam_starts_from_the_picker_shows_its_invite_and_ends_on_a_second_enter() {
        use singularity_proto::{Command, Device};
        let (mut app, mut rx) = playing_app(1);
        let dev = |id: &str, name: &str, active: bool, engine: bool| Device { id: id.into(), name: name.into(), kind: "speaker".into(), active, volume: Some(50), is_engine: engine };
        app.devices = vec![dev("solo", "Singularity", true, true), dev("phone", "Pixel", false, false)];
        app.run(Action::Jam);
        sent(&mut rx);
        let s = render(&mut app, 160, 60);
        assert!(s.contains("✚ Start a Jam on Singularity"), "{s}");
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::StartJam]);
        assert!(app.devices_open, "the picker stays open for the invite");
        // the Daemon answers: the invite, its QR and the guest show, and the bar says a Jam runs
        app.jam = Some(jam(true, &["Guest Friend"]));
        let s = render(&mut app, 160, 60);
        assert!(s.contains("◎ Jam on Singularity · 1 guest") && s.contains("socialsession/FakeJamToken123") && s.contains("♪ Guest Friend"), "{s}");
        // the QR's finder pattern (the visuals draw half blocks too)
        assert!(s.contains("█▀▀▀▀▀█"), "the QR is drawn when there's room: {s}");
        assert!(s.contains("◎ Jam · 1"), "{s}");
        // a short terminal: no room for the QR, the link stays
        let s = render(&mut app, 160, 24);
        assert!(s.contains("socialsession/FakeJamToken123") && !s.contains("█▀▀▀▀▀█"), "{s}");
        // ending asks again
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![]);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![Command::EndJam]);
        // someone else's Jam can't be ended from here
        app.jam = Some(jam(false, &[]));
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(sent(&mut rx), vec![]);
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
        app.lists["spotify:collection"].rows().iter().map(|r| r.uri.clone()).collect()
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
