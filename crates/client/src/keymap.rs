//! Keys (#20, D19): the standard keymap, a vim preset, and the user's `keys.toml` on top. Problems (an unknown action,
//! a key that can't be read, one key bound twice) are reported, never fatal.

use crate::app::Action;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// One key with its modifiers, normalised: a character carries its own case (no Shift), Ctrl+letter is lowercase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Chord {
    pub fn of(k: KeyEvent) -> Chord {
        let mut mods = k.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let code = match k.code {
            KeyCode::Char(c) => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::Char(if mods.contains(KeyModifiers::CONTROL) { c.to_ascii_lowercase() } else { c })
            }
            KeyCode::BackTab => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            c => c,
        };
        Chord { code, mods }
    }

    /// `"Ctrl+K"`, `"Shift+Right"`, `"l"`, `"Space"`, `"F10"`…
    pub fn parse(s: &str) -> Option<Chord> {
        let s = s.trim();
        // a lone "+" is the plus key, not a separator
        let (mods_part, key) = match s.rsplit_once('+') {
            Some((m, "")) if !m.is_empty() => (m.strip_suffix('+')?, "+"),
            Some((m, k)) if !m.is_empty() => (m, k),
            _ => ("", s),
        };
        let mut mods = KeyModifiers::NONE;
        for m in mods_part.split('+').filter(|m| !m.is_empty()) {
            mods |= match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "c" => KeyModifiers::CONTROL,
                "alt" | "meta" | "m" => KeyModifiers::ALT,
                "shift" | "s" => KeyModifiers::SHIFT,
                _ => return None,
            };
        }
        let code = match key.to_ascii_lowercase().as_str() {
            "space" => KeyCode::Char(' '),
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" if mods.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "insert" | "ins" => KeyCode::Insert,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "menu" => KeyCode::Menu,
            f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => KeyCode::F(f[1..].parse().ok()?),
            _ => {
                let mut cs = key.chars();
                let c = cs.next()?;
                if cs.next().is_some() {
                    return None;
                }
                // Shift+l is L
                KeyCode::Char(if mods.contains(KeyModifiers::SHIFT) { c.to_ascii_uppercase() } else { c })
            }
        };
        Some(Chord::of(KeyEvent::new(code, mods)))
    }

    /// Short, for hints and the palette: `^K`, `⇧→`, `Alt+←`, `Space`.
    pub fn short(&self) -> String {
        prefix(self.mods) + &self.key_name()
    }

    fn key_name(&self) -> String {
        match self.code {
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(c) if self.mods.contains(KeyModifiers::CONTROL) => c.to_ascii_uppercase().to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Left => "←".into(),
            KeyCode::Right => "→".into(),
            KeyCode::Up => "↑".into(),
            KeyCode::Down => "↓".into(),
            KeyCode::Enter => "⏎".into(),
            KeyCode::Esc => "Esc".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::BackTab => "⇧Tab".into(),
            KeyCode::Backspace => "Backspace".into(),
            KeyCode::Delete => "Del".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::PageUp => "PgUp".into(),
            KeyCode::PageDown => "PgDn".into(),
            KeyCode::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        }
    }
}

fn prefix(mods: KeyModifiers) -> String {
    let mut s = String::new();
    if mods.contains(KeyModifiers::CONTROL) {
        s.push('^');
    }
    if mods.contains(KeyModifiers::ALT) {
        s.push_str("Alt+");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        s.push('⇧');
    }
    s
}

/// The standard keymap: what each action starts with (the first key is the one shown).
const STANDARD: &[(Action, &[&str])] = &[
    (Action::TogglePlay, &["Space"]),
    (Action::Next, &["n"]),
    (Action::Previous, &["b"]),
    (Action::SeekForward, &["Shift+Right", "Right"]),
    (Action::SeekBack, &["Shift+Left", "Left"]),
    (Action::SeekForwardFine, &["."]),
    (Action::SeekBackFine, &[","]),
    (Action::VolumeUp, &["+", "="]),
    (Action::VolumeDown, &["-"]),
    (Action::Shuffle, &["s"]),
    (Action::Repeat, &["r"]),
    (Action::Like, &["l"]),
    (Action::Back, &["Alt+Left", "Backspace"]),
    (Action::Forward, &["Alt+Right"]),
    (Action::Search, &["Ctrl+L"]),
    (Action::Filter, &["/", "Ctrl+F"]),
    (Action::NextLook, &["v"]),
    (Action::NextVariant, &["t"]),
    (Action::ToggleIdle, &["I"]),
    (Action::Devices, &["d"]),
    (Action::FocusNext, &["Tab"]),
    (Action::MovePane, &["m"]),
    (Action::Palette, &["Ctrl+K", ":"]),
    (Action::Help, &["?"]),
    (Action::Quit, &["q", "Esc"]),
];

