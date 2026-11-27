//! The Shown Design's rules (D87 Q33, D89) at its interface, over an in-memory store: no App.

use super::*;
use crate::layout::{Node, Widget};

/// A Shown Design over memory (with the Theme `singularity` on screen) and what that memory holds.
fn shown() -> (ShownDesign, Memory) {
    let mem = Memory::default();
    let mut s = ShownDesign::default();
    s.set_store(Box::new(mem.clone()));
    assert_eq!(s.reload(), None);
    (s, mem)
}

const THEME: &str = "singularity";

fn files(m: &Memory) -> Vec<(String, String, bool)> {
    m.files.borrow().values().map(|s| (s.id.clone(), s.name.clone(), s.edited)).collect()
}

/// The built-in Singularity layout with two Widgets swapped: an edit.
fn edit(a: Node, x: Widget, y: Widget) -> Arrangement {
    let mut a = a;
    let (px, py) = (a.pane_of(x).unwrap(), a.pane_of(y).unwrap());
    a.put(px, Some(y));
    a.put(py, Some(x));
    a.into()
}

fn said(o: &Outcome) -> Option<&str> {
    o.said()
}

#[test]
fn an_edit_of_a_built_in_design_is_kept_at_once_as_its_edited_copy() {
    let (mut s, mem) = shown();
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    let o = s.edited(&a, THEME, false);
    assert_eq!(said(&o), Some("Kept as Singularity (edited)"), "never asked, the bar says so (Q33)");
    assert!(o.saves_ui(), "config.toml names the copy");
    assert_eq!(o.kept_as.as_deref(), Some("Singularity (edited)"), "Close words it as \"· kept as …\"");
    assert_eq!(o.steps().last(), Some(&Step::Say("Kept as Singularity (edited)".into())), "config.toml first, then the bar");
    assert_eq!(s.file(), Some("singularity-edited"));
    assert!(s.on_edited_copy());
    assert_eq!(files(&mem), [("singularity-edited".to_string(), "Singularity (edited)".to_string(), true)]);
    assert_eq!(s.saved()[0].arrangement(), a);
    // the built-in Design itself never changes
    assert_eq!((DESIGNS[0].layout)(), Node::singularity());
    // more edits overwrite it, never a pile, and say nothing more
    let b = edit(Node::singularity(), Widget::Home, Widget::Lyrics);
    let o = s.edited(&b, THEME, false);
    assert_eq!(o, Outcome::default());
    assert_eq!(files(&mem).len(), 1);
    assert_eq!(s.saved()[0].arrangement(), b);
}

#[test]
fn in_arrange_mode_the_copy_is_kept_quietly() {
    let (mut s, mem) = shown();
    let o = s.edited(&edit(Node::singularity(), Widget::Stream, Widget::Queue), THEME, true);
    assert_eq!((said(&o), o.kept_as.as_deref()), (None, None), "Done asks for a name instead");
    assert!(s.on_edited_copy() && files(&mem).len() == 1);
}

#[test]
fn done_asks_for_a_name_only_on_the_edited_copy_reading_the_folder_first() {
    let (mut s, mem) = shown();
    assert_eq!(s.done_prompt(), None, "the built-in Design: nothing was kept");
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    let _ = s.edited(&a, THEME, true);
    // a Design saved elsewhere meanwhile (another Client, a file copied in): the suggestion steps past it
    mem.files.borrow_mut().insert("singularity-2".into(), saved("singularity-2", "Singularity 2", "singularity", THEME, &a, false));
    assert_eq!(s.done_prompt(), Some(("Singularity 3".to_string(), None)));
    let _ = s.save_as("Mine", &a, THEME);
    assert_eq!(s.done_prompt(), None, "a Design of your own: its file has the changes");
}

#[test]
fn undo_into_a_design_whose_write_fails_says_so_and_leaves_things_as_they_are() {
    let (mut s, mem) = shown();
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    let _ = s.save_as("Mine", &a, THEME);
    let mine = s.mark();
    s.show_built_in(0);
    mem.fail_next.set(true);
    let o = s.restore(mine, &a, THEME);
    assert_eq!(o.steps(), [Step::Say("Couldn't save the layout: disk full".into())]);
    assert_eq!(s.file(), None, "still the built-in Design");
    assert_eq!(s.find("mine").unwrap().arrangement(), a);
}

#[test]
fn naming_the_copy_when_the_write_fails_says_so_and_keeps_the_copy() {
    let (mut s, mem) = shown();
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    let _ = s.edited(&a, THEME, true);
    mem.fail_next.set(true);
    let o = s.name_edit("Mine", &a, THEME).expect("not refused: the prompt closes");
    assert_eq!(o.steps(), [Step::Say("Couldn't save the layout: disk full".into())]);
    assert_eq!(s.file(), Some("singularity-edited"));
    assert_eq!(files(&mem), [("singularity-edited".to_string(), "Singularity (edited)".to_string(), true)]);
}

