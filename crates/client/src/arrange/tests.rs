use super::*;
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
/// The pointer moved there and rests (a held Pane's zone has had its dwell).
fn drag(app: &mut App, x: u16, y: u16) {
    sweep(app, x, y);
    app.tile_settle(Instant::now() + DWELL);
}
/// The pointer passes through, without resting.
fn sweep(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Drag(MouseButton::Left), x, y);
}
fn release(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Up(MouseButton::Left), x, y);
}
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn shift(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::SHIFT));
}

/// A drawn Pane's rectangle, from the hit boxes of the last frame.
fn pane(app: &App, slot: &str) -> Rect {
    app.hits.iter().find_map(|(r, h)| matches!(h, Hit::Focus(s) if s == slot).then_some(*r)).unwrap_or_else(|| panic!("no pane {slot}"))
}
/// A spot on a Pane's title bar that is neither a button nor a divider's.
fn title_spot(app: &App, slot: &str) -> (u16, u16) {
    let r = pane(app, slot);
    (r.x + 3, r.y)
}
fn centre(r: Rect) -> (u16, u16) {
    (r.x + r.width / 2, r.y + r.height / 2)
}
fn rects(app: &App) -> Vec<(String, Rect)> {
    app.layout.compute_shaped(app.pane_area(), app.cell_aspect()).slots
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
    let (x, y) = title_spot(&a, "queue");
    press(&mut a, x, y);
    release(&mut a, x + 1, y);
    assert_eq!(a.focus, "queue");
    assert_eq!(a.layout, before);
    assert!(a.tile.is_none() && a.naming.is_none());
}

#[test]
fn dragging_a_title_onto_a_centre_swaps_with_a_live_preview() {
    let mut a = app();
    let (old_stream, old_queue) = (pane(&a, "stream"), pane(&a, "queue"));
    let before = a.layout.clone();
    let (x, y) = title_spot(&a, "stream");
    press(&mut a, x, y);
    let (cx, cy) = centre(old_queue);
    drag(&mut a, cx, cy);
    // nothing is committed yet, but the whole layout is drawn as it would be
    assert_eq!(a.layout, before);
    let shot = render(&mut a, W, H);
    assert_eq!(pane(&a, "stream"), old_queue, "the preview puts it where the pointer is");
    assert_eq!(pane(&a, "queue"), old_stream);
    assert!(shot.contains(" ⠿ stream "), "the held Pane follows the pointer");
    release(&mut a, cx, cy);
    assert!(a.tile.is_none());
    assert_eq!(rects(&a).iter().find(|(s, _)| s == "stream").unwrap().1, old_queue);
    assert_eq!(a.focus, "stream");
    let after = render(&mut a, W, H);
    assert!(!after.contains(" ⠿ "), "it is down");
}

#[test]
fn an_edge_band_splits_the_pane_and_the_screen_edge_makes_a_column() {
    let mut a = app();
    let q = pane(&a, "queue");
    let (x, y) = title_spot(&a, "fft");
    press(&mut a, x, y);
    // the left edge of the queue: fft goes to its left
    drag(&mut a, q.x + 1, q.y + q.height / 2);
    let Some(Zone::Beside(t, Side::Left)) = a.tile.as_ref().unwrap().zone.clone() else { panic!("{:?}", a.tile.as_ref().unwrap().zone) };
    assert_eq!(t, "queue");
    release(&mut a, q.x + 1, q.y + q.height / 2);
    render(&mut a, W, H);
    let (f, q2) = (pane(&a, "fft"), pane(&a, "queue"));
    assert_eq!((f.y, f.height, f.right()), (q2.y, q2.height, q2.x), "side by side: {f:?} {q2:?}");
    // the screen's left edge: a column of its own, the full height
    let (x, y) = title_spot(&a, "lyrics");
    press(&mut a, x, y);
    drag(&mut a, 0, 20);
    release(&mut a, 0, 20);
    render(&mut a, W, H);
    let l = pane(&a, "lyrics");
    assert_eq!((l.x, l.y, l.height), (0, a.pane_area().y, a.pane_area().height));
    assert!((20..=56).contains(&l.width), "{l:?}");
}

#[test]
fn esc_or_a_drop_nowhere_leaves_the_layout_alone() {
    let mut a = app();
    let before = a.layout.clone();
    let q = pane(&a, "queue");
    let (x, y) = title_spot(&a, "stream");
    press(&mut a, x, y);
    drag(&mut a, centre(q).0, centre(q).1);
    assert!(a.tile.as_ref().unwrap().preview.is_some());
    key(&mut a, KeyCode::Esc);
    assert!(a.tile.is_none());
    release(&mut a, centre(q).0, centre(q).1);
    assert_eq!(a.layout, before);
    render(&mut a, W, H);
    assert_eq!(pane(&a, "queue"), q, "drawn as it was");
    // over its own place: nothing to drop on, and the empty slot says it goes back there
    press(&mut a, x, y);
    drag(&mut a, x + 5, y + 4);
    assert!(a.tile.as_ref().unwrap().preview.is_none());
    assert!(!render(&mut a, W, H).contains("drop here"));
    release(&mut a, x + 5, y + 4);
    assert_eq!(a.layout, before);
    assert!(a.naming.is_none());
}

#[test]
fn a_drop_that_would_squash_a_pane_is_not_offered() {
    use crate::layout::{Axis::*, Size::*};
    let mut a = App::new(None);
    // 50 columns hold four Panes of 12 side by side, not five
    let row = |x: Node, y: Node| Node::split(Horizontal, Ratio(0.5), x, y);
    let top = row(Node::slot("library"), row(Node::slot("home"), row(Node::slot("queue"), Node::slot("fft"))));
    a.layout = Node::split(Vertical, Ratio(0.5), top, Node::slot("lyrics"));
    render(&mut a, 50, 24);
    let before = a.layout.clone();
    let (x, y) = title_spot(&a, "lyrics");
    press(&mut a, x, y);
    let q = pane(&a, "queue");
    // a whole column at the screen's left edge: the four left in the row would be 9 columns each
    let edge = (0, q.y + q.height / 2);
    drag(&mut a, edge.0, edge.1);
    assert!(a.tile.as_ref().unwrap().zone.is_none(), "no highlight");
    assert_eq!(render(&mut a, 50, 24).matches('⠿').count(), 1, "only the ghost");
    assert_eq!(pane(&a, "queue"), q, "the original layout stays");
    release(&mut a, edge.0, edge.1);
    assert_eq!(a.layout, before);
    assert!(a.naming.is_none());
    // the queue's edge, 12 columns wide, is no split (it would squash, and be uncomfortable anyway): its centre's
    press(&mut a, x, y);
    drag(&mut a, q.right() - 1, q.y + q.height / 2);
    assert_eq!(a.tile.as_ref().unwrap().zone, Some(Zone::Swap("queue".into())));
    drag(&mut a, centre(q).0, centre(q).1);
    assert!(matches!(a.tile.as_ref().unwrap().zone, Some(Zone::Swap(_))));
    release(&mut a, centre(q).0, centre(q).1);
    assert_ne!(a.layout, before);
}

