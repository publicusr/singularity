//! Designs and themes in the running Client (#18): switching, `t` for the next palette, the user's theme files
//! (re-read when they change), and the pickers, which preview as you move and put things back on Esc.

use crate::app::{App, Hit};
use crate::design::DESIGNS;
use crate::theme::{self, Theme};
use crate::ui::{centered, popup, text, blank};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Design,
    Theme,
}

pub struct Picker {
    pub kind: PickerKind,
    /// id, name, what it is
    pub items: Vec<(String, String, String)>,
    pub sel: usize,
    /// what to go back to on Esc: design, theme id, layout and the saved layout's name
    before: (usize, String, crate::layout::Node, Option<String>),
}

/// The user's themes folder and what it held last time we looked.
pub struct ThemeDir {
    pub path: PathBuf,
    stamp: Option<(SystemTime, usize)>,
    polled: Instant,
}

impl ThemeDir {
    pub fn new(path: PathBuf) -> ThemeDir {
        ThemeDir { path, stamp: None, polled: Instant::now() }
    }
}

/// Newest change and number of files: a save, a new file or a deleted one all change it.
fn stamp(dir: &std::path::Path) -> Option<(SystemTime, usize)> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut newest = std::fs::metadata(dir).ok()?.modified().ok()?;
    let mut n = 0;
    for e in rd.flatten() {
        n += 1;
        if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
            newest = newest.max(t);
        }
    }
    Some((newest, n))
}

impl App {
    /// Switch design: its layout, its own palette (unless `theme` says otherwise), its Disk look.
    pub fn set_design(&mut self, d: usize) {
        let d = d.min(DESIGNS.len() - 1);
        self.design = d;
        self.layout = (DESIGNS[d].layout)();
        self.hole.look = DESIGNS[d].look;
        self.focus = "home".into();
        self.theme = theme::builtin(DESIGNS[d].themes[0]).expect("built in").into();
        self.want_widget_data();
    }

    /// Use theme `id` (built-in or the user's); false if there's no such theme.
    pub fn set_theme(&mut self, id: &str) -> bool {
        let t = self.user_themes.iter().find(|t| t.id == id).cloned().or_else(|| theme::builtin(id));
        match t {
            Some(t) => {
                self.theme = t.into();
                true
            }
            None => false,
        }
    }

    /// `ui.design`, `ui.layout` and `ui.theme` from the config, if they differ from what's on screen.
    pub fn apply_ui(&mut self, ui: &singularity_config::Ui) {
        match crate::design::by_id(&ui.design) {
            Some(d) if d != self.design || self.theme.id.is_empty() => {
                self.set_design(d);
                self.layout_name = None;
            }
            Some(_) => {}
            None => self.say(format!("config.toml: no design called {:?}", ui.design)),
        }
        let want = (!ui.layout.is_empty()).then_some(ui.layout.as_str());
        if want != self.layout_name.as_deref() {
            match want {
                Some(id) => match self.saved_layouts.iter().find(|s| s.id == id).cloned() {
                    Some(s) => self.use_layout(&s),
                    None => self.say(format!("config.toml: no saved layout called {id:?} in layouts/")),
                },
                None => {
                    let d = self.design;
                    self.set_design(d);
                    self.layout_name = None;
                }
            }
        }
        let want = if ui.theme.is_empty() { DESIGNS[self.design].themes[0] } else { ui.theme.as_str() };
        if want != self.theme.id && !self.set_theme(want) {
            self.say(format!("config.toml: no theme called {want:?} (built-in, or a file in themes/)"));
        }
    }

