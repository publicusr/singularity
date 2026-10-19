//! The Settings page (#57, D33): `config.toml` as a page in the main Pane. One row per setting with its value and
//! the comment above it in the file as its description. A change goes through `singularity_config::set` (the file
//! keeps its comments) and applies at once; the file stays the one source of truth (story 59), so an edit made in
//! the file shows here within a second. On/off flips (Enter, Space, a click); lists and numbers step with ←/→ or the
//! ◂ ▸ arrows; Enter on a number types one.
//! A second tab (#58) lists every action with its keys as they are now (preset + `keys.toml`), read-only: rebinding
//! happens in the file for now (D33).

use crate::app::{Action, App, Hit};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_config::toml_edit::Value;
use singularity_config::{Entry, Kind};

/// Where the value column starts, after the setting's name.
const NAME_W: u16 = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tab {
    #[default]
    Config,
    Keys,
}

#[derive(Clone, Debug, Default)]
pub struct SettingsView {
    pub tab: Tab,
    /// the highlight is on the tab row (↑ from the first row; ←/→ there switch tabs)
    pub on_tabs: bool,
    /// keys tab: selected action (index into `Action::ALL`) and first visible row
    pub key_sel: usize,
    pub key_top: usize,
    /// selected setting (index into `singularity_config::SETTINGS`)
    pub sel: usize,
    /// first visible line
    pub top: usize,
    pub visible: usize,
    /// a number being typed, for the selected setting
    pub typing: Option<String>,
}

/// One line of the page.
#[derive(Clone, Debug, PartialEq)]
enum Line {
    Section(&'static str),
    Setting(usize),
    Desc(String),
    Blank,
}

/// The rows, from `config.toml` as it is now.
pub fn entries(app: &App) -> Vec<Entry> {
    singularity_config::entries(&app.config_text)
}

/// A list setting's choices: (what the file says, what the page shows).
fn choices(app: &App, e: &Entry) -> Vec<(String, String)> {
    let own = || (String::new(), "the design's own".to_string());
    match (e.kind, e.key) {
        (Kind::Choice(list), _) => list.iter().map(|s| (s.to_string(), s.to_string())).collect(),
        (Kind::Named, "ui.design") => crate::design::DESIGNS.iter().map(|d| (d.id.to_string(), d.id.to_string())).collect(),
        (Kind::Named, "ui.theme") => {
            let mut v = vec![own()];
            v.extend(crate::theme::builtin_ids().into_iter().map(|t| (t.to_string(), t.to_string())));
            v.extend(app.user_themes.iter().map(|t| (t.id.clone(), t.id.clone())));
            v
        }
        (Kind::Named, "ui.layout") => {
            let mut v = vec![own()];
            v.extend(app.saved_layouts.iter().map(|l| (l.id.clone(), l.id.clone())));
            v
        }
        _ => vec![],
    }
}

/// How a value reads on the page.
fn shown(app: &App, e: &Entry) -> String {
    match (&e.kind, &e.value) {
        (Kind::Toggle, v) => if v.as_bool() == Some(true) { "on" } else { "off" }.to_string(),
        (Kind::Named, Value::String(s)) if s.value().is_empty() => choices(app, e).first().map(|c| c.1.clone()).unwrap_or_default(),
        (_, Value::String(s)) => s.value().clone(),
        (_, v) => number(v).map(fmt_number).unwrap_or_else(|| v.to_string()),
    }
}

fn number(v: &Value) -> Option<f64> {
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
}

fn fmt_number(n: f64) -> String {
    if n.fract() == 0.0 { format!("{n:.0}") } else { format!("{n}") }
}

fn number_value(n: f64, whole: bool) -> Value {
    if whole || n.fract() == 0.0 { Value::from(n.round() as i64) } else { Value::from(n) }
}

fn lines(es: &[Entry], w: usize) -> Vec<Line> {
    let mut out = vec![];
    let mut section = "";
    for (i, e) in es.iter().enumerate() {
        if e.section() != section {
            if !out.is_empty() {
                out.push(Line::Blank);
            }
            section = e.section();
            out.push(Line::Section(section));
        }
        out.push(Line::Setting(i));
        // the description under the setting, at most two lines
        let d = crate::lyrics::wrap(&e.description, w.saturating_sub(4).max(8));
        let n = d.len();
        for (k, l) in d.into_iter().take(2).enumerate() {
            out.push(Line::Desc(if k == 1 && n > 2 { format!("{}…", l.trim_end()) } else { l }));
        }
    }
    out
}

impl App {
    /// Read `config.toml`'s text again (after a change, by us or in the file).
    pub fn read_config_text(&mut self) {
        if let Some(p) = self.config_path() {
            self.config_text = std::fs::read_to_string(p).unwrap_or_default();
        }
    }