#[test]
fn the_x_closes_a_pane_and_the_core_four_have_none() {
    let mut a = app();
    let shot = render(&mut a, W, H);
    assert!(shot.contains(" × "));
    for core in ["home", "library", "queue", "lyrics"] {
        assert!(!a.hits.iter().any(|(_, h)| matches!(h, Hit::Close(s) if s == core)), "{core} has a ×");
    }
    let fft = pane(&a, "fft");
    let old_loudness = pane(&a, "loudness");
    let (r, _) = a.hits.iter().find(|(_, h)| matches!(h, Hit::Close(s) if s == "fft")).cloned().unwrap();
    assert!(r.y == fft.y && r.right() <= fft.right());
    press(&mut a, r.x + 1, r.y);
    release(&mut a, r.x + 1, r.y);
    assert!(!a.layout.has("fft"));
    render(&mut a, W, H);
    assert_eq!(pane(&a, "loudness").right(), fft.right(), "the neighbour took its room");
    assert!(pane(&a, "loudness").width > old_loudness.width);
    // Close refuses on Home with a notice
    a.focus = "home".into();
    a.run(Action::ClosePane);
    assert!(a.layout.has("home") && a.toast.as_ref().is_some_and(|(t, _)| t.contains("never closed")), "{:?}", a.toast);
}

#[test]
fn splitting_offers_only_widgets_not_on_screen_and_adds_the_one_picked() {
    let mut a = app();
    a.focus = "fft".into();
    a.run(Action::SplitRight);
    let p = a.widget_pick.as_ref().expect("the Widget list opens");
    assert!(p.items.contains(&"scope") && !p.items.contains(&"fft") && !p.items.contains(&"lyrics") && !p.items.contains(&"home"), "{:?}", p.items);
    let at = p.items.iter().position(|w| *w == "scope").unwrap();
    let shot = render(&mut a, W, H);
    assert!(shot.contains("New pane right of fft"), "{shot}");
    for _ in 0..at {
        key(&mut a, KeyCode::Down);
    }
    key(&mut a, KeyCode::Enter);
    assert!(a.widget_pick.is_none() && a.layout.has("scope") && a.layout.has("fft"));
    render(&mut a, W, H);
    let (f, s) = (pane(&a, "fft"), pane(&a, "scope"));
    assert_eq!((f.y, f.height, f.right()), (s.y, s.height, s.x));
    assert_eq!(a.focus, "scope");
    // the tree stays unique: splitting again never offers what's there
    a.run(Action::SplitDown);
    assert!(!a.widget_pick.as_ref().unwrap().items.contains(&"scope"));
    key(&mut a, KeyCode::Esc);
    assert!(a.widget_pick.is_none());
}

#[test]
fn a_split_with_no_room_says_so() {
    let mut a = App::new(None);
    render(&mut a, 60, 20);
    a.focus = "library".into();
    // 60x20: nothing in Singularity can be halved again
    a.run(Action::SplitRight);
    assert!(a.widget_pick.is_none() && a.toast.as_ref().is_some_and(|(t, _)| t.contains("No room")), "{:?}", a.toast);
}

#[test]
fn right_clicking_a_title_offers_split_move_and_close() {
    let mut a = app();
    let (x, y) = title_spot(&a, "fft");
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    let items: Vec<String> = a.menu.as_ref().unwrap().items.iter().map(|(l, _)| l.clone()).collect();
    assert_eq!(items, ["Split right", "Split down", "Move pane", "Close pane"]);
    let shot = render(&mut a, W, H);
    assert!(shot.contains("Split down"));
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.widget_pick.as_ref().unwrap().side, Side::Bottom);
    key(&mut a, KeyCode::Esc);
    let (x, y) = title_spot(&a, "home");
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    assert!(!a.menu.as_ref().unwrap().items.iter().any(|(l, _)| l == "Close pane"), "Home can't close");
}

#[test]
fn the_first_edit_of_a_built_in_design_asks_for_a_name_and_the_design_follows_its_file() {
    let (mut a, dir) = with_dirs();
    let (x, y) = title_spot(&a, "stream");
    let q = pane(&a, "queue");
    press(&mut a, x, y);
    drag(&mut a, centre(q).0, centre(q).1);
    release(&mut a, centre(q).0, centre(q).1);
    let n = a.naming.clone().expect("the naming prompt");
    assert_eq!((n.text.as_str(), n.purpose), ("Singularity 2", crate::app::NameFor::Fork));
    let shot = render(&mut a, W, H);
    assert!(shot.contains("Keep your changes as a design") && shot.contains("Singularity 2"), "{shot}");
    assert!(Node::singularity() != a.layout, "the edit shows meanwhile");
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
    let lyrics = pane(&a, "lyrics");
    let (x, y) = title_spot(&a, "loudness");
    press(&mut a, x, y);
    drag(&mut a, centre(lyrics).0, centre(lyrics).1);
    release(&mut a, centre(lyrics).0, centre(lyrics).1);
    assert!(a.naming.is_none());
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
    a.layout = Node::singularity().swapped("stream", "queue").unwrap();
    a.edited();
    assert_eq!(a.naming.as_ref().unwrap().text, "Singularity 3", "2 is taken");
    for _ in 0..13 {
        key(&mut a, KeyCode::Backspace);
    }
    for c in "Singularity 2".chars() {
        key(&mut a, KeyCode::Char(c));
    }
    key(&mut a, KeyCode::Enter);
    assert!(a.naming.is_some(), "still asking");
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("already")));
    assert_eq!(crate::layouts::load_dir(&dir.path().join("layouts")).0.len(), 1);
}

