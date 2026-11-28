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
    let one_line = Shape { cols: 1, cover: 1 };
    let two_lines = Shape { cols: 1, cover: 2 };
    let cols = |cols, cover| Shape { cols, cover };
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
        // two columns of 38 and the gap between them
        ("almost room for two columns", 77, 40, two_lines),
        ("two columns", 78, 24, cols(2, 3)),
        ("big", 118, 38, cols(3, 4)),
        ("full screen", 178, 47, cols(3, 5)),
        // the cover grows with the room: the height caps it (seven entries a column) and so does the width (the name
        // beside it), never past six rows
        ("wide and short", 178, 24, cols(3, 3)),
        ("two narrow columns, tall", 78, 60, cols(2, 5)),
        ("three wide columns, tall", 240, 60, cols(3, 6)),
        ("never more than three columns", 260, 66, cols(3, 6)),
    ] {
        assert_eq!(Shape::of(w, h), want, "{what} ({w}×{h})");
    }
    // a blank row between entries in columns (covers never touch); one list keeps its look
    assert_eq!((one_line.pitch(), two_lines.pitch(), cols(3, 4).pitch()), (1, 2, 5));
}

#[test]
fn columns_share_the_entries_evenly_when_they_all_fit_and_fill_up_when_they_dont() {
    let s = Shape::of(118, 38);
    // seven entries of four rows and a gap fit a column (the last needs no gap)
    assert_eq!((s.rows(38, 21), s.rows(38, 22), s.rows(38, 10), s.rows(38, 500), s.rows(38, 1)), (7, 7, 4, 7, 1));
    // one list: as many as fit
    assert_eq!((Shape::of(58, 18).rows(18, 3), Shape::of(40, 30).rows(30, 3)), (18, 15));
    // columns side by side with a gap, inside the Pane
    let r = Rect::new(10, 5, 118, 38);
    let cols: Vec<Rect> = (0..3).map(|c| s.column(r, c)).collect();
    assert!(cols.windows(2).all(|p| p[1].x > p[0].right()), "{cols:?}");
    assert!(cols.iter().all(|c| c.y == r.y && c.height == r.height && c.right() <= r.right() && c.width >= COL_W), "{cols:?}");
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

fn lib(kind: &str, subtitle: &str, authors: &[&str]) -> LibraryItem {
    LibraryItem {
        uri: "spotify:album:a".into(),
        name: "A".into(),
        subtitle: subtitle.into(),
        kind: kind.into(),
        image_url: None,
        depth: 0,
        own: false,
        authors: authors.iter().map(|a| a.to_string()).collect(),
    }
}

#[test]
fn the_author_tag_names_the_first_author_whole_and_never_all_of_them() {
    for (kind, sub, authors, want) in [
        // the Daemon's names, whole: a comma inside one isn't a list
        ("album", "Tyler, The Creator", &["Tyler, The Creator"][..], "Tyler, The Creator"),
        ("album", "Tyler, The Creator, Kali Uchis", &["Tyler, The Creator", "Kali Uchis"], "Tyler, The Creator & more"),
        ("album", "Nova Drift", &["Nova Drift"], "Nova Drift"),
        ("playlist", "Playlist · Nova Drift", &["Nova Drift"], "Nova Drift"),
        ("show", "Podcast · Pub", &["Pub"], "Pub"),
        ("show", "Audiobook · Ada Quill", &["Ada Quill"], "Ada Quill"),
        // a book keeps Spotify's access line first (D58)
        ("show", "Listen free until Oct 31 · Ada Quill, Rowan Tide", &["Ada Quill", "Rowan Tide"], "Listen free until Oct 31 · Ada Quill & more"),
        // without names (a Home Card's subtitle, an older Daemon): only what Spotify already ends with "and more", or a
        // list of three or more, is shortened; anything else stays whole
        ("playlist", "Mira Vale, Lumen Fields and more", &[], "Mira Vale & more"),
        ("playlist", "Nova Drift, Mira Vale & more", &[], "Nova Drift & more"),
        ("album", "Nova Drift, Mira Vale, Lumen Fields", &[], "Nova Drift & more"),
        ("album", "Earth, Wind & Fire", &[], "Earth, Wind & Fire"),
        ("album", "Tyler, The Creator", &[], "Tyler, The Creator"),
        ("playlist", "Playlist", &[], "Playlist"),
        ("playlist", "Your personal DJ", &[], "Your personal DJ"),
        ("liked", "250 songs", &[], "250 songs"),
        ("artist", "Artist", &[], "Artist"),
        ("folder", "", &[], ""),
    ] {
        assert_eq!(tag(&lib(kind, sub, authors)), want, "{kind} {sub:?} {authors:?}");
    }
}

#[test]
fn a_tag_too_long_for_its_column_ends_in_an_ellipsis() {
    assert_eq!(clip("Listen free until Oct 31 · Ada Quill", 12), "Listen free…");
    assert_eq!((clip("Nova Drift", 10), clip("Nova Drift", 30), clip("Nova", 0)), ("Nova Drift".into(), "Nova Drift".into(), String::new()));
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
        authors: vec![],
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
    t.draw(|f| crate::ui::frame(f, app)).unwrap();
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
    assert!(rows >= 6 && side.visible == 3 * rows, "{rows} rows a column, {} shown", side.visible);
    // Home tops the first column; the next column starts level with it, to its right
    let (home, second) = (entry(&app, 0).unwrap(), entry(&app, rows).unwrap());
    assert_eq!(second.y, home.y);
    assert!(second.x >= home.right(), "{home:?} {second:?}");
    assert_eq!(entry(&app, 2 * rows).unwrap().y, home.y);
    // an entry as tall as its bigger cover, and a blank row before the next (covers never touch)
    let cover = home.height;
    assert!(cover >= 4, "{cover}");
    assert_eq!(entry(&app, 1).unwrap().y, home.y + cover + 1);
    // a glyph standing in for a cover is level with the name, not the tag under it; Home's is its ⌂ as big as the
    // cover, on every row of it (in braille on cells), and no small ⌂ is left on its name row
    let line = |y: u16| s.lines().nth(y as usize).unwrap().to_string();
    assert!(line(home.y).contains("Home") && !line(home.y).contains('⌂'), "{}", line(home.y));
    let slot = |y: u16| line(y).chars().skip(home.x as usize + 1).take(2 * cover as usize).collect::<String>();
    assert!((home.y..home.y + cover).all(|y| slot(y).chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))), "{}", slot(home.y));
    let mix = entry(&app, 1).unwrap().y;
    assert!(line(mix).contains('≡') && line(mix).contains("Mix 0"), "{}", line(mix));
    assert!((mix + 1..mix + cover).all(|y| !line(y).contains('≡') && !line(y).contains('⌂')), "the glyph is on the name row only");
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

    // the wheel moves the view a column a notch and leaves the highlight where it is (D86)
    let at = entry(&app, 1).unwrap();
    app.pointer(MouseEvent { kind: MouseEventKind::ScrollDown, column: at.x + 1, row: at.y, modifiers: KeyModifiers::NONE }, at.x as f64 + 1.5, false);
    render(&mut app);
    assert_eq!((app.sidebar.sel, app.sidebar.top), (rows - 1, rows));
    assert!(entry(&app, rows - 1).is_none() && entry(&app, rows).is_some());
    // on to the end, which shows
    for _ in 0..50 {
        app.pointer(MouseEvent { kind: MouseEventKind::ScrollDown, column: at.x + 1, row: at.y, modifiers: KeyModifiers::NONE }, at.x as f64 + 1.5, false);
    }
    render(&mut app);
    assert_eq!(app.sidebar.sel, rows - 1);
    assert!(entry(&app, 100).is_some(), "the end shows");
    // a key moves the highlight on from where it was, and the view goes back to it
    key(&mut app, KeyCode::Down);
    render(&mut app);
    assert_eq!((app.sidebar.sel, app.sidebar.top), (rows, rows));

    // a click on an entry in the third column opens that one
    let row = app.sidebar.top + 2 * rows + 1;
    let r = entry(&app, row).unwrap();
    app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x + 8, row: r.y + 1, modifiers: KeyModifiers::NONE });
    app.mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: r.x + 8, row: r.y + 1, modifiers: KeyModifiers::NONE });
    assert_eq!(app.sidebar.sel, row);
    assert_eq!(app.list_view().map(|v| v.uri.clone()), Some(format!("spotify:playlist:p{}", row - FIXED)));
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

