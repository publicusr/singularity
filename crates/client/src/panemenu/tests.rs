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
    t.draw(|f| crate::ui::frame(f, app)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..H).map(|y| (0..W).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn buffer(app: &mut App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| crate::ui::frame(f, app)).unwrap();
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
    a.hear(singularity_proto::Event::Library(Library { items: vec![LibraryItem { uri: "spotify:playlist:p".into(), name: "Night Drive".into(), subtitle: String::new(), kind: "playlist".into(), image_url: None, depth: 0, own: false, authors: vec![] }] }));
    a.playback.queue = (0..3).map(|i| QueueEntry { item: Item { uri: format!("spotify:track:q{i}"), name: format!("Song {i}"), duration_ms: 150_000, ..Default::default() }, source: "queue".into(), uid: format!("u{i}") }).collect();
    render(&mut a);
    a
}

fn pid(a: &App, w: Widget) -> PaneId {
    a.layout.pane_of(w).unwrap_or_else(|| panic!("no Pane shows {w}"))
}
/// Pane `w`'s rectangle, where the Screen plan has it now.
fn pane(a: &App, w: Widget) -> Rect {
    rect_of(a, pid(a, w))
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
    assert_eq!(menu_items(&a), ["Replace widget…", "Split  ←  →  ↑  ↓", "Close pane", "Arrange"]);
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
    assert_eq!(menu_items(&a), ["Split  ←  →  ↑  ↓", "Arrange"], "nor Close");
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
    let d = a.dividers().first().expect("a divider").grip;
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
    assert!(a.arrange.tile.as_ref().is_some_and(|t| t.live), "a drag from the name lifts the Pane");
    a.tile_cancel();
    // (the drag turned Arrange mode on)
    a.arrange_done();
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
    assert!(a.layout.valid(a.pane_area()));
    // it's a layout edit: undo puts fft back
    a.close_if_open(Modal::Naming);
    a.run(Action::UndoLayout);
    assert_eq!(a.layout.widget_in(fft), Some(Widget::Fft));
}

#[test]
fn picking_one_on_screen_swaps_the_two_panes_widgets() {
    let mut a = app();
    let (fft, stream) = (pid(&a, Widget::Fft), pid(&a, Widget::Stream));
    let before = a.layout.compute(a.pane_area()).panes.iter().map(|(p, r)| (p.id, *r)).collect::<Vec<_>>();
    a.open_widget_picker(fft);
    render(&mut a);
    let i = a.widget_choices().iter().position(|c| c.widget == Widget::Stream).expect("stream is listed");
    assert_eq!(a.widget_choices()[i].from, Some(stream));
    // a click on its row
    let r = a.hit_rect(&Hit::PickWidget(i)).expect("its row is drawn");
    click(&mut a, r.x + 3, r.y);
    assert_eq!((a.layout.widget_in(fft), a.layout.widget_in(stream)), (Some(Widget::Stream), Some(Widget::Fft)));
    let after = a.layout.compute(a.pane_area()).panes.iter().map(|(p, r)| (p.id, *r)).collect::<Vec<_>>();
    assert_eq!(before, after, "only what the Panes show changed");
}

