//! The Shown Design (D89): which Design is on screen (a built-in one, or one of yours with its saved file), the
//! "(edited)" copy an edit of a built-in Design is kept as (D87 Q33), what undo brings back and Delete leaves, and what
//! goes to config.toml. It writes through a `Store` over the layouts folder (tests use an in-memory one).
//!
//! The layout itself and the Theme on screen stay the App's: each edit hands them in, and what the bar should say
//! and whether config.toml should follow come back as an `Outcome`.

use crate::arrangement::Arrangement;
use crate::design::{DESIGNS, Design};
use crate::layouts::{Saved, edited_id, file_id};
use std::path::PathBuf;

/// Where the user's Designs are kept: the layouts folder, or (in tests) memory.
pub trait Store {
    /// The saved Designs, by name, and what couldn't be read.
    fn read(&self) -> (Vec<Saved>, Vec<String>);
    /// Write `s` as `<s.id>`, replacing what was there.
    fn write(&mut self, s: &Saved) -> Result<(), String>;
    /// Forget Design `id` (gone already: nothing happens). Err: why it couldn't be removed.
    fn remove(&mut self, id: &str) -> Result<(), String>;
}

/// The layouts folder: one `<id>.toml` per Design.
pub struct Folder(pub PathBuf);

impl Store for Folder {
    fn read(&self) -> (Vec<Saved>, Vec<String>) {
        crate::layouts::load_dir(&self.0)
    }

    fn write(&mut self, s: &Saved) -> Result<(), String> {
        crate::layouts::write(&self.0, &s.id, &s.name, &s.design, s.theme.as_deref().unwrap_or(""), &s.arrangement(), s.edited).map(|_| ())
    }

    fn remove(&mut self, id: &str) -> Result<(), String> {
        match std::fs::remove_file(self.0.join(format!("{id}.toml"))) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

/// What a change to the Shown Design asks of the App, in the order it happens: lines for the bar (the last one shows)
/// and saving config.toml (another Design or file is on screen); and, when an edit of a built-in Design was just kept
/// as its copy outside Arrange mode, that copy's name ("Singularity (edited)").
#[must_use]
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    steps: Vec<Step>,
    pub kept_as: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Say(String),
    SaveUi,
}

impl Outcome {
    fn say(&mut self, s: impl Into<String>) {
        self.steps.push(Step::Say(s.into()));
    }

    fn save_ui(&mut self) {
        self.steps.push(Step::SaveUi);
    }

    /// What the bar ends up saying, if anything.
    #[cfg(test)]
    pub fn said(&self) -> Option<&str> {
        self.steps.iter().rev().find_map(|s| match s {
            Step::Say(t) => Some(t.as_str()),
            Step::SaveUi => None,
        })
    }

    /// Whether config.toml is saved.
    #[cfg(test)]
    pub fn saves_ui(&self) -> bool {
        self.steps.contains(&Step::SaveUi)
    }

    /// The steps, in order, for the App to carry out.
    pub fn steps(self) -> Vec<Step> {
        self.steps
    }
}

/// What undo (and Esc on a New design… or the Design picker) brings back: the file that was on screen, and what it
/// said about itself then (None: the built-in Design).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mark {
    file: Option<String>,
    /// (name, design, edited) from that file
    about: Option<(String, String, bool)>,
}

impl Mark {
    /// The saved Design's id (None: the built-in one).
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }
}

/// What config.toml remembers: `ui.design`, `ui.theme` ("" for the Design's own) and `ui.layout` ("" for none).
#[derive(Debug, PartialEq)]
pub struct Remembered {
    pub design: String,
    pub theme: String,
    pub layout: String,
}

/// The Design on screen and the user's own Designs.
#[derive(Default)]
pub struct ShownDesign {
    /// the built-in Design (index into `DESIGNS`): the one shown, or the one the saved Design was made from
    design: usize,
    /// the saved Design on screen (its id); None: the built-in one
    file: Option<String>,
    /// the user's Designs, as last read
    saved: Vec<Saved>,
    /// None: nowhere to save (no config folder)
    store: Option<Box<dyn Store>>,
}

impl ShownDesign {
    /// Keep the user's Designs in `store` from now on (read them with `reload`).
    pub fn set_store(&mut self, store: Box<dyn Store>) {
        self.store = Some(store);
    }

    // ── what is shown ─────────────────────────────────────────────────

    /// The built-in Design (index into `DESIGNS`): shown, or the one the Design on screen was made from.
    pub fn design(&self) -> usize {
        self.design
    }

