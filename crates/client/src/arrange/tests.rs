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
    let floor = drawn_panes(&a);
    let before = a.layout.clone();
    let (x, y) = title_spot(&a, Widget::Stream);
    press(&mut a, x, y);
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
    let (area, aspect) = (a.pane_area(), a.cell_aspect());
    a.layout = a.layout.split(pid(&a, Widget::Queue), Side::Right, None, area, aspect).unwrap();
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
    // over the top bar or the player: no Pane, no drop
    for out in [0, H - 2] {
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
    assert!(a.layout.fits(a.pane_area(), a.cell_aspect()));
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
fn splitting_offers_only_widgets_not_on_screen_and_adds_the_one_picked() {
    let mut a = app();
    a.focus_on(Widget::Fft);
    a.run(Action::SplitRight);
    let p = a.widget_pick().expect("the Widget list opens");
    assert!(p.items.contains(&Widget::Scope) && !p.items.contains(&Widget::Fft) && !p.items.contains(&Widget::Lyrics) && !p.items.contains(&Widget::Home), "{:?}", p.items);
    let at = p.items.iter().position(|w| *w == Widget::Scope).unwrap();
    let shot = render(&mut a, W, H);
    assert!(shot.contains("New pane right of fft"), "{shot}");
    for _ in 0..at {
        key(&mut a, KeyCode::Down);
    }
    key(&mut a, KeyCode::Enter);
    assert!(a.widget_pick().is_none() && a.layout.has(Widget::Scope) && a.layout.has(Widget::Fft));
    render(&mut a, W, H);
    let (f, s) = (pane(&a, Widget::Fft), pane(&a, Widget::Scope));
    assert_eq!((f.y, f.height, f.right()), (s.y, s.height, s.x));
    assert_eq!(a.focused(), Some(Widget::Scope));
    // the tree stays unique: splitting again never offers what's there
    a.run(Action::SplitDown);
    assert!(!a.widget_pick().unwrap().items.contains(&Widget::Scope));
    key(&mut a, KeyCode::Esc);
    assert!(a.widget_pick().is_none());
}

#[test]
fn a_split_with_no_room_says_so() {
    let mut a = App::new(None);
    render(&mut a, 50, 14);
    a.focus_on(Widget::Library);
    // 50 columns: Singularity's four columns of 12 take 48 of them, no room for a fifth
    a.run(Action::SplitRight);
    assert!(a.widget_pick().is_none() && a.toast.as_ref().is_some_and(|(t, _)| t.contains("No room")), "{:?}", a.toast);
}

#[test]
fn right_clicking_a_title_offers_replace_split_close_and_move() {
    let mut a = app();
    let (x, y) = title_spot(&a, Widget::Fft);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    let items: Vec<String> = a.menu().unwrap().items.iter().map(|(l, _)| l.clone()).collect();
    assert_eq!(items, ["Replace widget…", "Split right", "Split down", "Close pane", "Move pane"]);
    let shot = render(&mut a, W, H);
    assert!(shot.contains("Split down"));
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.widget_pick().unwrap().side, Side::Bottom);
    key(&mut a, KeyCode::Esc);
    let (x, y) = title_spot(&a, Widget::Home);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    assert!(!a.menu().unwrap().items.iter().any(|(l, _)| l == "Close pane"), "Home can't close");
}

#[test]
fn the_first_edit_of_a_built_in_design_asks_for_a_name_and_the_design_follows_its_file() {
    let (mut a, dir) = with_dirs();
    let (x, y) = title_spot(&a, Widget::Stream);
    let q = pane(&a, Widget::Queue);
    press(&mut a, x, y);
    drag(&mut a, centre(q).0, centre(q).1);
    release(&mut a, centre(q).0, centre(q).1);
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
    // further edits save into the same file without asking
    render(&mut a, W, H);
    let lyrics = pane(&a, Widget::Lyrics);
    let (x, y) = title_spot(&a, Widget::Loudness);
    press(&mut a, x, y);
    drag(&mut a, centre(lyrics).0, centre(lyrics).1);
    release(&mut a, centre(lyrics).0, centre(lyrics).1);
    assert!(a.naming().is_none());
    let on_disk = crate::layouts::load_dir(&dir.path().join("layouts")).0;
    assert_eq!(on_disk.len(), 1);
    assert_eq!(on_disk[0].tree, a.layout);
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
    a.layout = swapped(Node::singularity(), Widget::Stream, Widget::Queue);
    a.edited();
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
    assert_eq!(crate::layouts::load_dir(&dir.path().join("layouts")).0.len(), 1);
}