/// The braille dots in `slot` (a cell's dots in each of its four dot rows), as lines of dots: what's lit, row by row.
fn dot_rows(buf: &Buffer, slot: Rect) -> Vec<u32> {
    let bit = |x: u16, y: u16, dy: u32| buf[(x, y)].symbol().chars().next().filter(|c| ('\u{2800}'..='\u{28ff}').contains(c)).map_or(0, |c| (c as u32 - 0x2800) & [0x09, 0x12, 0x24, 0xc0][dy as usize]);
    (0..4 * slot.height as u32).map(|d| (slot.x..slot.right()).map(|x| bit(x, slot.y + (d / 4) as u16, d % 4).count_ones()).sum()).collect()
}

/// #76 (the owner, 2026-11-28: "a larger version of the og icon"): where the others have covers, Home has its ⌂ as big
/// as one, drawn in braille on cells, in the colour its glyph had (the accent while Home is open, else dim), on the
/// row's own background (the highlight's or the Pane's): no square behind it. In one-line lists it stays the one-cell
/// glyph.
#[test]
fn home_is_its_house_as_big_as_a_cover() {
    for (theme, open, highlighted) in [("singularity", true, true), ("singularity", false, false), ("classic", false, true), ("classic", true, false)] {
        let th = crate::theme::builtin(theme).unwrap();
        for (w, h) in [(40, 30), (78, 24), (118, 38), (178, 47), (240, 60)] {
            let what = format!("{theme}, Home open {open}, highlighted {highlighted}, {w}×{h}");
            let mut app = app(3);
            if !open {
                app.page = crate::app::Page::Search;
            }
            if !highlighted {
                app.focus_on(Widget::Home);
            }
            let r = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(r);
            draw(&mut buf, r, &mut app, &th);
            let cover = Shape::of(w, h).cover;
            let home = entry(&app, 0).unwrap();
            let slot = Rect::new(home.x + 1, home.y, cover * 2, cover);
            // four fifths of the slot's height, over half its width (⌂ is a little taller than wide), from a one-dot
            // peak to a floor
            let dots = dot_rows(&buf, slot);
            let lit: Vec<u32> = dots.iter().copied().filter(|&n| n > 0).collect();
            assert_eq!(lit.len(), (dots.len() as f32 * 0.8).round() as usize, "{what}: {dots:?}");
            assert_eq!(lit[0], 1, "{what}: the roof's peak {dots:?}");
            assert!(*lit.last().unwrap() > slot.width as u32, "{what}: the floor {dots:?}");
            let row = |y: u16| (home.x..home.right()).map(|x| buf[(x, y)].symbol()).collect::<String>();
            assert!(!row(home.y).contains('⌂') && row(home.y).contains("Home"), "{}", row(home.y));
            let (fg, bg) = (if open { th.accent } else { th.dim }, if highlighted { th.selection } else { th.bg });
            for y in slot.y..slot.bottom() {
                for x in slot.x..slot.right() {
                    let c = &buf[(x, y)];
                    // the row's own background, as beside the slot: no square of another colour behind
                    assert_eq!((c.bg, buf[(slot.right(), y)].bg), (bg.color(), bg.color()), "{what} ({x}, {y})");
                    if c.symbol() != " " {
                        assert_eq!(c.fg, fg.color(), "{what} ({x}, {y})");
                    }
                }
            }
            // the next entry starts below it as before
            assert!(entry(&app, 1).unwrap().y >= slot.bottom());
        }
    }
    // a one-line list keeps the glyph
    let mut app = app(3);
    let th = crate::theme::builtin("singularity").unwrap();
    let r = Rect::new(0, 0, 30, 20);
    let mut buf = Buffer::empty(r);
    draw(&mut buf, r, &mut app, &th);
    let home = entry(&app, 0).unwrap();
    assert_eq!(buf[(home.x + 1, home.y)].symbol(), "⌂");
}

