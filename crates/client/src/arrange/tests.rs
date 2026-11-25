use super::*;
use crate::layout::Node;
use ratatui::crossterm::event::{KeyModifiers, MouseButton};
use crate::design::DESIGNS;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| crate::ui::draw(f, app)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
}
fn press(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Down(MouseButton::Left), x, y);
}
/// The pointer moved there with the button down.
fn drag(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Drag(MouseButton::Left), x, y);
}
fn release(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Up(MouseButton::Left), x, y);
}
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// The Pane showing `w`.
fn pid(app: &App, w: Widget) -> PaneId {
    app.layout.pane_of(w).unwrap_or_else(|| panic!("no Pane shows {w}"))
}
/// The drawn Pane showing `w`'s rectangle, from the hit boxes of the last frame.
fn pane(app: &App, w: Widget) -> Rect {
    let id = pid(app, w);
    app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Focus(p) if *p == id).then_some(*r)).unwrap_or_else(|| panic!("no pane {w}"))
}
/// A spot on the title bar of the Pane showing `w` that is neither a button nor a divider's.
fn title_spot(app: &App, w: Widget) -> (u16, u16) {
    let r = pane(app, w);
    (r.x + 3, r.y)
}
/// `n` with the Panes showing `x` and `y` trading Widgets.
fn swapped(n: impl Into<Arrangement>, x: Widget, y: Widget) -> Arrangement {
    let n = n.into();
    n.swap(n.pane_of(x).unwrap(), n.pane_of(y).unwrap()).unwrap()
}
/// Every Pane's rectangle as drawn in the last frame (its own hit box), by identity.
fn drawn_panes(app: &App) -> Vec<(PaneId, Rect)> {
    let mut v: Vec<(PaneId, Rect)> = app.hits.iter().filter_map(|(r, h)| match h {
        Hit::Focus(p) => Some((*p, *r)),
        _ => None,
    }).collect();
    v.sort_by_key(|(p, _)| *p);
    v
}
/// Every Pane's rectangle where Arrange mode puts it: the floor plan laid out in the window less the Tray's rows.
fn arranged_floor(app: &App) -> Vec<(PaneId, Rect)> {
    floor_in(app, true)
}
/// Every Pane's rectangle where it belongs now (arranging or not).
fn laid_out(app: &App) -> Vec<(PaneId, Rect)> {
    floor_in(app, app.arranging.is_some())
}
fn floor_in(app: &App, arranging: bool) -> Vec<(PaneId, Rect)> {
    let body = crate::ui::rows(app.window, arranging)[1];
    let mut v: Vec<(PaneId, Rect)> = app.layout.compute(body).panes.into_iter().map(|(p, r)| (p.id, r)).collect();
    v.sort_by_key(|(p, _)| *p);
    v
}
/// The cells of row `y` from `x0` to `x1`.
fn row(buf: &Buffer, y: u16, x0: u16, x1: u16) -> String {
    (x0..x1).map(|x| buf[(x, y)].symbol().to_string()).collect()
}
fn render_buf(app: &mut App, w: u16, h: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| crate::ui::draw(f, app)).unwrap();
    t.backend().buffer().clone()
}
fn centre(r: Rect) -> (u16, u16) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

const W: u16 = 160;
const H: u16 = 48;

fn app() -> App {
    let mut a = App::new(None);
    render(&mut a, W, H);
    a
}

fn with_dirs() -> (App, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut a = App::new(None);
    a.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
    render(&mut a, W, H);
    (a, dir)
}

#[test]
fn a_title_press_without_movement_is_a_click() {
    let mut a = app();
    let before = a.layout.clone();
    let (x, y) = title_spot(&a, Widget::Queue);
    press(&mut a, x, y);
    release(&mut a, x + 1, y);
    assert_eq!(a.focused(), Some(Widget::Queue));
    assert_eq!(a.layout, before);
    assert!(a.tile.is_none() && a.naming().is_none());
}

#[test]
fn dragging_a_title_onto_another_pane_swaps_their_widgets_and_nothing_moves_meanwhile() {
    let mut a = app();
    let (stream, queue) = (pid(&a, Widget::Stream), pid(&a, Widget::Queue));
    let before = a.layout.clone();
    let (x, y) = title_spot(&a, Widget::Stream);
    press(&mut a, x, y);
    // the drag turns Arrange mode on: the Panes make room for the Tray once, as it starts (Q39)
    drag(&mut a, x + 4, y + 3);
    render(&mut a, W, H);
    let floor = drawn_panes(&a);
    let q = pane(&a, Widget::Queue);
    let (cx, cy) = centre(q);
    // on the way and over the queue: every Pane is drawn where it was, the queue is highlighted at once
    for (px, py) in [(x + 4, y + 3), (cx - 10, cy), (cx, cy)] {
        drag(&mut a, px, py);
        let shot = render(&mut a, W, H);
        assert_eq!(drawn_panes(&a), floor, "a Pane moved during the drag");
        assert!(shot.contains("⠿ stream"), "the chip follows the pointer");
        assert_eq!(a.layout, before, "nothing is committed yet");
    }
    assert_eq!(a.tile.as_ref().unwrap().over, Some(queue));
    let shot = render(&mut a, W, H);
    assert!(shot.contains(" swap with queue "), "{shot}");
    release(&mut a, cx, cy);
    assert!(a.tile.is_none());
    // the contents moved, the Panes stayed
    assert_eq!((a.layout.widget_in(queue), a.layout.widget_in(stream)), (Some(Widget::Stream), Some(Widget::Queue)));
    assert_eq!(crate::arrangement::outline(a.layout.plan()), crate::arrangement::outline(before.plan()));
    assert_eq!(a.focused(), Some(Widget::Stream), "the focus goes with the Widget");
    let after = render(&mut a, W, H);
    assert_eq!(drawn_panes(&a), floor);
    assert!(!after.contains('⠿') && !after.contains("swap with"), "it is down");
}

#[test]
fn a_drop_on_an_empty_pane_fills_it_and_leaves_its_own_empty() {
    let mut a = app();
    let area = a.pane_area();
    a.layout = a.layout.split(pid(&a, Widget::Queue), Side::Right, None, area).unwrap();
    render(&mut a, W, H);
    let empty = a.layout.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    let fft = pid(&a, Widget::Fft);
    let r = drawn_panes(&a).into_iter().find(|(p, _)| *p == empty).unwrap().1;
    let (x, y) = title_spot(&a, Widget::Fft);
    press(&mut a, x, y);
    drag(&mut a, centre(r).0, centre(r).1);
    assert!(render(&mut a, W, H).contains(" put here "));
    release(&mut a, centre(r).0, centre(r).1);
    assert_eq!((a.layout.widget_in(empty), a.layout.widget_in(fft)), (Some(Widget::Fft), None));
    // an empty Pane's title carries nothing: pressing and dragging it is no drag
    render(&mut a, W, H);
    let f = drawn_panes(&a).into_iter().find(|(p, _)| *p == fft).unwrap().1;
    let before = a.layout.clone();
    press(&mut a, f.x + 3, f.y);
    drag(&mut a, centre(r).0, centre(r).1);
    assert!(!a.tile.as_ref().is_some_and(|t| t.live));
    release(&mut a, centre(r).0, centre(r).1);
    assert_eq!(a.layout, before);
}


#[test]
fn esc_or_a_drop_on_its_own_pane_or_off_the_panes_leaves_the_layout_alone() {
    let mut a = app();
    let before = a.layout.clone();
    let q = pane(&a, Widget::Queue);
    let (x, y) = title_spot(&a, Widget::Stream);
    press(&mut a, x, y);
    drag(&mut a, centre(q).0, centre(q).1);
    assert!(a.tile.as_ref().unwrap().over.is_some());
    key(&mut a, KeyCode::Esc);
    assert!(a.tile.is_none());
    release(&mut a, centre(q).0, centre(q).1);
    assert_eq!(a.layout, before);
    // over its own Pane: nothing highlighted, and a release there is no drop
    press(&mut a, x, y);
    drag(&mut a, x + 5, y + 4);
    assert!(a.tile.as_ref().unwrap().over.is_none());
    let shot = render(&mut a, W, H);
    assert!(!shot.contains("swap with") && shot.contains("⠿ stream"), "{shot}");
    release(&mut a, x + 5, y + 4);
    assert_eq!(a.layout, before);
    // over the top bar or the legend: no Pane, no drop
    for out in [0, H - 1] {
        press(&mut a, x, y);
        drag(&mut a, centre(q).0, centre(q).1);
        drag(&mut a, centre(q).0, out);
        release(&mut a, centre(q).0, out);
        assert_eq!(a.layout, before, "let go at row {out}");
    }
    assert!(a.naming().is_none() && !a.history.can_undo());
}

#[test]
fn every_drop_is_allowed_even_into_a_pane_smaller_than_the_widget_likes() {
    let mut a = app();
    let before = a.layout.clone();
    // the gravity well (it asks for a shape and 5 rows) onto the small fft Pane: swapped, the floor plan untouched
    let (well, fft) = (pid(&a, Widget::Well), pid(&a, Widget::Fft));
    let f = pane(&a, Widget::Fft);
    let (x, y) = title_spot(&a, Widget::Well);
    press(&mut a, x, y);
    drag(&mut a, centre(f).0, centre(f).1);
    assert_eq!(a.tile.as_ref().unwrap().over, Some(fft));
    release(&mut a, centre(f).0, centre(f).1);
    assert_eq!((a.layout.widget_in(fft), a.layout.widget_in(well)), (Some(Widget::Well), Some(Widget::Fft)));
    assert_eq!(crate::arrangement::outline(a.layout.plan()), crate::arrangement::outline(before.plan()));
    assert!(a.layout.fits(a.pane_area()));
    render(&mut a, W, H);
}

#[test]
fn the_x_closes_a_pane_and_only_homes_has_none() {
    let mut a = app();
    let shot = render(&mut a, W, H);
    assert!(shot.contains(" × "));
    let closes = |a: &App, w: Widget| a.hits.iter().any(|(_, h)| matches!(h, Hit::Close(p) if *p == pid(a, w)));
    assert!(!closes(&a, Widget::Home), "Home has a ×");
    // D87: the library, the queue and lyrics may go (without a Pane, the first two open as popups)
    for w in [Widget::Library, Widget::Queue, Widget::Lyrics] {
        assert!(closes(&a, w), "{w} has no ×");
    }
    let fft = pane(&a, Widget::Fft);
    let old_loudness = pane(&a, Widget::Loudness);
    let (r, _) = a.hits.iter().find(|(_, h)| matches!(h, Hit::Close(p) if Some(*p) == a.layout.pane_of(Widget::Fft))).cloned().unwrap();
    assert!(r.y == fft.y && r.right() <= fft.right());
    press(&mut a, r.x + 1, r.y);
    release(&mut a, r.x + 1, r.y);
    assert!(!a.layout.has(Widget::Fft));
    render(&mut a, W, H);
    assert_eq!(pane(&a, Widget::Loudness).right(), fft.right(), "the neighbour took its room");
    assert!(pane(&a, Widget::Loudness).width > old_loudness.width);
    // Close refuses on Home with a notice
    a.focus_on(Widget::Home);
    a.run(Action::ClosePane);
    assert!(a.layout.has(Widget::Home) && a.toast.as_ref().is_some_and(|(t, _)| t.contains("never closed")), "{:?}", a.toast);
}

#[test]
fn splitting_makes_an_empty_pane_and_the_picker_fills_it() {
    let mut a = app();
    a.focus_on(Widget::Stream);
    let fft = pid(&a, Widget::Stream);
    a.run(Action::SplitRight);
    let new = a.widget_picker().map(|p| p.target).expect("the Widget picker opens");
    assert_ne!(new, fft);
    assert_eq!(a.layout.get(new).map(|p| p.widget), Some(None), "the new Pane is empty");
    assert_eq!(a.focus, Some(new));
    let shot = render(&mut a, W, H);
    assert!(shot.contains("Choose a widget"), "{shot}");
    // Scope isn't on screen: it's offered with the rest; picking it fills the new Pane
    let i = a.widget_choices().iter().position(|c| c.widget == Widget::Scope).expect("scope is offered");
    for _ in 0..i {
        key(&mut a, KeyCode::Down);
    }
    key(&mut a, KeyCode::Enter);
    assert!(a.widget_picker().is_none());
    assert_eq!(a.layout.widget_in(new), Some(Widget::Scope));
    render(&mut a, W, H);
    let (f, s) = (pane(&a, Widget::Stream), pane(&a, Widget::Scope));
    assert_eq!((f.y, f.height, f.right()), (s.y, s.height, s.x));
    assert_eq!(a.focused(), Some(Widget::Scope));
}

#[test]
fn a_split_with_no_room_says_so() {
    let mut a = App::new(None);
    render(&mut a, 50, 14);
    a.focus_on(Widget::Library);
    let before = a.layout.clone();
    // 50 columns: Singularity's four columns of 12 take 48 of them, no room for a fifth
    a.run(Action::SplitRight);
    assert!(a.widget_picker().is_none(), "no picker");
    assert_eq!(a.layout, before, "nothing changed");
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("No room for a new Pane right of")), "{:?}", a.toast);
}

#[test]
fn right_clicking_a_title_offers_replace_split_close_and_move() {
    let mut a = app();
    let (x, y) = title_spot(&a, Widget::Stream);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    let items: Vec<String> = a.menu().unwrap().items.iter().map(|(l, _)| l.clone()).collect();
    assert_eq!(items, ["Replace widget…", "Split  ←  →  ↑  ↓", "Close pane", "Arrange"]);
    let shot = render(&mut a, W, H);
    assert!(shot.contains("Split  ←  →  ↑  ↓"));
    let fft = pid(&a, Widget::Stream);
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Enter);
    let new = a.widget_picker().expect("the picker on the new Pane").target;
    render(&mut a, W, H);
    let r = |id: PaneId| drawn_panes(&a).into_iter().find(|(p, _)| *p == id).unwrap().1;
    assert_eq!(r(fft).right(), r(new).x, "Enter on the row splits right");
    key(&mut a, KeyCode::Esc);
    let (x, y) = title_spot(&a, Widget::Home);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    assert!(!a.menu().unwrap().items.iter().any(|(l, _)| l == "Close pane"), "Home can't close");
}

#[test]
fn edits_to_a_built_in_design_are_kept_as_edited_and_done_asks_for_a_name() {
    let (mut a, dir) = with_dirs();
    let layouts = dir.path().join("layouts");
    let (x, y) = title_spot(&a, Widget::Stream);
    let q = pane(&a, Widget::Queue);
    press(&mut a, x, y);
    drag(&mut a, centre(q).0, centre(q).1);
    release(&mut a, centre(q).0, centre(q).1);
    // never asked while editing (Q33): kept at once as the unnamed "(edited)", the built-in untouched
    assert!(a.naming().is_none() && a.arranging.is_some());
    let saved = crate::layouts::load_dir(&layouts).0;
    assert!(saved.len() == 1 && saved[0].edited && saved[0].name == "Singularity (edited)" && saved[0].arrangement() == a.layout, "{saved:?}");
    assert_eq!(a.layout_name.as_deref(), Some("singularity-edited"));
    // Done asks for a name, since something changed
    key(&mut a, KeyCode::Esc);
    assert!(a.arranging.is_none());
    let n = a.naming().cloned().expect("the naming prompt");
    assert_eq!((n.text.as_str(), n.purpose), ("Singularity 2", crate::app::NameFor::Fork));
    let shot = render(&mut a, W, H);
    assert!(shot.contains("Keep your changes as a design") && shot.contains("Singularity 2"), "{shot}");
    assert!(a.layout != Node::singularity(), "the edit shows meanwhile");
    // type a name over the suggestion
    for _ in 0..13 {
        key(&mut a, KeyCode::Backspace);
    }
    for c in "Mine".chars() {
        key(&mut a, KeyCode::Char(c));
    }
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout_name.as_deref(), Some("mine"));
    let text = std::fs::read_to_string(dir.path().join("layouts/mine.toml")).unwrap();
    assert!(text.contains("name = \"Mine\"") && text.contains("theme = \"singularity\"") && !text.contains("edited"), "{text}");
    assert!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap().contains("layout = \"mine\""));
    // the unnamed copy went into the named one: no second file
    assert!(!layouts.join("singularity-edited.toml").exists());
    // further edits save into the same file without asking, Done included
    render(&mut a, W, H);
    let lyrics = pane(&a, Widget::Lyrics);
    let (x, y) = title_spot(&a, Widget::Loudness);
    press(&mut a, x, y);
    drag(&mut a, centre(lyrics).0, centre(lyrics).1);
    release(&mut a, centre(lyrics).0, centre(lyrics).1);
    key(&mut a, KeyCode::Esc);
    assert!(a.naming().is_none() && a.arranging.is_none());
    let on_disk = crate::layouts::load_dir(&dir.path().join("layouts")).0;
    assert_eq!(on_disk.len(), 1);
    assert_eq!(on_disk[0].arrangement(), a.layout);
    // the built-in Design itself is untouched
    assert_eq!((DESIGNS[0].layout)(), Node::singularity());
    // and a restart brings it back
    let mut again = App::new(None);
    again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
    assert_eq!(again.layout, a.layout);
}

