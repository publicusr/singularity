//! Theme tokens (#18). Every Widget takes its colours from here, never hard-coded (spec: Design principles).
//! All backgrounds are pure black (OLED); `ui.transparent` lets the terminal's own background show through instead.
//!
//! Built-in themes are the designs' palettes and their variants; user themes are TOML files in
//! `~/.config/singularity/themes/`, optionally starting from a built-in (`base = "cockpit-ice"`).

use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(h: u32) -> Rgb {
        Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }
    pub fn color(self) -> Color {
        Color::Rgb(self.0, self.1, self.2)
    }
    pub fn scale(self, k: f32) -> Rgb {
        let f = |c: u8| (c as f32 * k).round().clamp(0.0, 255.0) as u8;
        Rgb(f(self.0), f(self.1), f(self.2))
    }
    pub fn lerp(self, o: Rgb, t: f32) -> Rgb {
        let f = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(f(self.0, o.0), f(self.1, o.1), f(self.2, o.2))
    }
    /// `#rrggbb` (or `rrggbb`)
    pub fn parse(s: &str) -> Option<Rgb> {
        let h = s.trim().trim_start_matches('#');
        (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok().map(Rgb::hex)).flatten()
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    /// `cockpit-ice`; a user theme's file name without `.toml`
    pub id: String,
    pub name: String,
    pub bg: Rgb,
    pub fg: Rgb,
    pub dim: Rgb,
    pub accent: Rgb,
    pub accent2: Rgb,
    pub border: Rgb,
    pub border_focus: Rgb,
    pub selection: Rgb,
    pub error: Rgb,
    /// low → high colour ramp for Visuals and meters (never tops out in pure white)
    pub ramp: Vec<Rgb>,
    pub borders: border::Set<'static>,
    /// per-Pane border colours, taken in turn (Cockpit's btop look); empty: all `border`
    pub boxes: Vec<Rgb>,
    /// Pane titles numbered ¹²³ (Cockpit)
    pub numbered: bool,
    /// every other row dimmed, like a CRT (Terminal)
    pub scanlines: bool,
}

impl Theme {
    /// The default design: terminal-driven, cream + ice on black.
    pub fn singularity() -> Theme {
        builtin("singularity").expect("built in")
    }

    pub fn s(&self, c: Rgb) -> Style {
        Style::default().fg(c.color())
    }
    pub fn base(&self) -> Style {
        Style::default().fg(self.fg.color()).bg(self.bg.color())
    }
    pub fn title(&self) -> Style {
        self.s(self.accent).add_modifier(Modifier::BOLD)
    }
    pub fn chip(&self, c: Rgb) -> Style {
        Style::default().fg(self.bg.color()).bg(c.color()).add_modifier(Modifier::BOLD)
    }
    /// The border colour of the `i`-th Pane.
    pub fn pane_border(&self, i: usize) -> Rgb {
        if self.boxes.is_empty() { self.border } else { self.boxes[i % self.boxes.len()] }
    }
}

const BLACK: Rgb = Rgb(0, 0, 0);

/// Cockpit's variants: name, ramp low→high, Pane borders, accent.
const COCKPIT: [(&str, &str, [u32; 3], [u32; 4], u32); 5] = [
    ("cockpit-btop", "Cockpit · btop", [0x77ca9b, 0xcbc06c, 0xdc4c4c], [0x556d59, 0x5c588d, 0x6c6c4b, 0x805252], 0xeeeeee),
    ("cockpit-ice", "Cockpit · ice", [0x1f4f7a, 0x5fb4ff, 0xbfe6ff], [0x1d3b57, 0x24486b, 0x1f4560, 0x2a4f73], 0xbfe6ff),
    ("cockpit-amber", "Cockpit · amber", [0x7a3d00, 0xffa031, 0xffe0a3], [0x5a3a14, 0x6b4416, 0x5e4214, 0x6e3f12], 0xffd28a),
    ("cockpit-violet", "Cockpit · violet", [0x3d2a7a, 0xa98bff, 0xf0d6ff], [0x3a2d63, 0x463572, 0x3b3166, 0x4b3478], 0xe2d4ff),
    ("cockpit-mono", "Cockpit · mono", [0x3a3a3a, 0x9a9a9a, 0xf0f0f0], [0x2e2e2e, 0x343434, 0x2e2e2e, 0x383838], 0xf0f0f0),
];

/// Terminal's variants: name, accent, foreground, border.
const TERMINAL: [(&str, &str, u32, u32, u32); 3] = [
    ("terminal-green", "Terminal · phosphor green", 0x33ff66, 0x7dff9a, 0x15401f),
    ("terminal-amber", "Terminal · amber CRT", 0xffb000, 0xffd27a, 0x4a3300),
    ("terminal-ice", "Terminal · ice", 0x5fd7ff, 0xa8ecff, 0x123a4a),
];

