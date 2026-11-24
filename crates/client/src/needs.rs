//! Each Widget's smallest size (#36, D87): the whole Pane, frame included, below which it stops being usable. A Pane
//! smaller than that shows one dim, centred line, `needs 30 × 10`, instead of its Widget; a Pane too small even for
//! that line shows only its frame, the Widget's name in its title. Every drop is allowed, so this is what a Widget in
//! too small a Pane looks like. Picked from the size contact sheet (`tools/pty`'s `contact-sheet`).

use crate::layout::{MIN_H, MIN_W, WELL_ASPECT, WELL_MIN_H};
use crate::pane::Widget;
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The black hole's (the gravity well's and Event Horizon's) smallest size, its shape rule: four rows inside the frame
/// (one more than `WELL_MIN_H` leaves, where the picture is a bare ring), and as wide as `WELL_ASPECT` (4:3 in pixels,
/// cells twice as tall as wide) over them.
const HOLE: (u16, u16) = {
    let h = WELL_MIN_H + 1;
    let rows = (h - 2) as f64;
    ((rows * 2.0 * WELL_ASPECT + 0.5) as u16 + 2, h)
};

/// The smallest Pane (columns × rows, frame included) Widget `widget` is usable in. Every Widget has its arm, so a new
/// one is asked for its own.
pub fn smallest(widget: Widget) -> (u16, u16) {
    use Widget::*;
    match widget {
        Well | Hole => HOLE,
        // a line has to wrap to be read: in one row only its middle shows
        Karaoke => (20, 6),
        // a Card's name beside its cover (Classic gives Home 20 columns in a half-screen laptop window)
        Home => (16, 6),
        // the artists' names beside their minutes
        TopWeek => (20, 4),
        // the bitrate under the badges
        Stream => (16, 5),
        Lyrics | Library | Queue | NowPlaying | Top | Listening | Today | Marquee | Glow => (16, 4),
        // the meters and the ambient Visuals still read in the smallest Pane with two rows inside
        Fft | Loudness | Levels | Scope | Rain | Starfield => (MIN_W, MIN_H + 1),
        // one this build hasn't got draws nothing anyway
        Unknown(_) => (MIN_W, MIN_H),
    }
}

/// Whether a Pane of `rect` is too small for `widget`.
pub fn too_small(widget: Widget, rect: Rect) -> bool {
    let (w, h) = smallest(widget);
    rect.width < w || rect.height < h
}

