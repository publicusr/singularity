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
    /// covers and the black hole drawn in this Theme's colours (`Tint`), unless `ui.tint_art = false`
    pub tint: bool,
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
    /// The ramp's colour at `t` in 0..=1 (the accent if a theme somehow has no ramp).
    pub fn ramp_at(&self, t: f32) -> Rgb {
        let r = &self.ramp;
        if r.is_empty() {
            return self.accent;
        }
        let x = t.clamp(0.0, 1.0) * (r.len() - 1) as f32;
        let (i, f) = (x.floor() as usize, x.fract());
        r[i].lerp(r[(i + 1).min(r.len() - 1)], f)
    }

    /// The border colour of the `i`-th Pane.
    pub fn pane_border(&self, i: usize) -> Rgb {
        if self.boxes.is_empty() { self.border } else { self.boxes[i % self.boxes.len()] }
    }

    /// How covers and the black hole are coloured: the Theme's tint if it has one and `allowed` (`ui.tint_art`).
    pub fn art_tint(&self, allowed: bool) -> Option<Tint> {
        (allowed && self.tint).then(|| Tint { stops: std::iter::once(BLACK).chain(self.ramp.iter().copied()).collect() })
    }
}

/// A gradient map (#33): an image's brightness laid onto black → the Theme's ramp, so covers and the black hole take
/// the Theme's colours. Dark stays black (OLED); the brightest parts get the ramp's top, which is never pure white.
#[derive(Clone, Debug, PartialEq)]
pub struct Tint {
    stops: Vec<Rgb>,
}

impl Tint {
    /// Names this tint in cache keys: a tinted copy is made once per cover and tint.
    pub fn key(&self) -> String {
        // one buffer: this runs for every cover on every frame (#71)
        use std::fmt::Write;
        let mut s = String::with_capacity(self.stops.len() * 7);
        for (i, c) in self.stops.iter().enumerate() {
            let _ = write!(s, "{}{:02x}{:02x}{:02x}", if i == 0 { "" } else { "-" }, c.0, c.1, c.2);
        }
        s
    }

    /// The colour at `t` (0 = black … 1 = the ramp's top).
    pub fn at(&self, t: f32) -> Rgb {
        let s = t.clamp(0.0, 1.0) * (self.stops.len() - 1) as f32;
        let i = (s as usize).min(self.stops.len() - 2);
        self.stops[i].lerp(self.stops[i + 1], s - i as f32)
    }

    /// Recolour an image in place (one table lookup per pixel).
    pub fn apply(&self, img: &mut image::RgbImage) {
        // brightness lifted a little (^0.8), as the pixel-art hole does, so mid-tones don't sink into the dark stops
        let lut: Vec<Rgb> = (0..256).map(|l| self.at((l as f32 / 255.0).powf(0.8))).collect();
        for p in img.pixels_mut() {
            let [r, g, b] = p.0;
            let l = (54 * r as u32 + 183 * g as u32 + 19 * b as u32) >> 8;
            let c = lut[l as usize];
            p.0 = [c.0, c.1, c.2];
        }
    }

    /// Four evenly spaced stops (0…1 floats), as the black hole's shader takes them.
    pub fn stops4(&self) -> [[f32; 3]; 4] {
        [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0].map(|t| {
            let c = self.at(t);
            [c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0]
        })
    }
}

const BLACK: Rgb = Rgb(0, 0, 0);

/// A Cockpit variant: name, ramp low→high, Pane borders, accent.
type CockpitVariant = (&'static str, &'static str, [u32; 3], [u32; 4], u32);

/// Cockpit's variants: name, ramp low→high, Pane borders, accent.
const COCKPIT: [CockpitVariant; 5] = [
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

/// Palettes any design can wear (the Theme picker lists them for every design; `t` goes through them after the design's
/// own): id, name, fg, dim, accent, accent2, ramp low→high. Pure black behind, like every theme; borders and the
/// selection are shades of the accent. Warm ones first.
type Extra = (&'static str, &'static str, u32, u32, u32, u32, [u32; 4]);
const EXTRAS: [Extra; 10] = [
    ("ember", "Ember", 0xf4dcc8, 0x9a7860, 0xff7a2f, 0xffb347, [0x3a1206, 0xb8400f, 0xff7a2f, 0xffc88a]),
    ("sunset", "Sunset", 0xf6dccf, 0x9c7a78, 0xff6f61, 0xffb86b, [0x3b1230, 0xc2415c, 0xff7a4d, 0xffd08a]),
    ("terracotta", "Terracotta", 0xeedacb, 0x987a68, 0xe07f55, 0xecb98f, [0x35160c, 0x9c4a2c, 0xd9774f, 0xf0c4a0]),
    ("honey", "Honey", 0xf1e2c0, 0x9a8660, 0xf5b835, 0xffd978, [0x3a2606, 0xa86f0a, 0xf5b835, 0xffe4a0]),
    ("rose", "Rosé", 0xf3dde2, 0x9a7f88, 0xeea5b9, 0xf5c7b0, [0x3a1824, 0xa14a68, 0xe58aa5, 0xf8cbd6]),
    ("peach", "Peach", 0xfbe4d6, 0xa08674, 0xffab8a, 0xffd3a3, [0x3d1a10, 0xc4623f, 0xff9a76, 0xffd9bf]),
    ("sandstone", "Sandstone", 0xe9dcc8, 0x9a8c76, 0xd9b383, 0xc99567, [0x2e2214, 0x8a6a42, 0xc9a06e, 0xecd8b4]),
    ("paper", "Paper", 0xf3ecdf, 0x948d7e, 0xe0d2b0, 0xbfae88, [0x2b2820, 0x6e684f, 0xb8ab86, 0xece3cd]),
    ("lagoon", "Lagoon", 0xd4ecea, 0x70908e, 0x3fd0c0, 0x7fe3d6, [0x06302d, 0x108f88, 0x3fd0c0, 0xaaf0e8]),
    ("moss", "Moss", 0xdbe6d0, 0x7f9272, 0x9bc77d, 0xc7dc8f, [0x16301a, 0x4c8a45, 0x9bc77d, 0xd6ecb8]),
];

/// The ids of those shared palettes.
pub fn extra_ids() -> impl Iterator<Item = &'static str> {
    EXTRAS.iter().map(|e| e.0)
}