    /// Write one setting to `config.toml` and apply the file at once.
    fn settings_write(&mut self, key: &str, v: Value) {
        let Some(path) = self.config_path() else { return self.say("There's no config.toml to change") };
        if let Err(e) = singularity_config::set(&path, singularity_config::DEFAULT_CONFIG, key, v) {
            return self.say(format!("Couldn't change config.toml: {e}"));
        }
        self.config_written();
        self.apply_config(singularity_config::load_config(&path));
        self.read_config_text();
    }

    /// ←/→ (or ◂ ▸) on setting `i`: the next or previous choice, a step of a number; on/off flips.
    pub fn settings_step(&mut self, i: usize, d: i64) {
        let Some(e) = entries(self).into_iter().nth(i) else { return };
        let v = match e.kind {
            Kind::Toggle => Value::from(e.value.as_bool() != Some(true)),
            Kind::Choice(_) | Kind::Named => {
                let cs = choices(self, &e);
                if cs.is_empty() {
                    return;
                }
                let at = cs.iter().position(|c| Some(c.0.as_str()) == e.value.as_str()).unwrap_or(0) as i64;
                Value::from(cs[(at + d).rem_euclid(cs.len() as i64) as usize].0.as_str())
            }
            Kind::Number { min, max, step, whole } => {
                let n = number(&e.value).unwrap_or(min);
                // back onto the step grid, then one step, inside the range
                let n = ((n / step).round() + d as f64) * step;
                number_value(n.clamp(min, max), whole)
            }
        };
        self.settings_write(e.key, v);
    }

    /// Enter, Space or a click on setting `i`: on/off flips, a list steps on, a number starts being typed.
    pub fn settings_activate(&mut self, i: usize) {
        let Some(e) = entries(self).into_iter().nth(i) else { return };
        match e.kind {
            Kind::Number { .. } => {
                if let Some(v) = self.settings_view.as_mut() {
                    v.sel = i;
                    v.typing = Some(String::new());
                }
            }
            _ => self.settings_step(i, 1),
        }
    }

    /// A typed number: saved if it's in range, refused with a toast if not.
    fn settings_save_typed(&mut self, i: usize, typed: &str) {
        let Some(e) = entries(self).into_iter().nth(i) else { return };
        let Kind::Number { min, max, whole, .. } = e.kind else { return };
        match typed.trim().replace(',', ".").parse::<f64>() {
            Ok(n) if (min..=max).contains(&n) && (!whole || n.fract() == 0.0) => self.settings_write(e.key, number_value(n, whole)),
            _ => {
                let what = if whole { "a whole number" } else { "a number" };
                self.say(format!("{} takes {what} from {} to {}", e.name(), fmt_number(min), fmt_number(max)));
            }
        }
    }

    /// Typing goes to the number being edited.
    pub fn settings_typing(&self) -> bool {
        self.settings_view.as_ref().is_some_and(|v| v.typing.is_some())
    }

