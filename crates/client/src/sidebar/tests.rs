//! The library's size rule (#56): one list up to medium sizes, columns of bigger covers in wide and big Panes, and the
//! keys, the wheel and clicks across those columns.

use super::*;
use crate::app::Link;
use crate::layout::{Axis, Node, Size, Widget};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use singularity_proto::{Library, LibraryItem};

#[test]
fn the_size_rule_keeps_small_and_medium_panes_one_list_and_gives_wide_big_ones_columns() {
    let one_line = Shape { cols: 1, pitch: 1 };
    let two_lines = Shape { cols: 1, pitch: 2 };
    // (the contact sheet's Panes, inside their frames)
    for (what, w, h, want) in [
        ("tiny", 18, 4, one_line),
        ("narrow-tall", 22, 28, one_line),
        ("wide-short", 98, 6, one_line),
        ("medium", 58, 18, one_line),
        // the library popup (#38) stays under 24 rows
        ("the popup", 54, 21, one_line),
        ("wide but not tall", 160, 23, one_line),
        ("narrow but tall", 31, 60, one_line),
        ("the two-line list", 40, 30, two_lines),
        ("almost room for two columns", 75, 40, two_lines),
        ("two columns", 76, 24, Shape { cols: 2, pitch: 3 }),
        ("big", 118, 38, Shape { cols: 3, pitch: 3 }),
        ("full screen", 178, 47, Shape { cols: 3, pitch: 4 }),
        ("never more than three", 260, 66, Shape { cols: 3, pitch: 4 }),
    ] {
        assert_eq!(Shape::of(w, h), want, "{what} ({w}×{h})");
    }
}

#[test]
fn columns_share_the_entries_evenly_when_they_all_fit_and_fill_up_when_they_dont() {
    let s = Shape::of(118, 38);
    // 12 entries of three rows fit a column
    assert_eq!((s.rows(38, 21), s.rows(38, 36), s.rows(38, 37), s.rows(38, 500), s.rows(38, 1)), (7, 12, 12, 12, 1));
    // one list: as many as fit
    assert_eq!((Shape::of(58, 18).rows(18, 3), Shape::of(40, 30).rows(30, 3)), (18, 15));
    // columns side by side with a gap, inside the Pane
    let r = Rect::new(10, 5, 118, 38);
    let cols: Vec<Rect> = (0..3).map(|c| s.column(r, c)).collect();
    assert!(cols.windows(2).all(|p| p[1].x > p[0].right()), "{cols:?}");
    assert!(cols.iter().all(|c| c.y == r.y && c.height == r.height && c.right() <= r.right() && c.width >= 36), "{cols:?}");
}

#[test]
fn the_window_follows_the_highlight_an_entry_at_a_time_in_one_list_and_a_column_at_a_time_in_columns() {
    // one list (as before): the window moves only when the highlight leaves it
    assert_eq!((first(0, 0, 10, 1, 60), first(9, 0, 10, 1, 60), first(10, 0, 10, 1, 60), first(5, 3, 10, 1, 60), first(2, 3, 10, 1, 60)), (0, 0, 1, 3, 2));
    // never past the end
    assert_eq!((first(59, 0, 10, 1, 60), first(3, 55, 10, 1, 60), first(0, 0, 10, 1, 4)), (50, 3, 0));
    // columns of 12, three of them: the highlight in the fourth column moves the window one column on
    assert_eq!((first(35, 0, 12, 3, 100), first(36, 0, 12, 3, 100), first(40, 12, 12, 3, 100), first(11, 12, 12, 3, 100)), (0, 12, 12, 0));
    // the window stops where the last column (96 to 99) shows
    assert_eq!((first(99, 0, 12, 3, 100), first(99, 0, 12, 3, 30)), (72, 0));
}

#[test]
fn the_author_tag_names_the_first_author_and_never_all_of_them() {
    for (kind, sub, want) in [
        ("playlist", "Mira Vale, Lumen Fields and more", "Mira Vale & more"),
        ("playlist", "Nova Drift, Mira Vale & more", "Nova Drift & more"),
        ("album", "Nova Drift, Mira Vale", "Nova Drift & more"),
        ("album", "Nova Drift", "Nova Drift"),
        ("playlist", "Playlist · Nova Drift", "Nova Drift"),
        ("playlist", "Playlist", "Playlist"),
        ("show", "Podcast · Pub", "Pub"),
        ("show", "Listen free until Oct 31 · Ada Quill, Rowan Tide", "Ada Quill & more"),
        ("liked", "250 songs", "250 songs"),
        ("artist", "Artist", "Artist"),
        ("folder", "", ""),
        ("playlist", "Your personal DJ", "Your personal DJ"),
    ] {
        assert_eq!(tag(kind, sub), want, "{kind} {sub:?}");
    }
}