#[test]
fn the_library_queue_and_lyrics_can_be_replaced_away() {
    // D87: Home is the only Required Widget; an edit forks the built-in Design, and the fork may let them go
    for w in [Widget::Library, Widget::Queue, Widget::Lyrics] {
        let mut a = app();
        let id = pid(&a, w);
        let (x, y) = title_spot(&a, w);
        right_click(&mut a, x, y);
        assert_eq!(menu_items(&a), ["Replace widget…", "Split  ←  →  ↑  ↓", "Close pane", "Arrange"], "{w}");
        key(&mut a, KeyCode::Enter);
        let list = rows(&a);
        assert!(list.first().is_some_and(|(_, on)| !on), "{w}: Widgets off screen come first: {list:?}");
        let shot = render(&mut a);
        assert!(!shot.contains("stays on screen") && shot.contains("Enter puts it here"), "{w}: {shot}");
        let first = list[0].0;
        key(&mut a, KeyCode::Enter);
        assert!(!a.layout.has(w), "{w} left the screen");
        assert_eq!(a.layout.widget_in(id), Some(first), "{w}: its Pane shows what was picked");
    }
    // the built-in Designs themselves still have all four (D43; their trees don't change)
    for d in crate::design::DESIGNS {
        let tree = (d.layout)();
        assert!([Widget::Home, Widget::Library, Widget::Queue, Widget::Lyrics].iter().all(|c| tree.has(*c)), "{}", d.id);
    }
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

/// D87: a Replace or a swap changes only what the Panes show, never where they are, the gravity well's included (it
/// letterboxes its picture in whatever Pane it gets, #48).
#[test]
fn a_replace_or_swap_never_changes_any_panes_rect() {
    let rects = |a: &App| a.layout.compute(a.pane_area()).panes.iter().map(|(p, r)| (p.id, *r)).collect::<Vec<_>>();
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

// ── Split in four directions (#39) ────────────────────────────────────

use crate::tiling::Side;
const SIDES: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

/// Pane `id`'s rectangle, where the Screen plan has it now.
fn rect_of(a: &App, id: PaneId) -> Rect {
    a.plan().pane(id).map(|p| p.rect).unwrap_or_else(|| panic!("{id:?} isn't on screen"))
}
/// Whether `new` is on `side` of `old`, sharing that whole edge (the room `old` gave it).
fn beside(new: Rect, old: Rect, side: Side) -> bool {
    match side {
        Side::Left => new.right() == old.x && (new.y, new.height) == (old.y, old.height),
        Side::Right => old.right() == new.x && (new.y, new.height) == (old.y, old.height),
        Side::Top => new.bottom() == old.y && (new.x, new.width) == (old.x, old.width),
        Side::Bottom => old.bottom() == new.y && (new.x, new.width) == (old.x, old.width),
    }
}
/// The arrow of the open menu's Split row that splits toward `side`.
fn arrow(a: &App, side: Side) -> (u16, u16) {
    let r = a.hit_rect(&Hit::MenuSplit(side)).unwrap_or_else(|| panic!("no {side:?} arrow"));
    (r.x + 1, r.y)
}
/// Right-click the lyrics' body: their Pane's menu.
fn lyrics_menu(a: &mut App) {
    let r = pane(a, Widget::Lyrics);
    right_click(a, r.x + r.width / 2, r.y + r.height / 2);
    render(a);
}
/// The text of row `y` from `x0` to `x1`.
fn row_text(buf: &Buffer, x0: u16, x1: u16, y: u16) -> String {
    (x0..x1).map(|x| buf[(x, y)].symbol().to_string()).collect()
}

#[test]
fn the_pane_menu_has_one_split_row_whose_arrows_split_each_way_into_an_empty_pane_with_the_picker_on_it() {
    for side in SIDES {
        let mut a = app();
        let lyrics = pid(&a, Widget::Lyrics);
        let before = rect_of(&a, lyrics);
        lyrics_menu(&mut a);
        assert_eq!(menu_items(&a), ["Replace widget…", "Split  ←  →  ↑  ↓", "Close pane", "Arrange"], "one Split row");
        assert!(render(&mut a).contains("Split  ←  →  ↑  ↓"));
        let (x, y) = arrow(&a, side);
        click(&mut a, x, y);
        assert!(a.menu().is_none(), "{side:?}: the menu went");
        let new = a.widget_picker().map(|p| p.target).unwrap_or_else(|| panic!("{side:?}: the picker opens on the new Pane ({:?})", a.toast));
        assert_ne!(new, lyrics);
        assert_eq!(a.layout.get(new).map(|p| p.widget), Some(None), "{side:?}: it starts empty");
        assert_eq!(a.focus, Some(new));
        let shot = render(&mut a);
        assert!(shot.contains("Choose a widget") && shot.contains("Esc leaves it empty"), "{side:?}\n{shot}");
        let (n, l) = (rect_of(&a, new), rect_of(&a, lyrics));
        assert!(beside(n, l, side), "{side:?}: {n:?} beside {l:?}");
        assert_eq!(n.union(l), before, "{side:?}: the two share the room lyrics had");
        // picking fills it
        typed(&mut a, "scope");
        key(&mut a, KeyCode::Enter);
        assert_eq!(a.layout.widget_in(new), Some(Widget::Scope), "{side:?}");
        assert!(a.modal().is_none());
    }
}

#[test]
fn the_keyboard_lights_an_arrow_on_the_split_row_and_enter_splits_that_way() {
    // → is lit at first; ← steps back through ← ↓ ↑ (round at the ends)
    for (lefts, side) in [(1, Side::Left), (0, Side::Right), (3, Side::Top), (2, Side::Bottom)] {
        let mut a = app();
        let lyrics = pid(&a, Widget::Lyrics);
        lyrics_menu(&mut a);
        key(&mut a, KeyCode::Down);
        for _ in 0..lefts {
            key(&mut a, KeyCode::Left);
        }
        assert!(a.menu().is_some(), "← → stay in the menu");
        let buf = buffer(&mut a);
        for s in SIDES {
            let (x, y) = arrow(&a, s);
            assert_eq!(buf[(x, y)].bg == a.theme.accent.color(), s == side, "{side:?}: only its arrow is lit ({s:?})");
        }
        key(&mut a, KeyCode::Enter);
        let new = a.widget_picker().expect("the picker").target;
        render(&mut a);
        assert!(beside(rect_of(&a, new), rect_of(&a, lyrics), side), "{side:?}");
    }
    // → steps forward, and round
    let mut a = app();
    lyrics_menu(&mut a);
    key(&mut a, KeyCode::Down);
    for _ in 0..4 {
        key(&mut a, KeyCode::Right);
    }
    assert!(matches!(a.menu().unwrap().items[1].1, crate::app::MenuAct::Split(Side::Right)));
}

/// The side the open menu's Split row has lit.
fn lit(a: &App) -> Side {
    match a.menu().expect("a menu").items.iter().find_map(|(_, act)| match act {
        crate::app::MenuAct::Split(s) => Some(*s),
        _ => None,
    }) {
        Some(s) => s,
        None => panic!("no Split row"),
    }
}

#[test]
fn the_word_split_splits_along_the_panes_longer_side_and_that_arrow_is_lit_from_the_start() {
    // the lyrics: wider than tall (in real proportions): → is lit, even before the row is selected
    let mut a = app();
    let lyrics = pid(&a, Widget::Lyrics);
    lyrics_menu(&mut a);
    assert_eq!(lit(&a), Side::Right);
    let buf = buffer(&mut a);
    for s in SIDES {
        let (x, y) = arrow(&a, s);
        assert_eq!(buf[(x, y)].bg == a.theme.accent.color(), s == Side::Right, "→ lit before the row is selected ({s:?})");
    }
    let row = a.hit_rect(&Hit::MenuItem(1)).unwrap();
    click(&mut a, row.x + 2, row.y);
    let new = a.widget_picker().expect("the word splits").target;
    render(&mut a);
    assert!(beside(rect_of(&a, new), rect_of(&a, lyrics), Side::Right));
    key(&mut a, KeyCode::Esc);
    // the library: a tall column: ↓
    let mut a = app();
    let library = pid(&a, Widget::Library);
    let (x, y) = title_spot(&a, Widget::Library);
    right_click(&mut a, x, y);
    render(&mut a);
    assert_eq!(lit(&a), Side::Bottom);
    let row = a.hit_rect(&Hit::MenuItem(a.menu().unwrap().items.iter().position(|(_, x)| matches!(x, crate::app::MenuAct::Split(_))).unwrap())).unwrap();
    click(&mut a, row.x + 2, row.y);
    let new = a.widget_picker().expect("the word splits").target;
    render(&mut a);
    assert!(beside(rect_of(&a, new), rect_of(&a, library), Side::Bottom));
    // ← → move the lit arrow, and the word follows it
    let mut a = app();
    lyrics_menu(&mut a);
    key(&mut a, KeyCode::Down);
    key(&mut a, KeyCode::Right);
    assert_eq!(lit(&a), Side::Top);
    let row = a.hit_rect(&Hit::MenuItem(1)).unwrap();
    click(&mut a, row.x + 2, row.y);
    let new = a.widget_picker().unwrap().target;
    render(&mut a);
    assert!(beside(rect_of(&a, new), rect_of(&a, lyrics), Side::Top));
}

#[test]
fn a_default_split_the_long_way_with_no_room_goes_the_other_way() {
    use crate::layout::{Axis, Node, Size};
    // the scope 20 × 6: wider than tall, but two halves of 10 columns are under the minimum; two of 3 rows fit
    let mut a = App::new(None);
    let top = Node::split(Axis::Horizontal, Size::Cells(20), Node::pane(Widget::Scope), Node::pane(Widget::Home));
    a.layout = Node::split(Axis::Vertical, Size::Cells(6), top, Node::pane(Widget::Queue)).into();
    render(&mut a);
    let scope = pid(&a, Widget::Scope);
    assert_eq!((rect_of(&a, scope).width, rect_of(&a, scope).height), (20, 6));
    let (x, y) = (rect_of(&a, scope).x + 4, rect_of(&a, scope).y + 3);
    right_click(&mut a, x, y);
    render(&mut a);
    assert_eq!(lit(&a), Side::Bottom, "↓ lit: → has no room");
    let row = a.hit_rect(&Hit::MenuItem(1)).unwrap();
    click(&mut a, row.x + 2, row.y);
    let new = a.widget_picker().expect("split below").target;
    render(&mut a);
    assert!(beside(rect_of(&a, new), rect_of(&a, scope), Side::Bottom));
}

#[test]
fn a_split_and_its_pick_are_one_undo_step() {
    // without files: one Ctrl+Z takes the new Pane and its Widget away together
    let mut a = app();
    let before = a.layout.clone();
    a.focus_on(Widget::Lyrics);
    a.run(Action::SplitRight);
    typed(&mut a, "scope");
    key(&mut a, KeyCode::Enter);
    assert!(a.layout.has(Widget::Scope));
    a.run(Action::UndoLayout);
    assert_eq!(a.layout, before, "one undo: back to before the split");
    a.run(Action::RedoLayout);
    assert!(a.layout.has(Widget::Scope) && a.layout.panes().len() == before.panes().len() + 1, "one redo: both again");
    // left empty with Esc: one step too
    let mut a = app();
    a.focus_on(Widget::Lyrics);
    a.run(Action::SplitDown);
    key(&mut a, KeyCode::Esc);
    a.run(Action::UndoLayout);
    assert_eq!(a.layout, before);
    // a built-in Design, kept as its "(edited)": undo goes back to the built-in one, not to the split left unsaved
    let dir = tempfile::tempdir().unwrap();
    let mut a = App::new(None);
    a.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
    render(&mut a);
    let built_in = a.layout.clone();
    a.focus_on(Widget::Lyrics);
    a.run(Action::SplitRight);
    typed(&mut a, "scope");
    key(&mut a, KeyCode::Enter);
    assert!(a.modal().is_none(), "no name prompt (Q33)");
    assert_eq!(a.shown.file(), Some("singularity-edited"));
    a.run(Action::UndoLayout);
    assert_eq!(a.layout, built_in, "the split went with the pick");
    assert!(a.shown.file().is_none() && !a.layout.has(Widget::Scope));
}

#[test]
fn the_palette_has_split_left_right_up_and_down_and_finds_them_by_the_old_words() {
    for (words, action, side) in [("split left", Action::SplitLeft, Side::Left), ("split right", Action::SplitRight, Side::Right), ("split up", Action::SplitUp, Side::Top), ("split down", Action::SplitDown, Side::Bottom)] {
        let mut a = app();
        a.focus_on(Widget::Lyrics);
        let lyrics = pid(&a, Widget::Lyrics);
        a.run(Action::Palette);
        typed(&mut a, words);
        assert_eq!(a.palette_items().first(), Some(&action), "{words}");
        key(&mut a, KeyCode::Enter);
        let new = a.widget_picker().expect("the picker").target;
        render(&mut a);
        assert!(beside(rect_of(&a, new), rect_of(&a, lyrics), side), "{side:?}");
    }
    // what people typed before still finds a split
    let mut a = app();
    a.run(Action::Palette);
    typed(&mut a, "add pane below");
    assert_eq!(a.palette_items().first(), Some(&Action::SplitDown));
}

#[test]
fn esc_leaves_the_new_pane_empty_and_it_says_so() {
    let mut a = app();
    a.focus_on(Widget::Lyrics);
    let n = a.layout.panes().len();
    a.run(Action::SplitDown);
    let new = a.widget_picker().unwrap().target;
    key(&mut a, KeyCode::Esc);
    assert_eq!(a.modal(), Some(Modal::Arrange), "Esc leaves it empty, in Arrange mode");
    assert_eq!(a.layout.panes().len(), n + 1, "the Pane stays");
    assert_eq!(a.layout.get(new).map(|p| p.widget), Some(None));
    let buf = buffer(&mut a);
    let r = rect_of(&a, new);
    let y = r.y + 1 + (r.height - 3) / 2;
    let mid = row_text(&buf, r.x, r.right(), y);
    let at = mid.find("empty").unwrap_or_else(|| panic!("{mid:?}"));
    assert_eq!(buf[(r.x + mid[..at].chars().count() as u16, y)].fg, a.theme.dim.color(), "quietly");
    // a click outside the picker leaves it empty too
    a.run(Action::SplitRight);
    let other = a.widget_picker().unwrap().target;
    click(&mut a, 0, H - 1);
    assert!(a.modal() == Some(Modal::Arrange) && a.layout.get(other).is_some_and(|p| p.widget.is_none()));
    // the picker fills an empty Pane later: Replace widget… on it
    a.open_widget_picker(new);
    typed(&mut a, "rain");
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout.widget_in(new), Some(Widget::Rain));
}

#[test]
fn a_split_that_would_squash_a_pane_is_refused_with_the_reason() {
    use crate::layout::{Axis, Node, Size, MIN_H, MIN_W};
    // the smallest screen with Panes (the mini-player's limit): four columns of 12 fill its 50 columns
    let draw = |a: &mut App| {
        let mut t = Terminal::new(TestBackend::new(crate::mini::MIN_W, crate::mini::MIN_H)).unwrap();
        t.draw(|f| crate::ui::frame(f, a)).unwrap();
    };
    let columns = |first: Node| {
        let rest = Node::split(Axis::Horizontal, Size::Cells(MIN_W), Node::pane(Widget::Queue), Node::pane(Widget::Lyrics));
        let rest = Node::split(Axis::Horizontal, Size::Cells(MIN_W), Node::pane(Widget::Home), rest);
        Node::split(Axis::Horizontal, Size::Cells(MIN_W), first, rest)
    };
    for side in SIDES {
        let mut a = App::new(None);
        a.layout = columns(Node::pane(Widget::Scope)).into();
        draw(&mut a);
        // no room for a fifth column of 12; the scope's column a stack as tall as the screen holds, no room for one
        // more Pane of 3 rows
        let rows = (a.pane_area().height / MIN_H) as usize;
        assert!(a.pane_area().width < 5 * MIN_W && rows >= 2, "{:?}", a.pane_area());
        let stack = [Widget::Scope, Widget::Fft, Widget::Rain, Widget::Levels, Widget::Starfield, Widget::Glow, Widget::Marquee][..rows]
            .iter()
            .rev()
            .fold(None, |below: Option<Node>, w| Some(match below {
                None => Node::pane(*w),
                Some(b) => Node::split(Axis::Vertical, Size::Cells(MIN_H), Node::pane(*w), b),
            }))
            .unwrap();
        a.layout = columns(stack).into();
        draw(&mut a);
        assert!(a.layout.fits(a.pane_area()), "the stack fits");
        let before = a.layout.clone();
        a.focus_on(Widget::Scope);
        a.run(match side {
            Side::Left => Action::SplitLeft,
            Side::Right => Action::SplitRight,
            Side::Top => Action::SplitUp,
            Side::Bottom => Action::SplitDown,
        });
        assert_eq!(a.layout, before, "{side:?}: nothing changed");
        assert!(a.widget_picker().is_none(), "{side:?}: no picker");
        let said = a.toast.as_ref().map(|(t, _)| t.clone()).unwrap_or_default();
        assert!(said.starts_with("No room for a new Pane"), "{side:?}: {said:?}");
    }
}

#[test]
fn the_picker_offers_every_widget_and_says_what_a_pane_too_small_for_one_needs() {
    let mut a = app();
    // a short, narrow Pane: one under the stream, then one beside that
    a.focus_on(Widget::Stream);
    a.run(Action::SplitDown);
    key(&mut a, KeyCode::Esc);
    a.run(Action::SplitRight);
    let small = a.widget_picker().unwrap().target;
    render(&mut a);
    let r = rect_of(&a, small);
    let (kw, kh) = crate::needs::smallest(Widget::Karaoke);
    assert!(r.width < kw || r.height < kh, "the Pane is too small for Karaoke: {r:?}");
    // Karaoke is offered all the same, with the line it would show there
    let i = a.widget_choices().iter().position(|c| c.widget == Widget::Karaoke).expect("karaoke is offered");
    for _ in 0..i {
        key(&mut a, KeyCode::Down);
    }
    let buf = buffer(&mut a);
    let row = a.hit_rect(&Hit::PickWidget(i)).expect("its row is drawn");
    let line = row_text(&buf, row.x, row.right(), row.y);
    assert!(line.contains(&format!("needs {kw} × {kh}")), "{line:?}");
    // one that fits says nothing
    let j = a.widget_choices().iter().position(|c| c.widget == Widget::Scope).unwrap();
    if let Some(row) = a.hit_rect(&Hit::PickWidget(j)) {
        assert!(!row_text(&buf, row.x, row.right(), row.y).contains("needs"));
    }
    // picked anyway: the Pane shows its needs line
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.layout.widget_in(small), Some(Widget::Karaoke));
    assert!(render(&mut a).contains(&format!("needs {kw} × {kh}")));
}