#[test]
fn a_name_already_taken_is_refused_and_suggestions_count_up() {
    let (mut a, dir) = with_dirs();
    a.save_layout("Singularity 2");
    a.reset_design();
    a.enter_arrange();
    a.layout = swapped(Node::singularity(), Widget::Stream, Widget::Queue);
    a.edited();
    a.arrange_done();
    assert_eq!(a.naming().unwrap().text, "Singularity 3", "2 is taken");
    for _ in 0..13 {
        key(&mut a, KeyCode::Backspace);
    }
    for c in "Singularity 2".chars() {
        key(&mut a, KeyCode::Char(c));
    }
    key(&mut a, KeyCode::Enter);
    assert!(a.naming().is_some(), "still asking");
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("already")));
    // "Singularity 2" and the unnamed edit
    assert_eq!(crate::layouts::load_dir(&dir.path().join("layouts")).0.len(), 2);
}

#[test]
fn quick_edits_are_kept_as_one_unnamed_edit_per_design_that_later_edits_overwrite() {
    let (mut a, dir) = with_dirs();
    let layouts = dir.path().join("layouts");
    a.layout = swapped(Node::singularity(), Widget::Stream, Widget::Queue);
    a.edited();
    assert!(a.naming().is_none(), "never asked outside Arrange mode");
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t == "Kept as Singularity (edited)"), "{:?}", a.toast);
    assert_eq!(a.layout_name.as_deref(), Some("singularity-edited"));
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert!(saved[0].edited && saved[0].name == "Singularity (edited)" && saved[0].arrangement() == a.layout);
    // more edits overwrite it, never a pile
    a.layout = swapped(a.layout.clone(), Widget::Home, Widget::Lyrics);
    a.edited();
    assert!(a.naming().is_none());
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].arrangement(), a.layout);
    // it is in the Design picker, and survives a restart
    a.run(Action::PickDesign);
    assert!(a.picker().unwrap().items.iter().any(|i| i.1 == "Singularity (edited)"));
    key(&mut a, KeyCode::Esc);
    let mut again = App::new(None);
    again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
    assert_eq!(again.layout, a.layout);
    // from the built-in Design again, an unnamed edit replaces it
    a.set_design(0);
    a.layout_name = None;
    a.layout = swapped(Node::singularity(), Widget::Fft, Widget::Loudness);
    a.edited();
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].arrangement(), a.layout);
    // another Design has its own
    a.set_design(3);
    a.layout_name = None;
    a.layout = swapped(Node::cockpit(), Widget::Fft, Widget::Loudness);
    a.edited();
    assert_eq!(crate::layouts::load_dir(&layouts).0.len(), 2);
}

#[test]
fn reset_design_goes_back_to_the_built_in_one() {
    let (mut a, dir) = with_dirs();
    let layouts = dir.path().join("layouts");
    a.layout = swapped(Node::singularity(), Widget::Stream, Widget::Queue);
    a.edited();
    a.run(Action::ResetLayout);
    assert_eq!(a.layout, Node::singularity());
    assert_eq!(a.layout_name, None);
    assert!(crate::layouts::load_dir(&layouts).0.is_empty(), "the unnamed edit is dropped");
    // a Design of your own stays where it was saved
    a.layout = swapped(Node::singularity(), Widget::Stream, Widget::Queue);
    a.save_layout("Mine");
    a.layout = swapped(a.layout.clone(), Widget::Home, Widget::Lyrics);
    a.edited();
    a.run(Action::ResetLayout);
    assert_eq!(a.layout, Node::singularity());
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("Mine") && t.contains("picker")), "{:?}", a.toast);
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert_ne!(saved[0].arrangement(), Node::singularity());
}

#[test]
fn letting_go_of_a_divider_is_an_edit() {
    let (mut a, _dir) = with_dirs();
    let grip = a.dividers[0].grip;
    press(&mut a, grip.x, grip.y + 5);
    drag(&mut a, grip.x + 8, grip.y + 5);
    assert!(a.layout_name.is_none(), "not while it moves");
    release(&mut a, grip.x + 8, grip.y + 5);
    // kept silently, with the bar's note (Q33)
    assert!(a.naming().is_none() && a.layout_name.as_deref() == Some("singularity-edited"));
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t == "Kept as Singularity (edited)"), "{:?}", a.toast);
}

#[test]
fn a_divider_squashes_the_well_like_any_pane_and_it_says_what_it_needs() {
    let mut a = app();
    // the divider below the well, dragged as far up as it goes: the well is a Pane like any other (D87, #48), down to
    // the Pane minimum, and below its smallest size it says `needs W × H`
    let well = pane(&a, Widget::Well);
    let d = a.dividers.iter().find(|d| d.grip.y + 1 == well.bottom() && d.axis == crate::layout::Axis::Vertical).cloned().unwrap();
    press(&mut a, d.grip.x + 3, d.grip.y);
    drag(&mut a, d.grip.x + 3, 0);
    release(&mut a, d.grip.x + 3, 0);
    let shot = render(&mut a, W, H);
    assert!(a.layout.valid(a.pane_area()));
    assert_eq!(pane(&a, Widget::Well).height, crate::layout::MIN_H);
    let (nw, nh) = crate::needs::smallest(Widget::Well);
    assert!(shot.contains(&format!("needs {nw} × {nh}")), "{shot}");
}

// ── the keyboard in Arrange mode (#43) ────────────────────────────────

fn shift(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::SHIFT));
}

#[test]
fn m_enters_arrange_mode_and_arrows_move_the_focus_between_panes() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    assert_eq!(a.modal(), Some(Modal::Arrange), "m enters Arrange mode");
    let before = a.layout.clone();
    // the queue sits under home
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Up);
    assert!(a.is_focused(Widget::Home));
    key(&mut a, KeyCode::Down);
    assert!(a.is_focused(Widget::Queue));
    // nothing that way: the focus stays
    a.focus_on(Widget::Lyrics);
    key(&mut a, KeyCode::Right);
    assert!(a.is_focused(Widget::Lyrics));
    // nothing focused: the arrows start from Home's Pane
    a.focus = None;
    key(&mut a, KeyCode::Down);
    assert!(a.is_focused(Widget::Queue));
    assert_eq!(a.layout, before, "arrows only move the focus");
    // the legend shows the keys
    let shot = render(&mut a, W, H);
    let legend = shot.lines().last().unwrap();
    for k in ["←↑↓→ Enter pick up", "t Tray", "s+arrow split", "x close", "+−[]size", "Esc done"] {
        assert!(legend.contains(k), "{k:?} in {legend:?}");
    }
}

#[test]
fn enter_picks_up_arrows_aim_and_enter_swaps_while_nothing_moves() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Queue);
    render(&mut a, W, H);
    let (floor, before) = (drawn_panes(&a), a.layout.clone());
    let (q, h) = (pane(&a, Widget::Queue), pane(&a, Widget::Home));
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.moving(), Some(pid(&a, Widget::Queue)), "its Pane is lit");
    let shot = render(&mut a, W, H);
    assert!(shot.contains("⠿ queue"), "the chip: {shot}");
    assert!(shot.lines().last().unwrap().contains("←↑↓→ aim · Enter drop · Esc put back"));
    // ↑ aims at home: lit, saying what Enter does; nothing moved
    key(&mut a, KeyCode::Up);
    let home = crate::ui::pane_name(a.design, Widget::Home);
    let shot = render(&mut a, W, H);
    assert!(shot.contains(&format!("swap with {home}")), "{shot}");
    assert_eq!((drawn_panes(&a), &a.layout), (floor, &before));
    // Enter drops: the two swap, the focus goes with the queue
    key(&mut a, KeyCode::Enter);
    render(&mut a, W, H);
    assert_eq!((pane(&a, Widget::Queue), pane(&a, Widget::Home)), (h, q));
    assert!(a.tile.is_none() && a.is_focused(Widget::Queue) && a.arranging.is_some());
    // one undo step
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, before);
}

#[test]
fn esc_puts_a_picked_up_widget_back_and_with_nothing_picked_up_is_done() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Queue);
    let before = a.layout.clone();
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Esc);
    assert!(a.tile.is_none() && a.arranging.is_some(), "Esc ends the pick-up, not Arrange mode");
    assert_eq!(a.layout, before);
    // aimed back at its own Pane: nothing lit, and Enter puts it back
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Down);
    assert!(a.drag_marks().is_some_and(|(target, _)| target.is_none()));
    key(&mut a, KeyCode::Enter);
    assert!(a.tile.is_none());
    assert_eq!(a.layout, before);
    // an empty Pane has nothing to pick up
    let mut e = app();
    key(&mut e, KeyCode::Char('m'));
    e.focus_on(Widget::Fft);
    key(&mut e, KeyCode::Char('t'));
    key(&mut e, KeyCode::Enter);
    assert!(e.tile.is_none());
    // Esc is Done
    key(&mut a, KeyCode::Esc);
    assert!(a.arranging.is_none() && a.modal().is_none());
}

#[test]
fn t_sends_the_focused_widget_to_the_tray_and_a_picked_up_one_fills_the_empty_pane() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Queue);
    let spot = pid(&a, Widget::Queue);
    key(&mut a, KeyCode::Char('t'));
    assert!(a.layout.tray().contains(&Widget::Queue) && a.layout.widget_in(spot).is_none());
    // Home picked up and dropped on the empty Pane below it: "put here", and it fills it
    key(&mut a, KeyCode::Up);
    assert!(a.is_focused(Widget::Home));
    let was = pid(&a, Widget::Home);
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Down);
    let shot = render(&mut a, W, H);
    assert!(shot.contains("put here"), "{shot}");
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout.pane_of(Widget::Home), Some(spot));
    assert!(a.layout.widget_in(was).is_none() && a.focus == Some(spot));
}

#[test]
fn s_then_an_arrow_splits_x_closes_and_plus_minus_move_the_line() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Queue);
    render(&mut a, W, H);
    // + (or =) grows, - shrinks: by its border left or right of it (2 columns); ] taller, [ shorter: by the one above
    // or below it (a row; the queue's Pane is in a stack)
    let (w0, h0) = (pane(&a, Widget::Queue).width, pane(&a, Widget::Queue).height);
    let size = |a: &mut App| {
        render(a, W, H);
        (pane(a, Widget::Queue).width, pane(a, Widget::Queue).height)
    };
    key(&mut a, KeyCode::Char('-'));
    assert_eq!(size(&mut a), (w0 - 2, h0));
    key(&mut a, KeyCode::Char('+'));
    key(&mut a, KeyCode::Char('='));
    assert_eq!(size(&mut a), (w0 + 2, h0));
    key(&mut a, KeyCode::Char('['));
    assert_eq!(size(&mut a), (w0 + 2, h0 - 1));
    key(&mut a, KeyCode::Char(']'));
    key(&mut a, KeyCode::Char(']'));
    assert_eq!(size(&mut a), (w0 + 2, h0 + 1));
    // s waits for the arrow (the legend says so); ↓ splits below: a new empty Pane and the Widget picker on it
    let (q, n) = (pane(&a, Widget::Queue), a.layout.panes().len());
    key(&mut a, KeyCode::Char('s'));
    assert_eq!(a.layout.panes().len(), n);
    let shot = render(&mut a, W, H);
    assert!(shot.lines().last().unwrap().contains("←↑↓→ split that way"));
    key(&mut a, KeyCode::Down);
    assert_eq!(a.layout.panes().len(), n + 1);
    let new = a.widget_picker().map(|p| p.target).unwrap();
    key(&mut a, KeyCode::Esc);
    assert!(a.widget_picker().is_none() && a.layout.widget_in(new).is_none() && a.arranging.is_some());
    render(&mut a, W, H);
    let r = a.layout.compute(a.pane_area()).rect(new).unwrap();
    assert!(r.y > pane(&a, Widget::Queue).y && r.x == q.x, "below the queue");
    // any other key after s ends the wait and does nothing else
    let before = a.layout.clone();
    a.focus_on(Widget::Fft);
    key(&mut a, KeyCode::Char('s'));
    key(&mut a, KeyCode::Char('x'));
    assert_eq!(a.layout, before);
    key(&mut a, KeyCode::Left);
    assert_eq!(a.layout, before, "the wait is over: an arrow moves the focus again");
    // x closes the focused Pane: its Widget goes to the Tray; Home's never closes
    a.focus_on(Widget::Fft);
    key(&mut a, KeyCode::Char('x'));
    assert!(!a.layout.panes().iter().any(|p| p.widget == Some(Widget::Fft)) && a.layout.tray().contains(&Widget::Fft));
    a.focus_on(Widget::Home);
    key(&mut a, KeyCode::Char('x'));
    assert!(a.layout.pane_of(Widget::Home).is_some());
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("never closed")), "{:?}", a.toast);
}

/// The cells of `s` where it is drawn in `b`, with their styles.
fn drawn(b: &Buffer, s: &str) -> Vec<(String, ratatui::style::Style)> {
    let n = s.chars().count() as u16;
    let at = (0..b.area.height).flat_map(|y| (0..b.area.width.saturating_sub(n)).map(move |x| (x, y))).find(|&(x, y)| row(b, y, x, x + n) == s);
    let (x, y) = at.unwrap_or_else(|| panic!("no {s:?}"));
    (x..x + n).map(|x| (b[(x, y)].symbol().to_string(), b[(x, y)].style())).collect()
}

/// The keyboard's pick-up looks like the mouse's: the same lit Pane, the same lit target and label, the same chip.
#[test]
fn the_keyboard_lights_the_same_panes_and_shows_the_same_chip_as_the_mouse() {
    let mut k = app();
    key(&mut k, KeyCode::Char('m'));
    k.focus_on(Widget::Queue);
    key(&mut k, KeyCode::Enter);
    key(&mut k, KeyCode::Up);
    let kb = render_buf(&mut k, W, H);
    let mut m = app();
    m.run(Action::Arrange);
    render(&mut m, W, H);
    let (x, y) = title_spot(&m, Widget::Queue);
    press(&mut m, x, y);
    let (cx, cy) = centre(pane(&m, Widget::Home));
    drag(&mut m, cx, cy - 4);
    let mb = render_buf(&mut m, W, H);
    assert_eq!(k.drag_marks(), m.drag_marks());
    assert_eq!(k.moving(), m.moving());
    // the held Pane, and the target's frame, cell for cell
    let cells = |b: &Buffer, r: Rect, frame: bool| -> Vec<(String, ratatui::style::Style)> {
        (r.y..r.bottom()).flat_map(|y| (r.x..r.right()).map(move |x| (x, y))).filter(|&(x, y)| !frame || x == r.x || y == r.y || x == r.right() - 1 || y == r.bottom() - 1).map(|p| (b[p].symbol().to_string(), b[p].style())).collect()
    };
    let (q, h) = (pane(&k, Widget::Queue), pane(&k, Widget::Home));
    assert_eq!(cells(&kb, q, false), cells(&mb, q, false), "the held Pane");
    assert_eq!(cells(&kb, h, true), cells(&mb, h, true), "the target's frame");
    // the label and the chip: the same text in the same colours
    let home = crate::ui::pane_name(k.design, Widget::Home);
    for s in [format!(" swap with {home} "), " ⠿ queue ".to_string()] {
        assert_eq!(drawn(&kb, &s), drawn(&mb, &s), "{s}");
    }
}

/// While arranging the arrange keys are Arrange mode's (no seeking, shuffling or volume on them), and the other
/// playback keys work as ever; Shift+arrows still seek.
#[test]
fn the_playback_keys_the_arrange_keys_leave_free_work_while_arranging() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let mut a = App::new(Some(tx));
    a.link = crate::app::Link::Connected { version: "test".into() };
    a.playback.item = Some(singularity_proto::Item { name: "Song".into(), duration_ms: 200_000, ..Default::default() });
    render(&mut a, W, H);
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Queue);
    a.playback.position_ms = 60_000;
    while rx.try_recv().is_ok() {}
    // (seeking and the volume are sent once they settle: what the bar would show says whether they moved)
    let (pos, vol) = (a.display_position(0), a.display_volume());
    for code in [KeyCode::Left, KeyCode::Right, KeyCode::Char('+'), KeyCode::Char('-'), KeyCode::Char('s'), KeyCode::Esc] {
        key(&mut a, code);
    }
    use singularity_proto::Command as C;
    let playback = |c: &C| matches!(c, C::TogglePlay | C::Next | C::SetShuffle { .. });
    let sent = std::iter::from_fn(|| rx.try_recv().ok()).filter(playback).collect::<Vec<_>>();
    assert!(sent.is_empty(), "no playback on the arrange keys: {sent:?}");
    assert_eq!((a.display_position(0), a.display_volume()), (pos, vol));
    for code in [KeyCode::Char(' '), KeyCode::Char('n')] {
        key(&mut a, code);
        assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|c| playback(&c)), "{code:?} works while arranging");
    }
    shift(&mut a, KeyCode::Right);
    assert_ne!(a.display_position(0), pos, "Shift+→ seeks");
    assert!(a.arranging.is_some());
}