/// Every built-in theme's id, in picker order.
pub fn builtin_ids() -> Vec<&'static str> {
    let mut v = vec!["singularity", "event-horizon", "classic"];
    v.extend(COCKPIT.iter().map(|c| c.0));
    v.extend(TERMINAL.iter().map(|t| t.0));
    v
}

pub fn builtin(id: &str) -> Option<Theme> {
    let h = Rgb::hex;
    let plain = |id: &str, name: &str| Theme {
        id: id.into(),
        name: name.into(),
        bg: BLACK,
        fg: h(0xd6d6d6),
        dim: h(0x585858),
        accent: h(0xf2d3a0),
        accent2: h(0x8fb8ff),
        border: h(0x2a2a2a),
        border_focus: h(0x6b6250),
        selection: h(0x161616),
        error: h(0xdc4c4c),
        ramp: vec![h(0x26324d), h(0x5f86c9), h(0xb9c9e6), h(0xf2d3a0)],
        borders: border::PLAIN,
        boxes: vec![],
        numbered: false,
        scanlines: false,
    };
    match id {
        "singularity" => Some(plain(id, "Singularity")),
        // cream + ice: the colours of the disk itself (blackbody cream, Doppler-blue approaching side)
        "event-horizon" => Some(Theme {
            fg: h(0xe8e6e3),
            dim: h(0x5f5d5a),
            accent2: h(0x9cc4ff),
            border: h(0x1e1e1e),
            border_focus: h(0x6b6250),
            selection: h(0x161412),
            ramp: vec![h(0x3a2c1c), h(0xa8783e), h(0xf2d3a0), h(0xf8ead0)],
            borders: border::ROUNDED,
            ..plain(id, "Event Horizon")
        }),
        // the desktop app on OLED black
        "classic" => Some(Theme {
            fg: h(0xffffff),
            dim: h(0xa7a7a7),
            accent: h(0x1db954),
            accent2: h(0x1ed760),
            border: h(0x1f1f1f),
            border_focus: h(0x3e3e3e),
            selection: h(0x1a1a1a),
            ramp: vec![h(0x0d4a24), h(0x169c46), h(0x1db954), h(0x4fe387)],
            borders: border::ROUNDED,
            ..plain(id, "Classic+")
        }),
        _ => {
            if let Some((id, name, ramp, boxes, acc)) = COCKPIT.iter().find(|c| c.0 == id) {
                return Some(Theme {
                    fg: h(0xcccccc),
                    dim: h(0x6a6a6a),
                    accent: h(*acc),
                    accent2: h(ramp[2]),
                    border: h(boxes[3]),
                    border_focus: h(ramp[1]),
                    selection: h(boxes[3]).scale(0.5),
                    ramp: ramp.iter().map(|c| h(*c)).collect(),
                    borders: border::ROUNDED,
                    boxes: boxes.iter().map(|c| h(*c)).collect(),
                    numbered: true,
                    ..plain(id, name)
                });
            }
            let (id, name, acc, fg, bord) = TERMINAL.iter().find(|t| t.0 == id)?;
            let a = h(*acc);
            Some(Theme {
                fg: h(*fg),
                dim: a.scale(0.42),
                accent: a,
                accent2: h(*fg).lerp(h(0xffffff), 0.4),
                border: h(*bord),
                border_focus: a.scale(0.7),
                selection: h(*bord).scale(0.6),
                ramp: vec![a.scale(0.3), a.scale(0.65), a, h(*fg).lerp(h(0xffffff), 0.3)],
                scanlines: true,
                ..plain(id, name)
            })
        }
    }
}