    pub fn settings_key(&mut self, k: KeyEvent) -> bool {
        let Some(v) = self.settings_view.clone() else { return false };
        if let Some(mut typed) = v.typing {
            match k.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' || c == ',' => typed.push(c),
                KeyCode::Backspace => {
                    typed.pop();
                }
                KeyCode::Enter => {
                    if let Some(v) = self.settings_view.as_mut() {
                        v.typing = None;
                    }
                    self.settings_save_typed(v.sel, &typed);
                    return true;
                }
                KeyCode::Esc => {
                    if let Some(v) = self.settings_view.as_mut() {
                        v.typing = None;
                    }
                    return true;
                }
                _ => {}
            }
            if let Some(v) = self.settings_view.as_mut() {
                v.typing = Some(typed);
            }
            return true;
        }
        let other = if v.tab == Tab::Config { Tab::Keys } else { Tab::Config };
        if v.on_tabs || v.tab == Tab::Keys {
            match k.code {
                KeyCode::Left | KeyCode::Right => {
                    self.settings_tab(other);
                    return true;
                }
                KeyCode::Down if v.on_tabs => {
                    if let Some(v) = self.settings_view.as_mut() {
                        v.on_tabs = false;
                    }
                    return true;
                }
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Up if v.on_tabs => return true,
                _ => {}
            }
        }
        if v.tab == Tab::Keys {
            let n = Action::ALL.len();
            let step = |d: i64| (v.key_sel as i64 + d).clamp(0, n as i64 - 1) as usize;
            let sel = match k.code {
                KeyCode::Up if v.key_sel == 0 => return self.settings_to_tabs(),
                KeyCode::Up => step(-1),
                KeyCode::Down => step(1),
                KeyCode::PageUp => step(-(v.visible.max(1) as i64)),
                KeyCode::PageDown => step(v.visible.max(1) as i64),
                KeyCode::Home => 0,
                KeyCode::End => n - 1,
                _ => return false,
            };
            if let Some(v) = self.settings_view.as_mut() {
                v.key_sel = sel;
            }
            return true;
        }
        let n = singularity_config::SETTINGS.len();
        let step = |d: i64| (v.sel as i64 + d).clamp(0, n.saturating_sub(1) as i64) as usize;
        let sel = match k.code {
            KeyCode::Up if v.sel == 0 => return self.settings_to_tabs(),
            KeyCode::Up => step(-1),
            KeyCode::Down => step(1),
            KeyCode::PageUp => step(-(v.visible.max(1) as i64)),
            KeyCode::PageDown => step(v.visible.max(1) as i64),
            KeyCode::Home => 0,
            KeyCode::End => n.saturating_sub(1),
            KeyCode::Left | KeyCode::Right => {
                self.settings_step(v.sel, if k.code == KeyCode::Left { -1 } else { 1 });
                return true;
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.settings_activate(v.sel);
                return true;
            }
            _ => return false,
        };
        if let Some(v) = self.settings_view.as_mut() {
            v.sel = sel;
        }
        true
    }

    fn settings_to_tabs(&mut self) -> bool {
        if let Some(v) = self.settings_view.as_mut() {
            v.on_tabs = true;
        }
        true
    }

    /// Show tab `t` (the highlight stays on the tab row if it was there).
    pub fn settings_tab(&mut self, t: Tab) {
        if let Some(v) = self.settings_view.as_mut() {
            v.tab = t;
            v.typing = None;
        }
    }

    /// The wheel over the page moves the highlight (Shift: a page).
    pub fn settings_wheel(&mut self, d: i64, coarse: bool) {
        let Some(v) = self.settings_view.as_mut() else { return };
        v.on_tabs = false;
        let step = if coarse { v.visible.max(1) as i64 } else { 1 };
        let (sel, n) = match v.tab {
            Tab::Config => (&mut v.sel, singularity_config::SETTINGS.len()),
            Tab::Keys => (&mut v.key_sel, Action::ALL.len()),
        };
        *sel = (*sel as i64 + d * step).clamp(0, n.saturating_sub(1) as i64) as usize;
    }
}