/// On the pixel tier Home's ⌂ is the font's own (the terminal's, else one the system has), drawn big over the slot's
/// cells (big text's way): one RGBA image the slot's size, placed on it, its ink centred in four fifths of it, the
/// cells under it blank in the row's own colours. With no font that has ⌂, the braille house.
#[test]
fn on_the_pixel_tier_home_is_the_font_s_own_house_drawn_big() {
    // a made-up ⌂ whose ink is 0.6 by 0.7 em, its left edge 0.1 em from the pen and its bottom on the baseline
    let ink = |px: f32| Some((0.1 * px, 0.0, 0.6 * px, 0.7 * px));
    let (w, h) = (80, 80);
    let (px, pen, baseline) = super::centred(w, h, ink).unwrap();
    let (xmin, _, iw, ih) = ink(px).unwrap();
    assert!((ih - 0.8 * h as f32).abs() <= 1.0, "four fifths of the height: {ih}");
    let (left, top) = (pen + xmin, baseline - ih);
    assert!((left - (w as f32 - iw - left)).abs() <= 1.0 && (top - (h as f32 - ih - top)).abs() <= 1.0, "centred: {left} {top}");
    // a wide, short slot: the width caps it
    let (px, ..) = super::centred(40, 400, ink).unwrap();
    assert!(0.6 * px <= 0.8 * 40.0 + 0.01);
    assert_eq!(super::centred(80, 80, |_| None), None);

    let (w, h) = (160, 48);
    let draw_px = |app: &mut App, th: &Theme| {
        let r = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(r);
        draw(&mut buf, r, app, th);
        let mut out = vec![];
        app.kitty.as_mut().unwrap().flush(&mut out).unwrap();
        (buf, String::from_utf8_lossy(&out).into_owned())
    };
    let pixel_app = || {
        let mut app = app(3);
        app.cell_px = Some((10.0, 20.0));
        app.kitty = Some(crate::gfx::Kitty::new(false));
        app
    };
    // a system font has ⌂ (here the bundled one stands in, drawing its box)
    crate::bigtext::pretend_system_has(true);
    for theme in ["singularity", "classic"] {
        let th = crate::theme::builtin(theme).unwrap();
        let mut app = pixel_app();
        let (buf, apc) = draw_px(&mut app, &th);
        let home = entry(&app, 0).unwrap();
        let cover = home.height;
        let slot = Rect::new(home.x + 1, home.y, 2 * cover, cover);
        // one RGBA image the slot's size in pixels, placed at its top left over its cells
        let (pw, ph) = (slot.width as u32 * 10, slot.height as u32 * 20);
        assert_eq!(apc.matches("f=32").count(), 1, "{apc:.300}");
        assert!(apc.contains(&format!("s={pw},v={ph}")), "{apc:.300}");
        assert!(apc.contains(&format!("\x1b[{};{}H\x1b_Ga=p,", slot.y + 1, slot.x + 1)), "{apc:.300}");
        assert!(apc.contains(&format!("c={},r={},", slot.width, slot.height)), "{apc:.300}");
        // the cells under it blank, in the row's background (Home is highlighted)
        for y in slot.y..slot.bottom() {
            for x in slot.x..slot.right() {
                assert_eq!((buf[(x, y)].symbol(), buf[(x, y)].bg), (" ", th.selection.color()), "{theme} ({x}, {y})");
            }
        }
        // the colour is in the picture: Home not open (dim) is another image; open again, the first is placed again
        app.page = crate::app::Page::Search;
        let (_, dim) = draw_px(&mut app, &th);
        assert!(dim.contains("f=32"), "a new image for the dim ⌂: {dim:.200}");
        app.page = crate::app::Page::Home;
        let (_, again) = draw_px(&mut app, &th);
        assert!(!again.contains("f=32") && again.contains("a=p,"), "the accent one is still there: {again:.200}");
        // not highlighted: the cells keep the Pane's background
        app.focus_on(Widget::Home);
        let (buf, _) = draw_px(&mut app, &th);
        assert!((slot.y..slot.bottom()).all(|y| (slot.x..slot.right()).all(|x| buf[(x, y)].symbol() == " " && buf[(x, y)].bg == th.bg.color())), "{theme}");
    }
    // no font at all has ⌂: the braille house, no image
    crate::bigtext::pretend_system_has(false);
    assert!(!crate::bigtext::covers("⌂"), "the bundled font has no ⌂ (if it gets one, check this test's cells)");
    let th = crate::theme::builtin("singularity").unwrap();
    let mut app = pixel_app();
    let (buf, apc) = draw_px(&mut app, &th);
    let home = entry(&app, 0).unwrap();
    assert!(!apc.contains("f=32"), "{apc:.200}");
    assert!(dot_rows(&buf, Rect::new(home.x + 1, home.y, 2 * home.height, home.height)).iter().any(|&n| n > 0));
}