/// With Home set aside and the focused Pane closed, the focus never names a Pane that isn't there: the keys go on
/// working from a real Pane (undo too).
#[test]
fn the_focus_always_lands_on_a_real_pane() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Home);
    key(&mut a, KeyCode::Char('t'));
    assert!(a.layout.tray().contains(&Widget::Home));
    a.focus_on(Widget::Fft);
    key(&mut a, KeyCode::Char('x'));
    assert!(a.focus.is_some_and(|p| a.layout.contains(p)), "{:?}", a.focus);
    for code in [KeyCode::Up, KeyCode::Down, KeyCode::Left] {
        key(&mut a, code);
        assert!(a.focus.is_some_and(|p| a.layout.contains(p)));
    }
    // a focus left on a Pane that's gone (another path): the next key acts from a real one
    a.focus = Some(PaneId::UNSET);
    key(&mut a, KeyCode::Enter);
    assert!(a.tile.as_ref().is_some_and(|t| a.layout.contains(t.pane.id)), "picked up from a real Pane");
    key(&mut a, KeyCode::Esc);
    // undo back past the close: still a real Pane
    ctrl(&mut a, 'z');
    ctrl(&mut a, 'z');
    assert!(a.focus.is_some_and(|p| a.layout.contains(p)));
}

/// The legend drops what doesn't fit from the mouse's gesture and undo / redo first: the keys stay longest.
#[test]
fn the_legend_keeps_the_keys_on_small_screens() {
    for (w, h, want) in [(100u16, 30u16, &["←↑↓→ Enter pick up", "t Tray", "s+arrow split", "x close", "Esc done"][..]), (90, 50, &["←↑↓→ Enter pick up", "t Tray", "s+arrow split", "x close", "Esc done"][..])] {
        let mut a = App::new(None);
        render(&mut a, w, h);
        key(&mut a, KeyCode::Char('m'));
        let shot = render(&mut a, w, h);
        let legend = shot.lines().last().unwrap();
        for k in want {
            assert!(legend.contains(k), "{w}x{h}: {k:?} in {legend:?}");
        }
        assert!(!legend.contains("drag a title"), "{w}x{h}: the gesture goes first");
    }
}

/// Anything else that edits the layout while a Widget is picked up with Enter puts it back.
#[test]
fn a_pick_up_ends_when_something_else_edits_the_layout() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Fft);
    key(&mut a, KeyCode::Enter);
    // Ctrl+K → Close pane, on the focused (picked-up) Pane
    a.run(Action::ClosePane);
    render(&mut a, W, H);
    assert!(a.tile.is_none() && a.drag_marks().is_none());
    // Replace widget… from the palette: the picker opens, and the pick-up is gone with it
    a.focus_on(Widget::Loudness);
    key(&mut a, KeyCode::Enter);
    a.run(Action::ReplaceWidget);
    let i = a.widget_choices().iter().position(|c| c.widget == Widget::Scope).unwrap();
    a.widget_picked(i);
    render(&mut a, W, H);
    assert!(a.tile.is_none());
    // an edit with no frame in between: the next key finds the pick-up gone, and drops nothing
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Up);
    let id = pid(&a, Widget::Stream);
    a.close_pane(id);
    let after = a.layout.clone();
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout, after, "the old pick-up didn't drop");
    assert!(a.tile.as_ref().is_some_and(|t| t.on.as_ref() == Some(&a.layout)), "Enter picked up afresh");
}

/// While the mouse holds a divider the arrange keys wait: Enter picks nothing up, and the release records the move.
#[test]
fn the_keys_wait_while_the_mouse_holds_a_divider() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    render(&mut a, W, H);
    let start = a.layout.clone();
    let grip = a.dividers[0].grip;
    press(&mut a, grip.x, grip.y + 5);
    drag(&mut a, grip.x + 6, grip.y + 5);
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Char('x'));
    assert!(a.tile.is_none() && a.drag.is_some(), "still the divider's");
    drag(&mut a, grip.x + 8, grip.y + 5);
    release(&mut a, grip.x + 8, grip.y + 5);
    assert!(a.drag.is_none() && a.layout != start);
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, start, "the move was recorded");
}

/// Only the bare keys arrange: Ctrl+X doesn't close, Alt+T doesn't set aside, Ctrl+Enter picks nothing up, Ctrl+=
/// doesn't grow; `s` waits for its arrow until a mouse press.
#[test]
fn only_the_bare_keys_arrange() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Fft);
    render(&mut a, W, H);
    let before = a.layout.clone();
    a.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
    a.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
    a.key(KeyEvent::new(KeyCode::Char('='), KeyModifiers::CONTROL));
    assert_eq!(a.layout, before);
    assert!(a.tile.is_none());
    // `+` as the terminal may send it, with its Shift: grows
    a.focus_on(Widget::Queue);
    shift(&mut a, KeyCode::Char('+'));
    assert_ne!(a.layout, before);
    // s, then a click: the wait is over, and → moves the focus instead of splitting
    let n = a.layout.panes().len();
    key(&mut a, KeyCode::Char('s'));
    let (x, y) = centre(pane(&a, Widget::Queue));
    press(&mut a, x, y);
    release(&mut a, x, y);
    key(&mut a, KeyCode::Right);
    assert_eq!(a.layout.panes().len(), n);
    assert!(a.widget_picker().is_none());
}

/// The Tray by keyboard (owner, Q50): ↓ from a bottom Pane onto the Tray, ← → along it, Enter picks a Widget up
/// (the chip), ↑ aims into the Panes, Enter drops there; ↑ with nothing picked up goes back to the Panes.
#[test]
fn the_keyboard_takes_widgets_out_of_the_tray() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    // an empty Tray: ↓ from a bottom Pane goes to its "+ Widget" (#42), ↑ back
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Down);
    assert!(a.focus.is_none() && a.arranging.as_ref().unwrap().tray_focus == Some(crate::arrange::TrayStop::Add));
    key(&mut a, KeyCode::Up);
    assert!(a.focused().is_some() && a.arranging.as_ref().unwrap().tray_focus.is_none());
    for w in [Widget::Fft, Widget::Loudness] {
        a.focus_on(w);
        key(&mut a, KeyCode::Char('t'));
    }
    assert_eq!(a.layout.tray(), &[Widget::Fft, Widget::Loudness]);
    // ↓ from the queue: onto the Tray (its nearest Widget), lit; no Pane focused meanwhile
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Down);
    let on = |a: &App| match a.arranging.as_ref().unwrap().tray_focus {
        Some(crate::arrange::TrayStop::Widget(w)) => Some(w),
        _ => None,
    };
    assert!(on(&a).is_some() && a.focus.is_none());
    key(&mut a, KeyCode::Left);
    assert_eq!(on(&a), Some(Widget::Fft));
    key(&mut a, KeyCode::Left);
    assert_eq!(on(&a), Some(Widget::Fft), "nothing before it");
    key(&mut a, KeyCode::Right);
    assert_eq!(on(&a), Some(Widget::Loudness));
    let buf = render_buf(&mut a, W, H);
    let name = crate::ui::pane_name(a.design, Widget::Loudness);
    let lit = drawn(&buf, &format!(" ⠿ {name} "));
    assert_eq!(lit[1].1.bg, a.theme.chip(a.theme.accent).bg, "the focused one in the focus colour");
    let other = drawn(&buf, &format!(" ⠿ {} ", crate::ui::pane_name(a.design, Widget::Fft)));
    assert_eq!(other[1].1.bg, a.theme.chip(a.theme.accent2).bg, "the others as before");
    // the keys for a Pane do nothing here
    let before = a.layout.clone();
    for c in ['t', 's', '+', '-'] {
        key(&mut a, KeyCode::Char(c));
    }
    assert_eq!(a.layout, before);
    // Enter picks it up (the chip), Esc puts it back and the focus stays in the Tray
    key(&mut a, KeyCode::Enter);
    assert!(render(&mut a, W, H).contains(&format!("⠿ {name}")));
    key(&mut a, KeyCode::Esc);
    assert!(a.tile.is_none() && on(&a) == Some(Widget::Loudness) && a.arranging.is_some());
    // picked up again, ↑ aims at the Pane above it, Enter puts it there: that Pane's Widget goes into the Tray
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Up);
    let to = a.tile.as_ref().and_then(|t| t.over).expect("aimed at a Pane");
    let was = a.layout.widget_in(to);
    let label = match was {
        Some(w) => format!("swap with {}", crate::ui::pane_name(a.design, w)),
        None => "put here".into(),
    };
    assert!(render(&mut a, W, H).contains(&label), "{label}");
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout.widget_in(to), Some(Widget::Loudness));
    assert!(was.is_none_or(|w| a.layout.tray().contains(&w)) && !a.layout.tray().contains(&Widget::Loudness));
    assert!(a.focus == Some(to) && on(&a).is_none() && a.tile.is_none());
    // ↓ to the Tray and ↑ back: the focus is on a bottom Pane again
    key(&mut a, KeyCode::Down);
    assert!(on(&a).is_some());
    key(&mut a, KeyCode::Up);
    assert!(on(&a).is_none() && a.focus.is_some_and(|p| a.layout.contains(p)));
}

/// A Pane's Widget picked up and aimed down past the bottom Pane: the Tray lights up, and Enter sets it aside there.
#[test]
fn a_picked_up_widget_aimed_down_onto_the_tray_waits_there() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Queue);
    let spot = pid(&a, Widget::Queue);
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Down);
    assert!(a.tile.as_ref().is_some_and(|t| t.tray && t.over.is_none()));
    assert!(render(&mut a, W, H).contains("set aside here"));
    // ↑ aims back into the Panes, ↓ at the Tray again
    key(&mut a, KeyCode::Up);
    assert!(a.tile.as_ref().is_some_and(|t| !t.tray));
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter);
    assert!(a.layout.tray().contains(&Widget::Queue) && a.layout.widget_in(spot).is_none());
    assert!(a.tile.is_none() && a.focus == Some(spot));
}

/// `move_pane` in keys.toml still binds: it is another name for `arrange` (owner, Q51).
#[test]
fn move_pane_is_another_name_for_arrange() {
    assert_eq!(Action::from_id("move_pane"), Some(Action::Arrange));
    assert_eq!(Action::from_id("arrange"), Some(Action::Arrange));
}

/// A Widget picked up with Enter isn't the pointer's: a bare move leaves it be, a release doesn't drop it, a press
/// puts it back (and does what it does).
#[test]
fn the_pointer_leaves_a_picked_up_widget_alone_until_a_press() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Queue);
    let before = a.layout.clone();
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Up);
    let (x, y) = centre(pane(&a, Widget::Lyrics));
    mouse(&mut a, MouseEventKind::Moved, x, y);
    release(&mut a, x, y);
    assert!(a.tile.as_ref().is_some_and(|t| t.aim.is_some()), "still picked up");
    assert!(!a.moving_now(), "nothing follows the pointer");
    press(&mut a, x, y);
    assert!(a.tile.is_none() && a.is_focused(Widget::Lyrics));
    assert_eq!(a.layout, before);
}

/// Random mouse and key gestures on every Design at several sizes: whatever happens, the layout on screen stays
/// valid and nothing panics.
#[test]
fn random_gestures_leave_every_design_valid() {
    let mut n = 0u64;
    let mut next = move || {
        n = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = n;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for (d, design) in DESIGNS.iter().enumerate() {
        for (w, h) in [(100u16, 30u16), (160, 48), (220, 60)] {
            let mut a = App::new(None);
            a.set_design(d);
            render(&mut a, w, h);
            for _ in 0..120 {
                let slots: Vec<Widget> = a.layout.widgets();
                // (every Widget set aside in the Tray: back a step)
                if slots.is_empty() {
                    ctrl(&mut a, 'z');
                    continue;
                }
                let s = slots[(next() % slots.len() as u64) as usize];
                match next() % 8 {
                    6 => ctrl(&mut a, 'z'),
                    7 => ctrl(&mut a, 'y'),
                    0..=2 => {
                        render(&mut a, w, h);
                        let (x, y) = title_spot(&a, s);
                        let unpressed = drawn_panes(&a);
                        press(&mut a, x, y);
                        render(&mut a, w, h);
                        assert_eq!(drawn_panes(&a), unpressed, "{} {w}x{h}: a press moved a Pane", design.id);
                        // (once the drag is on, Arrange mode is: the Panes made room for the Tray then, Q39; until
                        // then nothing moves)
                        let mut floor = None;
                        for _ in 0..3 {
                            drag(&mut a, (next() % w as u64) as u16, (next() % (h - 1) as u64) as u16);
                            render(&mut a, w, h);
                            let now = drawn_panes(&a);
                            assert_eq!(now, laid_out(&a), "{} {w}x{h}: the Panes aren't the floor plan in their area", design.id);
                            if a.tile.as_ref().is_some_and(|t| t.live) {
                                assert!(a.arranging.is_some());
                                assert_eq!(floor.get_or_insert_with(|| now.clone()), &now, "{} {w}x{h}: a Pane moved mid-drag", design.id);
                            } else {
                                assert_eq!(now, unpressed, "{} {w}x{h}: a press without a drag moved a Pane", design.id);
                            }
                        }
                        if next() % 4 == 0 {
                            key(&mut a, KeyCode::Esc);
                        }
                        let (x, y) = ((next() % w as u64) as u16, (next() % (h - 4) as u64) as u16);
                        release(&mut a, x, y);
                    }
                    3 => {
                        a.focus_on(s);
                        key(&mut a, KeyCode::Char('m'));
                        for _ in 0..6 {
                            let code = [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down, KeyCode::Char('+'), KeyCode::Char('-'), KeyCode::Enter, KeyCode::Char('t'), KeyCode::Char('x')][(next() % 9) as usize];
                            key(&mut a, code);
                        }
                        key(&mut a, KeyCode::Esc);
                        key(&mut a, KeyCode::Esc);
                    }
                    4 => {
                        a.focus_on(s);
                        a.run([Action::SplitLeft, Action::SplitRight, Action::SplitUp, Action::SplitDown][(next() % 4) as usize]);
                        let n = a.widget_choices().len() as u64;
                        // a pick, or Esc: the new Pane stays empty
                        if a.widget_picker().is_some() && n > 0 && next() % 3 != 0 {
                            a.widget_picked((next() % n) as usize);
                        } else {
                            key(&mut a, KeyCode::Esc);
                        }
                    }
                    _ => {
                        a.focus_on(s);
                        a.close_pane(pid(&a, s));
                    }
                }
                render(&mut a, w, h);
                let body = a.pane_area();
                assert!(drawn_panes(&a).iter().all(|(_, r)| r.width >= 1 && r.height >= 1 && r.x >= body.x && r.y >= body.y && r.right() <= body.right() && r.bottom() <= body.bottom()), "{} {w}x{h}", design.id);
                let tree = a.layout.clone();
                assert!(tree.fits(a.pane_area()), "{} {w}x{h}: {tree:?}", design.id);
                // (Home may wait in the Tray, while Arrange mode is on)
                assert!(crate::arrangement::required_widgets().all(|c| tree.has(c) || tree.tray().contains(&c) && a.arranging.is_some()));
                assert!(a.tile.is_none() && a.widget_picker().is_none());
            }
        }
    }
}


// ── the chip and the highlight (D87) ──────────────────────────────────

fn ctrl(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

#[test]
fn the_chip_follows_the_pointer_and_the_highlight_says_what_a_release_does() {
    let mut a = app();
    let (x, y) = title_spot(&a, Widget::Stream);
    press(&mut a, x, y);
    let q = pane(&a, Widget::Queue);
    let (cx, cy) = centre(q);
    drag(&mut a, cx, cy);
    let buf = render_buf(&mut a, W, H);
    // the chip: one row, just right of the pointer, the Widget's name with its grip, at most 20 columns
    let chip = row(&buf, cy, cx + 2, (cx + 2 + crate::arrange::CHIP_MAX).min(W));
    assert!(chip.starts_with(" ⠿ stream "), "{chip:?}");
    assert_eq!(buf[(cx + 3, cy)].bg, a.theme.accent2.color());
    assert_ne!(buf[(cx + 3, cy - 1)].bg, a.theme.accent2.color(), "one row");
    // the target: its frame lit in the accent, the label in its middle
    assert_eq!(buf[(q.x, q.y + 2)].fg, a.theme.accent.color());
    let label = (q.y..q.bottom()).find(|&y| row(&buf, y, q.x, q.right()).contains(" swap with queue ")).expect("the label");
    assert!(label != cy && label.abs_diff(q.y + q.height / 2) <= 2, "near the middle, clear of the chip on the pointer's row");
    // by the screen's right edge the chip stops there (the pointer goes on over it), never off screen
    drag(&mut a, W - 1, cy);
    let buf = render_buf(&mut a, W, H);
    let r = a.chip_rect(Rect::new(0, 0, W, H), "⠿ stream").unwrap();
    assert!(r.right() == W && r.width <= crate::arrange::CHIP_MAX && r.height == 1, "{r:?}");
    assert!(row(&buf, cy, r.x, r.right()).contains("⠿ stream"));
    key(&mut a, KeyCode::Esc);
    release(&mut a, W - 1, cy);
    // a long name is cut to fit: never more than 20 columns
    let mut b = app();
    b.set_design(crate::design::by_id("cockpit").unwrap());
    render(&mut b, W, H);
    let (x, y) = title_spot(&b, Widget::Listening);
    press(&mut b, x, y);
    drag(&mut b, 40, 30);
    let buf = render_buf(&mut b, W, H);
    let r = b.chip_rect(Rect::new(0, 0, W, H), &b.drag_marks().unwrap().1).unwrap();
    assert_eq!(r.width, crate::arrange::CHIP_MAX);
    let text = row(&buf, r.y, r.x, r.right());
    assert!(text.starts_with(" ⠿ listening") && text.ends_with('…'), "{text:?}");
}

// ── undo ──────────────────────────────────────────────────────────────

#[test]
fn ctrl_z_undoes_a_drop_and_ctrl_y_redoes_it_and_the_bar_offers_it() {
    let mut a = app();
    let before = a.layout.clone();
    let (x, y) = title_spot(&a, Widget::Stream);
    let q = pane(&a, Widget::Queue);
    press(&mut a, x, y);
    drag(&mut a, centre(q).0, centre(q).1);
    release(&mut a, centre(q).0, centre(q).1);
    let after = a.layout.clone();
    assert_ne!(after, before);
    // in Arrange mode the legend says it; Done, and the bar offers it
    assert!(render(&mut a, W, H).lines().last().unwrap().contains("^Z undo"));
    key(&mut a, KeyCode::Esc);
    let shot = render(&mut a, W, H);
    assert!(shot.lines().last().unwrap().contains("↶ undo · ^Z"), "{}", shot.lines().last().unwrap());
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, before);
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("Undone · ^Y redoes")), "{:?}", a.toast);
    // after an undo the bar offers the redo
    assert!(render(&mut a, W, H).lines().last().unwrap().contains("↷ redo · ^Y"));
    ctrl(&mut a, 'y');
    assert_eq!(a.layout, after);
    // Ctrl+Shift+Z redoes too, where the terminal tells it apart
    ctrl(&mut a, 'z');
    a.key(KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT));
    assert_eq!(a.layout, after);
    // nothing more to redo
    ctrl(&mut a, 'y');
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t == "Nothing to redo"));
    // the bar's chip is a button
    render(&mut a, W, H);
    let (r, _) = a.hits.iter().find(|(_, h)| *h == Hit::Action(Action::UndoLayout)).cloned().expect("the undo chip");
    press(&mut a, r.x + 1, r.y);
    release(&mut a, r.x + 1, r.y);
    assert_eq!(a.layout, before);
    // the palette has both
    a.run(Action::Palette);
    for c in "undo lay".chars() {
        key(&mut a, KeyCode::Char(c));
    }
    assert_eq!(a.palette_items().first(), Some(&Action::UndoLayout));
    key(&mut a, KeyCode::Esc);
    assert!(Action::UndoLayout.label().len() <= 28 && Action::RedoLayout.label().len() <= 28);
}