#[test]
fn a_split_on_a_built_in_design_never_asks_for_a_name_and_says_where_it_went() {
    for pick in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::new(None);
        a.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
        render(&mut a);
        a.focus_on(Widget::Lyrics);
        a.run(Action::SplitRight);
        assert_eq!(a.modal(), Some(Modal::WidgetPicker), "the picker, not a naming prompt");
        assert!(a.toast.as_ref().is_some_and(|(t, _)| t == "Kept as Singularity (edited)"), "{:?}", a.toast);
        if pick {
            typed(&mut a, "scope");
            key(&mut a, KeyCode::Enter);
            assert!(a.layout.has(Widget::Scope));
        } else {
            key(&mut a, KeyCode::Esc);
        }
        render(&mut a);
        assert!(a.naming().is_none(), "never a name prompt (picked: {pick})");
        let saved = crate::layouts::load_dir(&dir.path().join("layouts")).0;
        assert!(saved.len() == 1 && saved[0].edited && saved[0].arrangement() == a.layout, "picked: {pick}: {saved:?}");
        // left empty: Arrange mode, whose Done asks only about what changed in it (nothing yet)
        if !pick {
            assert_eq!(a.modal(), Some(Modal::Arrange));
            key(&mut a, KeyCode::Esc);
            assert!(a.modal().is_none());
        }
    }
}

