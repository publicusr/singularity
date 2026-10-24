//! Slots and saved layouts (#19). A Slot shows one Widget from a curated group (`v` or its ◂ ▸ swap it); a layout
//! (the split tree, sizes included) can be saved under a name to `~/.config/singularity/layouts/` and comes back exactly,
//! next to the built-in Designs in the picker.

use crate::layout::Node;
use serde::Deserialize;
use std::path::Path;

/// Widgets that can take each other's place. Panes outside these (Home, the library, the black hole) keep theirs.
const GROUPS: [&[&str]; 2] = [
    &["fft", "loudness", "levels", "scope", "rain", "starfield"],
    &["lyrics", "queue", "nowplaying", "stream", "top", "listening", "today", "topweek", "marquee", "glow"],
];

/// What a Slot showing `widget` can show instead (itself included), if anything.
pub fn options(widget: &str) -> &'static [&'static str] {
    GROUPS.iter().find(|g| g.contains(&widget)).copied().unwrap_or(&[])
}

/// The next Widget for the Slot showing `widget`, skipping what's already on screen (`shown`).
pub fn next(widget: &str, shown: &[String], dir: i32) -> Option<&'static str> {
    let opts = options(widget);
    let i = opts.iter().position(|w| *w == widget)? as i32;
    let n = opts.len() as i32;
    (1..n).map(|k| opts[(i + dir * k).rem_euclid(n) as usize]).find(|w| !shown.iter().any(|s| s == w))
}

/// A layout as saved.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Saved {
    pub name: String,
    /// the Design it was made from (its colours and Disk look come along)
    pub design: String,
    pub tree: Node,
    /// file name without `.toml`
    #[serde(skip)]
    pub id: String,
}

/// A file name from what the user typed: letters, digits, `-` and `_` (spaces become `-`).
pub fn file_id(name: &str) -> String {
    let id: String = name.trim().chars().map(|c| if c.is_whitespace() { '-' } else { c }).filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
    id.to_lowercase()
}

pub fn save(dir: &Path, name: &str, design: &str, tree: &Node) -> Result<Saved, String> {
    let id = file_id(name);
    if id.is_empty() {
        return Err("a layout needs a name".into());
    }
    let saved = Saved { name: name.trim().into(), design: design.into(), tree: tree.clone(), id: id.clone() };
    let q = |s: &str| singularity_config::toml_edit::Value::from(s).to_string();
    let text = format!(
        "# A Singularity layout. Pick it with Ctrl+K → Design; delete this file to forget it.\n# Each split puts `a` left of (or above) `b`; `first` sizes `a` (cells, or a ratio), `last` sizes `b`.\nname = {}\ndesign = {}\ntree = {}\n",
        q(saved.name.trim()),
        q(design),
        tree_text(tree, 0)
    );
    singularity_config::write_atomic(&dir.join(format!("{id}.toml")), &text).map_err(|e| e.to_string())?;
    Ok(saved)
}

/// The tree as TOML, one split per line (inline tables may span lines since TOML 1.1).
fn tree_text(n: &Node, depth: usize) -> String {
    use crate::layout::{Axis, Size};
    match n {
        Node::Slot(s) => format!("{{ slot = {} }}", singularity_config::toml_edit::Value::from(s.as_str())),
        Node::Split { axis, first, a, b } => {
            let axis = match axis {
                Axis::Horizontal => "horizontal",
                Axis::Vertical => "vertical",
            };
            let first = match first {
                Size::Cells(c) => format!("{{ cells = {c} }}"),
                Size::Ratio(r) => format!("{{ ratio = {r} }}"),
                Size::Last(c) => format!("{{ last = {c} }}"),
            };
            let pad = "    ".repeat(depth + 1);
            format!("{{ split = {{ axis = \"{axis}\", first = {first},\n{pad}a = {},\n{pad}b = {},\n{}}} }}", tree_text(a, depth + 1), tree_text(b, depth + 1), "    ".repeat(depth))
        }
    }
}

/// The saved layouts, by name, and what couldn't be read.
pub fn load_dir(dir: &Path) -> (Vec<Saved>, Vec<String>) {
    let (mut out, mut problems) = (vec![], vec![]);
    let Ok(rd) = std::fs::read_dir(dir) else { return (out, problems) };
    for p in rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")) {
        let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| singularity_config::toml_edit::de::from_str::<Saved>(&t).map_err(|e| e.message().to_string())) {
            Ok(mut s) => {
                s.id = id;
                out.push(s);
            }
            Err(e) => problems.push(format!("layouts/{id}.toml: {e}")),
        }
    }
    out.sort_by_key(|s| s.name.to_lowercase());
    (out, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swapping_skips_what_is_already_on_screen() {
        let shown: Vec<String> = ["fft", "loudness", "lyrics"].map(String::from).to_vec();
        assert_eq!(next("fft", &shown, 1), Some("levels"));
        assert_eq!(next("fft", &shown, -1), Some("starfield"), "back from the first wraps to the last (the demos end each cycle)");
        assert_eq!(next("lyrics", &shown, 1), Some("queue"));
        assert_eq!(next("home", &shown, 1), None, "home keeps its Pane");
    }

    #[test]
    fn a_saved_layout_comes_back_identically() {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = Node::cockpit();
        let area = ratatui::layout::Rect::new(0, 0, 191, 57);
        for (i, pos) in [(0usize, 23u16), (2, 101), (4, 9)] {
            let d = tree.compute(area).dividers[i].clone();
            tree.drag(&d, pos);
        }
        tree.swap("fft", "scope");
        let s = save(dir.path(), "Night Shift!", "cockpit", &tree).unwrap();
        assert_eq!(s.id, "night-shift");
        let text = std::fs::read_to_string(dir.path().join("night-shift.toml")).unwrap();
        assert!(text.lines().count() > 10 && text.contains("{ slot = \"scope\" }"), "readable, one split per line:\n{text}");
        let (all, problems) = load_dir(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(all, vec![s]);
        assert_eq!(all[0].tree.compute(area), tree.compute(area));
        std::fs::write(dir.path().join("broken.toml"), "name = 1").unwrap();
        assert_eq!(load_dir(dir.path()).1.len(), 1, "a broken file is reported, the rest still load");
    }
}
