//! The Widget catalogue (D89): every Widget's fixed facts in one place. Its title, smallest size, swap group, whether
//! the Widget picker offers it, whether it is Required, whether it is the black hole, whether it draws its own frame,
//! and how it is drawn. One match over every Widget, no catch-all: a new Widget doesn't compile until it has them all.
//! The modules that use a fact (`ui`, `needs`, `layouts`, `panemenu`, `arrangement`, `layout`) ask here.

use crate::app::{Action, App, Hit};
use crate::layout::{MIN_H, MIN_W, WELL_ASPECT};
use crate::pane::Widget;
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Widgets that can take each other's place: ◂ ▸ in a Pane's title (and v) cycle through its group, in `Widget::ALL`'s
/// order. A Widget in none (Home, the library, the black hole) keeps its Pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    /// the meters and the ambient Visuals
    Instruments,
    /// lyrics, the queue, now playing, the stats and the like
    Side,
}

/// How a Widget is drawn inside its Pane's frame: (buffer, the Pane, inside its frame, app, theme).
pub type Draw = fn(&mut Buffer, Rect, Rect, &mut App, &Theme);

/// How a Widget that draws its own frame draws the whole Pane: (buffer, the Pane, app, theme).
pub type Framed = fn(&mut Buffer, Rect, &mut App, &Theme);

/// One Widget's fixed facts.
#[derive(Clone, Copy)]
pub struct Facts {
    /// its Pane's usual title (a Design may word it differently: `design::title`)
    pub title: &'static str,
    /// the smallest Pane, columns × rows with its frame, it is usable in (`needs`)
    pub smallest: (u16, u16),
    /// the Widgets it can give way to, if any
    pub group: Option<Group>,
    /// whether the Widget picker offers it
    pub offered: bool,
    /// a Required Widget: every Design gives it a Pane; it is moved, never closed or replaced away (D87)
    pub required: bool,
    /// the black hole (one at a time)
    pub hole: bool,
    /// draws its own frame (unless its Pane is too small for it), instead of the plain one
    pub own_frame: Option<Framed>,
    /// how it is drawn inside the frame
    pub draw: Draw,
}

/// The black hole's (the gravity well's and Event Horizon's) smallest size, its shape rule: four rows inside the frame
/// (with three the picture is a bare ring), and as wide as `WELL_ASPECT` (4:3 in pixels, cells twice as tall as wide)
/// over them.
const HOLE: (u16, u16) = {
    let h = 6;
    let rows = (h - 2) as f64;
    ((rows * 2.0 * WELL_ASPECT + 0.5) as u16 + 2, h)
};

/// A Widget that the picker offers, in `group`, with the plain frame and nothing else special.
fn plain(title: &'static str, smallest: (u16, u16), group: Option<Group>, draw: Draw) -> Facts {
    Facts { title, smallest, group, offered: true, required: false, hole: false, own_frame: None, draw }
}