/// The Pane menu right-clicked in Pane `id`'s body: the row of "Close pane", if it offers it.
fn close_row(a: &mut App, id: PaneId) -> Option<usize> {
    let r = rect_of(a, id);
    right_click(a, r.x + r.width / 2, r.y + r.height / 2);
    let m = a.menu().expect("the Pane menu");
    assert_eq!(m.target, Target::Pane(id));
    m.items.iter().position(|(_, x)| matches!(x, crate::app::MenuAct::ClosePane))
}

/// Row `i` of the open menu, chosen with a click.
fn choose(a: &mut App, i: usize) {
    render(a);
    let row = a.hit_rect(&Hit::MenuItem(i)).unwrap();
    click(a, row.x + 2, row.y);
}

/// Q105: "Close pane" follows the ×'s rule (`screen_plan::closable`): an ordinary Pane has both; a Pane too narrow for
/// a × still closes from the menu; the only Pane on screen has neither.
#[test]
fn close_pane_is_offered_wherever_the_pane_could_close() {
    use crate::layout::{Axis, Node, Size};
    // an ordinary Pane
    let mut a = app();
    let lyrics = pid(&a, Widget::Lyrics);
    assert!(a.plan().pane(lyrics).unwrap().title.close.is_some(), "a ×");
    let i = close_row(&mut a, lyrics).expect("Close pane");
    choose(&mut a, i);
    assert!(!a.layout.has(Widget::Lyrics), "closed");
    // too narrow for a ×
    let mut a = App::new(None);
    a.layout = Node::split(Axis::Horizontal, Size::Last(13), Node::pane(Widget::Home), Node::pane(Widget::Library)).into();
    render(&mut a);
    let library = pid(&a, Widget::Library);
    let p = a.plan().pane(library).cloned().unwrap();
    assert_eq!((p.rect.width, p.title.close), (13, None), "no room for a ×");
    let i = close_row(&mut a, library).expect("Close pane, though there's no ×");
    choose(&mut a, i);
    assert!(!a.layout.has(Widget::Library), "closed");
    assert_eq!(a.layout.panes().len(), 1);
    // the only Pane on screen (Home waiting in the Tray)
    let mut a = App::new(None);
    a.layout = crate::arrangement::Arrangement::new(Node::pane(Widget::Library), vec![Widget::Home]);
    render(&mut a);
    let library = pid(&a, Widget::Library);
    assert_eq!(a.plan().pane(library).unwrap().title.close, None, "no ×");
    assert_eq!(close_row(&mut a, library), None, "no Close pane");
}