#[test]
fn esc_keeps_one_unnamed_edit_per_design_that_later_edits_overwrite() {
    let (mut a, dir) = with_dirs();
    let layouts = dir.path().join("layouts");
    a.layout = swapped(Node::singularity(), Widget::Stream, Widget::Queue);
    a.edited();
    key(&mut a, KeyCode::Esc);
    assert!(a.naming().is_none());
    assert_eq!(a.layout_name.as_deref(), Some("singularity-edited"));
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert!(saved[0].edited && saved[0].name == "Singularity (edited)" && saved[0].tree == a.layout);
    // more edits overwrite it, never a pile
    a.layout = swapped(a.layout.clone(), Widget::Home, Widget::Lyrics);
    a.edited();
    assert!(a.naming().is_none());
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].tree, a.layout);
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
    key(&mut a, KeyCode::Esc);
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].tree, a.layout);
    // another Design has its own
    a.set_design(3);
    a.layout_name = None;
    a.layout = swapped(Node::cockpit(), Widget::Fft, Widget::Loudness);
    a.edited();
    key(&mut a, KeyCode::Esc);
    assert_eq!(crate::layouts::load_dir(&layouts).0.len(), 2);
}

#[test]
fn reset_design_goes_back_to_the_built_in_one() {
    let (mut a, dir) = with_dirs();
    let layouts = dir.path().join("layouts");
    a.layout = swapped(Node::singularity(), Widget::Stream, Widget::Queue);
    a.edited();
    key(&mut a, KeyCode::Esc);
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
    assert_ne!(saved[0].tree, Node::singularity());
}

#[test]
fn letting_go_of_a_divider_is_an_edit() {
    let (mut a, _dir) = with_dirs();
    let grip = a.dividers[0].grip;
    press(&mut a, grip.x, grip.y + 5);
    drag(&mut a, grip.x + 8, grip.y + 5);
    assert!(a.naming().is_none(), "not while it moves");
    release(&mut a, grip.x + 8, grip.y + 5);
    assert!(a.naming().is_some());
}

#[test]
fn a_divider_will_not_squash_the_well() {
    let mut a = app();
    // the divider below the well, dragged as far up as it goes, then as far down
    for target in [0u16, 100] {
        let well = pane(&a, Widget::Well);
        let d = a.dividers.iter().find(|d| d.grip.y + 1 == well.bottom() && d.axis == crate::layout::Axis::Vertical).cloned().unwrap();
        press(&mut a, d.grip.x + 3, d.grip.y);
        drag(&mut a, d.grip.x + 3, target);
        release(&mut a, d.grip.x + 3, target);
        render(&mut a, W, H);
        assert!(a.layout.valid(a.pane_area(), a.cell_aspect()));
        assert!(pane(&a, Widget::Well).height >= crate::layout::WELL_MIN_H);
    }
}

#[test]
fn move_mode_swaps_resizes_and_closes_from_the_keyboard() {
    let mut a = app();
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Char('m'));
    assert_eq!(a.move_mode.and_then(|p| a.layout.widget_in(p)), Some(Widget::Queue));
    let shot = render(&mut a, W, H);
    assert!(shot.contains("MOVE queue") && shot.contains("swap"), "the bar says so");
    // ↑ swaps with the neighbour above: the Widgets trade Panes, Move mode goes with the queue
    let (q, h) = (pane(&a, Widget::Queue), pane(&a, Widget::Home));
    assert_eq!(q.x, h.x, "queue sits under home");
    key(&mut a, KeyCode::Up);
    render(&mut a, W, H);
    assert_eq!(pane(&a, Widget::Queue), h);
    assert_eq!(pane(&a, Widget::Home), q);
    assert_eq!(a.move_mode.and_then(|p| a.layout.widget_in(p)), Some(Widget::Queue));
    // Shift+arrows don't split any more: the same swap back
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    render(&mut a, W, H);
    assert_eq!(pane(&a, Widget::Queue), q);
    // + grows, - shrinks (the queue's Pane is in a stack: by a row)
    let h0 = pane(&a, Widget::Queue).height;
    key(&mut a, KeyCode::Char('-'));
    render(&mut a, W, H);
    assert_eq!(pane(&a, Widget::Queue).height, h0 - 1);
    key(&mut a, KeyCode::Char('+'));
    key(&mut a, KeyCode::Char('+'));
    render(&mut a, W, H);
    assert_eq!(pane(&a, Widget::Queue).height, h0 + 1);
    // the mouse doesn't act behind it: a click only ends the mode
    let (x, y) = title_spot(&a, Widget::Lyrics);
    press(&mut a, x, y);
    assert!(a.move_mode.is_none() && a.tile.is_none());
    // x closes a Pane that may close; the four stay
    a.focus_on(Widget::Fft);
    key(&mut a, KeyCode::Char('m'));
    key(&mut a, KeyCode::Char('x'));
    assert!(!a.layout.has(Widget::Fft) && a.move_mode.is_none());
    a.focus_on(Widget::Home);
    key(&mut a, KeyCode::Char('m'));
    key(&mut a, KeyCode::Char('x'));
    assert!(a.layout.has(Widget::Home) && a.move_mode.is_some(), "refused, still moving");
    key(&mut a, KeyCode::Enter);
    assert!(a.move_mode.is_none());
    key(&mut a, KeyCode::Char('m'));
    key(&mut a, KeyCode::Esc);
    assert!(a.move_mode.is_none());
}