#[test]
fn every_kind_of_edit_is_one_undo_step() {
    let mut a = app();
    let start = a.layout.clone();
    let mut trees = vec![start.clone()];
    // close
    a.close_pane(pid(&a, Widget::Fft));
    trees.push(a.layout.clone());
    // split (an empty Pane) and the Widget picked for it: one step
    a.focus_on(Widget::Loudness);
    a.run(Action::SplitRight);
    let i = a.widget_choices().iter().position(|c| c.widget == Widget::Scope).unwrap();
    a.widget_picked(i);
    trees.push(a.layout.clone());
    // Arrange mode's keys: a pick-up dropped, a size step, to the Tray
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Char('m'));
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Enter);
    trees.push(a.layout.clone());
    key(&mut a, KeyCode::Char('-'));
    trees.push(a.layout.clone());
    key(&mut a, KeyCode::Char('t'));
    trees.push(a.layout.clone());
    key(&mut a, KeyCode::Esc);
    // a divider dragged and let go
    render(&mut a, W, H);
    let grip = a.dividers[0].grip;
    press(&mut a, grip.x, grip.y + 5);
    drag(&mut a, grip.x + 6, grip.y + 5);
    drag(&mut a, grip.x + 8, grip.y + 5);
    release(&mut a, grip.x + 8, grip.y + 5);
    trees.push(a.layout.clone());
    // Reset design
    a.run(Action::ResetLayout);
    trees.push(a.layout.clone());
    assert_eq!(a.layout, start);
    for (n, t) in trees.windows(2).rev().enumerate() {
        assert_ne!(t[0], t[1], "step {n} changed something");
        ctrl(&mut a, 'z');
        assert_eq!(a.layout, t[0], "undo {n}");
    }
    ctrl(&mut a, 'z');
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t == "Nothing to undo"));
    for t in &trees[1..] {
        ctrl(&mut a, 'y');
        assert_eq!(&a.layout, t);
    }
}

#[test]
fn undo_of_the_first_edit_of_a_built_in_design_goes_back_to_it_and_leaves_no_file() {
    let (mut a, dir) = with_dirs();
    let layouts = dir.path().join("layouts");
    let files = || crate::layouts::load_dir(&layouts).0;
    let built_in = a.layout.clone();
    let swap = |a: &mut App| {
        render(a, W, H);
        let (x, y) = title_spot(a, Widget::Stream);
        let q = pane(a, Widget::Queue);
        press(a, x, y);
        drag(a, centre(q).0, centre(q).1);
        release(a, centre(q).0, centre(q).1);
    };
    // 1. undone with Done's name prompt up: the prompt goes with the edit, and so does the "(edited)" it was kept as
    swap(&mut a);
    let edit = a.layout.clone();
    assert_eq!(files().len(), 1);
    key(&mut a, KeyCode::Esc);
    assert!(a.naming().is_some());
    ctrl(&mut a, 'z');
    assert!(a.naming().is_none() && a.layout == built_in && a.layout_name.is_none() && files().is_empty());
    ctrl(&mut a, 'y');
    assert_eq!(a.layout, edit);
    assert!(a.naming().is_none(), "a redo doesn't ask");
    // 2. kept as "Singularity (edited)": undo removes that file again and the config forgets it; redo brings it back
    assert_eq!(files().len(), 1);
    render(&mut a, W, H);
    ctrl(&mut a, 'z');
    assert!(a.layout == built_in && a.layout_name.is_none() && files().is_empty(), "{:?}", files());
    assert!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap().contains("layout = \"\""));
    ctrl(&mut a, 'y');
    assert_eq!(a.layout_name.as_deref(), Some("singularity-edited"));
    let f = files();
    assert!(f.len() == 1 && f[0].edited && f[0].arrangement() == edit);
    // 3. a name given: undo goes back to the built-in Design, and the named one stays in the picker as saved
    ctrl(&mut a, 'z');
    swap(&mut a);
    key(&mut a, KeyCode::Esc);
    for _ in 0..13 {
        key(&mut a, KeyCode::Backspace);
    }
    for c in "Mine".chars() {
        key(&mut a, KeyCode::Char(c));
    }
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout_name.as_deref(), Some("mine"));
    render(&mut a, W, H);
    ctrl(&mut a, 'z');
    assert!(a.layout == built_in && a.layout_name.is_none());
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("\"Mine\" is still in the Design picker")), "{:?}", a.toast);
    let f = files();
    assert!(f.len() == 1 && f[0].id == "mine" && f[0].arrangement() == edit);
}

#[test]
fn undo_in_a_design_of_your_own_rewrites_its_file() {
    let (mut a, dir) = with_dirs();
    let layouts = dir.path().join("layouts");
    a.layout = swapped(Node::singularity(), Widget::Stream, Widget::Queue);
    a.save_layout("Mine");
    let saved = a.layout.clone();
    a.close_pane(pid(&a, Widget::Fft));
    let closed = a.layout.clone();
    assert_eq!(crate::layouts::load_dir(&layouts).0[0].arrangement(), closed, "it saves itself");
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, saved);
    let f = crate::layouts::load_dir(&layouts).0;
    assert!(f.len() == 1 && f[0].arrangement() == saved && f[0].name == "Mine" && !f[0].edited, "the file is back too");
    ctrl(&mut a, 'y');
    assert_eq!(crate::layouts::load_dir(&layouts).0[0].arrangement(), closed);
    // undoing a Reset puts the Design of your own back on screen, and its file
    a.run(Action::ResetLayout);
    assert!(a.layout_name.is_none());
    ctrl(&mut a, 'z');
    assert_eq!(a.layout_name.as_deref(), Some("mine"));
    assert_eq!(a.layout, closed);
    assert!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap().contains("layout = \"mine\""));
    // another Design picked: the history was the last one's
    a.run(Action::PickDesign);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Enter);
    ctrl(&mut a, 'z');
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t == "Nothing to undo"), "{:?}", a.toast);
}

#[test]
fn an_undo_that_would_not_fit_the_window_is_refused() {
    // Home split twice: three columns where Singularity has one; one closed, the window shrinks, and the undo back
    // to three wouldn't fit
    let mut a = app();
    let area = a.pane_area();
    let home = pid(&a, Widget::Home);
    a.layout = a.layout.split(home, Side::Right, Some(Widget::Scope), area).unwrap().split(home, Side::Right, Some(Widget::Rain), area).unwrap();
    let three = a.layout.clone();
    a.close_pane(pid(&a, Widget::Rain));
    let two = a.layout.clone();
    let (w, h) = (50..160u16).map(|w| (w, 30u16)).find(|&(w, h)| {
        let body = Rect::new(0, 1, w, h - 5);
        !three.fits(body) && two.fits(body)
    }).expect("such a size");
    render(&mut a, w, h);
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, two);
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("doesn't fit this window")), "{:?}", a.toast);
    // back at a size it fits, it goes
    render(&mut a, W, H);
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, three);
}


// ── fast drags (owner, 2026-11-01: "it looks a bit like teleporting") ──

/// No dwell: a sweep across Panes highlights each the moment the pointer is over it, rearranges nothing, and a
/// release drops on the one under the pointer then.
#[test]
fn a_fast_sweep_highlights_each_pane_at_once_and_drops_on_the_last() {
    // (arranging already, so the Panes are where they stay for the whole drag)
    let arranging = || {
        let mut a = app();
        a.run(Action::Arrange);
        render(&mut a, W, H);
        a
    };
    let mut a = arranging();
    let before = a.layout.clone();
    let floor = drawn_panes(&a);
    let (x, y) = title_spot(&a, Widget::Stream);
    press(&mut a, x, y);
    let names = [Widget::Queue, Widget::Home, Widget::Library, Widget::Lyrics, Widget::Fft];
    let spots: Vec<(u16, u16)> = names.iter().map(|s| centre(pane(&a, *s))).collect();
    for (i, &(px, py)) in spots.iter().enumerate() {
        drag(&mut a, px, py);
        assert_eq!(a.tile.as_ref().unwrap().over, Some(pid(&a, names[i])), "{}", names[i]);
        let shot = render(&mut a, W, H);
        assert_eq!(drawn_panes(&a), floor, "nothing rearranges");
        assert!(shot.contains(&format!(" swap with {} ", crate::ui::pane_name(0, names[i]))), "{}", names[i]);
    }
    // a flick on to the queue and let go there at once: the queue it is
    let q = spots[0];
    drag(&mut a, q.0, q.1);
    release(&mut a, q.0, q.1);
    assert_eq!(a.layout, swapped(before.clone(), Widget::Stream, Widget::Queue));
    // a release on another cell than the last move (a finger slipping as it lifts): the Pane under the release
    let mut b = arranging();
    let (x, y) = title_spot(&b, Widget::Stream);
    press(&mut b, x, y);
    drag(&mut b, q.0, q.1);
    let l = centre(pane(&b, Widget::Library));
    release(&mut b, l.0, l.1);
    assert_eq!(b.layout, swapped(before, Widget::Stream, Widget::Library));
}

#[test]
fn the_frame_rate_is_the_motion_rate_while_anything_moves() {
    let mut a = app();
    a.config.visuals.unfocused_fps = 10;
    assert_eq!((a.frame_rate(true), a.frame_rate(false)), (30, 10), "idle: as before");
    let (x, y) = title_spot(&a, Widget::Stream);
    press(&mut a, x, y);
    assert_eq!(a.frame_rate(true), 30, "a press is no drag yet");
    drag(&mut a, x + 10, y + 5);
    assert_eq!((a.frame_rate(true), a.frame_rate(false)), (120, 120), "held: the motion rate, focused or not");
    for (set, got) in [(240, 240), (144, 144), (1000, 240), (5, 30)] {
        a.config.visuals.motion_fps = set;
        assert_eq!(a.frame_rate(true), got);
    }
    a.config.visuals.motion_fps = 240;
    // let go: nothing lands or springs, the steady rate at once
    release(&mut a, x + 10, y + 5);
    render(&mut a, W, H);
    assert_eq!(a.frame_rate(true), 30);
    // a divider being dragged moves with the pointer too
    let grip = a.dividers[0].grip;
    press(&mut a, grip.x, grip.y);
    assert_eq!(a.frame_rate(true), 240);
    release(&mut a, grip.x, grip.y);
    assert_eq!(a.frame_rate(true), 30);
}

#[test]
fn a_short_wide_pane_is_swapped_with_wherever_the_pointer_is_on_it() {
    // the instruments in a short row under Home: fft dropped anywhere on loudness swaps the two
    use crate::layout::{Axis::*, Size::*};
    let mut a = app();
    let s = Node::pane;
    let row = Node::split(Horizontal, Ratio(0.5), s(Widget::Fft), s(Widget::Loudness));
    a.layout = Node::split(Horizontal, Cells(30), s(Widget::Library), Node::split(Vertical, Ratio(0.7), Node::split(Vertical, Ratio(0.6), s(Widget::Home), s(Widget::Queue)), Node::split(Horizontal, Ratio(0.8), row, s(Widget::Lyrics)))).into();
    // (arranging already, so the Panes are where they stay for every drag)
    a.run(Action::Arrange);
    render(&mut a, W, H);
    let before = a.layout.clone();
    let l = pane(&a, Widget::Loudness);
    for (px, py) in [(l.x, l.y), (l.x + l.width / 2, l.bottom() - 1), (l.right() - 1, l.y + l.height / 2)] {
        let (x, y) = title_spot(&a, Widget::Fft);
        press(&mut a, x, y);
        drag(&mut a, px, py);
        assert_eq!(a.tile.as_ref().unwrap().over, Some(pid(&a, Widget::Loudness)), "({px}, {py}) in {l:?}");
        key(&mut a, KeyCode::Esc);
        release(&mut a, px, py);
    }
    let (x, y) = title_spot(&a, Widget::Fft);
    press(&mut a, x, y);
    drag(&mut a, l.x + l.width / 2, l.y);
    release(&mut a, l.x + l.width / 2, l.y);
    assert_eq!(a.layout, swapped(before.clone(), Widget::Fft, Widget::Loudness));
}