/// Widget `w`'s facts. Every Widget has its arm.
pub fn facts(w: Widget) -> Facts {
    use Group::*;
    use Widget::*;
    // the smallest size of most Widgets that show text
    const TEXT: (u16, u16) = (16, 4);
    // the meters and the ambient Visuals still read in the smallest Pane with two rows inside
    const VISUAL: (u16, u16) = (MIN_W, MIN_H + 1);
    match w {
        Fft => plain("fft", VISUAL, Some(Instruments), |b, _, i, a, t| crate::meters::spectrum(b, i, &a.meters, t)),
        Loudness => plain("loudness", VISUAL, Some(Instruments), |b, _, i, a, t| crate::meters::loudness(b, i, &a.meters, t)),
        Levels => plain("levels", VISUAL, Some(Instruments), |b, _, i, a, t| crate::widgets::levels(b, i, a, t)),
        Scope => plain("osc", VISUAL, Some(Instruments), |b, _, i, a, t| crate::widgets::scope(b, i, a, t)),
        Rain => plain("rain", VISUAL, Some(Instruments), |b, _, i, a, t| crate::widgets::rain(b, i, a, t)),
        Starfield => plain("starfield", VISUAL, Some(Instruments), |b, _, i, a, t| crate::ambient::starfield(b, i, a, t)),
        Lyrics => plain("tail -f lyrics.log", TEXT, Some(Side), |b, _, i, a, t| crate::lyrics::draw(b, i, a, t)),
        // a line has to wrap to be read: in one row only its middle shows
        Karaoke => plain("karaoke", (20, 6), Some(Side), |b, _, i, a, t| crate::karaoke::draw(b, i, a, t)),
        Queue => plain("queue", TEXT, Some(Side), |b, _, i, a, t| crate::queue::draw(b, i, a, t)),
        NowPlaying => plain("Now playing", TEXT, Some(Side), |b, _, i, a, t| crate::widgets::nowplaying(b, i, a, t)),
        // the bitrate under the badges
        Stream => plain("stream", (16, 5), Some(Side), |b, _, i, a, t| crate::setup::draw_stream(b, i, a, t)),
        Top => plain("top · system", TEXT, Some(Side), |b, _, i, a, t| crate::widgets::top(b, i, a, t)),
        Listening => plain("listening · 26 weeks", TEXT, Some(Side), |b, _, i, a, t| {
            crate::widgets::listening(b, i, a, t);
            // a click opens the whole story
            a.hit(i, Hit::Action(Action::Stats));
        }),
        Today => plain("today", TEXT, Some(Side), |b, _, i, a, t| {
            crate::stats::today(b, i, a, t);
            a.hit(i, Hit::Action(Action::Stats));
        }),
        // the artists' names beside their minutes
        TopWeek => plain("top this week", (20, 4), Some(Side), |b, _, i, a, t| crate::stats::top_week(b, i, a, t)),
        Marquee => plain("marquee", TEXT, Some(Side), |b, _, i, a, t| crate::ambient::marquee(b, i, a, t)),
        Glow => plain("glow", TEXT, Some(Side), |b, _, i, a, t| crate::ambient::glow(b, i, a, t)),
        Library => plain("~/library", TEXT, None, |b, _, i, a, t| crate::sidebar::draw(b, i, a, t)),
        // the main Pane, where every page opens: it draws the page it shows. A Card's name beside its cover (Classic
        // gives Home 20 columns in a half-screen laptop window)
        Home => Facts { offered: false, required: true, ..plain("home", (16, 6), None, |b, _, i, a, t| crate::ui::main_page(b, i, a, t)) },
        // the gravity well: its own (sagging) frame
        Well => Facts { hole: true, own_frame: Some(crate::visual::well::draw), ..plain("gravity well", HOLE, None, crate::visual::well::draw_plain) },
        // Event Horizon's plain black hole
        Hole => Facts { hole: true, ..plain("black hole", HOLE, None, crate::visual::well::draw_plain) },
        // another build's Widget: as before Panes had an identity, a "?" and nothing in it (its smallest size the
        // smallest Pane's: it draws nothing anyway)
        Unknown(_) => Facts { offered: false, ..plain("?", (MIN_W, MIN_H), None, |_, _, _, _, _| {}) },
    }
}

/// Draws Widget `w` inside its Pane's frame (`rect` the Pane, `inner` inside the frame): the one place each Widget is
/// drawn, so tests can draw every Widget at any size the way a Pane does.
pub fn draw(buf: &mut Buffer, rect: Rect, inner: Rect, w: Widget, app: &mut App, th: &Theme) {
    (facts(w).draw)(buf, rect, inner, app, th)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every Widget's facts, walked over `Widget::ALL`: the rules between them hold.
    #[test]
    fn every_widget_has_its_facts() {
        for &w in Widget::ALL {
            let f = facts(w);
            assert!(!f.title.is_empty(), "{w}: no title");
            assert!(f.smallest.0 >= MIN_W && f.smallest.1 >= MIN_H, "{w}: {:?} is under any Pane", f.smallest);
            // a Required Widget is never offered (it can't be replaced away), nor does it give way to others
            assert!(!(f.required && (f.offered || f.group.is_some())), "{w}");
            // the black hole keeps its Pane: it is never cycled into one by ◂ ▸
            assert!(!(f.hole && f.group.is_some()), "{w}");
            assert!(f.own_frame.is_none() || f.hole, "{w}: only the gravity well draws its own frame");
            // the picker offers every Widget that isn't Required
            assert_eq!(f.offered, !f.required, "{w}");
        }
        assert_eq!(Widget::ALL.iter().filter(|w| facts(**w).required).collect::<Vec<_>>(), [&Widget::Home], "D87: only Home is Required");
        assert_eq!(Widget::ALL.iter().filter(|w| facts(**w).hole).count(), 2);
        assert!(!facts(Widget::Unknown("x")).offered && !facts(Widget::Unknown("x")).required);
    }
}