#[test]
fn esc_keeps_one_unnamed_edit_per_design_that_later_edits_overwrite() {
    let (mut a, dir) = with_dirs();
    let layouts = dir.path().join("layouts");
    a.layout = Node::singularity().swapped("stream", "queue").unwrap();
    a.edited();
    key(&mut a, KeyCode::Esc);
    assert!(a.naming.is_none());
    assert_eq!(a.layout_name.as_deref(), Some("singularity-edited"));
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert!(saved[0].edited && saved[0].name == "Singularity (edited)" && saved[0].tree == a.layout);
    // more edits overwrite it, never a pile
    a.layout = a.layout.swapped("home", "lyrics").unwrap();
    a.edited();
    assert!(a.naming.is_none());
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].tree, a.layout);
    // it is in the Design picker, and survives a restart
    a.run(Action::PickDesign);
    assert!(a.picker.as_ref().unwrap().items.iter().any(|i| i.1 == "Singularity (edited)"));
    key(&mut a, KeyCode::Esc);
    let mut again = App::new(None);
    again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
    assert_eq!(again.layout, a.layout);
    // from the built-in Design again, an unnamed edit replaces it
    a.set_design(0);
    a.layout_name = None;
    a.layout = Node::singularity().swapped("fft", "loudness").unwrap();
    a.edited();
    key(&mut a, KeyCode::Esc);
    let saved = crate::layouts::load_dir(&layouts).0;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].tree, a.layout);
    // another Design has its own
    a.set_design(3);
    a.layout_name = None;
    a.layout = Node::cockpit().swapped("fft", "loudness").unwrap();
    a.edited();
    key(&mut a, KeyCode::Esc);
    assert_eq!(crate::layouts::load_dir(&layouts).0.len(), 2);
}

#[test]
fn reset_design_goes_back_to_the_built_in_one() {
    let (mut a, dir) = with_dirs();
    let layouts = dir.path().join("layouts");
    a.layout = Node::singularity().swapped("stream", "queue").unwrap();
    a.edited();
    key(&mut a, KeyCode::Esc);
    a.run(Action::ResetLayout);
    assert_eq!(a.layout, Node::singularity());
    assert_eq!(a.layout_name, None);
    assert!(crate::layouts::load_dir(&layouts).0.is_empty(), "the unnamed edit is dropped");
    // a Design of your own stays where it was saved
    a.layout = Node::singularity().swapped("stream", "queue").unwrap();
    a.save_layout("Mine");
    a.layout = a.layout.swapped("home", "lyrics").unwrap();
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
    assert!(a.naming.is_none(), "not while it moves");
    release(&mut a, grip.x + 8, grip.y + 5);
    assert!(a.naming.is_some());
}

#[test]
fn a_divider_will_not_squash_the_well() {
    let mut a = app();
    // the divider below the well, dragged as far up as it goes, then as far down
    let well = pane(&a, "well");
    for target in [0u16, 100] {
        let d = a.dividers.iter().find(|d| d.grip.y + 1 == well.bottom() && d.axis == crate::layout::Axis::Vertical).cloned().unwrap();
        press(&mut a, d.grip.x + 3, d.grip.y);
        drag(&mut a, d.grip.x + 3, target);
        release(&mut a, d.grip.x + 3, target);
        render(&mut a, W, H);
        assert!(a.layout.valid(a.pane_area(), a.cell_aspect()));
        assert!(pane(&a, "well").height >= crate::layout::WELL_MIN_H);
    }
}

#[test]
fn move_mode_swaps_splits_resizes_and_closes_from_the_keyboard() {
    let mut a = app();
    a.focus = "queue".into();
    key(&mut a, KeyCode::Char('m'));
    assert_eq!(a.move_mode.as_deref(), Some("queue"));
    let shot = render(&mut a, W, H);
    assert!(shot.contains("MOVE queue") && shot.contains("swap"), "the bar says so");
    // ↑ swaps with the neighbour above
    let (q, h) = (pane(&a, "queue"), pane(&a, "home"));
    assert_eq!(q.x, h.x, "queue sits under home");
    key(&mut a, KeyCode::Up);
    render(&mut a, W, H);
    assert_eq!(pane(&a, "queue"), h);
    assert_eq!(pane(&a, "home"), q);
    // Shift+→ puts it on the right of what's to its right, as a split
    shift(&mut a, KeyCode::Right);
    render(&mut a, W, H);
    let (qq, nb) = (pane(&a, "queue"), pane(&a, "well"));
    assert_eq!(qq.x, nb.right(), "beside the well, on its right: {qq:?} {nb:?}");
    // + grows, - shrinks
    let w = pane(&a, "queue").width;
    key(&mut a, KeyCode::Char('-'));
    render(&mut a, W, H);
    assert_eq!(pane(&a, "queue").width, w - 2);
    key(&mut a, KeyCode::Char('+'));
    key(&mut a, KeyCode::Char('+'));
    render(&mut a, W, H);
    assert_eq!(pane(&a, "queue").width, w + 2);
    // the mouse doesn't act behind it: a click only ends the mode
    let (x, y) = title_spot(&a, "lyrics");
    press(&mut a, x, y);
    assert!(a.move_mode.is_none() && a.tile.is_none());
    // x closes a Pane that may close; the four stay
    a.focus = "fft".into();
    key(&mut a, KeyCode::Char('m'));
    key(&mut a, KeyCode::Char('x'));
    assert!(!a.layout.has("fft") && a.move_mode.is_none());
    a.focus = "home".into();
    key(&mut a, KeyCode::Char('m'));
    key(&mut a, KeyCode::Char('x'));
    assert!(a.layout.has("home") && a.move_mode.is_some(), "refused, still moving");
    key(&mut a, KeyCode::Enter);
    assert!(a.move_mode.is_none());
    key(&mut a, KeyCode::Char('m'));
    key(&mut a, KeyCode::Esc);
    assert!(a.move_mode.is_none());
}

#[test]
fn move_mode_at_the_screen_edge_makes_a_column() {
    let mut a = app();
    a.focus = "lyrics".into();
    key(&mut a, KeyCode::Char('m'));
    // nothing to the right of it: → says so, ⇧→ is a column on the screen's right edge
    key(&mut a, KeyCode::Right);
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("Nothing right of") && t.contains("⇧→ makes it a whole column")), "{:?}", a.toast);
    shift(&mut a, KeyCode::Right);
    render(&mut a, W, H);
    let l = pane(&a, "lyrics");
    assert_eq!((l.right(), l.y, l.height), (W, a.pane_area().y, a.pane_area().height));
}

#[test]
fn a_move_with_no_room_changes_nothing() {
    let mut b = App::new(None);
    render(&mut b, 60, 20);
    b.focus = "stream".into();
    key(&mut b, KeyCode::Char('m'));
    let before = b.layout.clone();
    shift(&mut b, KeyCode::Right);
    assert_eq!(b.layout, before, "a column of 12 wouldn't fit beside this much");
    assert!(b.toast.as_ref().is_some_and(|(t, _)| t.contains("No room")), "{:?}", b.toast);
}