/// The owner (2026-11-02): "I can't drag the lyrics pane to take up a bit of Home, it snaps right back". Sizes are
/// changed by the border: it lights up under the pointer, asks for a resize pointer, and dragging it resizes.
#[test]
fn the_border_lights_up_under_the_pointer_and_dragging_it_resizes() {
    use crate::layout::{Axis, Size};
    let mut a = app();
    a.layout = Node::split(Axis::Horizontal, Size::Ratio(0.5), Node::pane(Widget::Home), Node::pane(Widget::Lyrics)).into();
    render(&mut a, W, H);
    let home = pane(&a, Widget::Home);
    // the border between them: lit while the pointer is on it, with a left-right resize pointer
    let (gx, gy) = (home.right() - 1, home.y + home.height / 2);
    mouse(&mut a, MouseEventKind::Moved, gx, gy);
    assert_eq!(a.pointer_shape(), "ew-resize");
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| crate::ui::draw(f, &mut a)).unwrap();
    assert_eq!(t.backend().buffer()[(gx, gy)].fg, a.theme.accent.color(), "the grabbable border is lit");
    mouse(&mut a, MouseEventKind::Moved, home.x + 5, gy);
    assert_eq!(a.pointer_shape(), "", "off the border: the terminal's own pointer");
    // and dragging it makes Home narrower
    press(&mut a, gx, gy);
    drag(&mut a, gx - 20, gy);
    release(&mut a, gx - 20, gy);
    render(&mut a, W, H);
    assert_eq!(pane(&a, Widget::Home).width, home.width - 20);
}

/// The owner (2026-11-05): "the cursor bugging out and having the arrow left, dot in the middle and arrow right like
/// for resizing always". Ghostty ignores OSC 22 with an empty name, so putting the pointer back sends `text` (its
/// own) too; and the resize pointer goes away whenever the pointer is off the border, however it got there.
#[test]
fn the_resize_pointer_goes_back_off_the_border_in_every_terminal() {
    use crate::layout::{Axis, Size};
    // the reset names a shape Ghostty knows, then the empty name kitty and foot reset on
    assert_eq!(crate::arrange::pointer_shape_seq(""), "\x1b]22;default\x1b\\");
    assert_eq!(crate::arrange::pointer_shape_seq("ew-resize"), "\x1b]22;ew-resize\x1b\\");
    let mut a = app();
    a.layout = Node::split(Axis::Horizontal, Size::Ratio(0.5), Node::pane(Widget::Home), Node::pane(Widget::Lyrics)).into();
    render(&mut a, W, H);
    let home = pane(&a, Widget::Home);
    let (gx, gy) = (home.right() - 1, home.y + home.height / 2);
    let on = |a: &mut App| {
        mouse(a, MouseEventKind::Moved, gx, gy);
        assert_eq!(a.pointer_shape(), "ew-resize");
    };
    // a border dragged and let go away from it (no move after the release)
    on(&mut a);
    press(&mut a, gx, gy);
    drag(&mut a, gx - 10, gy);
    release(&mut a, gx - 10, gy);
    render(&mut a, W, H);
    assert_eq!(a.pointer_shape(), "", "let go off the border");
    // a click elsewhere straight after hovering it (a terminal that sent no move in between)
    render(&mut a, W, H);
    let home = pane(&a, Widget::Home);
    let (gx, gy) = (home.right() - 1, home.y + home.height / 2);
    mouse(&mut a, MouseEventKind::Moved, gx, gy);
    assert_eq!(a.pointer_shape(), "ew-resize");
    mouse(&mut a, MouseEventKind::ScrollDown, home.x + 3, gy);
    assert_eq!(a.pointer_shape(), "", "the wheel elsewhere");
    // the window loses focus, or the terminal is resized, with the pointer on the border
    mouse(&mut a, MouseEventKind::Moved, gx, gy);
    a.pointer_gone();
    assert_eq!(a.pointer_shape(), "", "focus lost / resized");
    // the idle takeover woken by a move off the border
    mouse(&mut a, MouseEventKind::Moved, gx, gy);
    a.idle_since = Some(std::time::Instant::now());
    mouse(&mut a, MouseEventKind::Moved, home.x + 3, gy);
    assert_eq!(a.pointer_shape(), "", "a waking move off the border");
}

/// A whole new tree (another Design, a saved one, undo) goes through `set_layout`: drawn at once where it puts every
/// Pane, the focus staying on the Widget it was on (review of #35: identities of another tree are other Panes).
#[test]
fn a_new_design_is_drawn_at_once_and_the_focus_stays_on_its_widget() {
    let mut a = app();
    a.focus_on(Widget::Queue);
    a.set_layout(Node::classic());
    assert_eq!(a.focused(), Some(Widget::Queue));
    render(&mut a, W, H);
    let want = Node::classic().compute(a.pane_area()).rect_of(Widget::Home).unwrap();
    assert_eq!(pane(&a, Widget::Home), want, "the first frame is the new layout");
}

#[test]
fn an_empty_pane_has_nothing_to_move() {
    let mut a = app();
    let area = a.pane_area();
    a.layout = a.layout.split(pid(&a, Widget::Queue), Side::Right, None, area).unwrap();
    render(&mut a, W, H);
    let empty = a.layout.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    a.focus = Some(empty);
    key(&mut a, KeyCode::Char('m'));
    key(&mut a, KeyCode::Enter);
    assert!(a.tile.is_none());
    a.open_pane_menu(empty, None);
    assert!(!a.menu().unwrap().items.iter().any(|(l, _)| l == "Move pane"));
}

/// Something opening on top mid-drag (a popup, the login code): the drag ends, so its chip isn't drawn over it.
#[test]
fn a_popup_opening_mid_drag_lets_the_widget_go() {
    let mut a = app();
    let (x, y) = title_spot(&a, Widget::Stream);
    press(&mut a, x, y);
    let q = centre(pane(&a, Widget::Queue));
    drag(&mut a, q.0, q.1);
    assert!(a.tile.as_ref().is_some_and(|t| t.live));
    a.run(Action::Palette);
    assert!(a.tile.is_none());
    let shot = render(&mut a, W, H);
    assert!(!shot.contains("⠿ stream") && !shot.contains("swap with"), "{shot}");
    key(&mut a, KeyCode::Esc);
    // the login code is a layer, not a popup: the next frame lets go (Arrange mode is on now: the Panes are shorter)
    render(&mut a, W, H);
    let (x, y) = title_spot(&a, Widget::Stream);
    let q = centre(pane(&a, Widget::Queue));
    press(&mut a, x, y);
    drag(&mut a, q.0, q.1);
    assert!(a.tile.as_ref().is_some_and(|t| t.live));
    a.hear(singularity_proto::Event::Catalogue(singularity_proto::CatalogueStatus::LoginPending { url: "u".into(), code: "ABCD".into() }));
    let shot = render(&mut a, W, H);
    assert!(a.tile.is_none() && !shot.contains("⠿ stream"));
    release(&mut a, q.0, q.1);
}

// ── Arrange mode and the Tray (#41) ───────────────────────────────────

/// The middle of the Tray strip (Arrange mode: the three rows under the player bar).
fn tray_spot(a: &App) -> (u16, u16) {
    let t = a.tray_rect();
    (t.x + t.width / 2, t.y + 1)
}
/// The middle of the chip of the Widget at place `i` in the Tray, as last drawn.
fn tray_item(a: &App, i: usize) -> (u16, u16) {
    let r = a.hit_rect(&Hit::TrayItem(a.layout.tray()[i])).unwrap_or_else(|| panic!("no Tray item {i}"));
    (r.x + r.width / 2, r.y)
}
/// (as macros, so the spots can be worked out from the App the gesture then goes to)
macro_rules! click {
    ($a:expr, $at:expr) => {{
        let at = $at;
        click_at($a, at)
    }};
}
macro_rules! drag_to {
    ($a:expr, $from:expr, $to:expr) => {{
        let (from, to) = ($from, $to);
        drag_at($a, from, to)
    }};
}
fn click_at(a: &mut App, (x, y): (u16, u16)) {
    press(a, x, y);
    release(a, x, y);
}
/// A drag from (x, y) to (tx, ty), the way a hand does it: a few steps, then the release there.
fn drag_at(a: &mut App, (x, y): (u16, u16), (tx, ty): (u16, u16)) {
    press(a, x, y);
    drag(a, x + 4, y.saturating_sub(2).max(1));
    drag(a, tx, ty);
    release(a, tx, ty);
    render(a, W, H);
}

#[test]
fn a_title_drag_turns_arrange_mode_on_and_a_drop_on_the_tray_sets_the_widget_aside() {
    let mut a = app();
    let before = a.layout.clone();
    let stream = pid(&a, Widget::Stream);
    let (x, y) = title_spot(&a, Widget::Stream);
    press(&mut a, x, y);
    drag(&mut a, x + 4, y + 2);
    assert!(a.arranging.is_some(), "the drag turned it on");
    let shot = render(&mut a, W, H);
    // the Panes made room for the Tray, by the floor plan's own rule, and stay put from then on
    let floor = arranged_floor(&a);
    assert_eq!(drawn_panes(&a), floor, "the Panes are the floor plan in the shorter area");
    assert!(shot.contains(" Tray ") && shot.contains("ARRANGE") && shot.contains(" Done "), "{shot}");
    let (tx, ty) = tray_spot(&a);
    drag(&mut a, tx, ty);
    assert!(a.tile.as_ref().is_some_and(|t| t.tray && t.over.is_none()), "the Tray is highlighted");
    assert!(render(&mut a, W, H).contains(" set aside here "));
    assert_eq!(drawn_panes(&a), floor);
    release(&mut a, tx, ty);
    assert_eq!(a.layout.tray(), [Widget::Stream]);
    assert_eq!(a.layout.widget_in(stream), None, "its Pane is left empty");
    assert_eq!(crate::arrangement::outline(a.layout.plan()), crate::arrangement::outline(before.plan()));
    assert!(a.arranging.is_some(), "still arranging after the drop");
    let shot = render(&mut a, W, H);
    assert!(shot.contains("⠿ stream") && shot.contains("empty"), "{shot}");
}

#[test]
fn a_widget_from_the_tray_fills_an_empty_pane_or_swaps_with_what_is_there() {
    let mut a = app();
    let (stream, queue) = (pid(&a, Widget::Stream), pid(&a, Widget::Queue));
    drag_to!(&mut a, title_spot(&a, Widget::Stream), tray_spot(&a));
    // onto the queue: the queue goes into the Tray in its place
    let to = centre(pane(&a, Widget::Queue));
    let from = tray_item(&a, 0);
    press(&mut a, from.0, from.1);
    drag(&mut a, from.0 + 3, from.1 - 3);
    drag(&mut a, to.0, to.1);
    assert!(render(&mut a, W, H).contains(" swap with queue "));
    release(&mut a, to.0, to.1);
    assert_eq!((a.layout.widget_in(queue), a.layout.tray()), (Some(Widget::Stream), &[Widget::Queue][..]));
    render(&mut a, W, H);
    // onto the empty Pane: it fills it, and the Tray is empty
    let to = centre(drawn_panes(&a).into_iter().find(|(p, _)| *p == stream).unwrap().1);
    drag_to!(&mut a, tray_item(&a, 0), to);
    assert_eq!(a.layout.widget_in(stream), Some(Widget::Queue));
    assert!(a.layout.tray().is_empty());
    assert!(render(&mut a, W, H).contains("drop a Widget here to set it aside"));
    // a click on a Widget in the Tray, or a drop back on the Tray, changes nothing
    drag_to!(&mut a, title_spot(&a, Widget::Fft), tray_spot(&a));
    let now = a.layout.clone();
    click!(&mut a, tray_item(&a, 0));
    let (tx, ty) = tray_spot(&a);
    drag_to!(&mut a, tray_item(&a, 0), (tx + 20, ty));
    assert_eq!(a.layout, now);
}

#[test]
fn closing_sends_the_widget_to_the_tray_and_clear_tray_never_removes_home() {
    let mut a = app();
    a.run(Action::Arrange);
    render(&mut a, W, H);
    let fft = pid(&a, Widget::Fft);
    // each Pane has a split and a close button
    assert!(a.hit_rect(&Hit::SplitPane(fft)).is_some() && a.hit_rect(&Hit::Close(fft)).is_some());
    let r = a.hit_rect(&Hit::Close(fft)).unwrap();
    click!(&mut a, (r.x + 1, r.y));
    assert!(!a.layout.contains(fft), "the neighbour took the room");
    assert_eq!(a.layout.tray(), [Widget::Fft]);
    render(&mut a, W, H);
    // Home into the Tray: Done is unavailable, and says why
    drag_to!(&mut a, title_spot(&a, Widget::Home), tray_spot(&a));
    assert_eq!(a.layout.tray(), [Widget::Fft, Widget::Home]);
    let shot = render(&mut a, W, H);
    assert!(shot.lines().last().unwrap().contains("Home still needs a Pane"), "{}", shot.lines().last().unwrap());
    key(&mut a, KeyCode::Esc);
    let done = a.hit_rect(&Hit::ArrangeDone).unwrap();
    click!(&mut a, (done.x + 1, done.y));
    assert!(a.arranging.is_some(), "neither Esc nor Done ends it");
    // Home has no × in the Tray; Clear Tray removes the rest
    assert!(a.hit_rect(&Hit::TrayRemove(Widget::Home)).is_none() && a.hit_rect(&Hit::TrayRemove(Widget::Fft)).is_some());
    let c = a.hit_rect(&Hit::ClearTray).unwrap();
    click!(&mut a, (c.x + 1, c.y));
    assert_eq!(a.layout.tray(), [Widget::Home]);
    render(&mut a, W, H);
    assert!(a.hit_rect(&Hit::ClearTray).is_none(), "nothing left it could clear");
    // Home back into its (empty) Pane: Done ends Arrange mode, and the app looks as it did
    let empty = a.layout.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    let to = centre(drawn_panes(&a).into_iter().find(|(p, _)| *p == empty).unwrap().1);
    drag_to!(&mut a, tray_item(&a, 0), to);
    key(&mut a, KeyCode::Esc);
    assert!(a.arranging.is_none() && a.layout.has(Widget::Home));
    let shot = render(&mut a, W, H);
    assert!(!shot.contains(" Tray ") && !shot.contains("ARRANGE") && !shot.contains('⊞'), "{shot}");
}

#[test]
fn the_x_on_one_widget_in_the_tray_removes_it_and_undo_redo_cover_tray_edits() {
    let mut a = app();
    let start = a.layout.clone();
    drag_to!(&mut a, title_spot(&a, Widget::Stream), tray_spot(&a));
    drag_to!(&mut a, title_spot(&a, Widget::Fft), tray_spot(&a));
    let two = a.layout.clone();
    let x = a.hit_rect(&Hit::TrayRemove(Widget::Stream)).unwrap();
    click!(&mut a, (x.x, x.y));
    // a second click in the same burst (no frame between) lands on the same spot: nothing more goes
    click!(&mut a, (x.x, x.y));
    assert_eq!(a.layout.tray(), [Widget::Fft], "only that one went");
    assert!(!a.layout.has(Widget::Stream));
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, two);
    ctrl(&mut a, 'z');
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, start);
    ctrl(&mut a, 'y');
    assert_eq!(a.layout.tray(), [Widget::Stream]);
    assert!(a.arranging.is_some(), "undo and redo work in Arrange mode");
}

#[test]
fn the_palette_and_the_pane_menu_turn_arrange_mode_on_and_outside_it_nothing_looks_different() {
    let mut a = app();
    let plain = render(&mut a, W, H);
    a.run(Action::Palette);
    for c in "arrange".chars() {
        key(&mut a, KeyCode::Char(c));
    }
    assert_eq!(a.palette_items().first(), Some(&Action::Arrange));
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.modal(), Some(Modal::Arrange));
    let shot = render(&mut a, W, H);
    assert!(shot.contains('⊞') && shot.contains("Esc done"), "{shot}");
    key(&mut a, KeyCode::Esc);
    assert!(a.modal().is_none());
    a.toast = None;
    assert_eq!(render(&mut a, W, H), plain, "the app looks as before");
    // the Pane menu's Arrange (not offered while arranging)
    let (x, y) = title_spot(&a, Widget::Queue);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    let i = a.menu().unwrap().items.iter().position(|(l, _)| l == "Arrange").unwrap();
    for _ in 0..i {
        key(&mut a, KeyCode::Down);
    }
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.modal(), Some(Modal::Arrange));
    render(&mut a, W, H);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    assert!(!a.menu().unwrap().items.iter().any(|(l, _)| l == "Arrange"));
}