/// Every built-in theme's id, in picker order.
pub fn builtin_ids() -> Vec<&'static str> {
    let mut v = vec!["singularity", "event-horizon", "classic"];
    v.extend(COCKPIT.iter().map(|c| c.0));
    v.extend(TERMINAL.iter().map(|t| t.0));
    v.extend(extra_ids());
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
        tint: false,
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
                    tint: true,
                    ..plain(id, name)
                });
            }
            if let Some((id, name, fg, dim, acc, acc2, ramp)) = EXTRAS.iter().find(|e| e.0 == id) {
                let a = h(*acc);
                return Some(Theme {
                    fg: h(*fg),
                    dim: h(*dim),
                    accent: a,
                    accent2: h(*acc2),
                    border: a.scale(0.22),
                    border_focus: a.scale(0.6),
                    selection: a.scale(0.14),
                    ramp: ramp.iter().map(|c| h(*c)).collect(),
                    borders: border::ROUNDED,
                    tint: true,
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
                tint: true,
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
    /// covers and the black hole in this theme's colours
    tint: Option<bool>,
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
    t.tint = f.tint.unwrap_or(t.tint);
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
# it, and pick it with Ctrl+K → "Pick a theme" or t. Saved changes show at once.
# Anything you leave out comes from `base`, a built-in theme:
# singularity, event-horizon, classic, cockpit-btop, cockpit-ice, cockpit-amber, cockpit-violet, cockpit-mono,
# terminal-green, terminal-amber, terminal-ice, ember, sunset, terracotta, honey, rose, peach, sandstone, paper,
# lagoon, moss.
name = "Example (rose)"
base = "singularity"
borders = "rounded"     # plain, rounded, double or thick
tint = false            # true: covers and the black hole in this theme's colours (black → ramp)

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

    /// WCAG contrast ratio of two colours.
    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let lum = |c: Rgb| {
            let f = |v: u8| {
                let v = v as f64 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
        };
        let (a, b) = (lum(a), lum(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn the_shared_palettes_are_readable_on_black_and_on_the_selection() {
        assert!(extra_ids().count() >= 8);
        for id in extra_ids() {
            let t = builtin(id).unwrap();
            for (what, c, min) in [("fg", t.fg, 4.5), ("accent", t.accent, 4.5), ("accent2", t.accent2, 4.5), ("dim", t.dim, 3.0)] {
                assert!(contrast(c, t.bg) >= min, "{id}: {what} {:.1}", contrast(c, t.bg));
                assert!(contrast(c, t.selection) >= min, "{id}: {what} on the selection {:.1}", contrast(c, t.selection));
            }
            assert!(t.ramp.windows(2).all(|w| w[0].0 as u32 + w[0].1 as u32 + w[0].2 as u32 <= w[1].0 as u32 + w[1].1 as u32 + w[1].2 as u32), "{id}: the ramp rises");
            assert_ne!(*t.ramp.last().unwrap(), Rgb(255, 255, 255), "{id}");
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
    fn tints_map_brightness_onto_the_ramp() {
        // Cockpit, Terminal and the shared palettes tint; Singularity, Event Horizon and Classic+ keep real colours
        for id in builtin_ids() {
            let tints = id.starts_with("cockpit") || id.starts_with("terminal") || extra_ids().any(|e| e == id);
            assert_eq!(builtin(id).unwrap().art_tint(true).is_some(), tints, "{id}");
        }
        let th = builtin("terminal-green").unwrap();
        assert!(th.art_tint(false).is_none(), "ui.tint_art = false: real colours");
        let t = th.art_tint(true).unwrap();
        assert_eq!(t.at(0.0), BLACK);
        assert_eq!(t.at(1.0), *th.ramp.last().unwrap());
        let mut img = image::RgbImage::from_fn(3, 1, |x, _| image::Rgb([[0, 0, 0], [200, 30, 30], [255, 255, 255]][x as usize]));
        t.apply(&mut img);
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0], "black stays black");
        let top = th.ramp.last().unwrap();
        assert_eq!(img.get_pixel(2, 0).0, [top.0, top.1, top.2], "white becomes the ramp's top");
        let mid = img.get_pixel(1, 0).0;
        assert!(mid[1] > mid[0] && mid[1] > mid[2], "a red pixel turns green: {mid:?}");
        assert_eq!(t.stops4()[0], [0.0; 3]);
        assert_ne!(t.key(), builtin("terminal-amber").unwrap().art_tint(true).unwrap().key());
        // theme files can switch it either way
        assert!(parse_file("t", "base = \"singularity\"\ntint = true\n").unwrap().tint);
        assert!(!parse_file("t", "base = \"cockpit-ice\"\ntint = false\n").unwrap().tint);
    }

    #[test]
    fn bad_theme_files_say_what_is_wrong() {
        assert!(parse_file("a", "[colors]\naccent = \"red\"\n").unwrap_err().contains("accent"));
        assert!(parse_file("b", "base = \"nope\"\n").unwrap_err().contains("nope"));
        assert!(parse_file("c", "colour = 1\n").unwrap_err().contains("c.toml"));
        assert!(parse_file("d", "[colors]\nramp = [\"#000000\"]\n").unwrap_err().contains("two"));
    }
}