#[test]
fn popups_and_move_mode_do_not_let_the_mouse_through() {
    let mut a = app();
    a.focus = "queue".into();
    key(&mut a, KeyCode::Char('m'));
    let before = a.layout.clone();
    let (x, y) = title_spot(&a, "fft");
    drag(&mut a, x, y);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    assert!(a.menu.is_none() && a.tile.is_none() && a.layout == before);
    // while the naming prompt is up, the keyboard goes to it, not to Move mode
    let (mut b, _d) = with_dirs();
    b.focus = "queue".into();
    key(&mut b, KeyCode::Char('m'));
    key(&mut b, KeyCode::Up);
    assert!(b.naming.is_some());
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
            // Panes in motion, caught at random moments of it
            a.motion.on = true;
            render(&mut a, w, h);
            for _ in 0..120 {
                let slots: Vec<String> = a.layout.slot_names();
                let s = slots[(next() % slots.len() as u64) as usize].clone();
                match next() % 8 {
                    6 => ctrl(&mut a, 'z'),
                    7 => ctrl(&mut a, 'y'),
                    0..=2 => {
                        // grabbed where its title is drawn: mid-way, another Pane's ◂ ▸ can be on that spot (and a
                        // press there swaps that Pane's Widget, the queue's for the now-playing one)
                        a.motion.skip(motion::SETTLE);
                        render(&mut a, w, h);
                        let (x, y) = title_spot(&a, &s);
                        press(&mut a, x, y);
                        for _ in 0..3 {
                            drag(&mut a, (next() % w as u64) as u16, (next() % (h - 1) as u64) as u16);
                            render(&mut a, w, h);
                            a.motion.skip(motion::SETTLE * (next() % 4) as u32 / 3);
                        }
                        if next() % 4 == 0 {
                            key(&mut a, KeyCode::Esc);
                        }
                        let (x, y) = ((next() % w as u64) as u16, (next() % (h - 4) as u64) as u16);
                        release(&mut a, x, y);
                    }
                    3 => {
                        a.focus = s;
                        key(&mut a, KeyCode::Char('m'));
                        for _ in 0..4 {
                            let code = [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down, KeyCode::Char('+'), KeyCode::Char('-')][(next() % 6) as usize];
                            if next() % 2 == 0 { shift(&mut a, code) } else { key(&mut a, code) }
                        }
                        key(&mut a, KeyCode::Esc);
                    }
                    4 => {
                        a.focus = s;
                        a.run(if next() % 2 == 0 { Action::SplitRight } else { Action::SplitDown });
                        if let Some(p) = a.widget_pick.as_ref() {
                            let i = (next() % p.items.len() as u64) as usize;
                            a.widget_chosen(i);
                        }
                    }
                    _ => {
                        a.focus = s.clone();
                        a.close_pane(&s);
                    }
                }
                render(&mut a, w, h);
                a.motion.skip(motion::SETTLE * (next() % 3) as u32 / 4);
                render(&mut a, w, h);
                let body = a.pane_area();
                assert!(a.drawn.iter().all(|r| r.width >= 1 && r.height >= 1 && r.x >= body.x && r.y >= body.y && r.right() <= body.right() && r.bottom() <= body.bottom()), "{} {w}x{h}: {:?}", design.id, a.drawn);
                let tree = a.layout.clone();
                assert!(tree.valid(a.pane_area(), a.cell_aspect()), "{} {w}x{h}: {tree:?}", design.id);
                assert!(["home", "library", "queue", "lyrics"].iter().all(|c| tree.has(c)));
                assert!(a.tile.is_none() && a.widget_pick.is_none());
            }
        }
    }
}


// ── the held Pane, motion (owner, 2026-11-01) ─────────────────────────

fn render_buf(app: &mut App, w: u16, h: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| crate::ui::draw(f, app)).unwrap();
    t.backend().buffer().clone()
}

fn row(buf: &Buffer, y: u16, x0: u16, x1: u16) -> String {
    (x0..x1).map(|x| buf[(x, y)].symbol().to_string()).collect()
}

/// Where a Pane is drawn this frame (on its way somewhere, maybe), from the last render.
fn drawn(a: &App, slot: &str) -> Rect {
    let slots = a.shown_layout().compute_shaped(a.pane_area(), a.cell_aspect()).slots;
    a.drawn[slots.iter().position(|(s, _)| s == slot).unwrap()]
}