#[test]
fn each_built_in_design_has_one_edited_copy() {
    let (mut s, mem) = shown();
    let _ = s.edited(&edit(Node::singularity(), Widget::Stream, Widget::Queue), THEME, false);
    s.show_built_in(crate::design::by_id("cockpit").unwrap());
    assert_eq!(s.file(), None);
    let o = s.edited(&edit(Node::cockpit(), Widget::Fft, Widget::Loudness), "cockpit-btop", false);
    assert_eq!(said(&o), Some("Kept as Cockpit (edited)"));
    let ids: Vec<String> = files(&mem).into_iter().map(|f| f.0).collect();
    assert_eq!(ids, ["cockpit-edited", "singularity-edited"]);
    // from the built-in Singularity again, an edit replaces its copy
    s.show_built_in(0);
    let c = edit(Node::singularity(), Widget::Fft, Widget::Loudness);
    let _ = s.edited(&c, THEME, false);
    assert_eq!(files(&mem).len(), 2);
    assert_eq!(s.find("singularity-edited").unwrap().arrangement(), c);
}

#[test]
fn naming_the_copy_moves_it_into_a_design_of_your_own() {
    let (mut s, mem) = shown();
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    let _ = s.edited(&a, THEME, true);
    assert_eq!(s.suggest_name(), "Singularity 2");
    let o = s.name_edit("Mine", &a, THEME).expect("saved");
    assert_eq!(said(&o), Some("Saved design \"Mine\" (Ctrl+K → Pick a design)"));
    assert!(o.saves_ui());
    assert_eq!(s.file(), Some("mine"));
    assert_eq!(files(&mem), [("mine".to_string(), "Mine".to_string(), false)], "no second file");
    // further edits go into its file, silently
    let b = edit(a.plan().clone(), Widget::Home, Widget::Lyrics);
    let o = s.edited(&b, THEME, false);
    assert_eq!(o, Outcome::default());
    assert_eq!(s.find("mine").unwrap().arrangement(), b);
    assert_eq!(files(&mem).len(), 1);
}

#[test]
fn a_name_already_taken_is_refused_and_suggestions_count_up() {
    let (mut s, mem) = shown();
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    let _ = s.save_as("Singularity 2", &a, THEME);
    s.show_built_in(0);
    let _ = s.edited(&a, THEME, true);
    assert_eq!(s.suggest_name(), "Singularity 3", "2 is taken");
    let o = s.name_edit(" Singularity 2 ", &a, THEME).unwrap_err();
    assert_eq!(said(&o), Some("There is already a design called \"Singularity 2\""));
    assert_eq!(s.file(), Some("singularity-edited"), "still the copy");
    assert_eq!(files(&mem).len(), 2);
}

#[test]
fn delete_removes_the_design_on_screen_and_leaves_a_built_in_one() {
    let (mut s, mem) = shown();
    // a built-in one: nothing to delete
    assert_eq!(s.delete(), None);
    // its edited copy: the copy goes, its built-in Design stays on screen
    let cockpit = crate::design::by_id("cockpit").unwrap();
    s.show_built_in(cockpit);
    let a = edit(Node::cockpit(), Widget::Fft, Widget::Loudness);
    let _ = s.edited(&a, THEME, false);
    let o = s.delete().unwrap().expect("deleted");
    assert_eq!((said(&o), o.saves_ui()), (Some("Deleted \"Cockpit (edited)\" · back to Cockpit"), true));
    assert_eq!((s.file(), s.design()), (None, cockpit));
    assert!(files(&mem).is_empty(), "the unnamed edit is gone");
    // a Design of your own (made from Cockpit): gone, and the default Design is on screen
    let _ = s.save_as("Mine", &a, THEME);
    let _ = s.save_as("Other", &a, THEME);
    let o = s.delete().unwrap().expect("deleted");
    assert_eq!(said(&o), Some("Deleted \"Other\" · back to Singularity"));
    assert_eq!((s.file(), s.design()), (None, 0));
    assert_eq!(files(&mem), [("mine".to_string(), "Mine".to_string(), false)], "only the one on screen goes");
    assert!(s.picker_items().iter().all(|i| i.1 != "Other"), "nor in the picker");
}

#[test]
fn a_delete_that_cant_remove_the_file_says_so_and_changes_nothing() {
    let (mut s, mem) = shown();
    let cockpit = crate::design::by_id("cockpit").unwrap();
    s.show_built_in(cockpit);
    let a = edit(Node::cockpit(), Widget::Fft, Widget::Loudness);
    let _ = s.save_as("Mine", &a, THEME);
    mem.fail_next.set(true);
    let o = s.delete().unwrap().expect_err("not deleted");
    assert_eq!(o.steps(), [Step::Say("Couldn't delete \"Mine\": read-only folder".into())]);
    assert_eq!((s.file(), s.design()), (Some("mine"), cockpit), "still on screen");
    assert_eq!(files(&mem).len(), 1);
    assert!(s.picker_items().iter().any(|i| i.1 == "Mine"));
}

