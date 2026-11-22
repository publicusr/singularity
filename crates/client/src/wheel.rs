//! The wheel over the Panes, by construction, as popups have it (`App::modal_scroll`): every Widget a Pane can show
//! is a [`Widget`], every page of the main Pane an `app::Page`, and `App::pane_wheel` / `App::page_wheel` match them
//! with no catch-all arm. A new Widget or page can't be added without saying what the wheel does over it, and the test
//! in `wheel/tests.rs` wheels every one of them at a size where it holds more than fits and checks that it moved.
//!
//! Lists move their highlight (`scroll::moved`; Shift: a page) and show a window that follows it (`scroll::window`),
//! text scrolls (`scroll::TextScroll`), a view that follows the song is held for a while (`scroll::Follow`). Wheeling a
//! Pane that scrolls focuses it, as a click does, so its highlight shows.

use crate::app::{App, Hit, Page};
use crate::scroll::moved;

/// Declares `Widget`, `Widget::ALL` and the Slot names from one list.
macro_rules! widgets {
    ($($(#[$doc:meta])* $v:ident = $slot:literal,)*) => {
        /// What a Pane shows, by its Slot name (`layout::Node::Slot`).
        ///
        /// Adding a Widget? Add it here; the compiler then asks for its arm in `App::pane_wheel` (what the wheel does
        /// over it) and in the wheel test's fixture (something that overflows it).
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub(crate) enum Widget {
            $($(#[$doc])* $v,)*
        }

        impl Widget {
            #[cfg_attr(not(test), allow(dead_code))]
            pub(crate) const ALL: &[Widget] = &[$(Widget::$v),*];

            /// The Slot name a layout uses for it.
            #[cfg_attr(not(test), allow(dead_code))]
            pub(crate) fn slot(self) -> &'static str {
                match self {
                    $(Widget::$v => $slot,)*
                }
            }

            /// The Widget a Slot shows (a saved layout may name one this build doesn't have).
            pub(crate) fn of(slot: &str) -> Option<Widget> {
                match slot {
                    $($slot => Some(Widget::$v),)*
                    _ => None,
                }
            }
        }
    };
}

widgets! {
    Library = "library",
    /// the main Pane: what the wheel does depends on its page (`App::page_wheel`)
    Home = "home",
    Queue = "queue",
    Lyrics = "lyrics",
    Karaoke = "karaoke",
    Stream = "stream",
    NowPlaying = "nowplaying",
    Top = "top",
    Listening = "listening",
    Today = "today",
    TopWeek = "topweek",
    Marquee = "marquee",
    Glow = "glow",
    Fft = "fft",
    Loudness = "loudness",
    Levels = "levels",
    Scope = "scope",
    Rain = "rain",
    Starfield = "starfield",
    Well = "well",
    Hole = "hole",
}

impl App {
    /// The wheel over Pane `w` at (`x`, `y`): `rows` notches down (negative: up), `coarse` with Shift (a page). Every
    /// Widget decides here: no catch-all arm.
    pub(crate) fn pane_wheel(&mut self, w: Widget, x: u16, y: u16, rows: i32, coarse: bool) {
        match w {
            Widget::Home => {
                self.focus = "home".into();
                self.page_wheel(self.current_page(), x, y, rows, coarse);
            }
            Widget::Library => {
                self.focus = "library".into();
                let n = crate::sidebar::len(self);
                let step = if coarse { self.sidebar.visible.max(1) as i64 } else { 1 };
                self.sidebar.move_sel(rows as i64 * step, n);
            }
            Widget::Queue => {
                self.focus = "queue".into();
                let n = self.playback.queue.len();
                let step = if coarse { self.queue_view.visible.max(1) as i64 } else { 1 };
                self.queue_view.move_sel(rows as i64 * step, n);
            }
            // three rows a notch; synced lyrics are held where the wheel puts them for a while, then follow the song
            // again; unsynced ones stay where the reader took them
            Widget::Lyrics => {
                if self.lyrics.wheel_lyrics(rows * if coarse { 10 } else { 3 }) {
                    self.focus = "lyrics".into();
                }
            }
            // the line being sung is the whole point when the lyrics are synced (the whole song is the lyrics
            // Widget's); unsynced, it shows the whole text, which the wheel scrolls for a while
            Widget::Karaoke => {
                if self.lyrics.wheel_karaoke(rows * if coarse { 10 } else { 3 }) {
                    self.focus = "karaoke".into();
                }
            }
            Widget::TopWeek => {
                self.focus = "topweek".into();
                self.top_week_scroll.by(rows * if coarse { 5 } else { 1 });
            }
            // they scale to their Pane (size tiers) rather than cut anything off, or are Visuals: nothing to scroll
            Widget::Stream | Widget::NowPlaying | Widget::Top | Widget::Listening | Widget::Today | Widget::Marquee | Widget::Glow => {}
            Widget::Fft | Widget::Loudness | Widget::Levels | Widget::Scope | Widget::Rain | Widget::Starfield | Widget::Well | Widget::Hole => {}
        }
    }

    /// The wheel over the main Pane showing `page`. Every page decides here: no catch-all arm.
    fn page_wheel(&mut self, page: Page, x: u16, y: u16, rows: i32, coarse: bool) {
        // a song row (or its name links) under the pointer, on the pages with songs above Shelves
        let on_song = self.hits.iter().any(|(r, h)| matches!(h, Hit::TopTrack(_) | Hit::SearchTrack(_)) && r.y == y && x >= r.x && x < r.right());
        match page {
            // Shelves scroll by lines (Shift over a Shelf: sideways, a Card at a time)
            Page::Home => self.shelves_wheel(x, y, rows, coarse),
            // track lists, Show and Book pages: the highlight, and the list follows it
            Page::List { .. } => {
                let len = self.view_order().len();
                if let Some(v) = self.view.as_mut() {
                    let step = if coarse { v.visible.max(1) as i32 } else { 1 };
                    v.sel = moved(v.sel, rows * step, len);
                    v.reveal();
                }
            }
            // popular tracks on top, Shelves below: over a track (or with no Shelves) the highlight, else the Shelves
            Page::Artist { .. } => {
                let Some(a) = self.artist.as_mut() else { return };
                let n = a.page.as_ref().map_or(0, |p| p.top_tracks.len());
                if n > 0 && (on_song || a.feed.shelves.is_empty()) {
                    a.top_sel = moved(a.top_sel, rows * if coarse { 5 } else { 1 }, n);
                } else {
                    self.shelves_wheel(x, y, rows, coarse);
                }
            }
            // no query yet: the recent searches; then songs (or episodes) and Shelves, as on an artist page
            Page::Search => {
                let Some(s) = self.searching_mut() else { return };
                let recent = s.recent().len();
                let step = rows * if coarse { 5 } else { 1 };
                if s.query().trim().is_empty() {
                    s.recent_sel = moved(s.recent_sel, step, recent);
                    return;
                }
                let n = s.songs();
                if n > 0 && (on_song || s.feed.shelves.is_empty()) {
                    s.track_sel = Some(s.track_sel.map_or(0, |i| moved(i, step, n)));
                    s.shelves.sel = None;
                } else {
                    self.shelves_wheel(x, y, rows, coarse);
                }
            }
            Page::Stats => {
                let n = self.stats_view.as_ref().and_then(|v| self.stats.get(&v.period)).map_or(0, |s| crate::stats::entries(s).len());
                if let Some(v) = self.stats_view.as_mut() {
                    let step = if coarse { v.visible.max(1) as i32 } else { 1 };
                    v.sel = moved(v.sel, rows * step, n);
                }
            }
            Page::Settings => self.settings_wheel(rows as i64, coarse),
        }
    }

    /// The Shelves of the page on show (Home, an artist's, the search results'): by lines, or with Shift over a Shelf
    /// sideways.
    fn shelves_wheel(&mut self, x: u16, y: u16, rows: i32, coarse: bool) {
        match self.home_row_at(x, y) {
            Some(row) if coarse => self.feed_view().scroll_shelf(row, rows),
            _ => self.feed_view().scroll_lines(rows * 2),
        }
    }
}

#[cfg(test)]
mod tests;