fn ctrl(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

#[test]
fn a_held_pane_floats_live_over_the_empty_slot_it_would_land_in() {
    let mut a = app();
    let (q, s) = (pane(&a, "queue"), pane(&a, "stream"));
    let (x, y) = title_spot(&a, "stream");
    press(&mut a, x, y);
    let (cx, cy) = centre(q);
    drag(&mut a, cx, cy);
    let buf = render_buf(&mut a, W, H);
    let f = a.float_rect(a.pane_area()).expect("it floats");
    // its own size (the stream Pane is small), held by the spot it was grabbed by: the title row under the pointer
    // (here as low as it can go and leave room for its shadow)
    assert_eq!((f.width, f.height), (s.width, s.height));
    assert_eq!((f.x, f.y), (cx - (x - s.x), cy.min(a.pane_area().bottom() - f.height - 1)));
    // a real Pane: the grip and its title on the top border, the frame in the accent, its own Widget inside
    assert!(row(&buf, f.y, f.x, f.right()).contains("⠿ stream"), "{}", row(&buf, f.y, f.x, f.right()));
    assert_eq!(buf[(f.x, f.y + 1)].fg, a.theme.accent.color());
    assert!((f.y + 1..f.bottom() - 1).any(|y| row(&buf, y, f.x + 1, f.right() - 1).trim() != ""), "its content is drawn");
    // and a shadow: the column to its right and the row below are not the plain background
    assert_ne!(buf[(f.right(), f.y + 2)].bg, a.theme.bg.color());
    assert_ne!(buf[(f.x + 3, f.bottom())].bg, a.theme.bg.color());
    // where it lands (the queue's place, the queue having gone to the stream's) is an empty slot, outlined, that says so
    assert!((q.y..q.bottom()).any(|y| row(&buf, y, q.x, q.right()).contains("drop here")), "said where the held Pane doesn't hide it");
    assert_eq!((buf[(q.x, q.y)].symbol(), buf[(q.x + 2, q.y)].symbol(), buf[(q.x, q.y + 2)].symbol()), ("╭", "╌", "╎"));
    assert_eq!(pane(&a, "queue"), s, "the others are where the drop would put them");
    // it has no hit boxes of its own: what's under it is still the layout's
    assert!(!a.hits.iter().any(|(r, h)| matches!(h, Hit::Title(t) if t == "stream") && *r == Rect::new(f.x + 1, f.y, f.width - 2, 1)));
    release(&mut a, cx, cy);
    let after = render(&mut a, W, H);
    assert!(!after.contains("drop here") && !after.contains('⠿'), "nothing floats after the drop");
    assert_eq!(pane(&a, "stream"), q);
}

#[test]
fn a_big_pane_floats_smaller_and_never_leaves_the_layout() {
    let mut a = app();
    let body = a.pane_area();
    let (x, y) = title_spot(&a, "home");
    press(&mut a, x, y);
    for (px, py) in [(150u16, 40u16), (0, 1), (W - 1, H - 1), (80, 20)] {
        drag(&mut a, px, py);
        render(&mut a, W, H);
        let f = a.float_rect(body).unwrap();
        assert!(f.width as f32 <= body.width as f32 * LIFT_MAX && f.height as f32 <= body.height as f32 * LIFT_MAX, "{f:?}");
        assert!(f.width >= crate::layout::MIN_W && f.height >= crate::layout::MIN_H);
        // with its shadow, inside the layout
        assert_eq!(with_shadow(f, body), Rect::new(f.x, f.y, f.width + 1, f.height + 1), "{f:?} at ({px}, {py})");
    }
    key(&mut a, KeyCode::Esc);
    assert!(a.tile.is_none());
    assert!(!render(&mut a, W, H).contains('⠿'));
}

#[test]
fn panes_ease_to_their_new_places_and_the_held_one_lands_in_its_slot() {
    use motion::SETTLE;
    let mut a = app();
    a.motion.on = true;
    render(&mut a, W, H);
    let body = a.pane_area();
    let (q, st) = (pane(&a, "queue"), pane(&a, "stream"));
    let (x, y) = title_spot(&a, "stream");
    press(&mut a, x, y);
    let (cx, cy) = centre(q);
    drag(&mut a, cx, cy);
    // the candidate changed: the queue sets off from where it was…
    render(&mut a, W, H);
    let q0 = drawn(&a, "queue");
    assert!(q0.x.abs_diff(q.x) <= 2 && q0.y.abs_diff(q.y) <= 2, "{q0:?}");
    assert!(a.motion.busy());
    assert_eq!((a.frame_rate(false), a.frame_rate(true)), (120, 120), "the motion rate while things move, even unfocused");
    // …is on its way half-way through (every edge between the two places)…
    a.motion.skip(Duration::from_millis(50));
    render(&mut a, W, H);
    let qm = drawn(&a, "queue");
    let between = |v: u16, p: u16, r: u16| (p.min(r)..=p.max(r)).contains(&v);
    assert!(qm != q && qm != st, "{qm:?}");
    assert!(between(qm.x, q.x, st.x) && between(qm.y, q.y, st.y) && between(qm.right(), q.right(), st.right()) && between(qm.bottom(), q.bottom(), st.bottom()), "{qm:?}");
    // …and exactly there at the end; the held Pane has shrunk from its place to its floating size and follows the pointer
    a.motion.skip(SETTLE);
    render(&mut a, W, H);
    assert_eq!(drawn(&a, "queue"), st);
    let target = float_target(a.tile.as_ref().unwrap(), body, 1.0);
    assert_eq!(a.float_rect(body), Some(target));
    // never a rect outside the layout or under a cell, at any moment
    for (i, r) in a.drawn.iter().enumerate() {
        assert!(r.width >= 1 && r.height >= 1 && body.contains(r.as_position()) && r.right() <= body.right() && r.bottom() <= body.bottom(), "{i}: {r:?}");
    }
    // let go: it flies from where it floats into its slot, drawn over the rest until it lands
    release(&mut a, cx, cy);
    assert_eq!(a.motion.landing.as_deref(), Some("stream"));
    render(&mut a, W, H);
    a.motion.skip(Duration::from_millis(50));
    let shot = render(&mut a, W, H);
    let mid = a.float_rect(body).unwrap();
    assert!(between(mid.x, target.x, q.x) && between(mid.y, target.y, q.y) && mid != q, "{mid:?}");
    assert!(shot.contains('⠿'));
    a.motion.skip(SETTLE);
    let shot = render(&mut a, W, H);
    assert!(a.motion.landing.is_none() && a.float_rect(body).is_none(), "landed");
    assert!(!shot.contains('⠿'));
    assert_eq!(drawn(&a, "stream"), q);
    // Esc while held: the others go back, the held one flies back to its own place
    let h = pane(&a, "home");
    let (x, y) = title_spot(&a, "home");
    press(&mut a, x, y);
    let lib = pane(&a, "library");
    drag(&mut a, centre(lib).0, centre(lib).1);
    render(&mut a, W, H);
    a.motion.skip(SETTLE);
    render(&mut a, W, H);
    assert_eq!(drawn(&a, "library"), h, "the library would take home's place");
    key(&mut a, KeyCode::Esc);
    assert_eq!(a.motion.landing.as_deref(), Some("home"));
    render(&mut a, W, H);
    a.motion.skip(SETTLE * 2);
    render(&mut a, W, H);
    assert_eq!((drawn(&a, "library"), drawn(&a, "home")), (lib, h));
    assert!(a.motion.landing.is_none());
    assert!(!a.motion.busy());
}

// ── the owner's edge bug, Shift+arrows ────────────────────────────────

/// "Sometimes near one of the edges it bugs out and just extends one of the panes like 15% to one side" (owner,
/// 2026-11-01). Classic: the queue held a little way up over Home's bottom border landed in Home's bottom band, and
/// the drop re-split the two 50/50: Home shrank from 66% to 50% and nothing else happened.
#[test]
fn near_its_own_neighbours_edge_a_pane_is_not_just_resized() {
    let (mut a, _dir) = with_dirs();
    a.set_design(2);
    render(&mut a, W, H);
    let before = a.layout.clone();
    let h = pane(&a, "home");
    let (x, y) = title_spot(&a, "queue");
    press(&mut a, x, y);
    drag(&mut a, h.x + 20, h.bottom() - 2);
    let t = a.tile.as_ref().unwrap();
    assert!(t.zone.is_none() && t.preview.is_none(), "{:?}", t.zone);
    let shot = render(&mut a, W, H);
    assert!(!shot.contains("drop here"));
    assert_eq!(pane(&a, "home"), h, "home keeps its size");
    release(&mut a, h.x + 20, h.bottom() - 2);
    assert_eq!(a.layout, before);
    assert!(a.naming.is_none() && !a.history.can_undo(), "no edit, no fork");
    // higher up, in Home's top band, it does move: above Home
    press(&mut a, x, y);
    drag(&mut a, h.x + 20, h.y + 2);
    assert!(a.tile.as_ref().unwrap().preview.is_some());
    key(&mut a, KeyCode::Esc);
    release(&mut a, h.x + 20, h.y + 2);
    // Event Horizon's library already spans the left edge: ⇧← widened it by a column and forked the Design
    a.set_design(1);
    a.layout_name = None;
    render(&mut a, W, H);
    let before = a.layout.clone();
    a.focus = "library".into();
    key(&mut a, KeyCode::Char('m'));
    shift(&mut a, KeyCode::Left);
    assert_eq!(a.layout, before);
    assert!(a.naming.is_none());
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("already spans the left edge")), "{:?}", a.toast);
    let shot = render(&mut a, W, H);
    assert!(shot.lines().last().unwrap().contains("already spans the left edge"), "said in the bar");
}