#[test]
fn undo_brings_back_the_design_with_the_layout() {
    let (mut s, mem) = shown();
    let built_in: Arrangement = Node::singularity().into();
    let before = s.mark();
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    let _ = s.edited(&a, THEME, false);
    let edited = s.mark();
    assert_eq!(edited.file(), Some("singularity-edited"));
    // undo the edit: the copy goes with the edit that made it
    let o = s.restore(before.clone(), &built_in, THEME);
    assert_eq!((said(&o), o.saves_ui()), (Some("Back to Singularity"), true));
    assert!(s.file().is_none() && files(&mem).is_empty());
    // redo: the copy is back, saying the edit again
    let o = s.restore(edited, &a, THEME);
    assert_eq!(said(&o), None);
    assert!(o.saves_ui());
    assert_eq!(files(&mem), [("singularity-edited".to_string(), "Singularity (edited)".to_string(), true)]);
    assert_eq!(s.saved()[0].arrangement(), a);
    // undo within a Design of your own writes its file back; out of it to the built-in one, the file stays
    let _ = s.save_as("Mine", &a, THEME);
    let mine = s.mark();
    let b = edit(a.plan().clone(), Widget::Home, Widget::Lyrics);
    let _ = s.edited(&b, THEME, false);
    let o = s.restore(mine, &a, THEME);
    assert_eq!(o, Outcome::default());
    assert_eq!(s.find("mine").unwrap().arrangement(), a);
    let o = s.restore(before, &built_in, THEME);
    assert_eq!(said(&o), Some("Back to Singularity · \"Mine\" is still in the Design picker"));
    assert!(s.find("mine").is_some());
}

#[test]
fn a_new_design_is_never_saved_over_yours_or_where_an_edited_copy_goes() {
    let (mut s, mem) = shown();
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    let _ = s.save_as("Mine", &a, THEME);
    s.build_new();
    assert_eq!(s.file(), None);
    for taken in ["mine", "Karaoke edited"] {
        let o = s.save_new(taken, &a, THEME).unwrap_err();
        assert!(said(&o).is_some_and(|t| t.starts_with("There is already a design called")), "{taken}: {o:?}");
    }
    let o = s.save_new("!!", &a, THEME).unwrap_err();
    assert_eq!(said(&o), Some("Couldn't save the layout: a layout needs a name"));
    let o = s.save_new("Built", &a, THEME).expect("saved");
    assert_eq!(said(&o), Some("Saved design \"Built\" (Ctrl+K → Pick a design)"));
    assert_eq!(s.file(), Some("built"));
    assert_eq!(files(&mem).len(), 2);
}

#[test]
fn config_remembers_the_design_its_theme_and_file() {
    let (mut s, _mem) = shown();
    let r = s.remembered("singularity", None);
    assert_eq!((r.design.as_str(), r.theme.as_str(), r.layout.as_str()), ("singularity", "", ""), "the Design's own theme is left out");
    let r = s.remembered("terminal-amber", None);
    assert_eq!(r.theme, "terminal-amber");
    let _ = s.save_as("Mine", &edit(Node::singularity(), Widget::Stream, Widget::Queue), THEME);
    assert_eq!(s.remembered(THEME, None).layout, "mine");
    // while a New design… is built, the one it began from
    let from = s.mark();
    s.build_new();
    assert_eq!(s.remembered(THEME, from.file()).layout, "mine");
    s.resume(from);
    assert_eq!(s.file(), Some("mine"));
}

#[test]
fn without_a_store_nothing_is_kept() {
    let mut s = ShownDesign::default();
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    assert_eq!(s.edited(&a, THEME, false), Outcome::default());
    assert_eq!(s.file(), None);
    assert_eq!(said(&s.save_as("Mine", &a, THEME)), Some("No config folder to save layouts in"));
    assert!(s.save_new("Built", &a, THEME).is_ok(), "built all the same");
}

#[test]
fn the_picker_lists_the_built_in_designs_then_yours() {
    let (mut s, _mem) = shown();
    let a = edit(Node::singularity(), Widget::Stream, Widget::Queue);
    let _ = s.edited(&a, THEME, false);
    let items = s.picker_items();
    assert_eq!(items.len(), DESIGNS.len() + 1);
    assert_eq!(items.last().unwrap(), &("layout:singularity-edited".to_string(), "Singularity (edited)".to_string(), "your changes, unnamed · layouts/singularity-edited.toml".to_string()));
    assert_eq!(s.picker_current(), "layout:singularity-edited");
    s.show_built_in(1);
    assert_eq!(s.picker_current(), "event-horizon");
}