    pub fn built_in(&self) -> &'static Design {
        &DESIGNS[self.design]
    }

    /// The saved Design on screen (its file name); None: the built-in one.
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    /// The user's Designs, as last read.
    pub fn saved(&self) -> &[Saved] {
        &self.saved
    }

    pub fn find(&self, id: &str) -> Option<&Saved> {
        self.saved.iter().find(|s| s.id == id)
    }

    /// The saved Design on screen, as last read.
    fn own(&self) -> Option<&Saved> {
        self.find(self.file.as_deref()?)
    }

    /// Whether the Design on screen is the unnamed "<Design> (edited)" of a built-in one.
    pub fn on_edited_copy(&self) -> bool {
        self.own().is_some_and(|s| s.edited)
    }

    /// What undo would bring back of the Shown Design.
    pub fn mark(&self) -> Mark {
        Mark { file: self.file.clone(), about: self.own().map(|s| (s.name.clone(), s.design.clone(), s.edited)) }
    }

    /// What config.toml should say: the Design, the Theme `theme` unless it is the Design's own, and the saved Design
    /// (or, while a New design… is built, the one it began from: `building_from`).
    pub fn remembered(&self, theme: &str, building_from: Option<&str>) -> Remembered {
        let d = self.built_in();
        Remembered {
            design: d.id.to_string(),
            theme: if d.themes[0] == theme { String::new() } else { theme.to_string() },
            layout: self.file().or(building_from).unwrap_or_default().to_string(),
        }
    }

    /// The Design picker's rows (id, name, what it is): the built-in Designs, then the user's.
    pub fn picker_items(&self) -> Vec<(String, String, String)> {
        DESIGNS
            .iter()
            .map(|d| (d.id.to_string(), d.name.to_string(), d.blurb.to_string()))
            .chain(self.saved.iter().map(|s| (format!("layout:{}", s.id), s.name.clone(), if s.edited { format!("your changes, unnamed · layouts/{}.toml", s.id) } else { format!("yours · layouts/{}.toml", s.id) })))
            .collect()
    }

    /// The Design picker's row for what is shown.
    pub fn picker_current(&self) -> String {
        match &self.file {
            Some(l) => format!("layout:{l}"),
            None => self.built_in().id.to_string(),
        }
    }

    /// Done after changes in Arrange mode: if they went to a built-in Design's "(edited)" copy, the name its prompt
    /// suggests, after reading the folder again (and what couldn't be read, for the bar); None: no prompt.
    pub fn done_prompt(&mut self) -> Option<(String, Option<String>)> {
        if !self.on_edited_copy() {
            return None;
        }
        let problems = self.reload();
        Some((self.suggest_name(), problems))
    }

    /// "Singularity 2", "Singularity 3"…: the first name not taken by a Design of the user's.
    fn suggest_name(&self) -> String {
        let base = self.built_in().name;
        (2..).map(|n| format!("{base} {n}")).find(|c| !self.saved.iter().any(|s| s.id == file_id(c))).expect("a free name")
    }

    // ── switching ─────────────────────────────────────────────────────

    /// Built-in Design `d` is shown (the App puts its layout on screen).
    pub fn show_built_in(&mut self, d: usize) {
        self.design = d.min(DESIGNS.len() - 1);
        self.file = None;
    }

    /// Saved Design `s` is shown (with its built-in Design's colours and look; one unknown keeps the last).
    pub fn open(&mut self, s: &Saved) {
        self.design = crate::design::by_id(&s.design).unwrap_or(self.design);
        self.file = Some(s.id.clone());
    }

    /// The file on screen is `m`'s again, as it was (nothing is written): Esc on a New design… or the Design picker.
    pub fn resume(&mut self, m: Mark) {
        self.file = m.file;
    }

    /// A New design… is being built: no file is on screen (nothing is written until Done names it).
    pub fn build_new(&mut self) {
        self.file = None;
    }

    /// Read the user's Designs again; what couldn't be read, for the bar.
    pub fn reload(&mut self) -> Option<String> {
        let (saved, problems) = self.store.as_ref()?.read();
        self.saved = saved;
        (!problems.is_empty()).then(|| problems.join(" · "))
    }

    fn reread(&mut self, out: &mut Outcome) {
        if let Some(p) = self.reload() {
            out.say(p);
        }
    }

    // ── edits (Q33) ───────────────────────────────────────────────────

    /// The layout was edited to `a` (with Theme `theme` on screen). A Design of the user's own (or the unnamed edit of
    /// a built-in one) follows in its file; an edit of a built-in Design is kept at once as its "<Design> (edited)",
    /// never asking (Q33), and the bar says so unless `quiet` (Arrange mode, where Done asks for a name).
    pub fn edited(&mut self, a: &Arrangement, theme: &str, quiet: bool) -> Outcome {
        match self.own().cloned() {
            Some(s) => self.write(&s.id, &s.name, &s.design, s.edited, a, theme),
            None => {
                // (a file on screen that has since gone from the folder: kept as the copy too, but it wasn't the
                // built-in Design that was edited)
                let built_in = self.file.take().is_none();
                if self.store.is_none() {
                    return Outcome::default();
                }
                let mut out = self.keep_edited(a, theme);
                if !quiet && self.file.is_some() {
                    let copy = self.copy_name();
                    out.say(format!("Kept as {copy}"));
                    out.kept_as = built_in.then_some(copy);
                }
                out
            }
        }
    }

    /// Keep `a` as the built-in Design's one unnamed "<Design> (edited)", replacing the last (Esc on Done's prompt).
    pub fn keep_edited(&mut self, a: &Arrangement, theme: &str) -> Outcome {
        let d = self.built_in();
        self.write(&edited_id(d.id), &self.copy_name(), d.id, true, a, theme)
    }

    /// The name of the built-in Design's unnamed copy: "<Design> (edited)".
    fn copy_name(&self) -> String {
        format!("{} (edited)", self.built_in().name)
    }

    /// Save `a` as a Design of the user's called `name`: it is the one on screen from now on.
    pub fn save_as(&mut self, name: &str, a: &Arrangement, theme: &str) -> Outcome {
        let mut out = Outcome::default();
        if self.store.is_none() {
            out.say("No config folder to save layouts in");
            return out;
        }
        let id = file_id(name);
        if id.is_empty() {
            out.say("Couldn't save the layout: a layout needs a name");
            return out;
        }
        let s = saved(&id, name, self.built_in().id, theme, a, false);
        match self.put(&s) {
            Ok(()) => {
                self.file = Some(id);
                self.reread(&mut out);
                out.save_ui();
                out.say(format!("Saved design \"{}\" (Ctrl+K → Pick a design)", s.name));
            }
            Err(e) => out.say(format!("Couldn't save the layout: {e}")),
        }
        out
    }

    /// Done's prompt named the changes `name`: saved as a Design of the user's, and the unnamed "(edited)" copy they
    /// were kept in goes (the named one has it all). Err: a Design of the user's is already called that (the prompt
    /// stays).
    pub fn name_edit(&mut self, name: &str, a: &Arrangement, theme: &str) -> Result<Outcome, Outcome> {
        let id = file_id(name);
        if self.saved.iter().any(|s| s.id == id && !s.edited) {
            return Err(taken(name));
        }
        let was = self.file.clone();
        let mut out = self.save_as(name, a, theme);
        if let Some(id) = was.filter(|id| self.file.as_ref() != Some(id) && self.saved.iter().any(|s| s.id == *id && s.edited)) {
            self.remove(&id);
            self.reread(&mut out);
        }
        Ok(out)
    }

    /// Done's prompt named a New design… `name`: saved as a Design of the user's. Err (the prompt stays, the bar says
    /// why): the name is a Design of the user's already, or where a built-in Design's "(edited)" copy goes (it would
    /// later be written over), or it can't be saved.
    pub fn save_new(&mut self, name: &str, a: &Arrangement, theme: &str) -> Result<Outcome, Outcome> {
        let id = file_id(name);
        if self.saved.iter().any(|s| s.id == id) || DESIGNS.iter().any(|d| edited_id(d.id) == id) {
            return Err(taken(name));
        }
        let out = self.save_as(name, a, theme);
        // (without a config folder there is nowhere to save it: built all the same)
        match self.file.is_none() && self.store.is_some() {
            true => Err(out),
            false => Ok(out),
        }
    }

    /// "Delete design" (#80, replacing Reset design): the Design on screen goes from the folder and the picker, and a
    /// built-in one is shown (the App puts its layout on screen): the "(edited)" copy's own, or the default
    /// (Singularity) for a Design of the user's. None: a built-in Design is on screen (nothing to delete). Err: its
    /// file couldn't be removed; nothing changed (it stays on screen), the bar says why.
    pub fn delete(&mut self) -> Option<Result<Outcome, Outcome>> {
        let id = self.file.clone()?;
        let (name, edited) = self.find(&id).map_or((id.clone(), false), |s| (s.name.clone(), s.edited));
        let mut out = Outcome::default();
        if let Some(Err(e)) = self.store.as_mut().map(|st| st.remove(&id)) {
            out.say(format!("Couldn't delete \"{name}\": {e}"));
            return Some(Err(out));
        }
        self.file = None;
        if !edited {
            self.design = 0;
        }
        self.reread(&mut out);
        out.save_ui();
        out.say(format!("Deleted \"{name}\" · back to {}", self.built_in().name));
        Some(Ok(out))
    }

    /// Undo or redo put `a` back on screen: `m`'s Design comes back with it, its file saying `a` again. Back to the
    /// built-in Design, the unnamed edit of it goes with the edit that made it; a Design of the user's stays in the
    /// picker.
    pub fn restore(&mut self, m: Mark, a: &Arrangement, theme: &str) -> Outcome {
        let was = self.own().cloned();
        if let Some(id) = m.file {
            let (name, design, edited) = m.about.unwrap_or_else(|| (id.clone(), self.built_in().id.to_string(), false));
            return self.write(&id, &name, &design, edited, a, theme);
        }
        self.file = None;
        let base = self.built_in().name;
        let mut out = Outcome::default();
        match was {
            Some(w) if w.edited => {
                self.remove(&w.id);
                self.reread(&mut out);
                out.save_ui();
                out.say(format!("Back to {base}"));
            }
            Some(w) => {
                out.save_ui();
                out.say(format!("Back to {base} · \"{}\" is still in the Design picker", w.name));
            }
            None => {}
        }
        out
    }

    // ── the store ─────────────────────────────────────────────────────

    /// Write `a` as Design `id`, which is then the one on screen (config.toml follows if that's new).
    fn write(&mut self, id: &str, name: &str, design: &str, edited: bool, a: &Arrangement, theme: &str) -> Outcome {
        let mut out = Outcome::default();
        if self.store.is_none() {
            return out;
        }
        match self.put(&saved(id, name, design, theme, a, edited)) {
            Ok(()) => {
                let new = self.file.as_deref() != Some(id);
                self.file = Some(id.to_string());
                self.reread(&mut out);
                if new {
                    out.save_ui();
                }
            }
            Err(e) => out.say(format!("Couldn't save the layout: {e}")),
        }
        out
    }

    fn put(&mut self, s: &Saved) -> Result<(), String> {
        self.store.as_mut().map_or(Ok(()), |st| st.write(s))
    }

    /// (an "(edited)" copy that is no longer needed: one left behind does no harm)
    fn remove(&mut self, id: &str) {
        if let Some(st) = self.store.as_mut() {
            let _ = st.remove(id);
        }
    }
}

