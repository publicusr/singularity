//! Designs (#18, D8): each is a Layout preset and the Themes made for it, shipped together. Singularity is the default.

use crate::layout::Node;

pub struct Design {
    /// `ui.design` in config.toml
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub layout: fn() -> Node,
    /// its palette variants (`t` cycles them); the first is its own
    pub themes: &'static [&'static str],
    /// the black hole's Disk look it starts with
    pub look: usize,
    /// the player bar's prompt (`singularity:~$`); None: a plain "Playing from …" line
    pub prompt: Option<&'static str>,
    /// Pane titles that differ from the usual ones
    pub titles: &'static [(&'static str, &'static str)],
}

pub const DESIGNS: [Design; 5] = [
    Design {
        id: "singularity",
        name: "Singularity",
        blurb: "terminal look, gravity well",
        layout: Node::singularity,
        themes: &["singularity"],
        look: 0,
        prompt: Some("singularity"),
        titles: &[],
    },
    Design {
        id: "event-horizon",
        name: "Event Horizon",
        blurb: "cream and ice, big black hole",
        layout: Node::event_horizon,
        themes: &["event-horizon"],
        look: 1,
        prompt: None,
        titles: &[("queue", "up next"), ("lyrics", "lyrics")],
    },
    Design {
        id: "classic",
        name: "Classic+",
        blurb: "the desktop app, on black",
        layout: Node::classic,
        themes: &["classic"],
        look: 0,
        prompt: None,
        titles: &[("library", "Your Library"), ("home", "Home"), ("queue", "Queue"), ("lyrics", "Lyrics")],
    },
    Design {
        id: "cockpit",
        name: "Cockpit",
        blurb: "btop style, instruments first",
        layout: Node::cockpit,
        themes: &["cockpit-btop", "cockpit-ice", "cockpit-amber", "cockpit-violet", "cockpit-mono"],
        look: 4,
        prompt: None,
        titles: &[("fft", "spectrum"), ("library", "library")],
    },
    Design {
        id: "terminal",
        name: "Terminal",
        blurb: "phosphor on black, rain",
        layout: Node::terminal,
        themes: &["terminal-green", "terminal-amber", "terminal-ice"],
        look: 1,
        prompt: Some("you@singularity"),
        titles: &[("library", "tree"), ("rain", "rain --audio")],
    },
];

pub fn by_id(id: &str) -> Option<usize> {
    DESIGNS.iter().position(|d| d.id == id)
}

/// A Pane's title in design `d`.
pub fn title(d: usize, slot: &str, usual: &'static str) -> &'static str {
    DESIGNS[d].titles.iter().find(|(s, _)| *s == slot).map(|(_, t)| *t).unwrap_or(usual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_design_has_a_home_pane_and_real_themes() {
        for d in &DESIGNS {
            let slots = (d.layout)().compute(ratatui::layout::Rect::new(0, 0, 200, 60)).slots;
            assert!(slots.iter().any(|(s, _)| s == "home"), "{}: browsing needs a home Pane", d.id);
            for t in d.themes {
                assert!(crate::theme::builtin(t).is_some(), "{}: {t}", d.id);
            }
        }
        assert_eq!(DESIGNS[0].id, "singularity", "the default comes first");
    }
}
