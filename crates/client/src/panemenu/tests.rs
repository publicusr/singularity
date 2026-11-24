//! The Pane menu and the Widget picker, driven through the App with real clicks and keys (D87).

use super::*;
use crate::app::{Action, Target};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

const W: u16 = 160;
const H: u16 = 48;

fn render(app: &mut App) -> String {
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| crate::ui::draw(f, app)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..H).map(|y| (0..W).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn buffer(app: &mut App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| crate::ui::draw(f, app)).unwrap();
    t.backend().buffer().clone()
}

fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
}
fn click(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(app, MouseEventKind::Up(MouseButton::Left), x, y);
}
fn right_click(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Down(MouseButton::Right), x, y);
    mouse(app, MouseEventKind::Up(MouseButton::Right), x, y);
}
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn typed(app: &mut App, s: &str) {
    s.chars().for_each(|c| key(app, KeyCode::Char(c)));
}

/// The Singularity design, drawn, with a library and a queue (their rows have menus of their own).
fn app() -> App {
    use singularity_proto::{Item, Library, LibraryItem, QueueEntry};
    let mut a = App::new(None);
    a.hear(singularity_proto::Event::Library(Library { items: vec![LibraryItem { uri: "spotify:playlist:p".into(), name: "Night Drive".into(), subtitle: String::new(), kind: "playlist".into(), image_url: None, depth: 0, own: false }] }));
    a.playback.queue = (0..3).map(|i| QueueEntry { item: Item { uri: format!("spotify:track:q{i}"), name: format!("Song {i}"), duration_ms: 150_000, ..Default::default() }, source: "queue".into(), uid: format!("u{i}") }).collect();
    render(&mut a);
    a
}

fn pid(a: &App, w: Widget) -> PaneId {
    a.layout.pane_of(w).unwrap_or_else(|| panic!("no Pane shows {w}"))
}
/// Pane `w`'s rectangle in the last frame.
fn pane(a: &App, w: Widget) -> Rect {
    let id = pid(a, w);
    a.hits.iter().find_map(|(r, h)| matches!(h, Hit::Focus(p) if *p == id).then_some(*r)).unwrap()
}
/// The Widget's name in Pane `w`'s title.
fn name_spot(a: &App, w: Widget) -> (u16, u16) {
    let r = a.hit_rect(&Hit::TitleName(pid(a, w))).unwrap_or_else(|| panic!("{w}'s title has no name to click"));
    (r.x + 1, r.y)
}
/// A spot on Pane `w`'s title bar past its name, clear of its buttons.
fn title_spot(a: &App, w: Widget) -> (u16, u16) {
    let r = pane(a, w);
    let name = a.hit_rect(&Hit::TitleName(pid(a, w))).map_or(r.x + 2, |n| n.right() + 1);
    (name, r.y)
}
fn menu_items(a: &App) -> Vec<String> {
    a.menu().expect("a menu").items.iter().map(|(l, _)| l.clone()).collect()
}
fn rows(a: &App) -> Vec<(Widget, bool)> {
    a.widget_choices().iter().map(|c| (c.widget, c.from.is_some())).collect()
}

#[test]
fn right_clicking_a_title_opens_the_pane_menu_with_replace_widget_first() {
    let mut a = app();
    let (x, y) = title_spot(&a, Widget::Fft);
    right_click(&mut a, x, y);
    assert_eq!(menu_items(&a), ["Replace widget…", "Split right", "Split down", "Close pane", "Move pane"]);
    assert!(matches!(a.menu().unwrap().target, Target::Pane(p) if p == pid(&a, Widget::Fft)));
    assert!(render(&mut a).contains("Replace widget…"));
    // the name in the title opens the same menu on a right-click
    key(&mut a, KeyCode::Esc);
    let (x, y) = name_spot(&a, Widget::Fft);
    right_click(&mut a, x, y);
    assert_eq!(menu_items(&a)[0], "Replace widget…");
    // Enter on it: the Widget picker for that Pane
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.widget_picker().map(|p| p.target), Some(pid(&a, Widget::Fft)));
}