/// A Design as it is written (the theme left out when there is none).
fn saved(id: &str, name: &str, design: &str, theme: &str, a: &Arrangement, edited: bool) -> Saved {
    Saved { name: name.trim().into(), design: design.into(), theme: Some(theme.into()).filter(|t: &String| !t.is_empty()), tree: a.plan().clone(), edited, tray: a.tray().to_vec(), id: id.into() }
}

fn taken(name: &str) -> Outcome {
    let mut out = Outcome::default();
    out.say(format!("There is already a design called \"{}\"", name.trim()));
    out
}

/// The user's Designs in memory, for tests.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct Memory {
    pub files: std::rc::Rc<std::cell::RefCell<std::collections::BTreeMap<String, Saved>>>,
    /// the next write fails (as a full disk would), or the next remove (as a read-only folder would)
    pub fail_next: std::rc::Rc<std::cell::Cell<bool>>,
}

#[cfg(test)]
impl Store for Memory {
    fn read(&self) -> (Vec<Saved>, Vec<String>) {
        let mut all: Vec<Saved> = self.files.borrow().values().cloned().collect();
        all.sort_by_key(|s| s.name.to_lowercase());
        (all, vec![])
    }

    fn write(&mut self, s: &Saved) -> Result<(), String> {
        if self.fail_next.replace(false) {
            return Err("disk full".into());
        }
        self.files.borrow_mut().insert(s.id.clone(), s.clone());
        Ok(())
    }

    fn remove(&mut self, id: &str) -> Result<(), String> {
        if self.fail_next.replace(false) {
            return Err("read-only folder".into());
        }
        self.files.borrow_mut().remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