/// A theme file as written. Everything is optional: what's missing comes from `base` (default: Singularity).
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ThemeFile {
    name: Option<String>,
    base: Option<String>,
    /// `plain`, `rounded`, `double` or `thick`
    borders: Option<String>,
    numbered: Option<bool>,
    scanlines: Option<bool>,
    colors: Colors,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Colors {
    fg: Option<String>,
    dim: Option<String>,
    accent: Option<String>,
    accent2: Option<String>,
    border: Option<String>,
    border_focus: Option<String>,
    selection: Option<String>,
    error: Option<String>,
    ramp: Option<Vec<String>>,
    boxes: Option<Vec<String>>,
}

/// Read a user theme (`id` is its file name without `.toml`). Errors say what's wrong, for a toast.
pub fn parse_file(id: &str, text: &str) -> Result<Theme, String> {
    let doc: singularity_config::toml_edit::DocumentMut = text.parse().map_err(|e: singularity_config::toml_edit::TomlError| format!("themes/{id}.toml: {}", e.message()))?;
    let f: ThemeFile = singularity_config::toml_edit::de::from_document(doc).map_err(|e| format!("themes/{id}.toml: {}", e.message()))?;
    let base = f.base.as_deref().unwrap_or("singularity");
    let mut t = builtin(base).ok_or_else(|| format!("themes/{id}.toml: no built-in theme called {base:?}"))?;
    t.id = id.into();
    t.name = f.name.unwrap_or_else(|| id.into());
    let col = |s: &Option<String>, d: Rgb, what: &str| -> Result<Rgb, String> {
        match s {
            None => Ok(d),
            Some(s) => Rgb::parse(s).ok_or_else(|| format!("themes/{id}.toml: {what} = {s:?} isn't a #rrggbb colour")),
        }
    };
    let c = &f.colors;
    t.fg = col(&c.fg, t.fg, "fg")?;
    t.dim = col(&c.dim, t.dim, "dim")?;
    t.accent = col(&c.accent, t.accent, "accent")?;
    t.accent2 = col(&c.accent2, t.accent2, "accent2")?;
    t.border = col(&c.border, t.border, "border")?;
    t.border_focus = col(&c.border_focus, t.border_focus, "border_focus")?;
    t.selection = col(&c.selection, t.selection, "selection")?;
    t.error = col(&c.error, t.error, "error")?;
    let list = |v: &Option<Vec<String>>, what: &str| -> Result<Option<Vec<Rgb>>, String> {
        v.as_ref().map(|v| v.iter().map(|s| col(&Some(s.clone()), BLACK, what)).collect()).transpose()
    };
    if let Some(r) = list(&c.ramp, "ramp")? {
        if r.len() < 2 {
            return Err(format!("themes/{id}.toml: ramp needs at least two colours"));
        }
        t.ramp = r;
    }
    if let Some(b) = list(&c.boxes, "boxes")? {
        t.boxes = b;
    }
    if let Some(b) = &f.borders {
        t.borders = match b.as_str() {
            "plain" => border::PLAIN,
            "rounded" => border::ROUNDED,
            "double" => border::DOUBLE,
            "thick" => border::THICK,
            other => return Err(format!("themes/{id}.toml: borders = {other:?} (plain, rounded, double or thick)")),
        };
    }
    t.numbered = f.numbered.unwrap_or(t.numbered);
    t.scanlines = f.scanlines.unwrap_or(t.scanlines);
    Ok(t)
}

/// The user's themes (`themes/*.toml`), sorted by name, and what was wrong with any that couldn't be read.
pub fn load_dir(dir: &std::path::Path) -> (Vec<Theme>, Vec<String>) {
    let mut themes = vec![];
    let mut problems = vec![];
    let Ok(rd) = std::fs::read_dir(dir) else { return (themes, problems) };
    // (files starting with "_" are skipped: the example, or a theme put aside)
    let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('_'))).collect();
    paths.sort();
    for p in paths {
        let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("theme").to_string();
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| parse_file(&id, &t)) {
            Ok(t) => themes.push(t),
            Err(e) => problems.push(e),
        }
    }
    (themes, problems)
}

/// Written next to the user's themes the first time the folder is made, as a starting point.
pub const EXAMPLE_THEME: &str = r##"# A theme for Singularity. Copy this file to a name without the "_" (files starting with "_" are skipped), change
# it, and pick it with Ctrl+K → "Theme" or t. Saved changes show at once.
# Anything you leave out comes from `base`, a built-in theme:
# singularity, event-horizon, classic, cockpit-btop, cockpit-ice, cockpit-amber, cockpit-violet, cockpit-mono,
# terminal-green, terminal-amber, terminal-ice.
name = "Example (rose)"
base = "singularity"
borders = "rounded"     # plain, rounded, double or thick

[colors]
accent = "#f4a7b9"
accent2 = "#a7c7f4"
border_focus = "#6b4f57"
# low → high, for the meters and Visuals
ramp = ["#3b2430", "#8a4a62", "#d9849d", "#f4c7d3"]
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_exists_and_backgrounds_are_black() {
        for id in builtin_ids() {
            let t = builtin(id).unwrap();
            assert_eq!(t.id, id);
            assert_eq!(t.bg, BLACK, "{id}");
            assert!(t.ramp.len() >= 3, "{id}");
        }
    }

    #[test]
    fn a_theme_file_starts_from_its_base() {
        let t = parse_file("mine", "name = \"Mine\"\nbase = \"cockpit-ice\"\nborders = \"double\"\n[colors]\naccent = \"#ff0000\"\n").unwrap();
        assert_eq!((t.id.as_str(), t.name.as_str(), t.accent), ("mine", "Mine", Rgb(255, 0, 0)));
        assert_eq!(t.ramp, builtin("cockpit-ice").unwrap().ramp, "the rest is the base's");
        assert!(t.numbered && t.borders == border::DOUBLE);
        let ex = parse_file("example", EXAMPLE_THEME).unwrap();
        assert_eq!(ex.name, "Example (rose)");
    }

    #[test]
    fn bad_theme_files_say_what_is_wrong() {
        assert!(parse_file("a", "[colors]\naccent = \"red\"\n").unwrap_err().contains("accent"));
        assert!(parse_file("b", "base = \"nope\"\n").unwrap_err().contains("nope"));
        assert!(parse_file("c", "colour = 1\n").unwrap_err().contains("c.toml"));
        assert!(parse_file("d", "[colors]\nramp = [\"#000000\"]\n").unwrap_err().contains("two"));
    }
}