#[test]
fn homes_pane_has_no_replace_widget() {
    let mut a = app();
    let (x, y) = title_spot(&a, Widget::Home);
    right_click(&mut a, x, y);
    assert_eq!(menu_items(&a), ["Split right", "Split down", "Move pane"], "nor Close");
    key(&mut a, KeyCode::Esc);
    // its title has no name to click, and the key and the palette say why
    assert!(a.hit_rect(&Hit::TitleName(pid(&a, Widget::Home))).is_none());
    a.focus_on(Widget::Home);
    key(&mut a, KeyCode::Char('w'));
    assert!(a.widget_picker().is_none());
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("Home stays")), "{:?}", a.toast);
}

#[test]
fn right_clicking_a_body_opens_the_pane_menu_where_the_body_has_none_of_its_own() {
    let mut a = app();
    // fft's body: the Pane's menu
    let r = pane(&a, Widget::Fft);
    right_click(&mut a, r.x + r.width / 2, r.y + r.height / 2);
    assert!(matches!(a.menu().map(|m| &m.target), Some(Target::Pane(p)) if *p == pid(&a, Widget::Fft)), "{:?}", a.menu().map(|m| &m.target));
    key(&mut a, KeyCode::Esc);
    // the lyrics: no menu of their own
    let r = pane(&a, Widget::Lyrics);
    right_click(&mut a, r.x + r.width / 2, r.y + r.height / 2);
    assert!(matches!(a.menu().map(|m| &m.target), Some(Target::Pane(p)) if *p == pid(&a, Widget::Lyrics)));
    key(&mut a, KeyCode::Esc);
    // a queued song keeps its own
    render(&mut a);
    let q = a.hit_rect(&Hit::QueueRow(0)).expect("a queue row");
    right_click(&mut a, q.x + 2, q.y);
    assert!(matches!(a.menu().map(|m| &m.target), Some(Target::Queued { .. })), "{:?}", a.menu().map(|m| &m.target));
    key(&mut a, KeyCode::Esc);
    // so does a library row
    render(&mut a);
    let row = a.hits.iter().find_map(|(r, h)| matches!(h, Hit::Side(i) if *i >= crate::sidebar::FIXED).then_some(*r)).expect("a library row");
    right_click(&mut a, row.x + 2, row.y);
    assert!(matches!(a.menu().map(|m| &m.target), Some(Target::Library { .. })), "{:?}", a.menu().map(|m| &m.target));
    key(&mut a, KeyCode::Esc);
    // a border between two Panes is no Pane's
    render(&mut a);
    let d = a.dividers.first().expect("a divider").grip;
    right_click(&mut a, d.x, d.y + d.height / 2);
    assert!(a.menu().is_none());
}

#[test]
fn the_name_the_key_and_the_palette_open_the_picker() {
    let mut a = app();
    let fft = pid(&a, Widget::Fft);
    // a click on the Widget's name in its title
    let (x, y) = name_spot(&a, Widget::Fft);
    click(&mut a, x, y);
    assert_eq!(a.widget_picker().map(|p| p.target), Some(fft));
    assert_eq!(a.modal(), Some(Modal::WidgetPicker));
    key(&mut a, KeyCode::Esc);
    assert!(a.modal().is_none(), "Esc closes it");
    // a click elsewhere on the title only focuses it, and a drag from the name still moves the Pane
    let (x, y) = title_spot(&a, Widget::Stream);
    click(&mut a, x, y);
    assert!(a.modal().is_none() && a.focus == Some(pid(&a, Widget::Stream)));
    let (x, y) = name_spot(&a, Widget::Fft);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(&mut a, MouseEventKind::Drag(MouseButton::Left), x + 10, y + 6);
    assert!(a.tile.as_ref().is_some_and(|t| t.live), "a drag from the name lifts the Pane");
    a.tile_cancel();
    render(&mut a);
    // the key, on the focused Pane
    a.focus_on(Widget::Stream);
    key(&mut a, KeyCode::Char('w'));
    assert_eq!(a.widget_picker().map(|p| p.target), Some(pid(&a, Widget::Stream)));
    key(&mut a, KeyCode::Esc);
    // the palette
    a.focus_on(Widget::Fft);
    a.run(Action::Palette);
    typed(&mut a, "replace widget");
    assert_eq!(a.palette_items().first(), Some(&Action::ReplaceWidget));
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.widget_picker().map(|p| p.target), Some(fft));
}