/// The first line to show: the selected setting and its description in view (its section title too, at the top),
/// never starting with the tail of a description.
fn scrolled(all: &[Line], sel: usize, top: usize, visible: usize) -> usize {
    let mut top = top;
    let mut end = 0;
    if let Some(at) = all.iter().position(|l| *l == Line::Setting(sel)) {
        end = at + all[at + 1..].iter().take_while(|l| matches!(l, Line::Desc(_))).count();
        if at < top + 1 {
            top = at.saturating_sub(1);
        } else if end >= top + visible {
            top = end + 1 - visible;
        }
    }
    top = top.min(all.len().saturating_sub(visible));
    if matches!(all.get(top), Some(Line::Desc(_))) {
        // back to its setting if the selection still fits, else past the description
        let owner = (0..top).rev().find(|&k| matches!(all[k], Line::Setting(_))).unwrap_or(0);
        top = if end < owner + visible { owner } else { (top..all.len()).find(|&k| !matches!(all[k], Line::Desc(_))).unwrap_or(top) };
    }
    top
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let Some(mut view) = app.settings_view.clone() else { return };
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    if w < 24 || r.height < 3 {
        return;
    }
    let mut hits: Vec<(Rect, Hit)> = vec![];
    let mut y = r.y;
    // the tabs (one per file), the file's path, and what's wrong with it
    let focused = app.focus == "home";
    let mut tx = x;
    for (t, label) in [(Tab::Config, " config.toml "), (Tab::Keys, " keys.toml ")] {
        let n = label.chars().count() as u16;
        let st = if t != view.tab { th.s(th.dim) } else if focused && view.on_tabs { th.chip(th.accent).add_modifier(Modifier::UNDERLINED) } else { th.chip(th.accent) };
        text(buf, tx, y, n, label, st);
        hits.push((Rect::new(tx, y, n, 1), Hit::SettingsTab(t)));
        tx += n + 1;
    }
    let home = |p: std::path::PathBuf| p.display().to_string().replace(&std::env::var("HOME").unwrap_or_default(), "~");
    let (path, problems) = match view.tab {
        Tab::Config => (app.config_path().map(home).map(|p| format!("{p} · changes apply at once, and so do edits in the file")), &app.config_problems),
        Tab::Keys => (app.keys_path().map(home).map(|p| format!("{p} · {} preset · change keys there: action = \"key\"", app.keymap.preset)), &app.keys_problems),
    };
    text(buf, tx + 1, y, (x + w).saturating_sub(tx + 1), &path.unwrap_or_default(), th.s(th.border));
    y += 1;
    let problems = problems.clone();
    if view.tab == Tab::Keys {
        y += 1;
        draw_keys(buf, Rect::new(x, y, w, r.bottom().saturating_sub(y)), app, th, &mut view, &problems, &mut hits);
        for (rect, h) in hits {
            app.hit(rect, h);
        }
        app.settings_view = Some(view);
        return;
    }
    for p in problems.iter().take(3) {
        y += 1;
        text(buf, x, y, w, &format!("! {p}"), th.s(th.error));
    }
    y += 1;
    let es = entries(app);
    let all = lines(&es, w.saturating_sub(NAME_W) as usize);
    view.visible = r.bottom().saturating_sub(y).max(1) as usize;
    view.sel = view.sel.min(es.len().saturating_sub(1));
    view.top = scrolled(&all, view.sel, view.top, view.visible);
    let vx = x + NAME_W;
    let vw = w.saturating_sub(NAME_W);
    for line in all.iter().skip(view.top).take(view.visible) {
        match line {
            Line::Section(s) => text(buf, x, y, w, &format!("[{s}]"), th.s(th.dim).add_modifier(Modifier::BOLD)),
            Line::Blank => {}
            Line::Desc(d) => text(buf, vx, y, vw, d, th.s(th.dim)),
            Line::Setting(i) => {
                let e = &es[*i];
                let selected = focused && !view.on_tabs && *i == view.sel;
                let bg = if selected { th.selection.color() } else { th.bg.color() };
                text(buf, x, y, w, &" ".repeat(w as usize), th.base().bg(bg));
                text(buf, x + 1, y, NAME_W - 1, e.name(), th.s(th.fg).bg(bg));
                hits.push((Rect::new(x, y, w, 1), Hit::Setting(*i)));
                let typing = view.typing.as_ref().filter(|_| *i == view.sel);
                match (e.kind, typing) {
                    (_, Some(t)) => {
                        let s = format!("{t}█");
                        text(buf, vx, y, vw, &s, th.s(th.accent).bg(bg));
                        if let Kind::Number { min, max, .. } = e.kind {
                            let hint = format!("   {}–{} · Enter saves · Esc", fmt_number(min), fmt_number(max));
                            text(buf, vx + s.chars().count() as u16, y, vw.saturating_sub(s.chars().count() as u16), &hint, th.s(th.dim).bg(bg));
                        }
                    }
                    (Kind::Toggle, None) => {
                        let on = e.value.as_bool() == Some(true);
                        let st = if on { th.chip(th.accent) } else { th.s(th.dim).bg(bg) };
                        text(buf, vx, y, 5, if on { " on  " } else { " off " }, st);
                    }
                    (_, None) => {
                        let s = shown(app, e);
                        let n = s.chars().count() as u16;
                        text(buf, vx, y, 2, "◂ ", th.s(th.dim).bg(bg));
                        text(buf, vx + 2, y, vw.saturating_sub(4), &s, th.s(th.accent).bg(bg).add_modifier(Modifier::BOLD));
                        let ax = (vx + 2 + n + 1).min(x + w - 1);
                        text(buf, ax, y, 1, "▸", th.s(th.dim).bg(bg));
                        hits.push((Rect::new(vx, y, 2, 1), Hit::SettingStep(*i, -1)));
                        hits.push((Rect::new(ax, y, 1, 1), Hit::SettingStep(*i, 1)));
                    }
                }
            }
        }
        y += 1;
    }
    for (rect, h) in hits {
        app.hit(rect, h);
    }
    app.settings_view = Some(view);
}