    /// Remember the design, theme and layout in config.toml (so they survive a restart).
    pub fn save_ui(&mut self) {
        let own = DESIGNS[self.design].themes[0] == self.theme.id;
        let (design, theme) = (DESIGNS[self.design].id.to_string(), if own { String::new() } else { self.theme.id.clone() });
        let layout = self.layout_name.clone().unwrap_or_default();
        self.config.ui.design = design.clone();
        self.config.ui.theme = theme.clone();
        self.config.ui.layout = layout.clone();
        if let Some(path) = self.config_path() {
            let set = |k: &str, v: &str| singularity_config::set(&path, singularity_config::DEFAULT_CONFIG, k, v.into());
            let r = set("ui.design", &design).and_then(|_| set("ui.theme", &theme)).and_then(|_| set("ui.layout", &layout));
            if let Err(e) = r {
                self.say(format!("Couldn't save to config.toml: {e}"));
            }
        }
    }

    /// `t`: the design's next palette, then the shared ones, then the user's themes, round again.
    pub fn next_variant(&mut self) {
        let mut ids: Vec<String> = DESIGNS[self.design].themes.iter().map(|s| s.to_string()).collect();
        ids.extend(theme::extra_ids().map(String::from));
        ids.extend(self.user_themes.iter().map(|t| t.id.clone()));
        if ids.len() < 2 {
            return self.say(format!("{} has one palette · Ctrl+K → Pick a theme for the others", DESIGNS[self.design].name));
        }
        let i = ids.iter().position(|id| *id == self.theme.id).map(|i| (i + 1) % ids.len()).unwrap_or(0);
        self.set_theme(&ids[i]);
        let name = self.theme.name.clone();
        self.save_ui();
        self.say(format!("Theme: {name}"));
    }

    /// Read the user's themes again if the folder changed (about once a second); a theme being edited applies live.
    pub fn poll_themes(&mut self, force: bool) {
        let Some(dir) = self.theme_dir.as_mut() else { return };
        if !force && dir.polled.elapsed().as_secs_f32() < 1.0 {
            return;
        }
        dir.polled = Instant::now();
        let now = stamp(&dir.path);
        if !force && now == dir.stamp {
            return;
        }
        dir.stamp = now;
        let (themes, problems) = theme::load_dir(&dir.path);
        self.user_themes = themes;
        if let Some(t) = self.user_themes.iter().find(|t| t.id == self.theme.id).cloned() {
            self.theme = t.into();
        }
        if let Some(p) = self.picker.as_mut().filter(|p| p.kind == PickerKind::Theme) {
            let sel = p.items.get(p.sel).map(|i| i.0.clone());
            p.items = theme_items(&self.user_themes);
            p.sel = sel.and_then(|s| p.items.iter().position(|i| i.0 == s)).unwrap_or(p.sel.min(p.items.len() - 1));
        }
        if !problems.is_empty() {
            self.say(problems.join(" · "));
        }
    }

    /// Make the themes folder with an example the first time.
    pub fn init_theme_dir(&mut self, path: PathBuf) {
        if !path.exists() && std::fs::create_dir_all(&path).is_ok() {
            let _ = std::fs::write(path.join("_example.toml"), theme::EXAMPLE_THEME);
        }
        self.theme_dir = Some(ThemeDir::new(path));
        self.poll_themes(true);
    }

    pub fn open_picker(&mut self, kind: PickerKind) {
        self.poll_themes(true);
        self.reload_layouts();
        let items = match kind {
            // the built-in designs, then the user's saved layouts
            PickerKind::Design => DESIGNS
                .iter()
                .map(|d| (d.id.to_string(), d.name.to_string(), d.blurb.to_string()))
                .chain(self.saved_layouts.iter().map(|s| (format!("layout:{}", s.id), s.name.clone(), if s.edited { format!("your changes, unnamed · layouts/{}.toml", s.id) } else { format!("yours · layouts/{}.toml", s.id) })))
                .collect(),
            PickerKind::Theme => theme_items(&self.user_themes),
        };
        let current = match (kind, &self.layout_name) {
            (PickerKind::Design, Some(l)) => format!("layout:{l}"),
            (PickerKind::Design, None) => DESIGNS[self.design].id.to_string(),
            (PickerKind::Theme, _) => self.theme.id.clone(),
        };
        let sel = items.iter().position(|(id, ..)| *id == current).unwrap_or(0);
        self.picker = Some(Picker { kind, items, sel, before: (self.design, self.theme.id.clone(), self.layout.clone(), self.layout_name.clone()) });
    }