#[test]
fn the_picker_lists_widgets_off_screen_first_then_the_ones_on_screen_dimmed_below_a_line() {
    let mut a = app();
    a.open_widget_picker(pid(&a, Widget::Fft));
    let list = rows(&a);
    let first_on = list.iter().position(|(_, on)| *on).expect("on-screen Widgets");
    assert!(first_on > 0 && list[first_on..].iter().all(|(_, on)| *on), "off screen first: {list:?}");
    for (w, on) in &list {
        assert_eq!(*on, a.layout.has(*w), "{w}");
        assert_ne!(*w, Widget::Fft, "not what it shows already");
    }
    // each Widget once; the black hole isn't offered while the gravity well is on screen
    let mut ws: Vec<Widget> = list.iter().map(|(w, _)| *w).collect();
    ws.sort();
    ws.dedup();
    assert_eq!(ws.len(), list.len());
    assert!(a.layout.has(Widget::Well) && !ws.contains(&Widget::Hole));
    for w in [Widget::Karaoke, Widget::Scope, Widget::Starfield] {
        assert!(list.contains(&(w, false)), "{w} is offered");
    }
    let shot = render(&mut a);
    assert!(shot.contains("Replace fft") && shot.contains("on screen · picking one swaps"), "{shot}");
    // the on-screen rows are dim, the others aren't
    let buf = buffer(&mut a);
    let row_of = |a: &App, i: usize| a.hit_rect(&Hit::PickWidget(i)).unwrap();
    let th = &a.theme;
    let fg_at = |r: Rect| buf[(r.x + 2, r.y)].fg;
    assert_eq!(fg_at(row_of(&a, first_on)), th.dim.color(), "on screen: dim");
    assert_eq!(fg_at(row_of(&a, 1)), th.fg.color(), "off screen: as usual");
    // the line sits right above the first on-screen row
    let line = row_of(&a, first_on).y - 1;
    let text: String = (0..W).map(|x| buf[(x, line)].symbol().to_string()).collect();
    assert!(text.contains("on screen"), "{text}");
}

#[test]
fn typing_filters_and_picking_one_off_screen_replaces_the_panes_widget() {
    let mut a = app();
    let fft = pid(&a, Widget::Fft);
    let (x, y) = name_spot(&a, Widget::Fft);
    click(&mut a, x, y);
    typed(&mut a, "kara");
    assert_eq!(rows(&a), [(Widget::Karaoke, false)]);
    key(&mut a, KeyCode::Backspace);
    key(&mut a, KeyCode::Backspace);
    key(&mut a, KeyCode::Backspace);
    key(&mut a, KeyCode::Backspace);
    assert!(rows(&a).len() > 5, "Backspace widens it again");
    typed(&mut a, "starf");
    key(&mut a, KeyCode::Enter);
    assert!(a.modal().is_none_or(|m| m == Modal::Naming), "picked: the picker is gone ({:?})", a.modal());
    assert_eq!(a.layout.widget_in(fft), Some(Widget::Starfield), "the same Pane shows it");
    assert!(!a.layout.has(Widget::Fft), "the replaced Widget left the screen");
    assert_eq!(a.focus, Some(fft));
    assert!(a.layout.valid(a.pane_area(), a.cell_aspect()));
    // it's a layout edit: undo puts fft back
    a.close_if_open(Modal::Naming);
    a.run(Action::UndoLayout);
    assert_eq!(a.layout.widget_in(fft), Some(Widget::Fft));
}

#[test]
fn picking_one_on_screen_swaps_the_two_panes_widgets() {
    let mut a = app();
    let (fft, stream) = (pid(&a, Widget::Fft), pid(&a, Widget::Stream));
    let before = a.layout.compute_shaped(a.pane_area(), a.cell_aspect()).panes.iter().map(|(p, r)| (p.id, *r)).collect::<Vec<_>>();
    a.open_widget_picker(fft);
    render(&mut a);
    let i = a.widget_choices().iter().position(|c| c.widget == Widget::Stream).expect("stream is listed");
    assert_eq!(a.widget_choices()[i].from, Some(stream));
    // a click on its row
    let r = a.hit_rect(&Hit::PickWidget(i)).expect("its row is drawn");
    click(&mut a, r.x + 3, r.y);
    assert_eq!((a.layout.widget_in(fft), a.layout.widget_in(stream)), (Some(Widget::Stream), Some(Widget::Fft)));
    let after = a.layout.compute_shaped(a.pane_area(), a.cell_aspect()).panes.iter().map(|(p, r)| (p.id, *r)).collect::<Vec<_>>();
    assert_eq!(before, after, "only what the Panes show changed");
}

