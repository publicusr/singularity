//! Scrolling, wherever there's more than fits. Every popup is an `app::Modal` and `App::modal_scroll` says what the
//! wheel does in it; every Widget is a `wheel::Widget` and every main-Pane page an `app::Page`, and `App::pane_wheel`
//! says what it does over them. A list moves its selection with [`moved`] and shows a [`window`] that follows it, text
//! taller than its box scrolls with a [`TextScroll`], and a view that follows the song (lyrics) is a [`Follow`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::cell::Cell;
use std::time::{Duration, Instant};

/// A list's selection `rows` rows on (negative: up), kept inside a list of `len` items.
pub fn moved(sel: usize, rows: i32, len: usize) -> usize {
    (sel as i64 + rows as i64).clamp(0, len.saturating_sub(1) as i64) as usize
}

/// The first row to show so that `sel` stays in a window of `rows` rows over `len` items.
pub fn window(sel: usize, rows: usize, len: usize) -> usize {
    let rows = rows.max(1);
    sel.saturating_sub(rows - 1).min(len.saturating_sub(rows))
}

/// Text that can be taller than its popup: how far it's scrolled, and how far it can go (learnt when it's drawn, so
/// the wheel never takes it past its end; drawing takes `&self`, hence the cells).
#[derive(Default, Clone, Debug, PartialEq)]
pub struct TextScroll {
    offset: Cell<usize>,
    max: Cell<usize>,
}

impl TextScroll {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn offset(&self) -> usize {
        self.offset.get()
    }

    /// `rows` rows on (negative: back), within what the last frame showed there is.
    pub fn by(&self, rows: i32) {
        self.offset.set((self.offset.get() as i64 + rows as i64).clamp(0, self.max.get() as i64) as usize);
    }

    pub fn reset(&self) {
        self.offset.set(0);
    }

    /// `len` rows of content in a box `rows` tall: the first one to show (the offset, clamped to the new end).
    pub fn window(&self, len: usize, rows: usize) -> usize {
        self.max.set(len.saturating_sub(rows));
        self.offset.set(self.offset.get().min(self.max.get()));
        self.offset.get()
    }

    /// Draw content of any height into `r`, scrolled: `body` draws into a tall scratch area shaped like `r` and
    /// returns how many rows it used.
    pub fn draw(&self, buf: &mut Buffer, r: Rect, body: impl FnOnce(&mut Buffer, Rect) -> u16) {
        if r.is_empty() {
            return;
        }
        let tall = Rect::new(r.x, r.y, r.width, 256.max(r.height));
        let mut scratch = Buffer::empty(tall);
        // the popup's own background
        let base = buf[(r.x, r.y)].style();
        scratch.set_style(tall, base);
        let used = body(&mut scratch, tall).min(tall.height);
        let first = self.window(used as usize, r.height as usize) as u16;
        for y in 0..r.height.min(tall.height - first) {
            for x in r.x..r.right() {
                buf[(x, r.y + y)] = scratch[(x, r.y + first + y)].clone();
            }
        }
    }
}

/// How long a view that follows the song stays where the wheel left it before it follows again.
pub const HOLD: Duration = Duration::from_secs(5);

/// A view that follows something by itself (lyrics follow the song) until the wheel takes it: then it stays where the
/// wheel put it until [`HOLD`] after the last notch, and goes back to following. Drawing takes `&self`, hence the cells.
#[derive(Default, Debug)]
pub struct Follow {
    /// the first row the last frame showed, and the last first row the wheel may take it to
    shown: Cell<usize>,
    max: Cell<usize>,
    /// where the wheel put it, and when
    held: Cell<Option<(usize, Instant)>>,
}

impl Follow {
    /// `rows` rows on (negative: back) from what's on screen, within what there is; held there for a while.
    pub fn by(&self, rows: i32) {
        let from = self.held.get().map_or(self.shown.get(), |(at, _)| at) as i64;
        let to = (from + rows as i64).clamp(0, self.max.get() as i64) as usize;
        self.held.set(Some((to, Instant::now())));
    }

    /// Whether the wheel has the view.
    pub fn held(&self) -> bool {
        self.held.get().is_some_and(|(_, t)| t.elapsed() < HOLD)
    }

    /// The first row to show: `auto` (where following puts it), unless the wheel has the view; `max` is the last first
    /// row the wheel may take it to.
    pub fn first(&self, auto: usize, max: usize) -> usize {
        self.max.set(max);
        let first = match self.held.get() {
            Some((at, _)) if self.held() => at.min(max),
            _ => {
                self.held.set(None);
                auto
            }
        };
        self.shown.set(first);
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_following_view_is_held_by_the_wheel_then_follows_again() {
        let f = Follow::default();
        assert_eq!(f.first(7, 20), 7, "following");
        f.by(3);
        assert!(f.held());
        assert_eq!(f.first(8, 20), 10, "from what was on screen, and held while the song moves on");
        f.by(-100);
        assert_eq!(f.first(9, 20), 0, "stops at the top");
        f.by(100);
        assert_eq!(f.first(9, 20), 20, "and at the end");
        // the hold runs out
        f.held.set(Some((20, Instant::now() - HOLD)));
        assert_eq!(f.first(9, 20), 9);
        assert!(!f.held());
    }

    #[test]
    fn lists_and_text_stay_inside() {
        assert_eq!((moved(0, -1, 5), moved(3, 1, 5), moved(4, 1, 5), moved(2, 10, 5), moved(0, 1, 0)), (0, 4, 4, 4, 0));
        assert_eq!((window(0, 3, 10), window(5, 3, 10), window(9, 3, 10), window(2, 10, 4)), (0, 3, 7, 0));
        let s = TextScroll::default();
        // nothing drawn yet: nothing to scroll
        s.by(3);
        assert_eq!(s.offset(), 0);
        assert_eq!(s.window(10, 4), 0);
        s.by(100);
        assert_eq!(s.offset(), 6);
        s.by(-2);
        assert_eq!(s.offset(), 4);
        // the content shrank: the offset follows its end
        assert_eq!(s.window(6, 4), 2);
        s.by(-100);
        assert_eq!(s.offset(), 0);
    }

    #[test]
    fn drawn_text_scrolls_and_keeps_the_background() {
        use ratatui::style::{Color, Style};
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 6));
        let r = Rect::new(1, 1, 8, 3);
        buf.set_style(r, Style::default().bg(Color::Blue));
        let s = TextScroll::default();
        let body = |b: &mut Buffer, a: Rect| {
            for i in 0..5u16 {
                b.set_string(a.x, a.y + i, format!("line {i}"), Style::default());
            }
            5
        };
        s.draw(&mut buf, r, body);
        s.by(5);
        s.draw(&mut buf, r, body);
        let row = |b: &Buffer, y: u16| (r.x..r.right()).map(|x| b[(x, y)].symbol().to_string()).collect::<String>();
        assert_eq!((row(&buf, 1), row(&buf, 3)), ("line 2  ".into(), "line 4  ".into()));
        assert_eq!(buf[(8, 1)].bg, Color::Blue);
        // the rows around the box are untouched
        assert_eq!(row(&buf, 4), "        ");
    }
}