/// The keys tab: what's wrong with `keys.toml`, then every action with its keys and its name in the file.
fn draw_keys(buf: &mut Buffer, r: Rect, app: &App, th: &Theme, view: &mut SettingsView, problems: &[String], hits: &mut Vec<(Rect, Hit)>) {
    let (x, w) = (r.x, r.width);
    let mut y = r.y;
    for p in problems.iter().take(3) {
        text(buf, x, y, w, &format!("! {p}"), th.s(th.error));
        y += 1;
    }
    if !problems.is_empty() {
        y += 1;
    }
    let n = Action::ALL.len();
    view.visible = r.bottom().saturating_sub(y).max(1) as usize;
    view.key_sel = view.key_sel.min(n - 1);
    if view.key_sel < view.key_top {
        view.key_top = view.key_sel;
    } else if view.key_sel >= view.key_top + view.visible {
        view.key_top = view.key_sel + 1 - view.visible;
    }
    view.key_top = view.key_top.min(n.saturating_sub(view.visible));
    let focused = app.focus == "home";
    let keys_of = |a: Action| app.keymap.keys_of(a).iter().map(|c| c.short()).collect::<Vec<_>>().join("  ");
    // the keys column fits the longest keys, up to a third of the page
    let kw = (Action::ALL.iter().map(|a| keys_of(*a).chars().count()).max().unwrap_or(0) as u16 + 3).clamp(NAME_W, (w / 3).max(NAME_W));
    for (i, a) in Action::ALL.iter().enumerate().skip(view.key_top).take(view.visible) {
        let selected = focused && !view.on_tabs && i == view.key_sel;
        let bg = if selected { th.selection.color() } else { th.bg.color() };
        text(buf, x, y, w, &" ".repeat(w as usize), th.base().bg(bg));
        let keys = keys_of(*a);
        let (keys, kst) = if keys.is_empty() { ("—".to_string(), th.s(th.border)) } else { (keys, th.s(th.accent).add_modifier(Modifier::BOLD)) };
        text(buf, x + 1, y, kw - 2, &keys, kst.bg(bg));
        // the action's name in keys.toml at the right, if the label still fits beside it
        let (label, id) = (a.label(), a.id());
        let (lc, iw) = (label.chars().count() as u16, id.chars().count() as u16);
        let room = w.saturating_sub(kw);
        let show_id = room >= lc + iw + 3;
        text(buf, x + kw, y, if show_id { room - iw - 2 } else { room }, label, th.s(th.fg).bg(bg));
        if show_id {
            text(buf, x + w - iw - 1, y, iw, id, th.s(th.dim).bg(bg));
        }
        hits.push((Rect::new(x, y, w, 1), Hit::KeyRow(i)));
        y += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(app: &mut App, c: KeyCode) {
        app.key(KeyEvent::new(c, KeyModifiers::NONE));
    }

    fn app_with_files() -> (App, tempfile::TempDir) {
        // a short path, so the tab line fits it and what follows (macOS's temp dir is long)
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let mut app = App::new(None);
        app.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
        app.run(crate::app::Action::Settings);
        (app, dir)
    }

    fn select(app: &mut App, key: &str) {
        let i = singularity_config::SETTINGS.iter().position(|(k, _)| *k == key).unwrap();
        app.settings_view.as_mut().unwrap().sel = i;
    }

    #[test]
    fn edits_write_the_file_keeping_its_comments_and_apply_at_once() {
        let (mut app, dir) = app_with_files();
        let file = || std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        // on/off flips with Enter
        select(&mut app, "idle.enabled");
        key(&mut app, KeyCode::Enter);
        assert!(!app.config.idle.enabled);
        assert!(file().contains("enabled = false") && file().contains("# after this many minutes without input"), "{}", file());
        // a list steps with →, wrapping round
        select(&mut app, "visuals.gpu");
        key(&mut app, KeyCode::Right);
        assert_eq!(app.config.visuals.gpu, singularity_config::GpuPower::HighPerformance);
        key(&mut app, KeyCode::Right);
        assert_eq!(app.config.visuals.gpu, singularity_config::GpuPower::LowPower);
        // the design changes what's on screen
        select(&mut app, "ui.design");
        key(&mut app, KeyCode::Right);
        assert_eq!(app.config.ui.design, "event-horizon");
        assert_eq!(crate::design::DESIGNS[app.design].id, "event-horizon");
        // numbers step, stay in range, and can be typed
        select(&mut app, "visuals.unfocused_fps");
        key(&mut app, KeyCode::Left);
        assert_eq!(app.config.visuals.unfocused_fps, 9);
        key(&mut app, KeyCode::Enter);
        for c in "45".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.config.visuals.unfocused_fps, 9, "45 is out of range");
        assert!(app.toast.as_ref().is_some_and(|t| t.0.contains("1 to 30")));
        key(&mut app, KeyCode::Enter);
        for c in "24".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.config.visuals.unfocused_fps, 24);
        assert!(file().contains("unfocused_fps = 24"));
        // while typing, letters aren't shortcuts (q doesn't quit) and Esc only stops typing
        select(&mut app, "idle.minutes");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('q'));
        key(&mut app, KeyCode::Esc);
        assert!(!app.quit && app.settings_view.is_some());
        key(&mut app, KeyCode::Right);
        assert_eq!(app.config.idle.minutes, 3.5);
        assert!(file().contains("minutes = 3.5"));
        // Esc leaves the page
        key(&mut app, KeyCode::Esc);
        assert!(app.settings_view.is_none() && !app.quit);
    }

    #[test]
    fn the_page_never_starts_mid_description_and_keeps_the_selection_in_view() {
        let es = singularity_config::entries(singularity_config::DEFAULT_CONFIG);
        for w in [20, 40, 120] {
            let all = lines(&es, w);
            for visible in 3..all.len() + 2 {
                let mut top = 0;
                // down through every setting, then back up
                for sel in (0..es.len()).chain((0..es.len()).rev()) {
                    top = scrolled(&all, sel, top, visible);
                    assert!(!matches!(all.get(top), Some(Line::Desc(_))), "w {w} visible {visible} sel {sel}: starts mid-description");
                    let at = all.iter().position(|l| *l == Line::Setting(sel)).unwrap();
                    assert!(at >= top && at < top + visible, "w {w} visible {visible} sel {sel}: top {top}");
                }
            }
        }
    }

    #[test]
    fn the_keys_tab_lists_every_action_with_its_keys_and_follows_keys_toml() {
        let (mut app, dir) = app_with_files();
        let draw = |app: &mut App| {
            let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 48)).unwrap();
            t.draw(|f| crate::ui::draw(f, app)).unwrap();
            let b = t.backend().buffer().clone();
            (0..48).map(|y| (0..160).map(|x| b[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
        };
        // ↑ from the first setting reaches the tabs; → there shows the keys
        draw(&mut app);
        key(&mut app, KeyCode::Up);
        key(&mut app, KeyCode::Right);
        assert_eq!(app.settings_view.as_ref().unwrap().tab, Tab::Keys);
        let s = draw(&mut app);
        assert!(s.contains("standard preset") && s.contains("Space") && s.contains("Play / pause") && s.contains("play_pause"), "{s}");
        // every action has a row, reached by moving down (an unbound one shows —)
        key(&mut app, KeyCode::Down);
        for _ in 0..Action::ALL.len() {
            key(&mut app, KeyCode::Down);
            draw(&mut app);
        }
        assert_eq!(app.settings_view.as_ref().unwrap().key_sel, Action::ALL.len() - 1);
        assert!(draw(&mut app).contains("Quit (music keeps playing)"));
        // a binding and a mistake in keys.toml show (what the watcher does)
        let kp = dir.path().join("keys.toml");
        std::fs::write(&kp, "schema = 1\npreset = \"vim\"\n[bindings]\nstats = \"Ctrl+T\"\nnope = \"x\"\n").unwrap();
        app.apply_keys(singularity_config::load_keys(&kp));
        app.settings_view.as_mut().unwrap().key_sel = Action::ALL.iter().position(|a| *a == Action::Stats).unwrap();
        let s = draw(&mut app);
        assert!(s.contains("vim preset") && s.contains("^T") && s.contains("no action called \"nope\""), "{s}");
        // ← goes back to the config tab; Esc leaves the page
        key(&mut app, KeyCode::Left);
        assert_eq!(app.settings_view.as_ref().unwrap().tab, Tab::Config);
        key(&mut app, KeyCode::Esc);
        assert!(app.settings_view.is_none());
    }

    #[test]
    fn the_page_shows_the_file_and_follows_edits_made_in_it() {
        let (mut app, dir) = app_with_files();
        let path = dir.path().join("config.toml");
        let text = std::fs::read_to_string(&path).unwrap().replace("# let the terminal's own background (and its transparency) show instead of black\ntransparent = false", "# see-through, please\ntransparent = true").replace("unfocused_fps = 10", "unfocused_fps = \"fast\"");
        std::fs::write(&path, text).unwrap();
        // what the watcher does
        app.apply_config(singularity_config::load_config(&path));
        app.read_config_text();
        let e = entries(&app);
        let t = e.iter().find(|e| e.key == "ui.transparent").unwrap();
        assert_eq!((t.description.as_str(), t.value.as_bool()), ("see-through, please", Some(true)));
        assert!(app.config_problems.iter().any(|p| p.contains("config.toml")), "{:?}", app.config_problems);
        assert_eq!(shown(&app, e.iter().find(|e| e.key == "ui.theme").unwrap()), "the design's own");
    }
}