#[test]
fn a_tray_holding_more_than_fits_scrolls() {
    let (w, h) = (100, 40);
    let mut a = App::new(None);
    let off: Vec<Widget> = Widget::ALL.iter().copied().filter(|w| !a.layout.has(*w)).collect();
    let n = off.len();
    a.layout = Arrangement::new(a.layout.plan().clone(), off);
    a.run(Action::Arrange);
    let shot = render(&mut a, w, h);
    let shown = (0..n).filter(|i| a.hit_rect(&Hit::TrayItem(a.layout.tray()[*i])).is_some()).count();
    assert!(shown < n && a.hit_rect(&Hit::TrayScroll(1)).is_some() && a.hit_rect(&Hit::TrayScroll(-1)).is_none(), "{shot}");
    // the wheel, anywhere, and the ›
    mouse(&mut a, MouseEventKind::ScrollDown, 50, 20);
    render(&mut a, w, h);
    assert!(a.hit_rect(&Hit::TrayItem(a.layout.tray()[0])).is_none() && a.hit_rect(&Hit::TrayScroll(-1)).is_some());
    for _ in 0..n {
        if let Some(r) = a.hit_rect(&Hit::TrayScroll(1)) {
            press(&mut a, r.x, r.y);
            render(&mut a, w, h);
        }
    }
    assert!(a.hit_rect(&Hit::TrayItem(a.layout.tray()[n - 1])).is_some() && a.hit_rect(&Hit::TrayScroll(1)).is_none(), "it reaches the end");
}

#[test]
fn the_tray_is_saved_with_the_design_and_survives_a_restart() {
    let (mut a, dir) = with_dirs();
    drag_to!(&mut a, title_spot(&a, Widget::Stream), tray_spot(&a));
    let text = std::fs::read_to_string(dir.path().join("layouts/singularity-edited.toml")).unwrap();
    assert!(text.contains("tray = [\"stream\"]"), "{text}");
    let mut again = App::new(None);
    again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
    assert_eq!(again.layout, a.layout);
    assert_eq!(again.layout.tray(), [Widget::Stream]);
    let shot = render(&mut again, W, H);
    assert!(again.arranging.is_none() && !shot.contains(" Tray "), "the Tray only shows in Arrange mode");
    // a Design left with Home in the Tray comes back in Arrange mode, where Done waits for it
    drag_to!(&mut a, title_spot(&a, Widget::Home), tray_spot(&a));
    let mut again = App::new(None);
    again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
    render(&mut again, W, H);
    assert_eq!(again.modal(), Some(Modal::Arrange));
}

// ── review of #41 ─────────────────────────────────────────────────────

/// Shrunk to the mini player while arranging (Home in the Tray, even): the player has the keys and the pointer;
/// at full size again, Arrange mode is back where it was.
#[test]
fn the_mini_player_is_never_behind_arrange_mode() {
    let mut a = app();
    drag_to!(&mut a, title_spot(&a, Widget::Home), tray_spot(&a));
    assert_eq!(a.modal(), Some(Modal::Arrange));
    render(&mut a, 40, 10);
    assert!(a.mini && a.modal().is_none(), "the mini player is on top");
    render(&mut a, W, H);
    assert_eq!(a.modal(), Some(Modal::Arrange));
    assert_eq!(a.layout.tray(), [Widget::Home]);
}

/// Previewing a saved Design whose Home waits in the Tray doesn't start Arrange mode (Esc puts the old one back);
/// previewing others while arranging isn't an edit, so Done asks for no name.
#[test]
fn previewing_designs_neither_starts_arrange_mode_nor_counts_as_an_edit() {
    let (mut a, dir) = with_dirs();
    let homeless = a.layout.to_tray(pid(&a, Widget::Home)).unwrap();
    crate::layouts::write(&dir.path().join("layouts"), "parked", "Parked", "singularity", "", &homeless, false).unwrap();
    crate::layouts::write(&dir.path().join("layouts"), "other", "Other", "singularity", "", &swapped(Node::singularity(), Widget::Fft, Widget::Loudness), true).unwrap();
    a.reload_layouts();
    let pick = |a: &mut App, name: &str| {
        a.run(Action::PickDesign);
        let i = a.picker().unwrap().items.iter().position(|it| it.1 == name).unwrap();
        a.picker_select(i);
        render(a, W, H);
    };
    pick(&mut a, "Parked");
    assert!(a.arranging.is_none(), "not while it's only previewed");
    key(&mut a, KeyCode::Esc);
    render(&mut a, W, H);
    assert!(a.arranging.is_none() && a.modal().is_none() && a.layout.has(Widget::Home));
    // arranging on the unnamed edit: previews and a kept pick aren't edits
    a.run(Action::Arrange);
    pick(&mut a, "Other");
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Esc);
    assert!(a.naming().is_none() && a.arranging.is_none(), "{:?}", a.modal());
}

/// ◂ ▸ skip a Widget waiting in the Tray: they aren't a recorded edit, so they mustn't take it out.
#[test]
fn the_arrows_in_a_title_skip_what_waits_in_the_tray() {
    let mut a = app();
    a.close_pane(pid(&a, Widget::Fft));
    assert_eq!(a.layout.tray(), [Widget::Fft]);
    let loud = pid(&a, Widget::Loudness);
    assert!(a.swap_widget(loud, -1));
    assert_ne!(a.layout.widget_in(loud), Some(Widget::Fft));
    assert_eq!(a.layout.tray(), [Widget::Fft]);
}

/// While arranging, no Pane takes keys: `w` opens no picker.
#[test]
fn arrange_mode_keeps_the_panes_keys() {
    let mut a = app();
    a.run(Action::Arrange);
    a.focus_on(Widget::Stream);
    // (`w` is Arrange mode's: "+ Widget", not the Pane's Replace widget…)
    key(&mut a, KeyCode::Char('w'));
    assert!(a.widget_picker().is_some_and(|p| p.tray), "the Tray's picker");
    assert!(a.arranging.is_some());
}

// ── the player bar stays, the Tray goes under it (#69, Q39) ────────────

/// A connected App with a paused song in the player bar, and the Daemon's end of the line.
fn playing() -> (App, tokio::sync::mpsc::Receiver<singularity_proto::Command>) {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let mut a = App::new(Some(tx));
    a.link = crate::app::Link::Connected { version: "test".into() };
    a.playback.item = Some(singularity_proto::Item { name: "Song".into(), duration_ms: 240_000, ..Default::default() });
    a.playback.status = singularity_proto::PlayStatus::Paused;
    render(&mut a, W, H);
    (a, rx)
}
fn sent(rx: &mut tokio::sync::mpsc::Receiver<singularity_proto::Command>) -> Vec<singularity_proto::Command> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

#[test]
fn in_arrange_mode_the_player_bar_stays_and_its_buttons_seek_bar_and_keys_work() {
    use singularity_proto::Command;
    let (mut a, mut rx) = playing();
    let was = a.player_rect();
    let plain = render_buf(&mut a, W, H);
    a.run(Action::Arrange);
    let shot = render_buf(&mut a, W, H);
    // the player bar as it was, the Tray's rows higher; the Tray right under it, the legend and Done under that
    let bar = a.player_rect();
    assert_eq!((bar.y + crate::arrange::TRAY_H, bar.height), (was.y, was.height));
    for i in 0..bar.height {
        assert_eq!(row(&shot, bar.y + i, 0, W), row(&plain, was.y + i, 0, W), "the player bar looks as before");
    }
    let tray = a.tray_rect();
    assert_eq!((tray.y, tray.bottom()), (bar.bottom(), H - 1));
    assert!(row(&shot, tray.y, 0, W).contains(" Tray "), "{}", row(&shot, tray.y, 0, W));
    assert!(row(&shot, H - 1, 0, W).contains("ARRANGE"));
    // its buttons
    let play = a.hit_rect(&Hit::Action(Action::TogglePlay)).expect("▶ in the player bar");
    assert!(play.y >= bar.y && play.bottom() <= bar.bottom());
    click_at(&mut a, (play.x + 1, play.y));
    let next = a.hit_rect(&Hit::Action(Action::Next)).unwrap();
    click_at(&mut a, (next.x + 1, next.y));
    assert_eq!(sent(&mut rx), vec![Command::Play { uri: None }, Command::Next]);
    // its seek bar: a press, a drag along it, one seek on the release
    render(&mut a, W, H);
    let seek = a.hits.iter().find_map(|(_, h)| match h {
        Hit::Bar(crate::scrub::Bar::Seek, r) => Some(*r),
        _ => None,
    }).expect("a seek bar");
    assert!(seek.y >= bar.y && seek.y < bar.bottom());
    press(&mut a, seek.x + 10, seek.y);
    drag(&mut a, seek.x + 20, seek.y);
    assert!(a.scrub.is_some() && sent(&mut rx).is_empty(), "scrubbing");
    release(&mut a, seek.x + 20, seek.y);
    assert!(matches!(sent(&mut rx)[..], [Command::Seek { .. }]));
    // its keys
    key(&mut a, KeyCode::Char(' '));
    assert_eq!(sent(&mut rx), vec![Command::Pause]);
    assert_eq!(a.modal(), Some(Modal::Arrange), "still arranging");
    assert!(a.tile.is_none() && !a.history.can_undo());
}

#[test]
fn the_panes_make_room_for_the_tray_by_one_rule_and_done_gives_it_back() {
    let (mut a, dir) = with_dirs();
    // an edit outside Arrange mode: kept as "Singularity (edited)", a file
    a.close_pane(pid(&a, Widget::Fft));
    render(&mut a, W, H);
    let file = dir.path().join("layouts/singularity-edited.toml");
    let saved = std::fs::read_to_string(&file).unwrap();
    let (plain, body, before) = (drawn_panes(&a), a.pane_area(), a.layout.clone());
    for _ in 0..2 {
        a.run(Action::Arrange);
        render(&mut a, W, H);
        // the Panes' area is the Tray's height shorter; every Pane is the floor plan laid out in it (its own
        // proportions), in its own columns
        let short = a.pane_area();
        assert_eq!((short.y, short.width, short.height + crate::arrange::TRAY_H), (body.y, body.width, body.height));
        let now = drawn_panes(&a);
        assert_eq!(now, arranged_floor(&a));
        assert!(now.iter().zip(&plain).all(|((p, r), (q, s))| p == q && (r.x, r.width) == (s.x, s.width) && r.height <= s.height));
        assert_eq!(a.layout, before, "the floor plan itself never changes for it");
        // Done: the room back, every Pane where it was, nothing written and nothing to undo
        key(&mut a, KeyCode::Esc);
        render(&mut a, W, H);
        assert!(a.arranging.is_none() && a.naming().is_none());
        assert_eq!((a.pane_area(), drawn_panes(&a)), (body, plain.clone()));
        assert_eq!(a.layout, before);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), saved, "the saved floor plan is the same");
    }
    assert!(a.history.can_undo());
    a.run(Action::UndoLayout);
    assert!(a.layout.has(Widget::Fft), "the one undo step is the close");
}

/// A drag that turns Arrange mode on makes the Panes shorter at once, before any frame is drawn: a release in the
/// same batch of input, on the queue where it is now (a cell that was another Pane's, where there is one), swaps with
/// the queue. Every Design that has a queue.
#[test]
fn a_drag_that_turns_arrange_mode_on_drops_on_the_pane_where_it_now_is() {
    let mut tried = 0;
    for (d, design) in DESIGNS.iter().enumerate() {
        let mut a = App::new(None);
        a.set_design(d);
        render(&mut a, W, H);
        if !a.layout.has(Widget::Queue) || a.arranging.is_some() {
            continue;
        }
        tried += 1;
        let before = a.layout.clone();
        let held = before.widgets().into_iter().find(|w| *w != Widget::Queue).unwrap();
        let old = pane(&a, Widget::Queue);
        let (x, y) = title_spot(&a, held);
        press(&mut a, x, y);
        drag(&mut a, x + 4, y + 2);
        assert!(a.arranging.is_some(), "{}: the drag turned Arrange mode on", design.id);
        assert_eq!(a.pane_area(), crate::ui::rows(a.window, true)[1], "{}: the Panes' area is shorter at once", design.id);
        let new = a.layout.compute(a.pane_area()).rect_of(Widget::Queue).unwrap();
        assert!(new.height < old.height || new.y < old.y, "{}: the queue moved up or shrank", design.id);
        // a cell of the queue now that wasn't before, if any; else its middle
        let inside = |r: Rect, (cx, cy): (u16, u16)| cx >= r.x && cx < r.right() && cy >= r.y && cy < r.bottom();
        let spot = (new.y..new.bottom()).map(|cy| (new.x + new.width / 2, cy)).find(|c| !inside(old, *c)).unwrap_or(centre(new));
        drag(&mut a, spot.0, spot.1);
        release(&mut a, spot.0, spot.1);
        assert_eq!(a.layout, swapped(before, held, Widget::Queue), "{}: released at {spot:?} on the queue's new {new:?}", design.id);
    }
    assert!(tried >= 3, "Designs with a queue: {tried}");
}

// ── New design… (#42) ─────────────────────────────────────────────────

/// The Tray's "+ Widget", as last drawn.
fn tray_add(a: &App) -> (u16, u16) {
    let r = a.hit_rect(&Hit::TrayAdd).expect("+ Widget on the Tray");
    (r.x + 2, r.y)
}
fn typed(a: &mut App, s: &str) {
    for c in s.chars() {
        key(a, KeyCode::Char(c));
    }
}
/// Every file in the config folder (the layouts and config.toml), with what it says.
fn files(dir: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
    let mut out = vec![];
    for d in [dir.to_path_buf(), dir.join("layouts")] {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.path().is_file() {
                out.push((e.path(), std::fs::read_to_string(e.path()).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// A whole build: the palette's New design…, a Widget added with the Tray's "+ Widget", the floor plan split, Home and
/// the pick dragged from the Tray into the Panes, Done, a name: a Design of your own with the current Theme.
#[test]
fn a_new_design_is_built_from_one_empty_pane_and_done_saves_it_under_a_name() {
    let (mut a, dir) = with_dirs();
    // a Theme other than the Design's own, so the saved one is the current one
    a.run(Action::NextVariant);
    let theme = a.theme.id.clone();
    assert_ne!(theme, DESIGNS[a.design].themes[0]);
    // the palette's "New design…"
    a.run(Action::Palette);
    typed(&mut a, "new design");
    assert_eq!(a.palette_items().first(), Some(&Action::NewDesign));
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.modal(), Some(Modal::Arrange));
    // one empty Pane filling the Panes' area, Home waiting in the Tray, Done waiting for it
    let shot = render(&mut a, W, H);
    let panes = drawn_panes(&a);
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].1, a.pane_area(), "it fills the screen");
    assert_eq!(a.layout.widget_in(panes[0].0), None);
    assert_eq!(a.layout.tray(), [Widget::Home]);
    assert!(shot.contains("empty") && shot.contains(" + Widget ") && shot.contains("Home still needs a Pane"), "{shot}");
    // "+ Widget": the picker offers what is neither on screen nor in the Tray, and the pick joins the Tray
    click!(&mut a, tray_add(&a));
    let rows = a.widget_choices();
    assert!(a.widget_picker().is_some_and(|p| p.tray));
    assert!(rows.iter().all(|c| c.from.is_none() && c.widget != Widget::Home) && rows.iter().any(|c| c.widget == Widget::Queue), "{rows:?}");
    let shot = render(&mut a, W, H);
    assert!(shot.contains("Choose a widget") && shot.contains("Enter puts it in the Tray"), "{shot}");
    typed(&mut a, "queue");
    key(&mut a, KeyCode::Enter);
    assert!(a.widget_picker().is_none() && a.arranging.is_some());
    assert_eq!(a.layout.tray(), [Widget::Home, Widget::Queue]);
    // the floor plan first: the Pane's ⊞ splits it, and Esc in the picker leaves the new Pane empty
    render(&mut a, W, H);
    let first = panes[0].0;
    let split = a.hit_rect(&Hit::SplitPane(first)).expect("the Pane's ⊞");
    click!(&mut a, (split.x, split.y));
    assert!(a.widget_picker().is_some_and(|p| p.after_split));
    key(&mut a, KeyCode::Esc);
    assert_eq!(a.layout.panes().len(), 2);
    assert!(a.layout.panes().iter().all(|p| p.widget.is_none()));
    // then the contents: Home and the queue from the Tray into the two Panes
    render(&mut a, W, H);
    let rects = drawn_panes(&a);
    drag_to!(&mut a, tray_item(&a, 0), centre(rects[0].1));
    drag_to!(&mut a, tray_item(&a, 0), centre(rects[1].1));
    assert_eq!((a.layout.widget_in(rects[0].0), a.layout.widget_in(rects[1].0)), (Some(Widget::Home), Some(Widget::Queue)));
    assert!(a.layout.tray().is_empty());
    // nothing is written while it's built
    assert!(crate::layouts::load_dir(&dir.path().join("layouts")).0.is_empty());
    // Done asks for the name (still arranging under it); the name saves it
    let done = a.hit_rect(&Hit::ArrangeDone).unwrap();
    click!(&mut a, (done.x + 1, done.y));
    assert_eq!(a.naming().map(|n| (n.text.as_str(), n.purpose)), Some(("", crate::app::NameFor::NewDesign)));
    assert!(render(&mut a, W, H).contains("Save design as"));
    typed(&mut a, "Built");
    key(&mut a, KeyCode::Enter);
    assert!(a.naming().is_none() && a.arranging.is_none());
    assert_eq!(a.layout_name.as_deref(), Some("built"));
    let text = std::fs::read_to_string(dir.path().join("layouts/built.toml")).unwrap();
    assert!(text.contains("name = \"Built\"") && text.contains(&format!("theme = \"{theme}\"")) && !text.contains("edited"), "{text}");
    assert!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap().contains("layout = \"built\""));
    let saved = crate::layouts::load_dir(&dir.path().join("layouts")).0;
    assert!(saved.len() == 1 && saved[0].arrangement() == a.layout);
    // a restart brings it back, Theme and all
    let mut again = App::new(None);
    again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
    assert_eq!((again.layout.clone(), again.theme.id.clone()), (a.layout.clone(), theme));
}

/// Esc on New design… before anything is placed leaves the Design on screen untouched (its files, its undo); after an
/// edit Esc is Done, which waits for Home, and undoing back to the blank start lets Esc leave again.
#[test]
fn esc_on_a_new_design_before_anything_is_placed_leaves_the_design_untouched() {
    let (mut a, dir) = with_dirs();
    a.close_pane(pid(&a, Widget::Fft));
    // (the bar's passing notes aside)
    a.toast = None;
    a.undo_offer = None;
    let plain = render(&mut a, W, H);
    let (layout, name, on_disk) = (a.layout.clone(), a.layout_name.clone(), files(dir.path()));
    assert!(name.is_some() && on_disk.iter().any(|(p, _)| p.ends_with("layouts/singularity-edited.toml")), "{on_disk:?}");
    a.run(Action::NewDesign);
    render(&mut a, W, H);
    assert_eq!(a.layout.tray(), [Widget::Home]);
    key(&mut a, KeyCode::Esc);
    assert!(a.arranging.is_none() && a.modal().is_none());
    assert_eq!((a.layout.clone(), a.layout_name.clone(), files(dir.path())), (layout.clone(), name.clone(), on_disk.clone()));
    assert_eq!(render(&mut a, W, H), plain, "the app as before");
    // a Widget added: Esc is Done, which waits for Home; nothing is written meanwhile
    a.run(Action::NewDesign);
    render(&mut a, W, H);
    click!(&mut a, tray_add(&a));
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout.tray().len(), 2);
    key(&mut a, KeyCode::Esc);
    assert!(a.arranging.is_some() && a.naming().is_none());
    assert_eq!(files(dir.path()), on_disk);
    // undone back to the blank start: Esc leaves, and the close from before the build is still undoable
    ctrl(&mut a, 'z');
    assert_eq!(a.layout.tray(), [Widget::Home]);
    key(&mut a, KeyCode::Esc);
    assert!(a.arranging.is_none());
    assert_eq!((a.layout.clone(), a.layout_name.clone(), files(dir.path())), (layout, name, on_disk));
    ctrl(&mut a, 'z');
    assert!(a.layout.has(Widget::Fft));
}

