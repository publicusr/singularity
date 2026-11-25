//! The Gesture (D89): the one press or drag in progress, if any. Seeking or a volume drag, a queued song or a list
//! row being dragged, a divider, a Widget carried in Arrange mode: each kind holds its own data, and there is one at a
//! time (`App::gesture`).
//!
//! It lives on the App, not in Arrange mode, though two of its kinds are Arrange mode's: the App's gates read it on
//! every pointer event before Arrange mode sees one (a scrub begun on the player bar stays the bar's, a held Widget
//! keeps the pointer), and ending one must reach both the App's work (a seek, a drop in the queue) and Arrange mode's
//! (a divider's move recorded). Arrange mode is lent it with each input, beside its Scene, and changes only its own
//! kinds.
//!
//! How each ends:
//!
//! | kind | the release | a release never heard (a bare move, a fresh press) | interrupted (a popup, undo, another layout) |
//! |---|---|---|---|
//! | seek, volume | sent at the bar's value | the same, at the last value heard | kept; a release in a popup ends it unsent |
//! | queued song, list row | dropped where shown | let go: nothing moves | kept; a release in a popup ends it unmoved |
//! | divider | the move is one undo step | the same | the same, before the interruption |
//! | carried Widget | dropped where shown | back where it was (one picked up with Enter: a press only) | back where it was |
//!
//! `App::release_unheard` and `App::gesture_interrupted` are the two doors for the last two columns.

use crate::app::App;
use crate::arrange::mode::{DividerDrag, TileDrag};
use crate::openlist::RowId;
use crate::playlists::RowDrag;
use crate::queue::QueueDrag;
use crate::scrub::{Bar, Scrub};
use ratatui::crossterm::event::{MouseButton, MouseEventKind};

/// The one press or drag in progress.
pub enum Gesture {
    /// The seek or the volume bar, pressed (and maybe dragged).
    Scrub(Scrub),
    /// A song in the queue (its Pane or its popup), pressed (and maybe dragged).
    Queue(QueueDrag),
    /// A row of your own playlist, pressed (and maybe dragged).
    Row(RowPress),
    /// A divider between Panes, held.
    Divider(DividerDrag),
    /// A Widget carried in Arrange mode: by its title, from the Tray, or picked up with Enter.
    Carry(TileDrag),
}

/// A row of your own playlist pressed: which, by identity (D81: a row taken out before the drag lands doesn't make it
/// move another), and once the pointer reaches another row, the drag under way.
pub struct RowPress {
    pub id: RowId,
    pub drag: Option<RowDrag>,
}

/// The kind of Gesture held, read from the slot (`Option<Gesture>`): each kind's data, if that is the one held.
pub trait Held {
    fn scrub(&self) -> Option<&Scrub>;
    fn queue(&self) -> Option<&QueueDrag>;
    fn row(&self) -> Option<&RowPress>;
    fn divider(&self) -> Option<&DividerDrag>;
    fn carry(&self) -> Option<&TileDrag>;
    fn carry_mut(&mut self) -> Option<&mut TileDrag>;
    /// The carried Widget, taken out (the slot is empty after); None, and the slot untouched, if none is carried.
    fn take_carry(&mut self) -> Option<TileDrag>;
    /// The held divider, taken out, likewise.
    fn take_divider(&mut self) -> Option<DividerDrag>;
}

impl Held for Option<Gesture> {
    fn scrub(&self) -> Option<&Scrub> {
        match self {
            Some(Gesture::Scrub(s)) => Some(s),
            _ => None,
        }
    }

    fn queue(&self) -> Option<&QueueDrag> {
        match self {
            Some(Gesture::Queue(d)) => Some(d),
            _ => None,
        }
    }

    fn row(&self) -> Option<&RowPress> {
        match self {
            Some(Gesture::Row(r)) => Some(r),
            _ => None,
        }
    }

    fn divider(&self) -> Option<&DividerDrag> {
        match self {
            Some(Gesture::Divider(d)) => Some(d),
            _ => None,
        }
    }

    fn carry(&self) -> Option<&TileDrag> {
        match self {
            Some(Gesture::Carry(t)) => Some(t),
            _ => None,
        }
    }

    fn carry_mut(&mut self) -> Option<&mut TileDrag> {
        match self {
            Some(Gesture::Carry(t)) => Some(t),
            _ => None,
        }
    }

    fn take_carry(&mut self) -> Option<TileDrag> {
        match self.take() {
            Some(Gesture::Carry(t)) => Some(t),
            other => {
                *self = other;
                None
            }
        }
    }

    fn take_divider(&mut self) -> Option<DividerDrag> {
        match self.take() {
            Some(Gesture::Divider(d)) => Some(d),
            other => {
                *self = other;
                None
            }
        }
    }
}

/// What a click was on, for a second one soon after (a double click plays): a list row by identity and where it
/// shows (a page arriving between the clicks doesn't make it another song, D81; nor is the same song further down the
/// list the same row), the others by their place in what they're shown in.
#[derive(Clone, Debug, PartialEq)]
pub enum Clicked {
    Row(RowId, usize),
    StatsRow(usize),
    TopWeek(usize),
    SearchTrack(usize),
    TopTrack(usize),
}

impl App {
    /// Whether a second click on `what` now is a double click; either way it is the last click from now on (a double
    /// click is forgotten, so a third one starts again).
    pub(crate) fn double_click(&mut self, what: Clicked) -> bool {
        let double = self.last_click.as_ref().is_some_and(|(t, c)| *c == what && t.elapsed() < crate::app::DOUBLE_CLICK);
        self.last_click = if double { None } else { Some((std::time::Instant::now(), what)) };
        double
    }

    /// A bare move or a fresh press while a Gesture is held: its release went somewhere else (another window, after
    /// the focus moved) and was never heard (D84, Q91). The one place that ends any Gesture that way, before anything
    /// else sees the event (even one that only wakes the idle UI): seeking and the volume are let go at the last
    /// value heard, as a release there would; a queued song or a list row is let go where it was (nothing moves); a
    /// divider and a carried Widget are Arrange mode's (`Arrange::unheard`).
    pub fn release_unheard(&mut self, kind: MouseEventKind) {
        if !matches!(kind, MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)) {
            return;
        }
        match self.gesture.take() {
            Some(Gesture::Scrub(s)) => match s.bar {
                Bar::Seek => self.seek_to(s.value),
                Bar::Volume => self.set_volume(s.value as u8),
            },
            Some(Gesture::Queue(_) | Gesture::Row(_)) | None => {}
            g @ Some(Gesture::Divider(_) | Gesture::Carry(_)) => {
                self.gesture = g;
                self.arrange_unheard(kind);
            }
        }
    }

    /// Something else happens while a Gesture is held: a popup opens, a layout is undone or redone, another layout
    /// replaces it (Q103). A held divider ends first as a release would (the move so far is one undo step), and a
    /// carried Widget goes back; then the interruption happens. Seeking and a drag in a list are kept (their release
    /// lands in a popup, which ends them without doing anything).
    pub fn gesture_interrupted(&mut self) {
        if self.gesture.divider().is_some() || self.gesture.carry().is_some() {
            self.arrange_interrupted();
        }
    }

    /// The release in a popup of a press begun before it opened: whatever it held ends without doing anything (a
    /// divider still held, the popup having come up some other way, ends as an interruption first: its move is kept).
    pub(crate) fn gesture_dropped(&mut self) {
        self.gesture_interrupted();
        self.gesture = None;
    }
}

#[cfg(test)]
mod tests;
