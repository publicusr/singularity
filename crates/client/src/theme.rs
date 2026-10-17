//! Theme tokens. Every Widget takes its colours from here, never hard-coded (spec: Design principles).
//! All backgrounds are pure black (OLED). Designs and palette variants arrive with ticket #18.

use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(h: u32) -> Rgb {
        Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }
    pub fn color(self) -> Color {
        Color::Rgb(self.0, self.1, self.2)
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // name, accent2 and ramp are consumed by Visuals and the theme picker (#15, #16, #18)
pub struct Theme {
    pub name: &'static str,
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
}

impl Theme {
    /// The default design: terminal-driven, cream + ice on black.
    pub fn singularity() -> Theme {
        let h = Rgb::hex;
        Theme {
            name: "Singularity",
            bg: h(0x000000),
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
        }
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
}