#[test]
fn shift_arrow_refusals_say_why_in_the_bar_and_change_nothing() {
    let mut a = app();
    // lyrics, under the gravity well: there is no room above the well for another Pane
    a.focus = "lyrics".into();
    key(&mut a, KeyCode::Char('m'));
    let before = a.layout.clone();
    shift(&mut a, KeyCode::Up);
    assert_eq!(a.layout, before);
    let said = a.toast.as_ref().map(|(t, _)| t.clone()).unwrap_or_default();
    assert!(said.starts_with("No room for") && said.contains("above gravity well") && said.ends_with("↑ swaps"), "{said}");
    let shot = render(&mut a, W, H);
    let bar = shot.lines().last().unwrap();
    assert!(bar.contains("MOVE") && bar.contains("No room for"), "{bar}");
    assert!(!a.history.can_undo());
    // the plain arrow does swap them
    key(&mut a, KeyCode::Up);
    assert_ne!(a.layout, before);
    // fft's neighbour on the left is loudness, its own sibling: the two would be too narrow side by side, so ⇧←
    // says so (the comfort rule) and ← swaps them
    let mut b = app();
    b.focus = "fft".into();
    let before = b.layout.clone();
    let (f, l) = (pane(&b, "fft"), pane(&b, "loudness"));
    assert!(f.width + l.width < 2 * crate::tiling::COMFY_W, "{f:?} {l:?}");
    key(&mut b, KeyCode::Char('m'));
    shift(&mut b, KeyCode::Left);
    assert_eq!(b.layout, before);
    let said = b.toast.as_ref().map(|(t, _)| t.clone()).unwrap_or_default();
    assert!(said.contains("too narrow to split") && said.ends_with("← swaps"), "{said}");
    key(&mut b, KeyCode::Left);
    render(&mut b, W, H);
    assert_eq!(pane(&b, "fft").right(), pane(&b, "loudness").x);
    // in a Pane with room for two, ⇧ splits: the queue below Home in Classic, as a row of its own
    let mut c = app();
    c.set_design(2);
    render(&mut c, W, H);
    c.focus = "lyrics".into();
    key(&mut c, KeyCode::Char('m'));
    let before = c.layout.clone();
    shift(&mut c, KeyCode::Left);
    assert_ne!(c.layout, before, "{:?}", c.toast);
}

/// Every ⇧arrow on every Pane of every Design: the Pane ends up further that way (or as the whole column / row at
/// that edge), the layout valid and one undo away from where it was; or nothing changes and the bar says why.
#[test]
fn shift_arrows_always_move_the_pane_that_way_or_say_why_not() {
    let (mut moved, mut refused) = (0, 0);
    for (d, design) in DESIGNS.iter().enumerate() {
        for (w, h) in [(W, H), (100, 30)] {
            let mut a = App::new(None);
            a.set_design(d);
            render(&mut a, w, h);
            let base = a.layout.clone();
            let body = a.pane_area();
            for s in base.slot_names() {
                for (code, side) in [(KeyCode::Left, Side::Left), (KeyCode::Right, Side::Right), (KeyCode::Up, Side::Top), (KeyCode::Down, Side::Bottom)] {
                    a.layout = base.clone();
                    a.toast = None;
                    a.history = Default::default();
                    render(&mut a, w, h);
                    let old = rects(&a).into_iter().find(|(x, _)| *x == s).unwrap().1;
                    a.focus = s.clone();
                    key(&mut a, KeyCode::Char('m'));
                    shift(&mut a, code);
                    let ctx = format!("{} {w}x{h} {s} ⇧{side:?}", design.id);
                    if a.layout == base {
                        refused += 1;
                        let said = a.toast.as_ref().map(|(t, _)| t.clone()).unwrap_or_default();
                        assert!(!said.is_empty() && !a.history.can_undo(), "{ctx}: nothing happened and nothing was said");
                        let shot = render(&mut a, w, h);
                        let start: String = said.chars().take(12).collect();
                        assert!(shot.lines().last().unwrap().contains(&start), "{ctx}: the bar says {said:?}");
                    } else {
                        moved += 1;
                        assert!(!a.layout.same_arrangement(&base), "{ctx}: only sizes changed");
                        assert!(a.layout.valid(body, a.cell_aspect()), "{ctx}");
                        let new = rects(&a).into_iter().find(|(x, _)| *x == s).unwrap().1;
                        let c = |r: Rect| (2 * r.x + r.width, 2 * r.y + r.height);
                        let (c0, c1) = (c(old), c(new));
                        let ok = match side {
                            Side::Left => c1.0 < c0.0 || (new.x == body.x && new.height == body.height),
                            Side::Right => c1.0 > c0.0 || (new.right() == body.right() && new.height == body.height),
                            Side::Top => c1.1 < c0.1 || (new.y == body.y && new.width == body.width),
                            Side::Bottom => c1.1 > c0.1 || (new.bottom() == body.bottom() && new.width == body.width),
                        };
                        assert!(ok, "{ctx}: {old:?} → {new:?}");
                        key(&mut a, KeyCode::Char('u'));
                        assert_eq!(a.layout, base, "{ctx}: u undoes it");
                    }
                    key(&mut a, KeyCode::Esc);
                }
            }
        }
    }
    eprintln!("{moved} moved, {refused} refused");
    assert!(moved > 100 && refused > 0);
}

// ── undo ──────────────────────────────────────────────────────────────