    /// Put saved layout `s` on screen (with the colours it was saved with, or its design's).
    pub fn use_layout(&mut self, s: &crate::layouts::Saved) {
        let d = crate::design::by_id(&s.design).unwrap_or(self.design);
        if d != self.design {
            self.set_design(d);
        }
        // the colours it was saved with (a theme that has since gone keeps the design's own)
        if let Some(t) = s.theme.as_deref() {
            self.set_theme(t);
        }
        self.layout = s.tree.clone();
        self.layout_name = Some(s.id.clone());
        self.focus = "home".into();
        self.want_widget_data();
    }

    pub fn reload_layouts(&mut self) {
        let Some(dir) = self.layouts_dir.clone() else { return };
        let (saved, problems) = crate::layouts::load_dir(&dir);
        self.saved_layouts = saved;
        if !problems.is_empty() {
            self.say(problems.join(" · "));
        }
    }

    /// Save what's on screen under `name`; it shows up in the Design picker.
    pub fn save_layout(&mut self, name: &str) {
        let Some(dir) = self.layouts_dir.clone() else { return self.say("No config folder to save layouts in") };
        match crate::layouts::save(&dir, name, DESIGNS[self.design].id, &self.theme.id, &self.layout) {
            Ok(s) => {
                self.layout_name = Some(s.id.clone());
                self.reload_layouts();
                self.save_ui();
                self.say(format!("Saved design \"{}\" (Ctrl+K → Pick a design)", s.name));
            }
            Err(e) => self.say(format!("Couldn't save the layout: {e}")),
        }
    }

    /// Typing a name: for "Save design as…" (#19) or a new playlist (#51).
    pub fn naming_key(&mut self, k: KeyEvent) {
        let Some(n) = self.naming.as_mut() else { return };
        let max = if matches!(n.purpose, crate::app::NameFor::Layout | crate::app::NameFor::Fork) { 40 } else { 100 };
        match k.code {
            KeyCode::Esc => {
                // the first edit of a built-in Design, left unnamed: kept as its one "(edited)" Design
                if self.naming.take().is_some_and(|n| n.purpose == crate::app::NameFor::Fork) {
                    self.keep_edited();
                }
            }
            KeyCode::Enter => {
                let Some(n) = self.naming.take() else { return };
                match n.purpose {
                    crate::app::NameFor::Layout if !n.text.trim().is_empty() => self.save_layout(&n.text),
                    crate::app::NameFor::Layout => {}
                    crate::app::NameFor::Fork if n.text.trim().is_empty() => self.keep_edited(),
                    // never over another Design of the user's
                    crate::app::NameFor::Fork => {
                        let id = crate::layouts::file_id(&n.text);
                        if self.saved_layouts.iter().any(|s| s.id == id && !s.edited) {
                            self.say(format!("There is already a design called \"{}\"", n.text.trim()));
                            self.naming = Some(n);
                        } else {
                            self.save_layout(&n.text);
                        }
                    }
                    crate::app::NameFor::NewPlaylist => self.create_playlist(&n.text),
                }
            }
            KeyCode::Backspace => {
                n.text.pop();
            }
            KeyCode::Char(c) if n.text.chars().count() < max => n.text.push(c),
            _ => {}
        }
    }

    /// Swap the Widget in the Slot showing `slot` for the next one in its group. False if that Slot can't swap.
    pub fn swap_widget(&mut self, slot: &str, dir: i32) -> bool {
        let shown = self.layout.slot_names();
        let Some(to) = crate::layouts::next(slot, &shown, dir) else { return false };
        self.layout.swap(slot, to);
        if self.focus == slot {
            self.focus = to.into();
        }
        self.want_widget_data();
        true
    }

