//! Swapping Widgets and saved layouts (#19). A Pane shows one Widget from a curated group (`v` or its ◂ ▸ swap it); a
//! layout (the split tree, sizes included) can be saved under a name to `~/.config/singularity/layouts/` and comes back exactly,
//! next to the built-in Designs in the picker.

use crate::arrangement::Arrangement;
use crate::layout::{Node, Widget};
use serde::Deserialize;
use std::path::Path;

/// Widgets that can take each other's place. Panes outside these (Home, the library, the black hole) keep theirs.
const GROUPS: [&[Widget]; 2] = {
    use Widget::*;
    [&[Fft, Loudness, Levels, Scope, Rain, Starfield], &[Lyrics, Karaoke, Queue, NowPlaying, Stream, Top, Listening, Today, TopWeek, Marquee, Glow]]
};

/// What a Pane showing `widget` can show instead (itself included), if anything.
pub fn options(widget: Widget) -> &'static [Widget] {
    GROUPS.iter().find(|g| g.contains(&widget)).copied().unwrap_or(&[])
}

/// The next Widget for the Pane showing `widget`, skipping what's already on screen (`shown`).
pub fn next(widget: Widget, shown: &[Widget], dir: i32) -> Option<Widget> {
    let opts = options(widget);
    let i = opts.iter().position(|w| *w == widget)? as i32;
    let n = opts.len() as i32;
    (1..n).map(|k| opts[(i + dir * k).rem_euclid(n) as usize]).find(|w| !shown.contains(w))
}

/// A layout as saved.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Saved {
    pub name: String,
    /// the Design it was made from (its colours and Disk look come along)
    pub design: String,
    /// the Theme in use when it was saved (built-in id or a file in `themes/`); None in older files: the design's own
    #[serde(default)]
    pub theme: Option<String>,
    pub tree: Node,
    /// the unnamed "<Design> (edited)" a first edit leaves when the user doesn't name it (D66): one per built-in Design
    #[serde(default)]
    pub edited: bool,
    /// the Widgets waiting in its Tray (D87); none in older files
    #[serde(default)]
    pub tray: Vec<Widget>,
    /// file name without `.toml`
    #[serde(skip)]
    pub id: String,
}

/// A file name from what the user typed: letters, digits, `-` and `_` (spaces become `-`).
pub fn file_id(name: &str) -> String {
    let id: String = name.trim().chars().map(|c| if c.is_whitespace() { '-' } else { c }).filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
    id.to_lowercase()
}

impl Saved {
    /// What it puts on screen: the floor plan, and the Tray it keeps.
    pub fn arrangement(&self) -> Arrangement {
        Arrangement::new(self.tree.clone(), self.tray.clone())
    }
}

pub fn save(dir: &Path, name: &str, design: &str, theme: &str, a: impl Into<Arrangement>) -> Result<Saved, String> {
    let id = file_id(name);
    if id.is_empty() {
        return Err("a layout needs a name".into());
    }
    write(dir, &id, name, design, theme, &a.into(), false)
}

/// The file an unnamed edit of built-in Design `design` lives in.
pub fn edited_id(design: &str) -> String {
    format!("{design}-edited")
}

/// Write `<id>.toml`; `edited` marks the unnamed edit of a built-in Design.
pub fn write(dir: &Path, id: &str, name: &str, design: &str, theme: &str, a: &Arrangement, edited: bool) -> Result<Saved, String> {
    let tree = a.plan();
    let saved = Saved { name: name.trim().into(), design: design.into(), theme: Some(theme.into()).filter(|t: &String| !t.is_empty()), tree: tree.clone(), edited, tray: a.tray().to_vec(), id: id.into() };
    let tray = match saved.tray.as_slice() {
        [] => String::new(),
        t => format!("tray = [{}]\n", t.iter().map(|w| singularity_config::toml_edit::Value::from(w.name()).to_string()).collect::<Vec<_>>().join(", ")),
    };
    let q = |s: &str| singularity_config::toml_edit::Value::from(s).to_string();
    let text = format!(
        "# A Singularity layout. Pick it with Ctrl+K → Pick a design; delete this file to forget it.\n# Each split puts `a` left of (or above) `b`; `first` sizes `a` (cells, or a ratio), `last` sizes `b`.\n# Each pane has its own id and shows `widget` (none: an empty pane); `tray` is what waits in the Tray.\nname = {}\ndesign = {}\n{}{}{tray}tree = {}\n",
        q(saved.name.trim()),
        q(design),
        saved.theme.as_deref().map(|t| format!("theme = {}\n", q(t))).unwrap_or_default(),
        if edited { "edited = true\n" } else { "" },
        tree_text(tree, 0)
    );
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    singularity_config::write_atomic(&dir.join(format!("{id}.toml")), &text).map_err(|e| e.to_string())?;
    Ok(saved)
}

/// All the Widgets a new Pane can show (before leaving out what's on screen).
pub fn all_widgets() -> impl Iterator<Item = Widget> {
    GROUPS.iter().flat_map(|g| g.iter().copied())
}