/// Owner, after #42: a New design… with things placed can be thrown away: Ctrl+D on its name prompt (which says so,
/// and that Esc keeps building) brings back the Design that was on screen, untouched (its files, its look, its undo);
/// nothing of the build is kept, and the bar says it was discarded.
#[test]
fn ctrl_d_on_a_new_designs_name_prompt_throws_it_away() {
    let (mut a, dir) = with_dirs();
    a.close_pane(pid(&a, Widget::Fft));
    a.toast = None;
    a.undo_offer = None;
    let plain = render(&mut a, W, H);
    let (layout, name, on_disk) = (a.layout.clone(), a.layout_name.clone(), files(dir.path()));
    a.run(Action::NewDesign);
    render(&mut a, W, H);
    drag_to!(&mut a, tray_item(&a, 0), centre(drawn_panes(&a)[0].1));
    assert!(a.layout.has(Widget::Home) && a.layout != layout);
    key(&mut a, KeyCode::Esc);
    assert_eq!(a.naming().map(|n| n.purpose), Some(crate::app::NameFor::NewDesign));
    let shot = render(&mut a, W, H);
    assert!(shot.contains("Enter save · Esc keep building · Ctrl+D discard it") && shot.contains("find it in Ctrl+K → Pick a design"), "{shot}");
    typed(&mut a, "Half");
    ctrl(&mut a, 'd');
    assert!(a.naming().is_none() && a.arranging.is_none() && a.modal().is_none());
    assert!(a.toast.as_ref().is_some_and(|t| t.0 == "New design discarded"), "{:?}", a.toast);
    a.toast = None;
    assert_eq!((a.layout.clone(), a.layout_name.clone(), files(dir.path())), (layout, name, on_disk));
    assert_eq!(render(&mut a, W, H), plain, "the app as before");
    // the close from before the build is still undoable
    ctrl(&mut a, 'z');
    assert!(a.layout.has(Widget::Fft));
}

/// Owner Q78: while a New design… is built the palette offers "Discard new design", the same as Ctrl+D on the prompt;
/// a Theme picked during the build goes too (on screen and in config.toml). Not offered otherwise.
#[test]
fn the_palettes_discard_new_design_puts_the_theme_back_too() {
    let (mut a, dir) = with_dirs();
    let config = || std::fs::read_to_string(dir.path().join("config.toml")).unwrap_or_default();
    a.run(Action::Palette);
    typed(&mut a, "discard");
    assert!(!a.palette_items().contains(&Action::DiscardNewDesign), "only while one is built");
    key(&mut a, KeyCode::Esc);
    let (layout, theme, on_disk) = (a.layout.clone(), a.theme.id.clone(), config());
    a.run(Action::NewDesign);
    render(&mut a, W, H);
    drag_to!(&mut a, tray_item(&a, 0), centre(drawn_panes(&a)[0].1));
    // another Theme while building: on screen and in config.toml
    a.run(Action::NextVariant);
    assert_ne!(a.theme.id, theme);
    assert_ne!(config(), on_disk);
    a.run(Action::Palette);
    typed(&mut a, "discard");
    assert_eq!(a.palette_items().first(), Some(&Action::DiscardNewDesign));
    assert!(render(&mut a, W, H).contains("Discard new design"));
    key(&mut a, KeyCode::Enter);
    assert!(a.arranging.is_none() && a.modal().is_none());
    assert_eq!((a.layout.clone(), a.theme.id.clone()), (layout, theme.clone()));
    assert!(a.toast.as_ref().is_some_and(|t| t.0 == "New design discarded"), "{:?}", a.toast);
    let mut again = App::new(None);
    again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
    assert_eq!(again.theme.id, theme, "config.toml names the Theme you had:\n{}", config());
}

/// The other name prompts have no such row, and Ctrl+D there is no discard.
#[test]
fn only_a_new_designs_name_prompt_offers_discard() {
    let (mut a, _dir) = with_dirs();
    a.run(Action::SaveLayout);
    let shot = render(&mut a, W, H);
    assert!(shot.contains("Save design as") && !shot.contains("Ctrl+D"), "{shot}");
    let layout = a.layout.clone();
    ctrl(&mut a, 'd');
    assert!(a.naming().is_some());
    assert_eq!(a.layout, layout);
}

/// A name already taken is refused and the prompt stays; Esc on it goes back to arranging, the build intact.
#[test]
fn a_new_designs_name_is_never_one_taken_and_esc_on_it_keeps_building() {
    let (mut a, dir) = with_dirs();
    crate::layouts::write(&dir.path().join("layouts"), "mine", "Mine", "singularity", "", &a.layout, false).unwrap();
    a.reload_layouts();
    a.run(Action::NewDesign);
    render(&mut a, W, H);
    drag_to!(&mut a, tray_item(&a, 0), centre(drawn_panes(&a)[0].1));
    let built = a.layout.clone();
    key(&mut a, KeyCode::Esc);
    assert_eq!(a.naming().map(|n| n.purpose), Some(crate::app::NameFor::NewDesign));
    key(&mut a, KeyCode::Esc);
    assert!(a.naming().is_none() && a.arranging.is_some());
    assert_eq!(a.layout, built);
    key(&mut a, KeyCode::Esc);
    typed(&mut a, "Mine");
    key(&mut a, KeyCode::Enter);
    assert!(a.naming().is_some() && a.arranging.is_some(), "refused: the prompt stays");
    assert!(a.toast.as_ref().is_some_and(|t| t.0.contains("There is already a design called \"Mine\"")), "{:?}", a.toast);
    assert_eq!(a.layout, built);
}

/// Review of #42: while a New design… is built, Pick a design, Save design as… and Reset design are not in the palette
/// and their keys do nothing (each would swap or save the Design under the build); Done names it. Afterwards they're back.
#[test]
fn a_new_design_being_built_hides_pick_save_and_reset_design() {
    let (mut a, dir) = with_dirs();
    let gone = [Action::PickDesign, Action::SaveLayout, Action::ResetLayout];
    a.run(Action::NewDesign);
    render(&mut a, W, H);
    a.run(Action::Palette);
    for word in ["design", "pick a design", "save design", "reset design"] {
        a.palette_mut().unwrap().query = word.into();
        assert!(a.palette_items().iter().all(|i| !gone.contains(i)), "{word}: {:?}", a.palette_items());
    }
    a.palette_mut().unwrap().query = "theme".into();
    assert!(a.palette_items().contains(&Action::PickTheme), "the rest stays");
    key(&mut a, KeyCode::Esc);
    // their keys (or any other way to run them) do nothing
    let blank = a.layout.clone();
    for g in gone {
        a.run(g);
        assert!(a.modal() == Some(Modal::Arrange) && a.picker().is_none() && a.naming().is_none(), "{g:?}: {:?}", a.modal());
        assert_eq!(a.layout, blank, "{g:?}");
    }
    assert!(a.building());
    assert!(crate::layouts::load_dir(&dir.path().join("layouts")).0.is_empty());
    // Esc leaves the build: they're back
    key(&mut a, KeyCode::Esc);
    assert!(!a.building());
    a.run(Action::Palette);
    a.palette_mut().unwrap().query = "design".into();
    assert!(gone.iter().all(|g| a.palette_items().contains(g)), "{:?}", a.palette_items());
}

/// Review of #42: a New design's name can't take the file a built-in Design's unnamed "(edited)" copy uses (a later edit
/// of that Design would write over it), and a name that makes no file name keeps the prompt, saying why.
#[test]
fn a_new_designs_name_never_takes_an_edited_copys_file_or_makes_no_file_name() {
    let (mut a, dir) = with_dirs();
    a.run(Action::NewDesign);
    render(&mut a, W, H);
    drag_to!(&mut a, tray_item(&a, 0), centre(drawn_panes(&a)[0].1));
    key(&mut a, KeyCode::Esc);
    for name in ["Singularity edited", "cockpit-edited", "...", "   ", ""] {
        a.toast = None;
        typed(&mut a, name);
        key(&mut a, KeyCode::Enter);
        assert!(a.naming().is_some_and(|n| n.text == name) && a.building(), "{name:?}: the prompt stays, the build too");
        let said = a.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
        let want = if crate::layouts::file_id(name).is_empty() {
            "Couldn't save the layout: a layout needs a name".to_string()
        } else {
            format!("There is already a design called \"{}\"", name.trim())
        };
        assert_eq!(said, want, "{name:?}");
        for _ in 0..name.chars().count() {
            key(&mut a, KeyCode::Backspace);
        }
    }
    assert!(crate::layouts::load_dir(&dir.path().join("layouts")).0.is_empty(), "nothing written");
}

/// Owner re-test (#57): with a Widget focused in the Tray, `x`, Delete or Backspace removes it, as its `×` does
/// (Home, which has no `×`, is refused as the Tray refuses it); the focus goes to the next Widget, else the one before, else back to the Panes.
/// The legend says `x remove` meanwhile.
#[test]
fn x_delete_or_backspace_removes_the_focused_widget_from_the_tray() {
    use crate::arrange::TrayStop;
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    for w in [Widget::Fft, Widget::Loudness, Widget::Lyrics] {
        a.focus_on(w);
        key(&mut a, KeyCode::Char('t'));
    }
    assert_eq!(a.layout.tray(), &[Widget::Fft, Widget::Loudness, Widget::Lyrics]);
    let stop = |a: &App| a.arranging.as_ref().unwrap().tray_focus;
    a.focus_on(Widget::Queue);
    render(&mut a, W, H);
    key(&mut a, KeyCode::Down);
    while stop(&a) != Some(TrayStop::Widget(Widget::Fft)) {
        key(&mut a, KeyCode::Left);
    }
    key(&mut a, KeyCode::Right);
    assert_eq!(stop(&a), Some(TrayStop::Widget(Widget::Loudness)));
    assert!(render(&mut a, W, H).contains("x remove"), "the legend says so");
    // `x`: gone, the focus on the next one
    key(&mut a, KeyCode::Char('x'));
    assert_eq!(a.layout.tray(), &[Widget::Fft, Widget::Lyrics]);
    assert_eq!(stop(&a), Some(TrayStop::Widget(Widget::Lyrics)));
    // Delete on the last one: the one before
    key(&mut a, KeyCode::Delete);
    assert_eq!(a.layout.tray(), &[Widget::Fft]);
    assert_eq!(stop(&a), Some(TrayStop::Widget(Widget::Fft)));
    // Backspace on the only one: back to the Panes
    key(&mut a, KeyCode::Backspace);
    assert!(a.layout.tray().is_empty() && stop(&a).is_none() && a.focused().is_some());
    assert!(!render(&mut a, W, H).contains("x remove"), "the Panes' legend again");
    // an undo brings it back, as for the `×`
    a.run(Action::UndoLayout);
    assert_eq!(a.layout.tray(), &[Widget::Fft]);
    // on a Pane, Delete and Backspace do nothing
    let before = a.layout.clone();
    key(&mut a, KeyCode::Delete);
    key(&mut a, KeyCode::Backspace);
    assert_eq!(a.layout, before);
    assert!(a.arranging.is_some());
    // Home cannot be removed (it has no `×`): refused with the words a removal of it gets, the focus stays on it
    let mut a = app();
    a.run(Action::NewDesign);
    render(&mut a, W, H);
    key(&mut a, KeyCode::Down);
    assert_eq!(stop(&a), Some(TrayStop::Widget(Widget::Home)));
    for k in [KeyCode::Char('x'), KeyCode::Delete, KeyCode::Backspace] {
        a.toast = None;
        key(&mut a, k);
        assert_eq!(a.layout.tray(), &[Widget::Home]);
        assert_eq!(stop(&a), Some(TrayStop::Widget(Widget::Home)));
        assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("never removed")), "{:?}", a.toast);
    }
}

/// Review of #83: a layout change that takes the focused Widget out of the Tray (Ctrl+Z undoing its `t`, the mouse's
/// `×` on its chip) moves the focus on as `x` does, so the legend never says `x remove` while `x` would close a Pane.
#[test]
fn the_trays_focus_follows_an_undo_or_the_mouse_taking_its_widget_away() {
    use crate::arrange::TrayStop;
    let stop = |a: &App| a.arranging.as_ref().unwrap().tray_focus;
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    for w in [Widget::Fft, Widget::Loudness] {
        a.focus_on(w);
        key(&mut a, KeyCode::Char('t'));
    }
    a.focus_on(Widget::Queue);
    render(&mut a, W, H);
    key(&mut a, KeyCode::Down);
    while stop(&a) != Some(TrayStop::Widget(Widget::Loudness)) {
        key(&mut a, KeyCode::Right);
    }
    // Ctrl+Z undoes the last `t`: Loudness is back in its Pane, the focus on the one before it in the Tray
    ctrl(&mut a, 'z');
    assert_eq!(a.layout.tray(), &[Widget::Fft]);
    assert_eq!(stop(&a), Some(TrayStop::Widget(Widget::Fft)));
    assert!(render(&mut a, W, H).contains("x remove"));
    let panes = a.layout.panes().len();
    key(&mut a, KeyCode::Char('x'));
    assert!(a.layout.tray().is_empty() && a.layout.panes().len() == panes, "x removed it from the Tray, no Pane closed");
    // the removal undone, then Fft's own `t` undone with the focus on it: nothing left in the Tray, so the focus is
    // back in the Panes and the legend says so
    ctrl(&mut a, 'z');
    assert_eq!(a.layout.tray(), &[Widget::Fft]);
    render(&mut a, W, H);
    key(&mut a, KeyCode::Down);
    while stop(&a) != Some(TrayStop::Widget(Widget::Fft)) {
        key(&mut a, KeyCode::Left);
    }
    ctrl(&mut a, 'z');
    assert!(a.layout.tray().is_empty() && stop(&a).is_none() && a.focused().is_some());
    let s = render(&mut a, W, H);
    assert!(!s.contains("x remove") && s.contains("× x close"), "the Panes' legend");
    // the mouse's × on the focused chip: the focus goes on as well
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    for w in [Widget::Fft, Widget::Loudness] {
        a.focus_on(w);
        key(&mut a, KeyCode::Char('t'));
    }
    a.focus_on(Widget::Queue);
    render(&mut a, W, H);
    key(&mut a, KeyCode::Down);
    while stop(&a) != Some(TrayStop::Widget(Widget::Fft)) {
        key(&mut a, KeyCode::Left);
    }
    render(&mut a, W, H);
    let x = a.hit_rect(&Hit::TrayRemove(Widget::Fft)).unwrap();
    click!(&mut a, (x.x, x.y));
    assert_eq!(a.layout.tray(), &[Widget::Loudness]);
    assert!(stop(&a).is_none_or(|s| s == TrayStop::Widget(Widget::Loudness)), "{:?}", stop(&a));
    let s = render(&mut a, W, H);
    let panes = a.layout.panes().len();
    let says_remove = s.contains("x remove");
    key(&mut a, KeyCode::Char('x'));
    if says_remove {
        assert!(a.layout.tray().is_empty() && a.layout.panes().len() == panes, "the legend said remove: x removed");
    } else {
        assert_eq!(a.layout.tray(), &[Widget::Loudness], "the legend said close: the Tray stays");
    }
}