/// The vim preset: the standard keys, with like on `f` (h/j/k/l move), plus moving around with h/j/k/l, g/G, Ctrl+D/U.
const VIM_CHANGES: &[(Action, &[&str])] = &[(Action::Like, &["f"])];
const VIM_NAV: &[(&str, KeyCode)] = &[("h", KeyCode::Left), ("j", KeyCode::Down), ("k", KeyCode::Up), ("l", KeyCode::Right), ("g", KeyCode::Home), ("G", KeyCode::End), ("Ctrl+D", KeyCode::PageDown), ("Ctrl+U", KeyCode::PageUp)];

pub struct Keymap {
    pub preset: String,
    /// action → keys, the first one shown
    keys: Vec<(Action, Vec<Chord>)>,
    /// keys that stand for a movement key (vim: j is ↓)
    nav: Vec<(Chord, KeyCode)>,
    pub problems: Vec<String>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::from_keys(&singularity_config::Keys::default())
    }
}

fn chords(keys: &[&str]) -> Vec<Chord> {
    keys.iter().map(|k| Chord::parse(k).expect("built-in keys parse")).collect()
}

impl Keymap {
    /// The preset, then the user's bindings on top.
    pub fn from_keys(k: &singularity_config::Keys) -> Keymap {
        let mut problems = vec![];
        let vim = match k.preset.as_str() {
            "standard" | "" => false,
            "vim" => true,
            other => {
                problems.push(format!("keys.toml: unknown preset {other:?} (\"standard\" or \"vim\")"));
                false
            }
        };
        let mut keys: Vec<(Action, Vec<Chord>)> = STANDARD.iter().map(|(a, ks)| (*a, chords(ks))).collect();
        let mut nav = vec![];
        if vim {
            for (a, ks) in VIM_CHANGES {
                if let Some(e) = keys.iter_mut().find(|(x, _)| x == a) {
                    e.1 = chords(ks);
                }
            }
            nav = VIM_NAV.iter().map(|(k, c)| (Chord::parse(k).expect("built-in keys parse"), *c)).collect();
        }
        // the user's bindings: each replaces that action's keys, and takes its keys away from everything else
        let mut taken: Vec<(Chord, Action)> = vec![];
        for (name, v) in &k.bindings {
            let Some(action) = Action::from_id(name) else {
                problems.push(format!("keys.toml: no action called {name:?}"));
                continue;
            };
            let mut mine = vec![];
            for s in v.keys() {
                let Some(c) = Chord::parse(&s) else {
                    problems.push(format!("keys.toml: can't read the key {s:?} (for {name})"));
                    continue;
                };
                if let Some((_, other)) = taken.iter().find(|(t, a)| *t == c && *a != action) {
                    problems.push(format!("keys.toml: {} is bound to both {} and {name}; {} keeps it", c.short(), other.id(), other.id()));
                    continue;
                }
                taken.push((c, action));
                mine.push(c);
            }
            // nothing readable: the action keeps its keys (an explicit empty list unbinds it)
            if mine.is_empty() && !v.keys().is_empty() {
                continue;
            }
            for (_, ks) in keys.iter_mut() {
                ks.retain(|c| !mine.contains(c));
            }
            nav.retain(|(c, _)| !mine.contains(c));
            match keys.iter_mut().find(|(a, _)| *a == action) {
                Some(e) => e.1 = mine,
                None => keys.push((action, mine)),
            }
        }
        Keymap { preset: if vim { "vim".into() } else { "standard".into() }, keys, nav, problems }
    }

    /// The action a key press means, if any.
    pub fn action(&self, k: KeyEvent) -> Option<Action> {
        let c = Chord::of(k);
        self.keys.iter().find(|(_, ks)| ks.contains(&c)).map(|(a, _)| *a)
    }

    /// The movement key a key stands for (vim's j → ↓), if any.
    pub fn nav(&self, k: KeyEvent) -> Option<KeyCode> {
        let c = Chord::of(k);
        self.nav.iter().find(|(n, _)| *n == c).map(|(_, code)| *code)
    }

    pub fn keys_of(&self, a: Action) -> &[Chord] {
        self.keys.iter().find(|(x, _)| *x == a).map(|(_, k)| k.as_slice()).unwrap_or(&[])
    }

    /// The key shown for an action (its first), or "".
    pub fn label(&self, a: Action) -> String {
        self.keys_of(a).first().map(Chord::short).unwrap_or_default()
    }

    /// Two actions' keys as one label: `⇧←→` when they share modifiers, else `b/n`.
    pub fn pair(&self, a: Action, b: Action) -> String {
        let (x, y) = (self.keys_of(a).first(), self.keys_of(b).first());
        match (x, y) {
            (Some(x), Some(y)) if x.mods == y.mods && !x.mods.is_empty() => format!("{}{}{}", prefix(x.mods), x.key_name(), y.key_name()),
            (Some(x), Some(y)) => format!("{}/{}", x.short(), y.short()),
            (Some(x), None) => x.short(),
            (None, Some(y)) => y.short(),
            (None, None) => String::new(),
        }
    }