#[test]
fn ctrl_z_undoes_a_drop_and_ctrl_y_redoes_it_and_the_bar_offers_it() {
    let mut a = app();
    let before = a.layout.clone();
    let (x, y) = title_spot(&a, "stream");
    let q = pane(&a, "queue");
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
    a.close_pane("fft");
    trees.push(a.layout.clone());
    // split, with the Widget list
    a.focus = "loudness".into();
    a.run(Action::SplitRight);
    let i = a.widget_pick.as_ref().unwrap().items.iter().position(|w| *w == "scope").unwrap();
    a.widget_chosen(i);
    trees.push(a.layout.clone());
    // Move mode: a swap, a split beside, a size step
    a.focus = "queue".into();
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
    a.focus = "home".into();
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
        let (x, y) = title_spot(a, "stream");
        let q = pane(a, "queue");
        press(a, x, y);
        drag(a, centre(q).0, centre(q).1);
        release(a, centre(q).0, centre(q).1);
    };
    // 1. undone while it still asks for a name: the prompt goes, nothing was written; redo asks again
    swap(&mut a);
    let edit = a.layout.clone();
    assert!(a.naming.is_some());
    ctrl(&mut a, 'z');
    assert!(a.naming.is_none() && a.layout == built_in && a.layout_name.is_none() && files().is_empty());
    ctrl(&mut a, 'y');
    assert_eq!(a.layout, edit);
    assert_eq!(a.naming.as_ref().map(|n| n.purpose), Some(crate::app::NameFor::Fork));
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
    a.layout = Node::singularity().swapped("stream", "queue").unwrap();
    a.save_layout("Mine");
    let saved = a.layout.clone();
    a.close_pane("fft");
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
    // a size where Singularity doesn't fit but Singularity without fft does
    let (w, h) = (40..160u16).flat_map(|w| (14..48u16).map(move |h| (w, h))).find(|&(w, h)| {
        let body = Rect::new(0, 1, w, h - 5);
        !Node::singularity().valid(body, 2.0) && Node::singularity().closed("fft").unwrap().valid(body, 2.0) && !crate::mini::wanted(Rect::new(0, 0, w, h))
    }).expect("such a size");
    let mut a = app();
    a.close_pane("fft");
    let closed = a.layout.clone();
    render(&mut a, w, h);
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, closed);
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("doesn't fit this window")), "{:?}", a.toast);
    // back at a size it fits, it goes
    render(&mut a, W, H);
    ctrl(&mut a, 'z');
    assert_eq!(a.layout, Node::singularity());
}


// ── fast drags, the motion rate (owner, 2026-11-01: "it looks a bit like teleporting") ──

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The pointer at (x, y) at moment `t`.
fn sweep_at(a: &mut App, t: Instant, x: u16, y: u16) {
    a.input_at = Some(t);
    sweep(a, x, y);
    a.input_at = None;
}

fn shown(a: &App) -> Option<Zone> {
    a.tile.as_ref().and_then(|t| t.zone.clone())
}

#[test]
fn a_fast_sweep_across_panes_leaves_the_preview_and_a_pause_shows_it() {
    let mut a = app();
    let before = a.layout.clone();
    let t0 = Instant::now();
    let (x, y) = title_spot(&a, "stream");
    a.input_at = Some(t0);
    press(&mut a, x, y);
    let names = ["queue", "home", "library", "lyrics", "fft"];
    let spots: Vec<(u16, u16)> = names.iter().map(|s| centre(pane(&a, s))).collect();
    // across five Panes in 100 ms, a frame every 10 ms: each is the one under the pointer for a moment, none is shown
    for (i, &(px, py)) in spots.iter().enumerate() {
        let t = t0 + ms(20 * (i as u64 + 1));
        sweep_at(&mut a, t, px, py);
        assert_eq!(a.tile.as_ref().unwrap().cand, Some(Zone::Swap(names[i].into())));
        a.tile_settle(t + ms(10));
        assert_eq!((shown(&a), a.tile.as_ref().unwrap().preview.is_some()), (None, false), "{}", names[i]);
        let shot = render(&mut a, W, H);
        assert_eq!(pane(&a, names[i]), rects(&a).iter().find(|(s, _)| s == names[i]).unwrap().1, "nothing rearranges");
        assert!(shot.contains('⠿'), "the held Pane follows the pointer all the way");
    }
    // resting on fft: still moving fast a moment ago, so not yet; once the pointer has been still for the speed
    // window it shows (and at the latest after the dwell)
    let last = t0 + ms(100);
    a.tile_settle(last + QUICK_DWELL);
    assert_eq!(shown(&a), None);
    a.tile_settle(last + WINDOW + ms(1));
    assert_eq!(shown(&a), Some(Zone::Swap("fft".into())));
    render(&mut a, W, H);
    assert_eq!(pane(&a, "stream"), before.compute_shaped(a.pane_area(), a.cell_aspect()).slots.iter().find(|(s, _)| s == "fft").unwrap().1);
    // a flick across the queue keeps that preview…
    let t = last + ms(200);
    let q = centre(pane(&a, "queue"));
    sweep_at(&mut a, t, q.0, q.1);
    a.tile_settle(t + ms(10));
    assert_eq!(shown(&a), Some(Zone::Swap("fft".into())));
    // …but let go there at once, the drop is where it was let go: with the queue
    release(&mut a, q.0, q.1);
    assert_eq!(a.layout, before.swapped("stream", "queue").unwrap());
}

#[test]
fn a_dwell_shows_the_zone_and_a_creeping_pointer_shows_it_sooner() {
    let mut a = app();
    let t0 = Instant::now();
    let (x, y) = title_spot(&a, "stream");
    a.input_at = Some(t0);
    press(&mut a, x, y);
    let q = pane(&a, "queue");
    let (cx, cy) = centre(q);
    // jumps in and stays, the hand twitching a couple of cells every 15 ms (never still): shown after the dwell
    sweep_at(&mut a, t0 + ms(10), cx, cy);
    for (after, on) in [(30u64, false), (60, false), (110, false), (120, true)] {
        if !on {
            sweep_at(&mut a, t0 + ms(10 + after), cx + (after as u16 / 15) % 2 * 2, cy);
        }
        a.tile_settle(t0 + ms(10 + after));
        assert_eq!(shown(&a).is_some(), on, "{after} ms");
    }
    key(&mut a, KeyCode::Esc);
    release(&mut a, cx, cy);
    // creeping into the queue a cell at a time (10 cells a second): shown after the short dwell
    let t1 = t0 + ms(1000);
    a.input_at = Some(t1);
    press(&mut a, x, y);
    a.input_at = None;
    let start = (q.x.saturating_sub(4), cy);
    sweep_at(&mut a, t1 + ms(10), start.0, start.1);
    let mut t = t1 + ms(300);
    sweep_at(&mut a, t, start.0, start.1);
    let mut x = start.0;
    while a.tile.as_ref().unwrap().cand != Some(Zone::Swap("queue".into())) {
        t += ms(100);
        x += 1;
        sweep_at(&mut a, t, x, start.1);
        a.tile_settle(t + QUICK_DWELL - ms(1));
        assert!(x < q.right(), "never reached the queue's centre");
    }
    assert!(a.tile.as_ref().unwrap().speed(t) < SLOW);
    a.tile_settle(t + QUICK_DWELL - ms(1));
    assert_ne!(shown(&a), Some(Zone::Swap("queue".into())), "not yet");
    a.tile_settle(t + QUICK_DWELL);
    assert_eq!(shown(&a), Some(Zone::Swap("queue".into())));
}