    /// Show item `i` (the preview).
    pub fn picker_select(&mut self, i: usize) {
        let Some(p) = self.picker.as_mut() else { return };
        p.sel = i.min(p.items.len().saturating_sub(1));
        let (kind, id) = (p.kind, p.items[p.sel].0.clone());
        match kind {
            PickerKind::Design => {
                if let Some(l) = id.strip_prefix("layout:") {
                    if let Some(s) = self.saved_layouts.iter().find(|s| s.id == l).cloned() {
                        self.use_layout(&s);
                    }
                } else if let Some(d) = crate::design::by_id(&id).filter(|d| *d != self.design || self.layout_name.is_some()) {
                    self.set_design(d);
                    self.layout_name = None;
                }
            }
            PickerKind::Theme => {
                self.set_theme(&id);
            }
        }
    }

    /// Enter: keep what's shown.
    pub fn picker_keep(&mut self) {
        if self.picker.take().is_some() {
            self.save_ui();
        }
    }

    /// Esc: put back what was there.
    pub fn picker_cancel(&mut self) {
        let Some(p) = self.picker.take() else { return };
        if p.before.0 != self.design {
            self.set_design(p.before.0);
        }
        self.set_theme(&p.before.1);
        self.layout = p.before.2;
        self.layout_name = p.before.3;
        self.want_widget_data();
    }

    pub fn picker_key(&mut self, k: KeyEvent) {
        let Some(p) = self.picker.as_ref() else { return };
        let (sel, n) = (p.sel, p.items.len());
        match k.code {
            KeyCode::Up => self.picker_select(sel.saturating_sub(1)),
            KeyCode::Down => self.picker_select((sel + 1).min(n.saturating_sub(1))),
            KeyCode::Home => self.picker_select(0),
            KeyCode::End => self.picker_select(n.saturating_sub(1)),
            KeyCode::Enter => self.picker_keep(),
            KeyCode::Esc | KeyCode::Char('q') => self.picker_cancel(),
            _ => {}
        }
    }
}

fn theme_items(user: &[Theme]) -> Vec<(String, String, String)> {
    let mut v: Vec<(String, String, String)> = theme::builtin_ids().into_iter().map(|id| {
        let t = theme::builtin(id).expect("built in");
        (t.id, t.name, "built in".to_string())
    }).collect();
    v.extend(user.iter().map(|t| (t.id.clone(), t.name.clone(), format!("themes/{}.toml", t.id))));
    v
}

pub fn picker_rect(area: Rect, p: &Picker) -> Rect {
    centered(area, 64, p.items.len() as u16 + 5)
}