/// The `needs W × H` line, dim and centred in `inner` (inside the Pane's frame), if it fits; nothing if it doesn't.
pub fn draw(buf: &mut Buffer, inner: Rect, widget: Widget, th: &Theme) {
    let (w, h) = smallest(widget);
    let line = format!("needs {w} × {h}");
    let len = line.chars().count() as u16;
    if inner.height == 0 || inner.width < len {
        return;
    }
    let x = inner.x + (inner.width - len) / 2;
    crate::ui::text(buf, x, inner.y + (inner.height - 1) / 2, len, &line, th.s(th.dim));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::layout::{Axis, Node, Size};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// `widget` in the top-left Pane, `w × h` with its frame, Home beside it and the queue under them.
    fn screen(app: &mut App, widget: Widget, w: u16, h: u16) -> (Vec<String>, ratatui::buffer::Buffer) {
        let filler = if widget == Widget::Home { Widget::Library } else { Widget::Home };
        let rest = if widget == Widget::Queue { Widget::Lyrics } else { Widget::Queue };
        let top = Node::split(Axis::Horizontal, Size::Cells(w), Node::pane(widget), Node::pane(filler));
        app.layout = Node::split(Axis::Vertical, Size::Cells(h), top, Node::pane(rest));
        let mut t = Terminal::new(TestBackend::new(160, 60)).unwrap();
        t.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        let rows = (0..60).map(|y| (0..160).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect();
        (rows, buf)
    }

    /// The top-left Pane's rows, frame included, read off its frame's corners on screen, and where it starts.
    fn pane(rows: &[String]) -> (u16, Vec<String>) {
        let y0 = rows.iter().position(|r| r.starts_with(['╭', '┌', '╔', '┏'])).expect("the Pane's frame");
        let top: Vec<char> = rows[y0].chars().collect();
        let w = 1 + top.iter().skip(1).position(|c| "╮┐╗┓".contains(*c)).expect("its top-right corner") + 1;
        let h = 1 + rows[y0 + 1..].iter().position(|r| r.starts_with(['╰', '└', '╚', '┗'])).expect("its bottom") + 1;
        (y0 as u16, rows[y0..y0 + h].iter().map(|r| r.chars().take(w).collect()).collect())
    }

    #[test]
    fn every_widget_states_a_smallest_size_and_the_hole_keeps_its_shape() {
        for &w in Widget::ALL {
            let (sw, sh) = smallest(w);
            assert!(sw >= crate::layout::MIN_W && sh >= crate::layout::MIN_H, "{w}: {sw}×{sh} is under any Pane");
        }
        // the black hole: at least the well's height, and as wide as 4:3 over the rows inside the frame
        for w in [Widget::Well, Widget::Hole] {
            let (sw, sh) = smallest(w);
            assert!(sh >= WELL_MIN_H, "{w}");
            let shape = (sw - 2) as f64 / ((sh - 2) as f64 * 2.0);
            assert!((shape - WELL_ASPECT).abs() < 0.15, "{w}: {sw}×{sh} is {shape:.2} wide, not the well's 4:3");
        }
    }

    #[test]
    fn a_gravity_well_too_small_for_the_hole_is_a_plain_pane() {
        let mut app = App::new(None);
        let (sw, sh) = smallest(Widget::Well);
        let hole = |p: &[String]| p.iter().any(|r| r.contains('▀') || r.contains('▄'));
        // one column short (too narrow for the line too): a plain frame round nothing, no sagging braille
        let (rows, _) = screen(&mut app, Widget::Well, sw - 1, sh);
        let (_, p) = pane(&rows);
        assert!(!hole(&p) && !p.iter().any(|r| r.chars().any(|c| ('\u{2800}'..='\u{28ff}').contains(&c))), "{p:#?}");
        // at its smallest size the hole draws (in its own sagging frame, so look at the whole screen)
        let (rows, _) = screen(&mut app, Widget::Well, sw, sh);
        assert!(hole(&rows), "{rows:#?}");
    }

    /// Cells under twice as tall as wide (a common 10×19 or 9×17 px font): the well's shape nudge never makes it narrower
    /// than its own smallest size, so a 6-row well still draws its hole.
    #[test]
    fn a_well_at_its_smallest_size_draws_with_squatter_cells() {
        let (sw, sh) = smallest(Widget::Well);
        let hole = |p: &[String]| p.iter().any(|r| r.contains('▀') || r.contains('▄'));
        for px in [(10.0, 19.0), (9.0, 17.0)] {
            let mut app = App::new(None);
            app.cell_px = Some(px);
            let (rows, _) = screen(&mut app, Widget::Well, sw, sh);
            assert!(hole(&rows) && !rows.iter().any(|r| r.contains("needs")), "{px:?}:\n{}", rows.join("\n"));
        }
    }

    /// A Pane showing its needs line doesn't act for the Widget it hides: no wheel, no keys, no title switch, and the
    /// idle takeover doesn't start from a hole that isn't drawn.
    #[test]
    fn a_pane_too_small_takes_no_wheel_keys_or_clicks() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
        use singularity_proto::{Item, QueueEntry};
        let mut app = App::new(None);
        app.playback.queue = (0..40).map(|i| QueueEntry { item: Item { uri: format!("spotify:track:q{i}"), name: format!("Queued {i}"), duration_ms: 150_000, ..Default::default() }, source: "queue".into(), uid: format!("u{i}") }).collect();
        let (sw, sh) = smallest(Widget::Queue);
        let (rows, _) = screen(&mut app, Widget::Queue, sw.max(20), sh - 1);
        let (y0, p) = pane(&rows);
        assert!(p.iter().any(|r| r.contains("needs")), "{p:#?}");
        app.focus = app.layout.pane_of(Widget::Queue);
        let (x, y) = (3, y0 + 1);
        for _ in 0..3 {
            app.pointer(MouseEvent { kind: MouseEventKind::ScrollDown, column: x, row: y, modifiers: KeyModifiers::NONE }, x as f64 + 0.5, false);
        }
        assert_eq!(app.queue_view.sel, 0, "the wheel moved a hidden highlight");
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.queue_view.sel, 0, "a key moved a hidden highlight");
        // big enough again: the same wheel moves it
        screen(&mut app, Widget::Queue, sw.max(20), sh + 4);
        app.pointer(MouseEvent { kind: MouseEventKind::ScrollDown, column: x, row: y, modifiers: KeyModifiers::NONE }, x as f64 + 0.5, false);
        assert_eq!(app.queue_view.sel, 1, "a Pane big enough takes the wheel");
        // top this week's title is its switch only while its list shows
        let mut app = App::new(None);
        let (sw, sh) = smallest(Widget::TopWeek);
        screen(&mut app, Widget::TopWeek, sw, sh - 1);
        assert!(!app.hits.iter().any(|(_, h)| *h == crate::app::Hit::TopWeekToggle), "a hidden list's switch");
        // the hole's origin goes with the hole
        let mut app = App::new(None);
        let (sw, sh) = smallest(Widget::Hole);
        screen(&mut app, Widget::Hole, sw, sh);
        assert!(app.hole_origin.is_some());
        screen(&mut app, Widget::Hole, sw, sh - 1);
        assert!(app.hole_origin.is_none(), "{:?}", app.hole_origin.map(|o| (o.cx, o.cy)));
    }

    #[test]
    fn a_pane_below_its_widgets_smallest_size_says_what_it_needs() {
        let mut app = App::new(None);
        use Widget::*;
        for widget in [Lyrics, Karaoke, Home, Queue, TopWeek, Fft, Hole] {
            let (sw, sh) = smallest(widget);
            let line = format!("needs {sw} × {sh}");
            // one row short: the line, dim, centred in the Pane, and nothing else inside
            let (w, h) = (sw.max(line.chars().count() as u16 + 2), sh - 1);
            let (rows, buf) = screen(&mut app, widget, w, h);
            let (y0, p) = pane(&rows);
            let shown = p.join("\n");
            let inside: Vec<Vec<char>> = p[1..p.len() - 1].iter().map(|r| r.chars().skip(1).take(r.chars().count() - 2).collect()).collect();
            let lw = line.chars().count();
            let find = |r: &Vec<char>| r.windows(lw).position(|c| c.iter().collect::<String>() == line);
            let at = inside.iter().position(|r| find(r).is_some()).unwrap_or_else(|| panic!("{widget} at {w}×{h}: no `{line}`\n{shown}"));
            assert!(at.abs_diff(inside.len() - 1 - at) <= 1, "{widget}: centred top to bottom\n{shown}");
            let l = find(&inside[at]).unwrap();
            let r = inside[at].len() - l - lw;
            assert!(l.abs_diff(r) <= 1, "{widget}: centred side to side ({l}, {r})\n{shown}");
            for (i, row) in inside.iter().enumerate() {
                if i != at {
                    assert!(row.iter().all(|c| *c == ' '), "{widget} at {w}×{h}: only the line inside\n{shown}");
                }
            }
            let cell = &buf[(1 + l as u16, y0 + 1 + at as u16)];
            assert_eq!(Some(cell.fg), app.theme.s(app.theme.dim).fg, "{widget}: the line is dim");
            // one column short: the same, where the line fits
            if sw - 3 >= lw as u16 {
                let (rows, _) = screen(&mut app, widget, sw - 1, sh);
                assert!(pane(&rows).1.iter().any(|r| r.contains(&line)), "{widget} at {}×{sh}", sw - 1);
            }
            // big enough: the Widget itself
            let (rows, _) = screen(&mut app, widget, sw, sh);
            assert!(!pane(&rows).1.iter().any(|r| r.contains("needs")), "{widget} at its smallest size draws itself");
        }
    }

    /// Keep established looks: no built-in Design gives a Widget less than its smallest size at the terminal sizes it is
    /// used at (the contact sheet's: a common 80×24 up to the PC's full screen, and a 13-inch laptop's).
    #[test]
    fn the_built_in_designs_need_no_line_at_common_sizes() {
        for d in 0..crate::design::DESIGNS.len() {
            for (w, h) in [(80u16, 24u16), (90, 50), (120, 36), (160, 48), (180, 54), (220, 60), (264, 68)] {
                let mut app = App::new(None);
                app.set_design(d);
                let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                t.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
                let buf = t.backend().buffer().clone();
                let s: String = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect();
                // (the stream Widget has a "needs 205 kB/s" of its own)
                assert!(!s.split("needs ").skip(1).any(|r| r.split_once(' ').is_some_and(|(n, rest)| n.parse::<u16>().is_ok() && rest.starts_with('×'))), "{} at {w}×{h}:\n{s}", crate::design::DESIGNS[d].id);
            }
        }
    }

    #[test]
    fn a_pane_too_small_for_the_line_shows_only_its_title() {
        let mut app = App::new(None);
        // 12×3: one row of ten cells inside, too few for `needs …`
        let (rows, _) = screen(&mut app, Widget::Lyrics, 12, 3);
        let (_, p) = pane(&rows);
        assert!(p[0].chars().any(char::is_alphabetic), "its name in the title: {p:#?}");
        let inside: String = p[1].chars().skip(1).take(10).collect();
        assert!(inside.trim().is_empty(), "nothing inside: {p:#?}");
    }
}