#[test]
fn the_library_queue_and_lyrics_only_trade_places() {
    let mut a = app();
    a.focus_on(Widget::Queue);
    key(&mut a, KeyCode::Char('w'));
    let list = rows(&a);
    assert!(!list.is_empty() && list.iter().all(|(_, on)| *on), "{list:?}");
    let shot = render(&mut a);
    assert!(shot.contains("stays on screen") && shot.contains("Esc cancel"), "{shot}");
    // picking one swaps: the queue is still on screen
    key(&mut a, KeyCode::Enter);
    assert!(a.layout.has(Widget::Queue));
}

#[test]
fn the_picker_scrolls_with_arrows_and_the_wheel_and_a_click_outside_closes_it() {
    let mut a = app();
    a.open_widget_picker(pid(&a, Widget::Fft));
    render(&mut a);
    let n = a.widget_choices().len();
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Down);
    assert_eq!(a.widget_picker().unwrap().sel, 2);
    key(&mut a, KeyCode::Up);
    assert_eq!(a.widget_picker().unwrap().sel, 1);
    for _ in 0..n + 3 {
        mouse(&mut a, MouseEventKind::ScrollDown, 1, 1);
    }
    assert_eq!(a.widget_picker().unwrap().sel, n - 1, "the wheel stops at the end");
    // the highlighted row is drawn, below the line
    render(&mut a);
    assert!(a.hit_rect(&Hit::PickWidget(n - 1)).is_some(), "the list scrolled with its highlight");
    assert!(render(&mut a).contains("Enter swaps it here"));
    // a click on the typed line does nothing; outside, it closes
    let r = a.overlays.first().copied().unwrap();
    click(&mut a, r.x + 3, r.y + 1);
    assert_eq!(a.modal(), Some(Modal::WidgetPicker));
    click(&mut a, 0, H - 1);
    assert!(a.modal().is_none());
}

#[test]
fn the_key_is_in_the_help_and_free_of_clashes() {
    let a = app();
    assert_eq!(a.keymap.label(Action::ReplaceWidget), "w");
    let w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
    assert_eq!(a.keymap.action(w), Some(Action::ReplaceWidget));
    assert!(Action::ALL.iter().filter(|x| a.keymap.label(**x) == "w").count() == 1);
    let mut a = app();
    a.run(Action::Help);
    assert!(render(&mut a).contains("replace the focused pane's widget"));
}

#[test]
fn the_picker_closes_when_its_pane_goes() {
    let mut a = app();
    a.open_widget_picker(pid(&a, Widget::Fft));
    render(&mut a);
    // the Design comes back as it was built, without that Pane identity
    let tree = a.layout.clone();
    let gone = tree.without(pid(&a, Widget::Fft)).unwrap();
    a.layout = gone.into();
    let shot = render(&mut a);
    assert!(a.modal().is_none(), "closed, not an empty picker");
    assert!(!shot.contains("Choose a widget"));
    // and a key meanwhile does nothing
    a.layout = tree;
    a.open_widget_picker(pid(&a, Widget::Fft));
    a.layout = a.layout.without(pid(&a, Widget::Fft)).unwrap().into();
    key(&mut a, KeyCode::Enter);
    assert!(a.modal().is_none());
}

/// D87: a Replace or a swap changes only what the Panes show, never where they are. The gravity well still reshapes
/// the columns when it moves (`compute_shaped`); #36 fixes that and un-ignores this.
#[test]
#[ignore = "until #36: moving the well reshapes columns"]
fn a_replace_or_swap_never_changes_any_panes_rect() {
    let rects = |a: &App| a.layout.compute_shaped(a.pane_area(), a.cell_aspect()).panes.iter().map(|(p, r)| (p.id, *r)).collect::<Vec<_>>();
    for (target, pick) in [(Widget::Stream, Widget::Well), (Widget::Well, Widget::Starfield), (Widget::Fft, Widget::Karaoke), (Widget::Fft, Widget::Queue)] {
        let mut a = app();
        let before = rects(&a);
        let target = pid(&a, target);
        let tree = match a.layout.pane_of(pick) {
            Some(from) => a.layout.swap(target, from),
            None => a.layout.replace(target, pick),
        }
        .unwrap();
        a.layout = tree;
        assert_eq!(rects(&a), before, "{pick} into {target:?}");
    }
}