/// The picker: the list, a swatch of the theme on screen, and how to keep or undo it.
pub fn draw_picker(buf: &mut Buffer, area: Rect, app: &mut App, th: &Theme) {
    let Some(p) = app.picker.as_ref() else { return };
    let r = picker_rect(area, p);
    let title = match p.kind {
        PickerKind::Design => "Design",
        PickerKind::Theme => "Theme",
    };
    let inner = popup(buf, r, th, title);
    let items = p.items.clone();
    let sel = p.sel;
    // a long list scrolls with the selection
    let rows = inner.height.saturating_sub(2).max(1) as usize;
    let first = crate::scroll::window(sel, rows, items.len());
    for (i, (_, name, what)) in items.iter().enumerate().skip(first).take(rows) {
        let y = inner.y + (i - first) as u16;
        let bg = if i == sel { th.selection.color() } else { th.bg.color() };
        blank(buf, inner.x, y, inner.width, th.base().bg(bg));
        text(buf, inner.x + 1, y, 2, if i == sel { "›" } else { " " }, th.s(th.accent).bg(bg));
        text(buf, inner.x + 3, y, 26, name, (if i == sel { th.title() } else { th.s(th.fg) }).bg(bg));
        text(buf, inner.x + 30, y, inner.width.saturating_sub(31), what, th.s(th.dim).bg(bg));
        app.hit(Rect::new(inner.x, y, inner.width, 1), Hit::PickerItem(i));
    }
    // the colours on screen, as a swatch
    let y = inner.bottom().saturating_sub(2);
    let mut x = inner.x + 1;
    for c in [th.fg, th.dim, th.accent, th.accent2, th.border_focus].into_iter().chain(th.ramp.iter().copied()) {
        text(buf, x, y, 2, "██", th.s(c));
        x += 2;
    }
    text(buf, inner.x + 1, inner.bottom().saturating_sub(1), inner.width.saturating_sub(2), "↑↓ preview · Enter keep · Esc put back", th.s(th.dim));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyModifiers;
    use ratatui::style::Color;
    use std::collections::HashSet;

    fn render(app: &mut App) -> ratatui::buffer::Buffer {
        let mut t = Terminal::new(TestBackend::new(160, 48)).unwrap();
        t.draw(|f| crate::ui::draw(f, app)).unwrap();
        t.backend().buffer().clone()
    }

    fn screen(b: &ratatui::buffer::Buffer) -> String {
        (0..b.area.height).map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn key(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn with_dirs() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(None);
        app.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
        (app, dir)
    }

    #[test]
    fn every_design_draws_all_its_panes() {
        let mut app = App::new(None);
        for (d, _) in DESIGNS.iter().enumerate() {
            app.set_design(d);
            app.meters.push(singularity_proto::SpectrumFrame { bands: vec![120; 64], level: 150, vu: [150, 150], beat: false, out_level: 150, out_vu: [150, 150], out_bands: [120; 3] });
            let s = screen(&render(&mut app));
            for (slot, _) in app.layout.compute(Rect::new(0, 1, 160, 43)).slots {
                let usual = match slot.as_str() {
                    "well" => "gravity well",
                    "hole" => "black hole",
                    "nowplaying" => "Now playing",
                    "scope" => "osc",
                    "fft" => "fft",
                    _ => "",
                };
                // most Panes are titled after their Slot
                let title = match crate::design::title(d, &slot, usual) {
                    "" => slot.as_str(),
                    t => t,
                };
                // (the main Pane is titled after what it shows: "home" here)
                assert!(s.contains(title), "{}: no {title:?} Pane\n{s}", DESIGNS[d].id);
            }
        }
    }

    /// Colours a theme sets directly.
    fn tokens(t: &Theme) -> HashSet<Color> {
        [t.fg, t.dim, t.accent, t.accent2, t.border, t.border_focus, t.selection].into_iter().chain(t.ramp.iter().copied()).chain(t.boxes.iter().copied()).map(|c| c.color()).collect()
    }

    #[test]
    fn a_palette_switch_recolours_everything() {
        let mut app = App::new(None);
        app.set_design(crate::design::by_id("cockpit").unwrap());
        app.meters.push(singularity_proto::SpectrumFrame { bands: vec![200; 64], level: 200, vu: [200, 200], beat: false, out_level: 200, out_vu: [200, 200], out_bands: [200; 3] });
        let btop = tokens(&app.theme);
        let before = render(&mut app);
        let used = (0..48).flat_map(|y| (0..160).map(move |x| (x, y))).filter(|&(x, y)| btop.contains(&before[(x, y)].fg)).count();
        assert!(used > 500, "btop's colours are on screen to begin with ({used} cells)");
        key(&mut app, KeyCode::Char('t'));
        assert_eq!(app.theme.id, "cockpit-ice");
        let ice = tokens(&app.theme);
        let b = render(&mut app);
        let stale: Vec<(u16, u16)> = (0..48).flat_map(|y| (0..160).map(move |x| (x, y))).filter(|&(x, y)| {
            let c = &b[(x, y)];
            [c.fg, c.bg].iter().any(|col| btop.contains(col) && !ice.contains(col))
        }).collect();
        assert!(stale.is_empty(), "cells still in btop colours: {:?}", &stale[..stale.len().min(10)]);
    }

    #[test]
    fn the_picker_previews_puts_back_and_keeps() {
        let (mut app, dir) = with_dirs();
        app.run(crate::app::Action::PickDesign);
        key(&mut app, KeyCode::Down);
        assert_eq!(DESIGNS[app.design].id, "event-horizon", "previewed at once");
        assert!(app.layout.compute(Rect::new(0, 0, 160, 43)).slots.iter().any(|(s, _)| s == "hole"));
        key(&mut app, KeyCode::Esc);
        assert_eq!((DESIGNS[app.design].id, app.theme.id.as_str()), ("singularity", "singularity"), "Esc puts it back");
        app.run(crate::app::Action::PickDesign);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert_eq!(DESIGNS[app.design].id, "classic");
        let cfg = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(cfg.contains("design = \"classic\"") && cfg.contains("# the design:"), "kept in config.toml, comments too:\n{cfg}");
        // a restart reads it back
        let mut again = App::new(None);
        again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
        assert_eq!((DESIGNS[again.design].id, again.theme.id.as_str()), ("classic", "classic"));
    }

    #[test]
    fn a_new_theme_file_shows_up_in_the_open_picker() {
        let (mut app, dir) = with_dirs();
        let themes = dir.path().join("themes");
        assert!(themes.join("_example.toml").exists(), "an example to start from");
        assert!(app.user_themes.is_empty(), "but not one of the themes");
        app.run(crate::app::Action::PickTheme);
        let n = app.picker.as_ref().unwrap().items.len();
        std::fs::write(themes.join("dusk.toml"), "name = \"Dusk\"\nbase = \"event-horizon\"\n[colors]\naccent = \"#ff8800\"\n").unwrap();
        app.theme_dir.as_mut().unwrap().polled -= std::time::Duration::from_secs(2);
        app.tick();
        let items = &app.picker.as_ref().unwrap().items;
        assert_eq!(items.len(), n + 1);
        let i = items.iter().position(|(id, ..)| id == "dusk").unwrap();
        app.picker_select(i);
        assert_eq!(app.theme.accent, crate::theme::Rgb::hex(0xff8800));
        let s = screen(&render(&mut app));
        assert!(s.contains("Dusk") && s.contains("themes/dusk.toml"), "{s}");
    }

    #[test]
    fn slots_swap_with_v_or_a_click_and_never_show_a_widget_twice() {
        let mut app = App::new(None);
        app.focus = "fft".into();
        key(&mut app, KeyCode::Char('v'));
        let slots = app.layout.slot_names();
        assert!(slots.contains(&"levels".to_string()) && !slots.contains(&"fft".to_string()), "{slots:?}");
        assert_eq!(app.focus, "levels", "focus follows the Widget");
        // click the ▸ of the loudness Pane: levels is taken, so scope
        render(&mut app);
        let r = app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Swap(s, 1) if s == "loudness").then_some(*r)).expect("a ◂ ▸ on loudness");
        app.mouse(ratatui::crossterm::event::MouseEvent { kind: ratatui::crossterm::event::MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left), column: r.x, row: r.y, modifiers: KeyModifiers::NONE });
        assert!(app.layout.slot_names().contains(&"scope".to_string()));
        // the well has no Widgets to swap: v changes its Disk look
        app.focus = "well".into();
        let look = app.hole.look;
        key(&mut app, KeyCode::Char('v'));
        assert_ne!(app.hole.look, look);
    }

    #[test]
    fn a_saved_layout_survives_a_restart_and_esc_puts_a_dragged_one_back() {
        let (mut app, dir) = with_dirs();
        let area = Rect::new(0, 1, 160, 43);
        let d = app.layout.compute(area).dividers[0].clone();
        app.layout.drag(&d, 41);
        app.swap_widget("fft", 1);
        let mine = app.layout.clone();
        app.run(crate::app::Action::SaveLayout);
        for c in "Wide library".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Enter);
        assert!(dir.path().join("layouts/wide-library.toml").exists());
        assert_eq!(app.layout_name.as_deref(), Some("wide-library"));
        // a restart
        let mut again = App::new(None);
        again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
        assert_eq!(again.layout, mine, "identical after a restart");
        assert_eq!(again.layout.compute(area), mine.compute(area));
        // in the picker, after the built-in designs
        again.run(crate::app::Action::PickDesign);
        let items = &again.picker.as_ref().unwrap().items;
        assert_eq!(items.last().unwrap().1, "Wide library");
        assert_eq!(again.picker.as_ref().unwrap().sel, items.len() - 1, "the one in use is selected");
        key(&mut again, KeyCode::Home);
        assert_eq!(again.layout, crate::layout::Node::singularity());
        key(&mut again, KeyCode::Esc);
        assert_eq!(again.layout, mine, "Esc puts the saved layout back");
    }

    #[test]
    fn a_saved_design_keeps_its_colours() {
        let (mut app, dir) = with_dirs();
        app.set_theme("terminal-amber");
        app.run(crate::app::Action::SaveLayout);
        for c in "Warm".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Enter);
        let text = std::fs::read_to_string(dir.path().join("layouts/warm.toml")).unwrap();
        assert!(text.contains("theme = \"terminal-amber\""), "{text}");
        // somewhere else, then pick it: the preview already has its colours
        let mut again = App::new(None);
        again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
        again.set_design(2);
        assert_eq!(again.theme.id, "classic");
        again.run(crate::app::Action::PickDesign);
        key(&mut again, KeyCode::End);
        assert_eq!(again.theme.id, "terminal-amber", "live preview");
        key(&mut again, KeyCode::Esc);
        assert_eq!(again.theme.id, "classic", "Esc puts the colours back");
        again.run(crate::app::Action::PickDesign);
        key(&mut again, KeyCode::End);
        key(&mut again, KeyCode::Enter);
        assert_eq!(again.theme.id, "terminal-amber");
        // saving over the name again takes the new colours
        again.set_theme("cockpit-ice");
        again.save_layout("Warm");
        assert_eq!(again.saved_layouts[0].theme.as_deref(), Some("cockpit-ice"));
        // an old file (no theme) keeps today's behaviour: the design's own colours
        let old: String = std::fs::read_to_string(dir.path().join("layouts/warm.toml")).unwrap().lines().filter(|l| !l.starts_with("theme")).collect::<Vec<_>>().join("\n");
        std::fs::write(dir.path().join("layouts/warm.toml"), old).unwrap();
        again.set_design(2);
        again.run(crate::app::Action::PickDesign);
        key(&mut again, KeyCode::End);
        assert_eq!(again.theme.id, DESIGNS[again.design].themes[0]);
    }

    #[test]
    fn transparency_hands_the_background_to_the_terminal() {
        let mut app = App::new(None);
        app.config.ui.transparent = true;
        let b = render(&mut app);
        assert!((0..48).all(|y| (0..160).all(|x| b[(x, y)].bg != Color::Rgb(0, 0, 0))));
        assert!((0..48).any(|y| (0..160).any(|x| b[(x, y)].bg == Color::Reset)));
    }

    #[test]
    fn the_theme_picker_lists_built_ins_then_the_users() {
        let user = theme::parse_file("rose", theme::EXAMPLE_THEME).unwrap();
        let items = theme_items(&[user]);
        assert_eq!(items[0].0, "singularity");
        assert_eq!(items.last().unwrap().2, "themes/rose.toml");
    }
}