    /// What moves around (for the help screen).
    pub fn nav_label(&self) -> &'static str {
        if self.nav.is_empty() { "↑↓←→ ⏎" } else { "hjkl ↑↓ ⏎" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_config::{Keys, OneOrMany};

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn keys_read_and_print() {
        for (s, short) in [("Ctrl+K", "^K"), ("Shift+Right", "⇧→"), ("Alt+Left", "Alt+←"), ("Space", "Space"), ("l", "l"), ("Shift+l", "L"), ("F10", "F10"), ("+", "+"), ("Ctrl++", "^+"), ("Shift+Tab", "⇧Tab")] {
            assert_eq!(Chord::parse(s).map(|c| c.short()).as_deref(), Some(short), "{s}");
        }
        for bad in ["", "Hyper+x", "ab", "F99"] {
            assert_eq!(Chord::parse(bad), None, "{bad}");
        }
        // what the terminal sends matches what the file says
        assert_eq!(Chord::of(ev(KeyCode::Char('L'), KeyModifiers::SHIFT)), Chord::parse("L").unwrap());
        assert_eq!(Chord::of(ev(KeyCode::Char('k'), KeyModifiers::CONTROL)), Chord::parse("ctrl+k").unwrap());
    }

    #[test]
    fn the_standard_map_is_what_the_help_shows() {
        let m = Keymap::default();
        assert_eq!(m.action(ev(KeyCode::Char(' '), KeyModifiers::NONE)), Some(Action::TogglePlay));
        assert_eq!(m.action(ev(KeyCode::Right, KeyModifiers::SHIFT)), Some(Action::SeekForward));
        assert_eq!(m.action(ev(KeyCode::Char('l'), KeyModifiers::NONE)), Some(Action::Like));
        assert_eq!(m.label(Action::Palette), "^K");
        assert_eq!(m.pair(Action::SeekBack, Action::SeekForward), "⇧←→");
        assert_eq!(m.pair(Action::Next, Action::Previous), "n/b");
        assert!(m.problems.is_empty() && m.nav(ev(KeyCode::Char('j'), KeyModifiers::NONE)).is_none());
    }

    #[test]
    fn vim_moves_with_hjkl_and_likes_with_f() {
        let m = Keymap::from_keys(&Keys { preset: "vim".into(), ..Default::default() });
        assert_eq!(m.nav(ev(KeyCode::Char('j'), KeyModifiers::NONE)), Some(KeyCode::Down));
        assert_eq!(m.nav(ev(KeyCode::Char('G'), KeyModifiers::SHIFT)), Some(KeyCode::End));
        assert_eq!(m.nav(ev(KeyCode::Char('d'), KeyModifiers::CONTROL)), Some(KeyCode::PageDown));
        assert_eq!(m.action(ev(KeyCode::Char('f'), KeyModifiers::NONE)), Some(Action::Like));
        assert_eq!(m.action(ev(KeyCode::Char('l'), KeyModifiers::NONE)), None, "l moves in vim");
    }

    #[test]
    fn user_bindings_take_keys_over_and_conflicts_are_reported() {
        let mut k = Keys::default();
        k.bindings.insert("like".into(), OneOrMany::One("s".into()));
        k.bindings.insert("shuffle".into(), OneOrMany::Many(vec!["s".into(), "Ctrl+S".into()]));
        k.bindings.insert("teleport".into(), OneOrMany::One("t".into()));
        k.bindings.insert("next".into(), OneOrMany::One("Hyper+n".into()));
        let m = Keymap::from_keys(&k);
        // (bindings are read in name order: "like" comes first and keeps s)
        assert_eq!(m.action(ev(KeyCode::Char('s'), KeyModifiers::NONE)), Some(Action::Like));
        assert_eq!(m.action(ev(KeyCode::Char('l'), KeyModifiers::NONE)), None, "like's old key is gone");
        assert_eq!(m.action(ev(KeyCode::Char('s'), KeyModifiers::CONTROL)), Some(Action::Shuffle));
        assert_eq!(m.problems.len(), 3, "{:?}", m.problems);
        assert!(m.problems.iter().any(|p| p.contains("s is bound to both like and shuffle")));
        assert!(m.problems.iter().any(|p| p.contains("teleport")));
        assert!(m.problems.iter().any(|p| p.contains("Hyper+n")));
        assert_eq!(m.label(Action::Next), "n", "an unreadable key is reported and the old one stays");
        let bad = Keymap::from_keys(&Keys { preset: "emacs".into(), ..Default::default() });
        assert!(bad.problems[0].contains("emacs") && bad.label(Action::Like) == "l");
    }
}