/// The tree as TOML, one split per line (inline tables may span lines since TOML 1.1).
fn tree_text(n: &Node, depth: usize) -> String {
    use crate::layout::{Axis, Size};
    match n {
        Node::Pane(p) => match p.widget {
            Some(w) => format!("{{ pane = {{ id = {}, widget = {} }} }}", p.id.0, singularity_config::toml_edit::Value::from(w.name())),
            None => format!("{{ pane = {{ id = {} }} }}", p.id.0),
        },
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
        use Widget::*;
        let shown = [Fft, Loudness, Lyrics];
        assert_eq!(next(Fft, &shown, 1), Some(Levels));
        assert_eq!(next(Fft, &shown, -1), Some(Starfield), "back from the first wraps to the last (the demos end each cycle)");
        assert_eq!(next(Lyrics, &shown, 1), Some(Karaoke));
        assert_eq!(next(Home, &shown, 1), None, "home keeps its Pane");
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
        tree.put(tree.pane_of(Widget::Fft).unwrap(), Some(Widget::Scope));
        let s = save(dir.path(), "Night Shift!", "cockpit", "cockpit-amber", &tree).unwrap();
        assert_eq!(s.id, "night-shift");
        let text = std::fs::read_to_string(dir.path().join("night-shift.toml")).unwrap();
        assert!(text.lines().count() > 10 && text.contains("widget = \"scope\" }"), "readable, one split per line:\n{text}");
        let (all, problems) = load_dir(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(all, vec![s.clone()]);
        assert_eq!(all[0].theme.as_deref(), Some("cockpit-amber"));
        // an old file has no theme line and still loads
        let old = text.lines().filter(|l| !l.starts_with("theme")).collect::<Vec<_>>().join("\n");
        std::fs::write(dir.path().join("night-shift.toml"), old).unwrap();
        assert_eq!(load_dir(dir.path()).0[0].theme, None);
        std::fs::write(dir.path().join("night-shift.toml"), text).unwrap();
        assert_eq!(all[0].tree.compute(area), tree.compute(area));
        std::fs::write(dir.path().join("broken.toml"), "name = 1").unwrap();
        assert_eq!(load_dir(dir.path()).1.len(), 1, "a broken file is reported, the rest still load");
    }

    /// A tree as files said it before Panes had an identity (D87): each leaf its Widget's name.
    fn old_text(n: &Node) -> String {
        match n {
            Node::Pane(p) => format!("{{ slot = \"{}\" }}", p.widget.unwrap()),
            Node::Split { axis, first, a, b } => {
                let axis = if *axis == crate::layout::Axis::Horizontal { "horizontal" } else { "vertical" };
                let first = match first {
                    crate::layout::Size::Cells(c) => format!("{{ cells = {c} }}"),
                    crate::layout::Size::Ratio(r) => format!("{{ ratio = {r} }}"),
                    crate::layout::Size::Last(c) => format!("{{ last = {c} }}"),
                };
                format!("{{ split = {{ axis = \"{axis}\", first = {first}, a = {}, b = {} }} }}", old_text(a), old_text(b))
            }
        }
    }

    /// Every Design saved before D87 loads to the same screen: the same Widgets in the same places. Saved again, it is
    /// written the new way, and that reads back exactly.
    #[test]
    fn an_old_layout_file_loads_to_the_same_screen_and_saves_the_new_way() {
        let dir = tempfile::tempdir().unwrap();
        let area = ratatui::layout::Rect::new(0, 1, 173, 51);
        for (i, d) in crate::design::DESIGNS.iter().enumerate() {
            let mut tree = (d.layout)();
            let div = tree.compute(area).dividers[0].clone();
            tree.drag(&div, 37);
            let old = format!("name = \"Old {i}\"\ndesign = \"{}\"\ntree = {}\n", d.id, old_text(&tree));
            let file = dir.path().join(format!("old-{i}.toml"));
            std::fs::write(&file, &old).unwrap();
            let (got, problems) = load_dir(dir.path());
            assert!(problems.is_empty(), "{}: {problems:?}\n{old}", d.id);
            let back = &got[0].tree;
            let shown = |n: &Node| n.compute(area).panes.into_iter().map(|(p, r)| (p.widget, r)).collect::<Vec<_>>();
            assert_eq!(shown(back), shown(&tree), "{}: the same screen", d.id);
            assert!(back.panes().iter().all(|p| p.id != crate::layout::PaneId::UNSET), "{}: every Pane has an identity", d.id);
            let s = save(dir.path(), &format!("Old {i}"), d.id, "", back).unwrap();
            let text = std::fs::read_to_string(&file).unwrap();
            assert!(!text.contains("slot =") && text.contains("pane = { id = "), "{}: written the new way\n{text}", d.id);
            assert_eq!(load_dir(dir.path()).0, vec![s], "{}: the new way reads back exactly", d.id);
            std::fs::remove_file(&file).unwrap();
        }
    }

    /// An empty Pane (a split makes one) is saved as a Pane without a Widget and comes back empty, its identity
    /// kept; a Widget this build hasn't got and two Panes claiming one identity load too.
    #[test]
    fn an_empty_pane_is_saved_and_loaded() {
        use crate::layout::{Axis, PaneId, Size};
        let dir = tempfile::tempdir().unwrap();
        let tree = Node::split(Axis::Horizontal, Size::Ratio(0.5), Node::pane(Widget::Home), Node::split(Axis::Vertical, Size::Cells(12), Node::empty(), Node::pane(Widget::Queue)));
        let mut ids: Vec<PaneId> = tree.panes().iter().map(|p| p.id).collect();
        let gap = ids[1];
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3, "an identity each: {tree:?}");
        assert_eq!(tree.widgets(), vec![Widget::Home, Widget::Queue]);
        assert!(tree.valid(ratatui::layout::Rect::new(0, 1, 120, 40)));
        let s = save(dir.path(), "Gap", "singularity", "", &tree).unwrap();
        let text = std::fs::read_to_string(dir.path().join("gap.toml")).unwrap();
        assert!(text.contains(&format!("{{ pane = {{ id = {} }} }}", gap.0)), "{text}");
        let (all, problems) = load_dir(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(all, vec![s]);
        assert_eq!(all[0].tree.get(gap).map(|p| p.widget), Some(None));
        // a Widget from another build, and two Panes claiming one identity (a hand-edited file)
        let odd = "name = \"Odd\"\ndesign = \"singularity\"\ntree = { split = { axis = \"horizontal\", first = { ratio = 0.5 }, a = { pane = { id = 4, widget = \"home\" } }, b = { split = { axis = \"vertical\", first = { ratio = 0.5 }, a = { slot = \"hologram\" }, b = { pane = { id = 4, widget = \"queue\" } } } } } }\n";
        std::fs::write(dir.path().join("odd.toml"), odd).unwrap();
        let (all, problems) = load_dir(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        let t = &all.iter().find(|s| s.id == "odd").unwrap().tree;
        assert_eq!(t.panes().iter().map(|p| p.widget.map(|w| w.name())).collect::<Vec<_>>(), vec![Some("home"), Some("hologram"), Some("queue")]);
        let mut ids: Vec<PaneId> = t.panes().iter().map(|p| p.id).collect();
        assert_eq!(ids[0], PaneId(4), "the first keeps its identity");
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3, "the others get their own: {t:?}");
    }

    /// A Widget from another build (newer, or one since retired) survives this one: it loads, shows as "?" and is
    /// written back unchanged when the Design is saved again (review of #35: it used to be erased).
    #[test]
    fn a_widget_this_build_has_not_got_survives_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let file = "name = \"Odd\"\ndesign = \"singularity\"\ntree = { split = { axis = \"horizontal\", first = { ratio = 0.5 }, a = { slot = \"home\" }, b = { pane = { id = 7, widget = \"hologram\" } } } }\n";
        std::fs::write(dir.path().join("odd.toml"), file).unwrap();
        let (all, problems) = load_dir(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        let tree = all[0].tree.clone();
        assert_eq!(Widget::of("hologram"), None, "(not a Widget of this build)");
        assert_eq!(crate::ui::pane_name(0, tree.widgets()[1]), "?");
        save(dir.path(), "Odd", "singularity", "", &tree).unwrap();
        let text = std::fs::read_to_string(dir.path().join("odd.toml")).unwrap();
        assert!(text.contains("{ pane = { id = 7, widget = \"hologram\" } }"), "{text}");
        assert_eq!(load_dir(dir.path()).0[0].tree, tree);
    }

    /// Identities near the top of the range (a hand-edited file) neither overflow nor collide: the tree is numbered
    /// afresh, and new Panes still get identities of their own.
    #[test]
    fn huge_identities_are_numbered_afresh() {
        use crate::layout::PaneId;
        let dir = tempfile::tempdir().unwrap();
        let file = format!(
            "name = \"Big\"\ndesign = \"singularity\"\ntree = {{ split = {{ axis = \"horizontal\", first = {{ ratio = 0.5 }}, a = {{ pane = {{ id = {}, widget = \"home\" }} }}, b = {{ pane = {{ widget = \"queue\" }} }} }} }}\n",
            u32::MAX
        );
        std::fs::write(dir.path().join("big.toml"), file).unwrap();
        let (all, problems) = load_dir(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        let t = &all[0].tree;
        let ids: Vec<PaneId> = t.panes().iter().map(|p| p.id).collect();
        assert!(ids[0] != ids[1] && !ids.contains(&PaneId::UNSET), "{ids:?}");
        let m = t.split_beside(ids[1], Some(Widget::Fft), crate::tiling::Side::Right).unwrap();
        let mut all_ids: Vec<PaneId> = m.panes().iter().map(|p| p.id).collect();
        all_ids.sort();
        all_ids.dedup();
        assert_eq!(all_ids.len(), 3);
    }
}