/// Review of #42 (full keyboard): "+ Widget" is the Tray's last focus stop: → past the last Widget, or ↓ from a bottom
/// Pane when the Tray is empty (in a New design once Home is placed); Enter opens the picker for the Tray, lit in the
/// focus colour meanwhile; ← goes back to the last Widget, ↑ to the Panes.
#[test]
fn the_keyboard_reaches_plus_widget_on_the_tray_even_when_it_is_empty() {
    use crate::arrange::TrayStop;
    let mut a = app();
    a.run(Action::NewDesign);
    render(&mut a, W, H);
    let stop = |a: &App| a.arranging.as_ref().unwrap().tray_focus;
    // Home waits: ↓ lands on it, → goes on to "+ Widget"
    key(&mut a, KeyCode::Down);
    assert_eq!(stop(&a), Some(TrayStop::Widget(Widget::Home)));
    key(&mut a, KeyCode::Right);
    assert_eq!(stop(&a), Some(TrayStop::Add));
    key(&mut a, KeyCode::Right);
    assert_eq!(stop(&a), Some(TrayStop::Add), "nothing after it");
    let buf = render_buf(&mut a, W, H);
    let lit = drawn(&buf, " + Widget ");
    assert_eq!(lit[1].1.bg, a.theme.chip(a.theme.accent).bg, "in the focus colour");
    key(&mut a, KeyCode::Left);
    assert_eq!(stop(&a), Some(TrayStop::Widget(Widget::Home)));
    // Home into the Pane: Enter, ↑, Enter; the Tray is empty now
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Enter);
    assert!(a.layout.tray().is_empty() && a.is_focused(Widget::Home));
    render(&mut a, W, H);
    // ↓ from the bottom Pane: "+ Widget" (the only stop); Enter opens the picker for the Tray; a pick joins it
    key(&mut a, KeyCode::Down);
    assert_eq!(stop(&a), Some(TrayStop::Add));
    key(&mut a, KeyCode::Enter);
    assert!(a.widget_picker().is_some_and(|p| p.tray));
    typed(&mut a, "queue");
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout.tray(), [Widget::Queue]);
    assert_eq!(stop(&a), Some(TrayStop::Add), "the focus stays on it");
    // ↑: back to the Pane above
    key(&mut a, KeyCode::Up);
    assert!(stop(&a).is_none() && a.is_focused(Widget::Home));
}

// ── the owner's Arrange-mode test (#41, #43) ──────────────────────────

/// Where `s` is drawn in `b`: its first cell.
fn spot(b: &Buffer, s: &str) -> Option<(u16, u16)> {
    let n = s.chars().count() as u16;
    (0..b.area.height).flat_map(|y| (0..b.area.width.saturating_sub(n)).map(move |x| (x, y))).find(|&(x, y)| row(b, y, x, x + n) == s)
}

/// The owner: the chip by a dragged title "teleporting all over the place" as the pointer went a little up and down.
/// Over small moves it keeps one place beside the pointer, the target's label doesn't hop round it, and by the
/// screen's right edge the chip stops instead of jumping to the pointer's other side.
#[test]
fn the_chip_keeps_its_place_by_the_pointer_over_small_moves() {
    let mut a = app();
    let (x, y) = title_spot(&a, Widget::Stream);
    press(&mut a, x, y);
    // (the first move turns Arrange mode on, which makes room for the Tray: the Panes are where it puts them after)
    drag(&mut a, x + 6, y + 3);
    render(&mut a, W, H);
    let (cx, cy) = centre(pane(&a, Widget::Queue));
    let mut labels = vec![];
    // a row up and down and a column or two sideways, round the queue's middle (where its label is)
    for (dx, dy) in [(0i32, 0i32), (1, -1), (0, 0), (1, 1), (-1, 0), (0, -1), (2, 1), (1, 0), (0, -1), (1, 1), (0, 0), (-1, -1)] {
        let (px, py) = ((cx as i32 + dx) as u16, (cy as i32 + dy) as u16);
        drag(&mut a, px, py);
        let buf = render_buf(&mut a, W, H);
        // the grip: two cells right of the pointer, then " ⠿ stream"
        assert_eq!(spot(&buf, "⠿ stream"), Some((px + 3, py)), "the chip beside the pointer at ({px}, {py})");
        labels.push(spot(&buf, " swap with queue ").expect("the label"));
    }
    let hops = labels.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(hops <= 1, "the label hopped round the chip: {labels:?}");
    // along the right edge and back, a column at a time: the chip moves at most as far as the pointer, never across it
    let mut was: Option<(u16, (u16, u16))> = None;
    for px in (W - 16..W).chain((W - 16..W).rev()) {
        drag(&mut a, px, cy);
        let buf = render_buf(&mut a, W, H);
        let at = spot(&buf, "⠿ stream").expect("the chip");
        assert!(at.0 >= px.min(W - 9), "the chip stays right of the pointer until the edge stops it: {at:?} at {px}");
        if let Some((p, c)) = was {
            assert!(at.0.abs_diff(c.0) <= px.abs_diff(p), "a column's move moved the chip {} columns", at.0.abs_diff(c.0));
        }
        was = Some((px, at));
    }
    key(&mut a, KeyCode::Esc);
    release(&mut a, W - 1, cy);
}

/// The owner: a Pane's Widget carried down into the Tray with the arrows was drawn above the Tray, by the play
/// button. Its chip is in the Tray's row, after what waits there (where Enter puts it), and nothing else on screen
/// changes but the lit Pane it came from, the Tray and the legend.
#[test]
fn a_widget_carried_into_the_tray_with_the_keys_is_shown_in_the_tray() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Fft);
    key(&mut a, KeyCode::Char('t'));
    a.focus_on(Widget::Queue);
    let before = render_buf(&mut a, W, H);
    let own = pane(&a, Widget::Queue);
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Down);
    assert!(a.tile.as_ref().is_some_and(|t| t.tray), "aimed at the Tray");
    let buf = render_buf(&mut a, W, H);
    let tray = a.tray_rect();
    let chip = spot(&buf, "⠿ queue").expect("the chip");
    assert_eq!(chip.1, tray.y + 1, "in the Tray's row, not on its frame or above it");
    let fft = spot(&buf, &format!(" ⠿ {} ", crate::ui::pane_name(a.design, Widget::Fft))).unwrap();
    assert!(chip.0 > fft.0 + 6 && chip.0 < tray.right() - 2, "after the Widget waiting there: {chip:?} {fft:?}");
    let inside = |r: Rect, x: u16, y: u16| x >= r.x && x < r.right() && y >= r.y && y < r.bottom();
    for y in 0..H - 1 {
        for x in 0..W {
            if !inside(tray, x, y) && !inside(own, x, y) {
                let (p, q) = (&before[(x, y)], &buf[(x, y)]);
                assert!(p.symbol() == q.symbol() && p.fg == q.fg && p.bg == q.bg, "({x}, {y}) changed from {p:?} to {q:?}");
            }
        }
    }
    // an empty Tray: at its start, in place of "drop a Widget here…"
    let mut e = app();
    key(&mut e, KeyCode::Char('m'));
    e.focus_on(Widget::Queue);
    key(&mut e, KeyCode::Enter);
    key(&mut e, KeyCode::Down);
    let buf = render_buf(&mut e, W, H);
    let t = e.tray_rect();
    assert_eq!(spot(&buf, "⠿ queue"), Some((t.x + 3, t.y + 1)));
    assert!(spot(&buf, "drop a Widget").is_none());
    // Enter sets it aside there
    key(&mut e, KeyCode::Enter);
    assert_eq!(e.layout.tray(), [Widget::Queue]);
}

/// Owner, Q70: `m`, which came into Arrange mode, leaves it as Esc does: a Widget picked up is put back first, `s`'s
/// wait for its arrow ends, and with nothing picked up it is Done (asking for a name if anything changed).
#[test]
fn m_leaves_arrange_mode_as_esc_does() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    assert!(a.arranging.is_some());
    key(&mut a, KeyCode::Char('m'));
    assert!(a.arranging.is_none() && a.modal().is_none(), "nothing changed: Done, no prompt");
    // a Widget picked up: put back, still arranging
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Queue);
    let before = a.layout.clone();
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Char('m'));
    assert!(a.tile.is_none() && a.arranging.is_some() && a.layout == before);
    // `s` waiting: the wait ends, nothing else
    key(&mut a, KeyCode::Char('s'));
    key(&mut a, KeyCode::Char('m'));
    assert!(a.arranging.as_ref().is_some_and(|r| !r.split_next));
    // from the Tray's focus too
    key(&mut a, KeyCode::Down);
    assert!(a.arranging.as_ref().unwrap().tray_focus.is_some());
    key(&mut a, KeyCode::Char('m'));
    assert!(a.arranging.is_none());
    // with a change: Done asks for its name, as Esc does
    let (mut a, _dir) = with_dirs();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Fft);
    key(&mut a, KeyCode::Char('x'));
    key(&mut a, KeyCode::Char('m'));
    assert!(a.arranging.is_none() && a.naming().is_some());
}

/// Owner, Q71: `w` opens "+ Widget"'s picker from anywhere in Arrange mode (a Pane or the Tray focused); a pick goes
/// into the Tray. While a Widget is carried the keys aim it, and `w` does nothing.
#[test]
fn w_opens_the_trays_widget_picker_anywhere_in_arrange_mode() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    a.focus_on(Widget::Fft);
    key(&mut a, KeyCode::Char('x'));
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Char('w'));
    assert!(a.widget_picker().is_some_and(|p| p.tray));
    key(&mut a, KeyCode::Esc);
    assert!(a.widget_picker().is_none() && a.arranging.is_some());
    // from the Tray's focus
    key(&mut a, KeyCode::Down);
    assert!(a.arranging.as_ref().unwrap().tray_focus.is_some());
    key(&mut a, KeyCode::Char('w'));
    assert!(a.widget_picker().is_some_and(|p| p.tray));
    typed(&mut a, "starfield");
    key(&mut a, KeyCode::Enter);
    assert!(a.layout.tray().contains(&Widget::Starfield));
    // carrying: nothing
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Enter);
    key(&mut a, KeyCode::Char('w'));
    assert!(a.widget_picker().is_none() && a.tile.is_some());
}

/// A Widget picked up from the Tray with Enter, aimed at the Tray (where it is): its chip is on its own place in the
/// Tray's row, never on the Tray's frame (where it covered "Tray" or "+ Widget"); nothing else on screen changes
/// but the Tray and the legend.
#[test]
fn a_widget_picked_up_from_the_tray_is_shown_in_its_place_there() {
    let mut a = app();
    key(&mut a, KeyCode::Char('m'));
    for w in [Widget::Fft, Widget::Loudness] {
        a.focus_on(w);
        key(&mut a, KeyCode::Char('t'));
    }
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Left);
    assert_eq!(a.arranging.as_ref().unwrap().tray_focus, Some(crate::arrange::TrayStop::Widget(Widget::Fft)));
    let before = render_buf(&mut a, W, H);
    let tray = a.tray_rect();
    let name = crate::ui::pane_name(a.design, Widget::Fft);
    let own = spot(&before, &format!(" ⠿ {name} ")).unwrap();
    key(&mut a, KeyCode::Enter);
    let buf = render_buf(&mut a, W, H);
    assert_eq!(spot(&buf, &format!("⠿ {name}")), Some((own.0 + 1, tray.y + 1)), "on its own place in the Tray's row");
    assert_eq!(row(&buf, tray.y, tray.x, tray.right()), row(&before, tray.y, tray.x, tray.right()), "the Tray's frame as it was");
    let inside = |x: u16, y: u16| x >= tray.x && x < tray.right() && y >= tray.y && y < tray.bottom();
    for y in 0..H - 1 {
        for x in 0..W {
            if !inside(x, y) {
                let (p, q) = (&before[(x, y)], &buf[(x, y)]);
                assert!(p.symbol() == q.symbol() && p.fg == q.fg && p.bg == q.bg, "({x}, {y}) changed from {p:?} to {q:?}");
            }
        }
    }
    // ↑ aims into the Panes, ↓ back at the Tray: on its place again
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Down);
    let buf = render_buf(&mut a, W, H);
    assert_eq!(spot(&buf, &format!("⠿ {name}")), Some((own.0 + 1, tray.y + 1)));
}

/// A Tray holding more than its strip shows: a Pane's Widget aimed at it scrolls the strip to its end, so the chip is
/// shown where Enter puts it (after the last Widget, not after the last one that happened to be shown).
#[test]
fn a_widget_aimed_at_a_full_tray_shows_its_end() {
    let mut a = App::new(None);
    render(&mut a, 100, 40);
    key(&mut a, KeyCode::Char('m'));
    for w in [Widget::Fft, Widget::Loudness, Widget::Lyrics, Widget::Stream, Widget::Library] {
        a.focus_on(w);
        key(&mut a, KeyCode::Char('t'));
    }
    for w in ["starfield", "rain", "levels", "glow"] {
        a.open_tray_picker();
        typed(&mut a, w);
        key(&mut a, KeyCode::Enter);
    }
    render(&mut a, 100, 40);
    if let Some(r) = a.arranging.as_mut() {
        r.tray_first = 0;
    }
    let buf = render_buf(&mut a, 100, 40);
    let last = *a.layout.tray().last().unwrap();
    let last_name = crate::ui::pane_name(a.design, last);
    assert!(spot(&buf, &format!(" ⠿ {last_name} ")).is_none(), "the strip doesn't show its end yet");
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Enter);
    while !a.tile.as_ref().is_some_and(|t| t.tray) {
        key(&mut a, KeyCode::Down);
    }
    let buf = render_buf(&mut a, 100, 40);
    let tray = a.tray_rect();
    // the last Widget whole, with its ×, then the chip whole, clear of it and of the ›
    let item = format!(" ⠿ {last_name} × ");
    let shown = spot(&buf, &item).expect("the last Widget shown whole");
    let chip = spot(&buf, "⠿ queue ").expect("the chip whole");
    assert!(chip.1 == tray.y + 1 && chip.0 > shown.0 + item.chars().count() as u16, "after the last Widget: {chip:?} {shown:?}");
    assert!(chip.0 + 7 < tray.right() - 2, "inside the strip: {chip:?}");
    // set aside there: the strip shows it
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout.tray().last(), Some(&Widget::Queue));
    let buf = render_buf(&mut a, 100, 40);
    assert!(spot(&buf, " ⠿ queue × ").is_some_and(|(_, y)| y == tray.y + 1), "{}", row(&buf, tray.y + 1, 0, 100));
}

/// `arrange` bound to a key with Ctrl (keys.toml): pressed while arranging it leaves, as Esc does (the legend says so).
#[test]
fn arrange_bound_with_ctrl_leaves_arrange_mode_too() {
    use singularity_config::{Keys, OneOrMany};
    let mut a = app();
    a.keymap = crate::keymap::Keymap::from_keys(&Keys { bindings: [("arrange".to_string(), OneOrMany::One("ctrl+e".into()))].into(), ..Default::default() });
    assert!(a.keymap.problems.is_empty(), "{:?}", a.keymap.problems);
    ctrl(&mut a, 'e');
    assert!(a.arranging.is_some());
    // a Widget picked up: put back first
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Enter);
    ctrl(&mut a, 'e');
    assert!(a.tile.is_none() && a.arranging.is_some());
    ctrl(&mut a, 'e');
    assert!(a.arranging.is_none());
}