#[test]
fn a_new_preview_mid_flight_turns_the_panes_without_a_jump() {
    let mut a = app();
    a.motion.on = true;
    render(&mut a, W, H);
    let (x, y) = title_spot(&a, "stream");
    press(&mut a, x, y);
    let (q, lib) = (pane(&a, "queue"), pane(&a, "library"));
    drag(&mut a, centre(q).0, centre(q).1);
    render(&mut a, W, H);
    a.motion.skip(ms(40));
    render(&mut a, W, H);
    let mid = drawn(&a, "queue");
    assert_ne!(mid, q, "on its way");
    // the pointer pauses over the library instead: the queue heads back from where it is, at the speed it had
    drag(&mut a, centre(lib).0, centre(lib).1);
    render(&mut a, W, H);
    let now = drawn(&a, "queue");
    assert!(now.x.abs_diff(mid.x) <= 1 && now.y.abs_diff(mid.y) <= 1 && now.width.abs_diff(mid.width) <= 2, "{mid:?} → {now:?}");
    let mut prev = now;
    for _ in 0..30 {
        a.motion.skip(ms(1000 / 240));
        render(&mut a, W, H);
        let r = drawn(&a, "queue");
        assert!(r.x.abs_diff(prev.x) <= 4 && r.y.abs_diff(prev.y) <= 4, "a step of a 240 fps frame: {prev:?} → {r:?}");
        prev = r;
    }
    a.motion.skip(motion::SETTLE);
    render(&mut a, W, H);
    assert_eq!(drawn(&a, "queue"), q, "back where it was");
    key(&mut a, KeyCode::Esc);
}

#[test]
fn the_frame_rate_is_the_motion_rate_while_anything_moves() {
    let mut a = app();
    a.config.visuals.unfocused_fps = 10;
    assert_eq!((a.frame_rate(true), a.frame_rate(false)), (30, 10), "idle: as before");
    let (x, y) = title_spot(&a, "stream");
    press(&mut a, x, y);
    assert_eq!(a.frame_rate(true), 30, "a press is no drag yet");
    drag(&mut a, x + 10, y + 5);
    assert_eq!((a.frame_rate(true), a.frame_rate(false)), (120, 120), "held: the motion rate, focused or not");
    for (set, got) in [(240, 240), (144, 144), (1000, 240), (5, 30)] {
        a.config.visuals.motion_fps = set;
        assert_eq!(a.frame_rate(true), got);
    }
    a.config.visuals.motion_fps = 240;
    // let go: the landing still moves (motion on), then back to the steady rate
    a.motion.on = true;
    release(&mut a, x + 10, y + 5);
    render(&mut a, W, H);
    assert!(a.motion.busy() && a.frame_rate(true) == 240);
    a.motion.skip(motion::SETTLE);
    render(&mut a, W, H);
    a.motion.skip(motion::SETTLE);
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
fn a_short_wide_pane_is_swapped_with_not_sandwiched() {
    // the instruments in a short row under Home: fft dropped anywhere on loudness swaps the two
    use crate::layout::{Axis::*, Size::*};
    let mut a = app();
    let s = |x| Node::slot(x);
    let row = Node::split(Horizontal, Ratio(0.5), s("fft"), s("loudness"));
    a.layout = Node::split(Horizontal, Cells(30), s("library"), Node::split(Vertical, Ratio(0.7), Node::split(Vertical, Ratio(0.6), s("home"), s("queue")), Node::split(Horizontal, Ratio(0.8), row, s("lyrics"))));
    render(&mut a, W, H);
    let before = a.layout.clone();
    let l = pane(&a, "loudness");
    assert!(l.height < 2 * crate::tiling::COMFY_H && l.width >= 2 * crate::tiling::COMFY_W, "{l:?}");
    for (px, py) in [(l.x + l.width / 2, l.y), (l.x + l.width / 2, l.bottom() - 1), (l.x + l.width / 2, l.y + l.height / 2)] {
        let (x, y) = title_spot(&a, "fft");
        press(&mut a, x, y);
        drag(&mut a, px, py);
        assert_eq!(shown(&a), Some(Zone::Swap("loudness".into())), "({px}, {py}) in {l:?}");
        key(&mut a, KeyCode::Esc);
        release(&mut a, px, py);
    }
    // and dropped there, the two trade places
    let (x, y) = title_spot(&a, "fft");
    press(&mut a, x, y);
    drag(&mut a, l.x + l.width / 2, l.y);
    release(&mut a, l.x + l.width / 2, l.y);
    assert_eq!(a.layout, before.swapped("fft", "loudness").unwrap());
}

/// The owner (2026-11-02): "I can't drag the lyrics pane to take up a bit of Home, it snaps right back". A drop that
/// only changes sizes is still refused, but says how to resize; the border lights up under the pointer, asks for a
/// resize pointer, and dragging it resizes.
#[test]
fn a_size_only_drop_says_to_drag_the_border_which_lights_up_and_resizes() {
    use crate::layout::{Axis, Size};
    let mut a = app();
    a.layout = Node::split(Axis::Horizontal, Size::Ratio(0.5), Node::slot("home"), Node::slot("lyrics"));
    render(&mut a, W, H);
    let (home, before) = (pane(&a, "home"), a.layout.clone());
    // lyrics by its title onto Home's right edge: the same two Panes side by side, only resized
    let (tx, ty) = title_spot(&a, "lyrics");
    press(&mut a, tx, ty);
    drag(&mut a, home.right() - 3, home.y + home.height / 2);
    release(&mut a, home.right() - 3, home.y + home.height / 2);
    assert_eq!(a.layout, before, "a size-only drop is no move");
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("drag the border")), "{:?}", a.toast);
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
    sweep(&mut a, gx - 20, gy);
    release(&mut a, gx - 20, gy);
    render(&mut a, W, H);
    assert_eq!(pane(&a, "home").width, home.width - 20);
}
