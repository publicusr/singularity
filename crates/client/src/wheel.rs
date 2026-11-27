//! The wheel over the Panes, by construction, as popups have it (`App::modal_scroll`): every Widget a Pane can show
//! is a [`Widget`], every page of the main Pane an `app::Page`, and `App::pane_wheel` / `App::page_wheel` match them
//! with no catch-all arm. A new Widget or page can't be added without saying what the wheel does over it, and the test
//! in `wheel/tests.rs` wheels every one of them at a size where it holds more than fits and checks that it moved.
//!
//! The wheel and the touchpad scroll a list's **view** and leave its highlight where it is (D86, `scroll::Wheeled`;
//! Shift: a page); the pointer moving over a row highlights it, a click picks it, and keys move the highlight as
//! before (`scroll::moved`), the view following it again (`scroll::window`). Text scrolls (`scroll::TextScroll`), a
//! view that follows the song is held for a while (`scroll::Follow`). Wheeling a Pane that scrolls focuses it, as a
//! click does, so its highlight shows.

use crate::app::{App, Hit, Page};

pub(crate) use crate::pane::Widget;

impl App {
    /// The wheel over Pane `w` at (`x`, `y`): `rows` notches down (negative: up), `coarse` with Shift (a page). Every
    /// Widget decides here: no catch-all arm.
    pub(crate) fn pane_wheel(&mut self, w: Widget, x: u16, y: u16, rows: i32, coarse: bool) {
        // a Pane showing its needs line: nothing it hides moves (#36)
        if self.layout.pane_of(w).is_some_and(|p| self.pane_is_short(p)) {
            return;
        }
        match w {
            Widget::Home => {
                self.focus_on(Widget::Home);
                self.page_wheel(x, y, rows, coarse);
            }
            Widget::Library => {
                self.focus_on(Widget::Library);
                // (in columns, a line is a column of entries)
                let s = &mut self.sidebar;
                let page = if s.cols > 1 { s.cols } else { s.visible };
                let step = if coarse { page.max(1) as i64 } else { 1 };
                s.scroll(rows as i64 * step);
            }
            Widget::Queue => {
                self.focus_on(Widget::Queue);
                let step = if coarse { self.queue_view.visible.max(1) as i32 } else { 1 };
                self.queue_view.scroll(rows * step);
            }
            // three rows a notch; synced lyrics are held where the wheel puts them for a while, then follow the song
            // again; unsynced ones stay where the reader took them
            Widget::Lyrics => {
                if self.lyrics.wheel_lyrics(rows * if coarse { 10 } else { 3 }) {
                    self.focus_on(Widget::Lyrics);
                }
            }
            // the line being sung is the whole point when the lyrics are synced (the whole song is the lyrics
            // Widget's); unsynced, it shows the whole text, which the wheel scrolls for a while
            Widget::Karaoke => {
                if self.lyrics.wheel_karaoke(rows * if coarse { 10 } else { 3 }) {
                    self.focus_on(Widget::Karaoke);
                }
            }
            Widget::TopWeek => {
                self.focus_on(Widget::TopWeek);
                self.top_week_scroll.by(rows * if coarse { 5 } else { 1 });
            }
            // they scale to their Pane (size tiers) rather than cut anything off, or are Visuals: nothing to scroll
            Widget::Stream | Widget::NowPlaying | Widget::Top | Widget::Listening | Widget::Today | Widget::Marquee | Widget::Glow => {}
            Widget::Fft | Widget::Loudness | Widget::Levels | Widget::Scope | Widget::Rain | Widget::Starfield | Widget::Well | Widget::Hole => {}
            // another build's Widget: nothing is drawn in it
            Widget::Unknown(_) => {}
        }
    }

    /// The wheel over the main Pane, by the page it shows. Every page decides here: no catch-all arm.
    fn page_wheel(&mut self, x: u16, y: u16, rows: i32, coarse: bool) {
        // a song row (or its name links) under the pointer, on the pages with songs above Shelves
        let on_song = self.hits.iter().any(|(r, h)| matches!(h, Hit::TopTrack(_) | Hit::SearchTrack(_)) && r.y == y && x >= r.x && x < r.right());
        match &mut self.page {
            // Shelves scroll by lines (Shift over a Shelf: sideways, a Card at a time)
            Page::Home => self.shelves_wheel(x, y, rows, coarse),
            // track lists, Show and Book pages: the list (the highlight stays; keys bring the list back to it)
            Page::List { uri, .. } => {
                if let Some(v) = self.open.view_mut(uri) {
                    let step = if coarse { v.visible.max(1) as i32 } else { 1 };
                    v.wheeled.by(&mut v.top, rows * step, v.sel);
                }
            }
            // popular tracks on top, Shelves below: over a track (or with no Shelves) the tracks, else the Shelves
            Page::Artist(a) => {
                let n = a.page.as_ref().map_or(0, |p| p.top_tracks.len());
                if n > 0 && (on_song || a.feed.shelves.is_empty()) {
                    a.top_wheeled.by(&mut a.top_first, rows * if coarse { 5 } else { 1 }, a.top_sel);
                } else {
                    self.shelves_wheel(x, y, rows, coarse);
                }
            }
            // no query yet: the recent searches; then songs (or episodes) and Shelves, as on an artist page
            Page::Search => {
                let s = &mut self.search;
                let recent = s.recent().len();
                let step = rows * if coarse { 5 } else { 1 };
                if s.query().trim().is_empty() {
                    if recent > 0 {
                        s.recent_wheeled.by(&mut s.recent_first, step, s.recent_sel);
                    }
                    return;
                }
                let n = s.songs();
                // a short Pane shows the songs or the Shelves: the wheel goes on from the last songs into the Shelves,
                // and back up from their top to the last songs, the highlight where it is
                let shelves = !s.feed.shelves.is_empty();
                if s.short && shelves && step > 0 && on_song && s.track_wheeled.at_end(s.track_first) {
                    s.shelves_shown = Some(true);
                    s.shelves.offset = 0;
                } else if s.short && n > 0 && step < 0 && s.showing_shelves() && s.shelves.offset == 0 {
                    s.shelves_shown = Some(false);
                    let key = s.track_key();
                    s.track_wheeled.by(&mut s.track_first, i32::MAX, key);
                } else if n > 0 && (on_song || s.feed.shelves.is_empty()) {
                    let key = s.track_key();
                    s.track_wheeled.by(&mut s.track_first, step, key);
                } else {
                    self.shelves_wheel(x, y, rows, coarse);
                }
            }
            Page::Stats(v) => {
                let step = if coarse { v.visible.max(1) as i32 } else { 1 };
                v.wheeled.by(&mut v.top, rows * step, v.sel);
            }
            Page::Settings(_) => self.settings_wheel(rows as i64, coarse),
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