#[test]
fn move_mode_with_nothing_that_way_says_so() {
    let mut a = app();
    a.focus_on(Widget::Lyrics);
    key(&mut a, KeyCode::Char('m'));
    let before = a.layout.clone();
    key(&mut a, KeyCode::Right);
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("Nothing right of")), "{:?}", a.toast);
    assert_eq!(a.layout, before);
    let shot = render(&mut a, W, H);
    assert!(shot.lines().last().unwrap().contains("Nothing right of"), "said in the bar");
}


#[test]
fn popups_and_move_mode_do_not_let_the_mouse_through() {
    let mut a = app();
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Char('m'));
    let before = a.layout.clone();
    let (x, y) = title_spot(&a, Widget::Fft);
    drag(&mut a, x, y);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    assert!(a.menu().is_none() && a.tile.is_none() && a.layout == before);
    // while the naming prompt is up, the keyboard goes to it, not to Move mode
    let (mut b, _d) = with_dirs();
    b.focus_on(Widget::Queue);
    key(&mut b, KeyCode::Char('m'));
    key(&mut b, KeyCode::Up);
    assert!(b.naming().is_some());
    let at = b.layout.clone();
    key(&mut b, KeyCode::Down);
    assert_eq!(b.layout, at);
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
                let s = slots[(next() % slots.len() as u64) as usize];
                match next() % 8 {
                    6 => ctrl(&mut a, 'z'),
                    7 => ctrl(&mut a, 'y'),
                    0..=2 => {
                        render(&mut a, w, h);
                        let (x, y) = title_spot(&a, s);
                        press(&mut a, x, y);
                        let floor = drawn_panes(&a);
                        for _ in 0..3 {
                            drag(&mut a, (next() % w as u64) as u16, (next() % (h - 1) as u64) as u16);
                            render(&mut a, w, h);
                            assert_eq!(drawn_panes(&a), floor, "{} {w}x{h}: a Pane moved mid-drag", design.id);
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
                        for _ in 0..4 {
                            let code = [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down, KeyCode::Char('+'), KeyCode::Char('-')][(next() % 6) as usize];
                            key(&mut a, code);
                        }
                        key(&mut a, KeyCode::Esc);
                    }
                    4 => {
                        a.focus_on(s);
                        a.run(if next() % 2 == 0 { Action::SplitRight } else { Action::SplitDown });
                        if let Some(p) = a.widget_pick() {
                            let i = (next() % p.items.len() as u64) as usize;
                            a.widget_chosen(i);
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
                assert!(tree.fits(a.pane_area(), a.cell_aspect()), "{} {w}x{h}: {tree:?}", design.id);
                assert!(crate::arrangement::REQUIRED.iter().all(|c| tree.has(*c)));
                assert!(a.tile.is_none() && a.widget_pick().is_none());
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
    // by the screen's right edge the chip goes left of the pointer, never off screen
    drag(&mut a, W - 1, cy);
    let buf = render_buf(&mut a, W, H);
    let r = a.chip_rect(Rect::new(0, 0, W, H), "⠿ stream").unwrap();
    assert!(r.right() < W && r.width <= crate::arrange::CHIP_MAX && r.height == 1, "{r:?}");
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
    // split, with the Widget list
    a.focus_on(Widget::Loudness);
    a.run(Action::SplitRight);
    let i = a.widget_pick().unwrap().items.iter().position(|w| *w == Widget::Scope).unwrap();
    a.widget_chosen(i);
    trees.push(a.layout.clone());
    // Move mode: a swap, a split beside, a size step
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Char('m'));
    key(&mut a, KeyCode::Up);
    trees.push(a.layout.clone());
    key(&mut a, KeyCode::Char('-'));
    trees.push(a.layout.clone());
    key(&mut a, KeyCode::Enter);
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
    // in Move mode, u and U
    a.focus_on(Widget::Home);
    key(&mut a, KeyCode::Char('m'));
    let here = a.layout.clone();
    key(&mut a, KeyCode::Char('u'));
    assert_eq!(a.layout, trees[trees.len() - 2]);
    key(&mut a, KeyCode::Char('U'));
    assert_eq!(a.layout, here);
    assert!(a.move_mode.is_some(), "still moving");
    a.toast = None;
    let shot = render(&mut a, W, H);
    assert!(shot.lines().last().unwrap().contains("u undo"));
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
    // 1. undone while it still asks for a name: the prompt goes, nothing was written; redo asks again
    swap(&mut a);
    let edit = a.layout.clone();
    assert!(a.naming().is_some());
    ctrl(&mut a, 'z');
    assert!(a.naming().is_none() && a.layout == built_in && a.layout_name.is_none() && files().is_empty());
    ctrl(&mut a, 'y');
    assert_eq!(a.layout, edit);
    assert_eq!(a.naming().map(|n| n.purpose), Some(crate::app::NameFor::Fork));
    // 2. Esc kept it as "Singularity (edited)": undo removes that file again and the config forgets it; redo brings it back
    key(&mut a, KeyCode::Esc);
    assert_eq!(files().len(), 1);
    render(&mut a, W, H);
    ctrl(&mut a, 'z');
    assert!(a.layout == built_in && a.layout_name.is_none() && files().is_empty(), "{:?}", files());
    assert!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap().contains("layout = \"\""));
    ctrl(&mut a, 'y');
    assert_eq!(a.layout_name.as_deref(), Some("singularity-edited"));
    let f = files();
    assert!(f.len() == 1 && f[0].edited && f[0].tree == edit);
    // 3. a name given: undo goes back to the built-in Design, and the named one stays in the picker as saved
    ctrl(&mut a, 'z');
    swap(&mut a);
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
    assert!(f.len() == 1 && f[0].id == "mine" && f[0].tree == edit);
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
    assert_eq!(crate::layouts::load_dir(&layouts).0[0].tree, closed, "it saves itself");
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, saved);
    let f = crate::layouts::load_dir(&layouts).0;
    assert!(f.len() == 1 && f[0].tree == saved && f[0].name == "Mine" && !f[0].edited, "the file is back too");
    ctrl(&mut a, 'y');
    assert_eq!(crate::layouts::load_dir(&layouts).0[0].tree, closed);
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
    let (area, aspect) = (a.pane_area(), a.cell_aspect());
    let home = pid(&a, Widget::Home);
    a.layout = a.layout.split(home, Side::Right, Some(Widget::Scope), area, aspect).unwrap().split(home, Side::Right, Some(Widget::Rain), area, aspect).unwrap();
    let three = a.layout.clone();
    a.close_pane(pid(&a, Widget::Rain));
    let two = a.layout.clone();
    let (w, h) = (50..160u16).map(|w| (w, 30u16)).find(|&(w, h)| {
        let body = Rect::new(0, 1, w, h - 5);
        !three.fits(body, 2.0) && two.fits(body, 2.0)
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
    let mut a = app();
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
    let mut b = app();
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
    let want = Node::classic().compute_shaped(a.pane_area(), a.cell_aspect()).rect_of(Widget::Home).unwrap();
    assert_eq!(pane(&a, Widget::Home), want, "the first frame is the new layout");
}

#[test]
fn an_empty_pane_has_nothing_to_move() {
    let mut a = app();
    let (area, aspect) = (a.pane_area(), a.cell_aspect());
    a.layout = a.layout.split(pid(&a, Widget::Queue), Side::Right, None, area, aspect).unwrap();
    render(&mut a, W, H);
    let empty = a.layout.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    a.focus = Some(empty);
    key(&mut a, KeyCode::Char('m'));
    assert!(a.move_mode.is_none());
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("nothing to move")), "{:?}", a.toast);
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
    // the login code is a layer, not a popup: the next frame lets go
    press(&mut a, x, y);
    drag(&mut a, q.0, q.1);
    assert!(a.tile.as_ref().is_some_and(|t| t.live));
    a.hear(singularity_proto::Event::Catalogue(singularity_proto::CatalogueStatus::LoginPending { url: "u".into(), code: "ABCD".into() }));
    let shot = render(&mut a, W, H);
    assert!(a.tile.is_none() && !shot.contains("⠿ stream"));
    release(&mut a, q.0, q.1);
}