fn app(items: usize) -> App {
    let (tx, rx) = tokio::sync::mpsc::channel(4096);
    std::mem::forget(rx);
    let mut app = App::new(Some(tx));
    app.link = Link::Connected { version: "test".into() };
    let item = |n: usize| LibraryItem {
        uri: format!("spotify:playlist:p{n}"),
        name: format!("Mix {n}"),
        subtitle: "Mira Vale, Lumen Fields and more".into(),
        kind: "playlist".into(),
        image_url: None,
        depth: 0,
        own: false,
    };
    app.hear(singularity_proto::Event::Library(Library { items: (0..items).map(item).collect() }));
    // the library takes most of a big screen, beside Home
    app.layout = Node::split(Axis::Horizontal, Size::Ratio(0.8), Node::pane(Widget::Library), Node::pane(Widget::Home)).into();
    app.focus_on(Widget::Library);
    app
}

fn render(app: &mut App) -> String {
    let (w, h) = (160, 48);
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| crate::ui::draw(f, app)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn entry(app: &App, row: usize) -> Option<Rect> {
    app.hits.iter().find_map(|(r, h)| (*h == Hit::Side(row)).then_some(*r))
}

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn a_big_library_pane_shows_columns_that_keys_the_wheel_and_clicks_cross() {
    let mut app = app(100);
    let s = render(&mut app);
    let side = app.sidebar;
    assert_eq!(side.cols, 3, "a big library Pane has three columns\n{s}");
    let rows = side.rows;
    assert!(rows >= 8 && side.visible == 3 * rows, "{rows} rows a column, {} shown", side.visible);
    // Home tops the first column; the next column starts level with it, to its right
    let (home, second) = (entry(&app, 0).unwrap(), entry(&app, rows).unwrap());
    assert_eq!(second.y, home.y);
    assert!(second.x >= home.right(), "{home:?} {second:?}");
    assert_eq!(entry(&app, 2 * rows).unwrap().y, home.y);
    // three rows an entry, the bigger cover's height
    assert_eq!(entry(&app, 1).unwrap().y, home.y + 3);
    assert_eq!(home.height, 3);
    // the short author tag, not the whole list of them
    assert!(s.contains("Mira Vale & more") && !s.contains("Lumen Fields"), "{s}");

    // ↓ goes down a column, then on to the top of the next one
    for _ in 0..rows {
        key(&mut app, KeyCode::Down);
    }
    render(&mut app);
    assert_eq!(app.sidebar.sel, rows);
    assert_eq!((app.sidebar.top, entry(&app, rows).unwrap().x), (0, second.x));
    // past the last column on screen, the window moves a column on, and the highlight shows
    for _ in 0..2 * rows {
        key(&mut app, KeyCode::Down);
    }
    render(&mut app);
    assert_eq!((app.sidebar.sel, app.sidebar.top), (3 * rows, rows));
    assert!(entry(&app, 3 * rows).is_some() && entry(&app, 0).is_none());
    // ↑ back into the first column brings it back
    for _ in 0..2 * rows + 1 {
        key(&mut app, KeyCode::Up);
    }
    render(&mut app);
    assert_eq!((app.sidebar.sel, app.sidebar.top), (rows - 1, 0));

    // the wheel moves the highlight an entry a notch, and the window follows it
    let at = entry(&app, 1).unwrap();
    for _ in 0..80 {
        app.pointer(MouseEvent { kind: MouseEventKind::ScrollDown, column: at.x + 1, row: at.y, modifiers: KeyModifiers::NONE }, at.x as f64 + 1.5, false);
    }
    render(&mut app);
    assert_eq!(app.sidebar.sel, rows - 1 + 80);
    assert!(entry(&app, app.sidebar.sel).is_some(), "the highlight is in view");
    assert!(entry(&app, 99).is_some(), "the end shows");

    // a click on an entry in the third column opens that one
    let row = app.sidebar.top + 2 * rows + 1;
    let r = entry(&app, row).unwrap();
    app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x + 8, row: r.y + 1, modifiers: KeyModifiers::NONE });
    app.mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: r.x + 8, row: r.y + 1, modifiers: KeyModifiers::NONE });
    assert_eq!(app.sidebar.sel, row);
    assert_eq!(app.open.view().map(|v| v.uri.clone()), Some(format!("spotify:playlist:p{}", row - FIXED)));
}

#[test]
fn a_short_library_spreads_over_the_columns() {
    let mut app = app(20);
    render(&mut app);
    // Home and 20 entries in three columns of seven
    assert_eq!((app.sidebar.cols, app.sidebar.rows), (3, 7));
    let (a, b, c) = (entry(&app, 0).unwrap(), entry(&app, 7).unwrap(), entry(&app, 14).unwrap());
    assert!(a.y == b.y && b.y == c.y && a.x < b.x && b.x < c.x);
    assert!(entry(&app, 20).is_some());
}

#[test]
fn a_shrunk_pane_keeps_the_highlight_in_view() {
    let mut app = app(100);
    render(&mut app);
    for _ in 0..70 {
        key(&mut app, KeyCode::Down);
    }
    render(&mut app);
    assert!(entry(&app, 70).is_some());
    // the same library in a narrow Pane: one list, and the window still holds the highlight
    app.layout = Node::split(Axis::Horizontal, Size::Ratio(0.2), Node::pane(Widget::Library), Node::pane(Widget::Home)).into();
    render(&mut app);
    assert_eq!(app.sidebar.cols, 1);
    assert!(entry(&app, 70).is_some(), "top {} visible {}", app.sidebar.top, app.sidebar.visible);
}

