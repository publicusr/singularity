//! Designs and themes in the running Client (#18): switching, `t` for the next palette, the user's theme files
//! (re-read when they change), and the pickers, which preview as you move and put things back on Esc.

use crate::app::{App, Hit};
use crate::design::DESIGNS;
use crate::theme::{self, Theme};
use crate::ui::{centered, popup, text};
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
    /// what to go back to on Esc: design and theme id
    before: (usize, String),
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
        self.theme = theme::builtin(DESIGNS[d].themes[0]).expect("built in");
        self.want_spectrum();
    }

    /// Use theme `id` (built-in or the user's); false if there's no such theme.
    pub fn set_theme(&mut self, id: &str) -> bool {
        let t = self.user_themes.iter().find(|t| t.id == id).cloned().or_else(|| theme::builtin(id));
        match t {
            Some(t) => {
                self.theme = t;
                true
            }
            None => false,
        }
    }

    /// `ui.design` and `ui.theme` from the config, if they differ from what's on screen.
    pub fn apply_ui(&mut self, ui: &singularity_config::Ui) {
        match crate::design::by_id(&ui.design) {
            Some(d) if d != self.design || self.theme.id.is_empty() => self.set_design(d),
            Some(_) => {}
            None => self.say(format!("config.toml: no design called {:?}", ui.design)),
        }
        let want = if ui.theme.is_empty() { DESIGNS[self.design].themes[0] } else { ui.theme.as_str() };
        if want != self.theme.id && !self.set_theme(want) {
            self.say(format!("config.toml: no theme called {want:?} (built-in, or a file in themes/)"));
        }
    }

    /// Remember the design and theme in config.toml (so they survive a restart).
    fn save_ui(&mut self) {
        let own = DESIGNS[self.design].themes[0] == self.theme.id;
        let (design, theme) = (DESIGNS[self.design].id.to_string(), if own { String::new() } else { self.theme.id.clone() });
        self.config.ui.design = design.clone();
        self.config.ui.theme = theme.clone();
        if let Some(path) = self.config_path() {
            let r = singularity_config::set(&path, singularity_config::DEFAULT_CONFIG, "ui.design", design.as_str().into())
                .and_then(|_| singularity_config::set(&path, singularity_config::DEFAULT_CONFIG, "ui.theme", theme.as_str().into()));
            if let Err(e) = r {
                self.say(format!("Couldn't save to config.toml: {e}"));
            }
        }
    }

    /// `t`: the design's next palette, then the user's themes, round again.
    pub fn next_variant(&mut self) {
        let mut ids: Vec<String> = DESIGNS[self.design].themes.iter().map(|s| s.to_string()).collect();
        ids.extend(self.user_themes.iter().map(|t| t.id.clone()));
        if ids.len() < 2 {
            return self.say(format!("{} has one palette · Ctrl+K → Theme for the others", DESIGNS[self.design].name));
        }
        let i = ids.iter().position(|id| *id == self.theme.id).map(|i| (i + 1) % ids.len()).unwrap_or(0);
        self.set_theme(&ids[i]);
        let name = self.theme.name.clone();
        self.save_ui();
        self.say(format!("Palette: {name}"));
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
            self.theme = t;
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
        let items = match kind {
            PickerKind::Design => DESIGNS.iter().map(|d| (d.id.to_string(), d.name.to_string(), d.blurb.to_string())).collect(),
            PickerKind::Theme => theme_items(&self.user_themes),
        };
        let current = match kind {
            PickerKind::Design => DESIGNS[self.design].id.to_string(),
            PickerKind::Theme => self.theme.id.clone(),
        };
        let sel = items.iter().position(|(id, ..)| *id == current).unwrap_or(0);
        self.picker = Some(Picker { kind, items, sel, before: (self.design, self.theme.id.clone()) });
    }

    /// Show item `i` (the preview).
    pub fn picker_select(&mut self, i: usize) {
        let Some(p) = self.picker.as_mut() else { return };
        p.sel = i.min(p.items.len().saturating_sub(1));
        let (kind, id) = (p.kind, p.items[p.sel].0.clone());
        match kind {
            PickerKind::Design => {
                if let Some(d) = crate::design::by_id(&id).filter(|d| *d != self.design) {
                    self.set_design(d);
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
    for (i, (_, name, what)) in items.iter().enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.bottom().saturating_sub(2) {
            break;
        }
        let bg = if i == sel { th.selection.color() } else { th.bg.color() };
        text(buf, inner.x, y, inner.width, &" ".repeat(inner.width as usize), th.base().bg(bg));
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
        for d in 0..DESIGNS.len() {
            app.set_design(d);
            app.meters.push(singularity_proto::SpectrumFrame { bands: vec![120; 64], level: 150, vu: [150, 150], beat: false });
            let s = screen(&render(&mut app));
            for (slot, _) in app.layout.compute(Rect::new(0, 1, 160, 43)).slots {
                let usual = match slot.as_str() {
                    "well" => "gravity well",
                    "hole" => "black hole",
                    "nowplaying" => "Now playing",
                    "scope" => "osc",
                    "fft" => "fft",
                    other => Box::leak(other.to_string().into_boxed_str()),
                };
                let title = crate::design::title(d, &slot, usual);
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
        app.meters.push(singularity_proto::SpectrumFrame { bands: vec![200; 64], level: 200, vu: [200, 200], beat: false });
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
