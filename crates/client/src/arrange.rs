//! Arranging in the running Client (D87, ADR 0005): Arrange mode and the Tray, a title drag that moves a Widget onto
//! another Pane or the Tray, its keys (#43), close and split (a split makes an empty Pane on any of the four sides
//! and opens the Widget picker on it, #39), and what an edit does to the Design on screen. The edits themselves are
//! `Arrangement`'s; layout undo is `history`.
//!
//! Arrange mode is a layer (`App::layers`): the player bar stays (its buttons, seek bar and keys work, so the music
//! goes on through a rearrange), the Tray strip goes under it and the legend and Done take the hints' row. The Panes'
//! area is the Tray's height shorter meanwhile, every Pane scaled by the floor plan's own rule (its proportions, and
//! the saved floor plan, never change for it), and Done gives the room back (Q39). Each Pane's title gets a split and
//! a close button. A title drag starts it (so a quick swap is one gesture: the mode stays on after the drop), as do
//! the palette's and the Pane menu's "Arrange", and Esc in a split's Widget picker. Done (or Esc) ends it, once Home
//! has a Pane again.
//!
//! A drag only moves contents. When a drag starts Arrange mode the Panes make room for the Tray once, as it starts
//! (Q39); after that, while a Widget is held, nothing on screen moves but a highlight on the Pane (or the Tray) under
//! the pointer (at once, no dwell, saying what a release would do) and a chip with the Widget's name by
//! the pointer. The release drops there: on a Pane the two Widgets swap (or an empty Pane is filled); on the Tray the
//! Widget waits there and its Pane is left empty; from the Tray onto a Pane it takes that Pane, whose Widget goes into
//! the Tray in its place. On its own Pane, off the Panes, on Esc, or when the release was never heard, the drag ends
//! and nothing changes. Every drop is allowed: no drop changes the floor plan.
//!
//! "New design…" (#42) starts Arrange mode from one empty Pane filling the screen, with Home waiting in the Tray, whose
//! "+ Widget" opens the Widget picker to put any Widget there. Nothing is written while it is built: Done asks for a
//! name and saves it as a Design of the user's own with the current Theme; Esc before anything is placed goes back to
//! the Design that was on screen, untouched.
//!
//! The keyboard (#43, replacing Move mode) keeps the same pick up → aim → drop shape: arrows move the focus between
//! Panes; Enter picks up the focused Widget (the same lit Pane and chip as the mouse's), arrows aim, Enter drops
//! (swap, or fill an empty Pane) and Esc puts it back. `t` sends the focused Widget to the Tray, `s` then an arrow
//! splits its Pane that way, `x` closes it, `+` (or `=`) `-` move its border left or right of it and `]` `[` the one
//! above or below it, `w` opens "+ Widget"'s picker; Esc (or `m`) with nothing picked up is Done.
//!
//! The Design files (owner, Q33): the name prompt never interrupts an edit. An edit to a built-in Design is kept at
//! once as its one unnamed "<Design> (edited)" (the built-in itself never changes; Reset design goes back): outside
//! Arrange mode the bar says so ("Kept as …"), and in Arrange mode Done asks for a name, if anything changed. A Design
//! of the user's own saves itself. Save design as… names one any time.
//!
//! Undo (owner, 2026-11-01): every committed edit (a drop, close, split, Tray edit, size step, divider
//! release, Reset design) can be undone, back to 50 edits, for the session. Undo restores the Arrangement and which
//! Design it was, and the files follow: a Design of the user's gets the restored tree written to its file (and one
//! that a Reset dropped comes back). Undoing the first edit of a built-in Design returns to the built-in one: the
//! unnamed "<Design> (edited)" that edit made is removed again (no orphan), while a Design the user named stays in the
//! picker as saved (it was a deliberate save, like after Reset design). With Done's name prompt up, undo closes it. A
//! state whose floor plan no longer fits the window is refused with a notice.

pub mod history;

use crate::app::{Action, App, Hit, Menu, MenuAct, Modal, NameFor, Naming, Target};
use crate::arrangement::{Arrangement, Refused};
use crate::layout::{Axis, Divider, Node, Pane, PaneId, Widget, is_hole};
use crate::theme::Theme;
use crate::tiling::{Side, neighbour};
use crate::ui::{blank, text};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use std::time::{Duration, Instant};

/// How long the bar offers "undo" after an edit.
pub const OFFER: Duration = Duration::from_secs(4);

/// The chip by the pointer is at most this wide, and this far right of it (by the screen's right edge it stops there
/// and the pointer goes on over it: it never jumps to the pointer's other side).
pub const CHIP_MAX: u16 = 20;
const CHIP_GAP: u16 = 2;

/// A Widget held by its Pane's title (or picked up from the Tray): where the press was, where the pointer is, and
/// (once it has moved far enough to be a drag rather than a click) what is under the pointer: another Pane, or the
/// Tray: what a release drops on.
pub struct TileDrag {
    /// the Pane picked up (its Widget is what moves); from the Tray, no Pane of the layout's (`PaneId::UNSET`)
    pub pane: Pane,
    from: (u16, u16),
    pub at: (u16, u16),
    pub live: bool,
    /// the Pane under the pointer, other than its own: highlighted, and where a release drops
    pub over: Option<PaneId>,
    /// the pointer is on the Tray (and the Widget comes from a Pane): highlighted, and a release puts it there
    pub tray: bool,
    /// picked up from the Tray
    pub from_tray: bool,
    /// picked up with Enter (#43): what the arrows aim at (its own Pane: nowhere); the pointer has no part in it
    pub aim: Option<Aim>,
    /// picked up with Enter: the layout it was picked up on (any other edit meanwhile ends the pick-up)
    on: Option<Arrangement>,
    /// the row the target's label was drawn on last frame (and its Pane): it stays there while the chip leaves it free
    label: Option<(PaneId, u16)>,
}

/// What a Widget picked up with Enter is aimed at: a Pane, or the Tray (↑ leaves it into the Pane above column `x`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Pane(PaneId),
    Tray(u16),
}

/// The Tray strip's height: the rows the Panes give up while arranging (Q39).
pub const TRAY_H: u16 = 3;

/// Arrange mode, while it is on: the Arrangement when it began and whether an edit was made since, so Done can
/// tell whether anything changed, the first Widget the Tray strip shows (when it holds more than fits), whether
/// `s` waits for the arrow that says which way to split, and the Widget in the Tray the keyboard's focus is on.
pub struct Arranging {
    since: Arrangement,
    /// an edit was made in it (previewing other Designs in the picker isn't one)
    edited: bool,
    pub tray_first: usize,
    pub split_next: bool,
    /// ↓ from a bottom Pane: the focus is on this stop in the Tray (no Pane is focused meanwhile)
    pub tray_focus: Option<TrayStop>,
    /// a New design… being built (#42)
    fresh: Option<Box<Fresh>>,
}

/// Where the keyboard's focus can be in the Tray: on one of its Widgets, or on its "+ Widget", the stop after the
/// last one (there even when the Tray is empty, #42).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayStop {
    Widget(Widget),
    Add,
}

/// A New design… being built (#42): what was on screen before it (Esc before anything is placed puts it back,
/// untouched) and the undo history from before it (the build's own steps don't reach into it). Nothing is written
/// until Done names it.
struct Fresh {
    before: Snapshot,
    history: history::History<Snapshot>,
    /// the Theme on screen when it began (a Theme picked meanwhile goes too when it's thrown away)
    theme: String,
}

/// What undo goes back to: the layout, the Design it was (its file, and what that said about it) and the focus.
#[derive(Clone, Debug)]
pub struct Snapshot {
    tree: Arrangement,
    shown: crate::shown::Mark,
    focus: Option<PaneId>,
}

/// The OSC 22 bytes that give the pointer `shape`; "" puts the ordinary arrow back. Ghostty (1.3) ignores an empty
/// name ("unknown cursor shape") and would keep the resize arrow for good, and `text` gives an I-beam (the owner,
/// 2026-11-05: "can it just be normal"); `default` is the plain arrow in Ghostty, kitty and foot alike.
pub fn pointer_shape_seq(shape: &str) -> String {
    format!("\x1b]22;{}\x1b\\", if shape.is_empty() { "default" } else { shape })
}

/// A border that can be grabbed, lit in the accent colour (only its line glyphs: a title on it keeps its colours).
pub fn light_grip(buf: &mut Buffer, grip: Rect, th: &Theme) {
    for y in grip.top()..grip.bottom() {
        for x in grip.left()..grip.right() {
            if let Some(c) = buf.cell_mut((x, y))
                && c.symbol().chars().next().is_some_and(|ch| ('\u{2500}'..='\u{257f}').contains(&ch))
            {
                c.set_fg(th.accent.color());
            }
        }
    }
}

/// The Pane menu's Split row: each arrow splits that way (`SPLIT_SIDES`, in this order).
pub const SPLIT_ROW: &str = "Split  ←  →  ↑  ↓";
pub const SPLIT_SIDES: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

/// The Pane the focus falls back on when it names none of `layout`'s: Home's, or (Home waiting in the Tray) the first.
pub fn fallback_pane(layout: &Arrangement) -> Option<PaneId> {
    layout.pane_of(Widget::Home).or_else(|| layout.panes().first().map(|p| p.id))
}

/// The side after (`right`) or before `side` on the Split row, round at the ends.
pub fn next_side(side: Side, right: bool) -> Side {
    let i = SPLIT_SIDES.iter().position(|s| *s == side).unwrap_or(0);
    SPLIT_SIDES[(i + if right { 1 } else { 3 }) % 4]
}

impl App {
    pub fn cell_aspect(&self) -> f64 {
        self.cell_px.map(|(w, h)| h / w).unwrap_or(2.0)
    }

    /// Where the Panes are drawn (the last frame's), or a roomy guess before the first.
    pub fn pane_area(&self) -> Rect {
        if self.body.width > 0 { self.body } else { Rect::new(0, 1, 200, 55) }
    }

    /// Why an edit was refused, in the bar's words.
    pub(crate) fn refused(&mut self, e: Refused) {
        let design = self.shown.design();
        self.say(e.why(|w| crate::ui::pane_name(design, w)));
    }

    /// The Pane to act on: the focused one, or (the focus is on none of this layout's) `fallback_pane`.
    pub fn focused_pane(&self) -> Option<PaneId> {
        self.focus.filter(|p| self.layout.contains(*p)).or_else(|| fallback_pane(&self.layout))
    }

    /// What the focused Pane shows (None: nothing is focused, or its Pane is empty).
    pub fn focused(&self) -> Option<Widget> {
        self.focus.and_then(|p| self.layout.widget_in(p))
    }

    /// Whether the focused Pane shows `w`.
    pub fn is_focused(&self, w: Widget) -> bool {
        self.focused() == Some(w)
    }

    /// Focus the Pane showing `w` (no Pane, if none does).
    pub fn focus_on(&mut self, w: Widget) {
        self.focus = self.layout.pane_of(w);
    }

    /// Show `tree` in place of the whole layout (a Design, an undo): the Panes of another tree are other Panes, so the
    /// focus stays on the Widget it was on.
    pub fn set_layout(&mut self, tree: impl Into<Arrangement>) {
        let was = self.focused();
        let tray = self.layout.tray().to_vec();
        self.layout = tree.into();
        self.settle_tray_focus(&tray);
        self.focus = was.and_then(|w| self.layout.pane_of(w));
        if self.arranging.is_some() && self.focus.is_none() {
            self.focus = fallback_pane(&self.layout);
        }
    }

    /// The Pane whose Widget is being moved (held by the mouse, or picked up with Enter): drawn highlighted.
    pub fn moving(&self) -> Option<PaneId> {
        self.tile.as_ref().filter(|t| t.live).map(|t| t.pane.id)
    }

    // ── the mouse ─────────────────────────────────────────────────────

    /// A press on a Pane's title bar: a click unless the pointer moves a few cells before the release.
    pub fn tile_down(&mut self, id: PaneId, x: u16, y: u16) {
        self.focus = Some(id);
        let pane = self.layout.get(id).unwrap_or(Pane { id, widget: None });
        self.tile = Some(TileDrag { pane, from: (x, y), at: (x, y), live: false, over: None, tray: false, from_tray: false, aim: None, on: None, label: None });
    }

    /// A press on Widget `w` in the Tray: a drag takes it onto a Pane.
    fn tray_down(&mut self, w: Widget, x: u16, y: u16) {
        if !self.layout.tray().contains(&w) {
            return;
        }
        let pane = Pane { id: PaneId::UNSET, widget: Some(w) };
        self.tile = Some(TileDrag { pane, from: (x, y), at: (x, y), live: false, over: None, tray: false, from_tray: true, aim: None, on: None, label: None });
    }

    /// Where the Tray strip is in Arrange mode: under the player bar, which is under the Panes (Q39).
    pub fn tray_rect(&self) -> Rect {
        self.rows(true)[3]
    }

    /// Where the player bar is: under the Panes.
    pub fn player_rect(&self) -> Rect {
        self.rows(self.arranging.is_some())[2]
    }

    /// The screen's rows (`ui::rows`) in this window, arranging or not; before the first frame, around a roomy guess.
    fn rows(&self, arranging: bool) -> [Rect; 5] {
        let w = if self.window.width > 0 { self.window } else { Rect::new(0, 0, 200, 60) };
        crate::ui::rows(w, arranging)
    }

    /// The Panes' area again, for the window's size and whether Arrange mode is on (it takes the Tray's rows), at once:
    /// a release in the same batch of input finds the Pane under it where the next frame draws it.
    fn relayout(&mut self) {
        if self.window.width > 0 {
            self.body = crate::ui::rows(self.window, self.arranging.is_some())[1];
        }
        self.tile_resized();
    }

    /// What a release at (x, y) would drop on, for a Widget held from Pane `own` (or the Tray): the Pane under it
    /// other than its own, or (for a Pane's Widget) the Tray.
    fn aim(&self, x: u16, y: u16, own: PaneId, from_tray: bool) -> (Option<PaneId>, bool) {
        let t = self.tray_rect();
        if self.arranging.is_some() && x >= t.x && x < t.right() && y >= t.y && y < t.bottom() {
            return (None, !from_tray);
        }
        (self.pane_under(x, y).filter(|p| *p != own), false)
    }

    /// The Pane under (x, y), in the layout as it is drawn.
    pub fn pane_under(&self, x: u16, y: u16) -> Option<PaneId> {
        let area = self.pane_area();
        self.layout.compute(area).panes.into_iter().find(|(_, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()).map(|(p, _)| p.id)
    }

    /// Esc while a Widget is held: nothing changes.
    pub fn tile_cancel(&mut self) {
        self.tile = None;
    }

    /// Mouse events while a Widget is held: only the left button's (another button pressed meanwhile doesn't drop it).
    /// One picked up with Enter is the keyboard's: the pointer doesn't aim or drop it.
    pub fn tile_pointer(&mut self, m: MouseEvent) {
        if self.tile.as_ref().is_some_and(|t| t.aim.is_some()) {
            return;
        }
        match m.kind {
            MouseEventKind::Drag(MouseButton::Left) => self.tile_move(m.column, m.row),
            MouseEventKind::Up(MouseButton::Left) => self.tile_up(m.column, m.row),
            _ => {}
        }
    }

    /// A bare move or a fresh press while a Widget or a divider is held: its release went somewhere else (another
    /// window, after the focus moved) and was never heard (#21: the Pane stayed held and swallowed every click). A held
    /// Widget stays where it was; a divider is let go where it is. (A Widget picked up with Enter was never the
    /// pointer's: a bare move leaves it be, a press puts it back.)
    pub fn release_unheard(&mut self, kind: MouseEventKind) {
        if !matches!(kind, MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)) {
            return;
        }
        if kind == MouseEventKind::Moved && self.tile.as_ref().is_some_and(|t| t.aim.is_some()) {
            return;
        }
        self.tile_cancel();
        if matches!(self.drag, Some(Hit::Divider(_))) {
            self.drag = None;
            self.divider_released();
        }
    }

    /// The pointer at (x, y) with the button down: past a few cells it is a drag (an empty Pane has nothing to carry),
    /// which turns Arrange mode on, and what is under it is the one highlighted, at once.
    fn tile_move(&mut self, x: u16, y: u16) {
        let Some(t) = self.tile.as_ref() else { return };
        if !t.live && (x.abs_diff(t.from.0) < 3 && y.abs_diff(t.from.1) < 2 || t.pane.widget.is_none()) {
            return;
        }
        let (own, from_tray) = (t.pane.id, t.from_tray);
        // on while the Widget is held, and after the drop: a quick swap stays one gesture (D87)
        self.enter_arrange();
        let (over, tray) = self.aim(x, y, own, from_tray);
        if let Some(t) = self.tile.as_mut() {
            t.live = true;
            t.at = (x, y);
            t.over = over;
            t.tray = tray;
        }
    }

    /// The terminal took a new size (`w` × `h`): the Panes' area is known at once, so a release in the same batch of
    /// input finds the Pane under it in the new window, not the old (#40 review).
    pub fn window_resized(&mut self, w: u16, h: u16) {
        self.window = Rect::new(0, 0, w, h);
        self.mini = crate::mini::wanted(self.window);
        self.relayout();
    }

    /// The window changed size while a Widget is held: the Panes moved under the pointer, so the one under it is found
    /// again (#21: a release dropped on a Pane of the old window).
    pub fn tile_resized(&mut self) {
        let Some((own, from_tray, (x, y))) = self.tile.as_ref().filter(|t| t.live && t.aim.is_none()).map(|t| (t.pane.id, t.from_tray, t.at)) else { return };
        let (over, tray) = self.aim(x, y, own, from_tray);
        if let Some(t) = self.tile.as_mut() {
            t.over = over;
            t.tray = tray;
        }
    }

    /// Let go at (x, y): on another Pane the two Widgets swap (an empty one is filled); on the Tray the Widget waits
    /// there; from the Tray onto a Pane it takes the Pane (its Widget goes into the Tray). Anywhere else nothing changes.
    fn tile_up(&mut self, x: u16, y: u16) {
        let Some(t) = self.tile.as_ref() else { return };
        if !t.live {
            let (id, from, w, from_tray) = (t.pane.id, t.from, t.pane.widget, t.from_tray);
            self.tile = None;
            // a click on a Widget in the Tray does nothing
            if from_tray {
                return;
            }
            // a click on the Widget's name: the Widget picker (D87)
            if self.hit_at(from.0, from.1) == Some(Hit::TitleName(id)) {
                return self.open_widget_picker(id);
            }
            // a click elsewhere on the title: the focus moved at the press; the hole's Pane also changes its look
            if is_hole(w) {
                self.run(Action::NextLook);
            }
            return;
        }
        // the release's own cell is the last word (a finger slips as it lifts; moves can come combined)
        self.tile_move(x, y);
        let Some(t) = self.tile.take() else { return };
        let edit = match (t.from_tray, t.pane.widget, t.over) {
            (true, Some(w), Some(to)) => self.layout.out_of_tray(w, to).map(|a| (a, Some(to))),
            (false, _, _) if t.tray => self.layout.to_tray(t.pane.id).map(|a| (a, None)),
            (false, _, Some(to)) => self.layout.swap(t.pane.id, to).map(|a| (a, Some(to))),
            _ => return,
        };
        match edit {
            Ok((a, to)) => {
                if to.is_some() {
                    self.focus = to;
                }
                self.commit_layout(a);
            }
            Err(e) => self.refused(e),
        }
    }

    /// While a Widget is held over another Pane: that Pane, what its label says ("swap with queue", "put here"), and
    /// the chip's text ("⠿ stream"). Over its own Pane or none: the chip alone.
    pub fn drag_marks(&self) -> Option<(Option<(PaneId, String)>, String)> {
        let t = self.tile.as_ref().filter(|t| t.live)?;
        let w = t.pane.widget?;
        let design = self.shown.design();
        let target = t.over.map(|o| {
            let label = match self.layout.widget_in(o) {
                Some(x) => format!("swap with {}", crate::ui::pane_name(design, x)),
                None => "put here".to_string(),
            };
            (o, label)
        });
        Some((target, format!("⠿ {}", crate::ui::pane_name(design, w))))
    }

    /// The chip by the pointer for `text`: one row, at most `CHIP_MAX` wide, on the pointer's row and a fixed step right
    /// of it, inside `area` (where the screen ends it stops, rather than jumping to the pointer's left: a small move
    /// there flipped it back and forth). Picked up with Enter: in the Pane aimed at, under its middle (where a target's
    /// label goes); aimed at the Tray, in the Tray's row (never on its frame): a Pane's Widget after what waits there
    /// (where it would land), one picked up from the Tray on its own place there.
    pub fn chip_rect(&self, area: Rect, text: &str) -> Option<Rect> {
        let t = self.tile.as_ref().filter(|t| t.live)?;
        let w = (text.chars().count() as u16 + 2).min(CHIP_MAX).min(area.width);
        if let Some(Aim::Tray(x)) = t.aim {
            let r = self.tray_rect();
            let (x0, x1) = (r.x + 2, r.right().saturating_sub(2));
            let w = w.min(x1.saturating_sub(x0));
            let first = self.arranging.as_ref().map_or(0, |a| a.tray_first);
            let l = tray_layout(self, r, first);
            let at = match (t.from_tray, t.pane.widget) {
                // over its own chip in the strip (the column it was picked up at, if that has scrolled away)
                (true, Some(me)) => {
                    let i = self.layout.tray().iter().position(|w| *w == me);
                    l.items.iter().find(|(j, ..)| Some(*j) == i).map_or(x.saturating_sub(w / 2), |(_, c, _)| c.x)
                }
                // after its last Widget shown (or at its start, empty)
                _ => l.items.last().map_or(x0, |(_, c, close)| close.unwrap_or(*c).right() + 1),
            };
            return Some(Rect::new(at.min(x1.saturating_sub(w)).max(x0), r.y + 1, w, 1));
        }
        if let Some(Aim::Pane(aim)) = t.aim {
            let r = self.layout.compute(self.pane_area()).rect(aim)?;
            let w = w.min(r.width);
            let y = if r.height >= 4 { r.y + r.height / 2 + 1 } else { r.y + r.height / 2 };
            return Some(Rect::new(r.x + (r.width - w) / 2, y.min(r.bottom().saturating_sub(1)), w, 1));
        }
        let (x, y) = t.at;
        let x = x.saturating_add(CHIP_GAP).min(area.right().saturating_sub(w)).max(area.x);
        Some(Rect::new(x, y.clamp(area.y, area.bottom().saturating_sub(1)), w, 1))
    }

    /// Where the label of target `id` (at `r`) goes this frame, clear of the chip, and remembered: it stays on its row
    /// while the chip leaves that free, so a pointer moving a row up and down doesn't make it hop round the chip.
    pub fn place_label(&mut self, id: PaneId, r: Rect, label: &str, chip: Option<Rect>) -> Option<Rect> {
        let l = label_rect(r, label, chip, self.label_row(id));
        if let Some(t) = self.tile.as_mut() {
            t.label = l.map(|l| (id, l.y));
        }
        l
    }

    /// The row target `id`'s label was drawn on last.
    pub fn label_row(&self, id: PaneId) -> Option<u16> {
        self.tile.as_ref()?.label.filter(|(p, _)| *p == id).map(|(_, y)| y)
    }

    /// The divider to light up: the one being dragged, else the one under the pointer.
    pub fn grip_lit(&self) -> Option<usize> {
        match &self.drag {
            Some(Hit::Divider(i)) => Some(*i),
            _ if self.tile.is_none() && matches!(self.modal(), None | Some(Modal::Arrange)) => self.grip_hover,
            _ => None,
        }
    }

    /// The pointer left the window or the layout changed under it (focus lost, a resize): no border is under it
    /// until the next mouse event says so.
    pub fn pointer_gone(&mut self) {
        self.grip_hover = None;
    }

    /// The mouse pointer's shape for the terminal (OSC 22: kitty, Ghostty, foot…; others ignore it): a resize arrow
    /// over a border that can be dragged, else the terminal's own ("").
    pub fn pointer_shape(&self) -> &'static str {
        match self.grip_lit().and_then(|i| self.dividers.get(i)) {
            Some(d) if d.axis == crate::layout::Axis::Horizontal => "ew-resize",
            Some(_) => "ns-resize",
            None => "",
        }
    }

    /// A divider dragged to `pos`, or as far toward it as it goes without leaving a Pane too small (or the well out of
    /// shape): a fast drag past that stops at the limit, not short of it (#21).
    pub fn drag_divider(&mut self, d: &Divider, pos: u16) {
        let area = self.pane_area();
        self.layout = self.layout.move_line(d, pos, area);
    }

    /// A divider let go: a layout that changed is an edit.
    pub fn divider_released(&mut self) {
        if let Some(before) = self.divider_from.take().filter(|b| *b != self.layout) {
            let mut was = self.snapshot();
            was.tree = before;
            self.remember(was);
            self.edited();
        }
    }

    // ── edits and what they do to the Design ──────────────────────────

    /// Show `tree` as the layout and treat it as an edit (one undo step).
    /// `tree` replaces the layout as the rest of the last edit (a split's pick): one undo takes both back.
    pub(crate) fn amend_layout(&mut self, tree: Arrangement) {
        let tray = self.layout.tray().to_vec();
        self.layout = tree;
        self.settle_tray_focus(&tray);
        self.undo_offer = Some((Instant::now(), Action::UndoLayout));
        self.want_widget_data();
        self.edited();
    }

    /// `tree` replaces the layout as an edit (one undo step).
    pub(crate) fn commit_layout(&mut self, tree: Arrangement) {
        self.commit_kept(tree);
    }

    /// `commit_layout`, and what `edited` says of it: Some, the built-in Design's copy it was just kept as.
    fn commit_kept(&mut self, tree: Arrangement) -> Option<String> {
        let was = self.snapshot();
        let tray = self.layout.tray().to_vec();
        self.layout = tree;
        self.settle_tray_focus(&tray);
        self.remember(was);
        self.want_widget_data();
        self.edited()
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { tree: self.layout.clone(), shown: self.shown.mark(), focus: self.focus }
    }

    /// An edit replaced `was`: undo can go back to it, and the bar offers that for a few seconds.
    fn remember(&mut self, was: Snapshot) {
        self.history.push(was);
        self.undo_offer = Some((Instant::now(), Action::UndoLayout));
    }

    // ── undo ──────────────────────────────────────────────────────────

    pub fn undo_layout(&mut self) {
        self.step_history(true);
    }

    pub fn redo_layout(&mut self) {
        self.step_history(false);
    }

    fn step_history(&mut self, back: bool) {
        // a held Widget is let go first (one picked up with Enter: Ctrl+Z gets here; the mouse and the bar never do
        // mid-drag, but be sure)
        self.tile = None;
        let area = self.pane_area();
        let now = self.snapshot();
        let fits = |s: &Snapshot| s.tree.fits(area);
        let got = if back { self.history.undo(now, fits) } else { self.history.redo(now, fits) };
        let (any, what) = if back { (self.history.can_undo(), "undo") } else { (self.history.can_redo(), "redo") };
        match got {
            Some(s) => {
                self.toast = None;
                self.restore(s);
                self.undo_offer = Some((Instant::now(), if back { Action::RedoLayout } else { Action::UndoLayout }));
                if self.toast.is_none() {
                    let key = self.keymap.label(if back { Action::RedoLayout } else { Action::UndoLayout });
                    let again = if key.is_empty() { String::new() } else { format!(" · {key} {}", if back { "redoes" } else { "undoes" }) };
                    self.say(format!("{}{again}", if back { "Undone" } else { "Redone" }));
                }
            }
            None if any => self.say(format!("Can't {what}: that layout doesn't fit this window")),
            None => self.say(format!("Nothing to {what}")),
        }
    }

    /// Put `s` back on screen, and its Design's file with it.
    fn restore(&mut self, s: Snapshot) {
        // Done's name prompt: the changes it would name are going
        if self.naming().is_some_and(|n| n.purpose == NameFor::Fork) {
            self.close_if_open(crate::app::Modal::Naming);
        }
        self.set_layout(s.tree);
        self.focus = s.focus.filter(|p| self.layout.contains(*p)).or_else(|| fallback_pane(&self.layout));
        self.want_widget_data();
        let o = self.shown.restore(s.shown, &self.layout, &self.theme.id);
        self.shown_did(o);
    }

    /// The layout was edited. A Design of the user's own (or the unnamed edit of a built-in one) follows in its file;
    /// an edit of a built-in Design is kept at once as its "<Design> (edited)", never asking (Q33): outside Arrange
    /// mode the bar says so, and in Arrange mode Done asks for a name. Some: the edit was just kept as that copy
    /// ("Singularity (edited)") outside Arrange mode.
    pub fn edited(&mut self) -> Option<String> {
        if let Some(a) = self.arranging.as_mut() {
            a.edited = true;
            a.split_next = false;
            // a New design… is written when Done names it
            if a.fresh.is_some() {
                return None;
            }
        }
        let mut o = self.shown.edited(&self.layout, &self.theme.id, self.arranging.is_some());
        let kept = o.kept_as.take();
        self.shown_did(o);
        kept
    }

    // ── Arrange mode ──────────────────────────────────────────────────

    /// Arrange mode on (the palette's and the Pane menu's "Arrange", a title drag, Esc in a split's picker).
    pub fn enter_arrange(&mut self) {
        if self.arranging.is_none() {
            self.arranging = Some(Arranging { since: self.layout.clone(), edited: false, tray_first: 0, split_next: false, tray_focus: None, fresh: None });
            self.relayout();
        }
    }

    /// "New design…" (#42): Arrange mode from one empty Pane filling the screen, with Home waiting in the Tray. The
    /// Design on screen stays as it is (in its file too) until Done names the new one; Esc before anything is placed
    /// goes back to it.
    pub fn new_design(&mut self) {
        if self.building() {
            return;
        }
        let before = self.snapshot();
        let history = std::mem::take(&mut self.history);
        // (from Arrange mode: what was arranged is kept as it is, and the build starts from the blank one)
        self.arranging = None;
        self.tile = None;
        self.undo_offer = None;
        self.layout = Arrangement::new(Node::empty(), vec![]);
        self.shown.build_new();
        self.focus = self.layout.panes().first().map(|p| p.id);
        self.want_widget_data();
        self.enter_arrange();
        if let Some(a) = self.arranging.as_mut() {
            a.fresh = Some(Box::new(Fresh { before, history, theme: self.theme.id.clone() }));
        }
    }

    /// Whether a New design… is being built.
    pub fn building(&self) -> bool {
        self.arranging.as_ref().is_some_and(|a| a.fresh.is_some())
    }

    /// The saved Design that was on screen when a New design… began (config.toml keeps naming it meanwhile).
    pub fn building_from(&self) -> Option<String> {
        self.arranging.as_ref()?.fresh.as_ref()?.before.shown.file().map(String::from)
    }

    /// Esc on a New design… before anything is placed, or Discard: the Design that was on screen, as it was, with its
    /// Theme and its undo history.
    pub fn cancel_new_design(&mut self) {
        let Some(f) = self.arranging.take().and_then(|a| a.fresh) else { return };
        let Fresh { before, history, theme } = *f;
        self.tile = None;
        self.undo_offer = None;
        self.layout = before.tree;
        self.shown.resume(before.shown);
        self.focus = before.focus.filter(|p| self.layout.contains(*p)).or_else(|| self.layout.pane_of(Widget::Home));
        self.history = history;
        if theme != self.theme.id && self.set_theme(&theme) {
            self.save_ui();
        }
        self.want_widget_data();
        self.relayout();
    }

    /// The name typed on Done's prompt for a New design…: saved as a Design of your own with the current Theme, and
    /// Arrange mode ends. The build's undo steps go with it: the new Design starts its own history. False if it
    /// couldn't be saved (a name taken, one that makes no file name, a write error: the bar says why): it is still
    /// being built.
    pub fn save_new_design(&mut self, name: &str) -> bool {
        let saved = self.shown.save_new(name, &self.layout, &self.theme.id);
        let ok = saved.is_ok();
        self.shown_did(saved.unwrap_or_else(|o| o));
        if !ok {
            return false;
        }
        self.arranging = None;
        self.tile = None;
        self.history = Default::default();
        self.undo_offer = None;
        self.relayout();
        true
    }

    /// The Tray's "+ Widget": the Widget picker, adding the pick to the Tray (#42).
    pub fn open_tray_picker(&mut self) {
        self.open_popup(crate::app::Popup::WidgetPicker(crate::panemenu::WidgetPicker { target: PaneId::UNSET, query: String::new(), sel: 0, after_split: false, tray: true }));
    }

    /// Before each frame: Home always has a Pane outside Arrange mode. A Design whose Home waits in the Tray (one left
    /// that way when the app quit) comes up in Arrange mode, where Done waits for it.
    pub fn arrange_if_homeless(&mut self) {
        // (not while the Design picker previews one: a preview isn't a Design you have)
        if self.arranging.is_none() && self.picker().is_none() && self.layout.unplaced().is_some() {
            self.enter_arrange();
        }
    }

    /// What keeps Done unavailable: "Home still needs a Pane", while it waits in the Tray.
    pub fn done_blocked(&self) -> Option<String> {
        let w = self.layout.unplaced()?;
        let name = crate::ui::pane_name(self.shown.design(), w);
        let mut c = name.chars();
        let name = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
        Some(format!("{name} still needs a Pane"))
    }

    /// Esc with nothing picked up: Done, but on a New design… with nothing placed yet, back to the Design that was on
    /// screen, untouched (#42).
    fn arrange_esc(&mut self) {
        if self.arranging.as_ref().is_some_and(|a| a.fresh.is_some() && a.since == self.layout) {
            return self.cancel_new_design();
        }
        self.arrange_done()
    }

    /// Done (or Esc): Arrange mode ends, once Home has a Pane. If anything changed in it and the changes went to a
    /// built-in Design's "<Design> (edited)", the name prompt comes now (Esc keeps it unnamed).
    pub fn arrange_done(&mut self) {
        if self.done_blocked().is_some() {
            return;
        }
        // a New design…: Done asks for its name, and Arrange mode stays on until it has one (#42)
        if self.building() {
            if self.naming().is_none() {
                self.open_popup(crate::app::Popup::Naming(Naming::new_design()));
            }
            return;
        }
        let Some(a) = self.arranging.take() else { return };
        self.tile = None;
        self.relayout();
        let changed = a.edited && a.since != self.layout;
        if changed && self.naming().is_none() && let Some((name, problems)) = self.shown.done_prompt() {
            if let Some(p) = problems {
                self.say(p);
            }
            self.open_popup(crate::app::Popup::Naming(Naming::fork(name)));
        }
    }

    /// Keys in Arrange mode (the shortcuts with Ctrl or Alt work as everywhere; no Pane takes keys): arrows move the
    /// focus between Panes, Enter picks up the focused Widget, `t` sends it to the Tray, `s` then an arrow splits its
    /// Pane that way, `x` closes it, `+` (or `=`) `-` move its border left or right of it, `]` `[` the one above or
    /// below it, `w` opens "+ Widget"'s picker, Esc (or `m`, the key that came in, Q70) is Done. With a Widget picked
    /// up, arrows aim, Enter drops and Esc (or `m`) puts it back. On a Widget in the Tray, `x` (or Delete, Backspace)
    /// removes it. The playback keys these don't take work as ever (the
    /// player bar stays).
    pub fn arrange_key(&mut self, k: KeyEvent) {
        use ratatui::crossterm::event::KeyModifiers;
        // a divider held by the mouse: the release ends it (and records it); no arrange key meanwhile
        if self.drag.is_some() {
            return;
        }
        // the key that came into Arrange mode leaves it: it does what Esc does (Q70)
        let k = if self.keymap.action(k) == Some(Action::Arrange) { KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE) } else { k };
        self.pick_up_check();
        // (a focus on a Widget that left the Tray, or on no Pane of this layout: the keys act from a real Pane)
        let tray = self.layout.tray().to_vec();
        let add = self.tray_add_x().is_some();
        if let Some(a) = self.arranging.as_mut()
            && a.tray_focus.is_some_and(|s| match s {
                TrayStop::Widget(w) => !tray.contains(&w),
                TrayStop::Add => !add,
            })
        {
            a.tray_focus = None;
        }
        let on_tray = self.arranging.as_ref().and_then(|a| a.tray_focus);
        if on_tray.is_none() && !self.focus.is_some_and(|p| self.layout.contains(p)) {
            self.focus = fallback_pane(&self.layout);
        }
        // only the bare keys (`+` may come with the Shift it needs); Shift+arrows seek, as everywhere
        let bare = k.modifiers.is_empty() || k.code == KeyCode::Char('+') && k.modifiers == KeyModifiers::SHIFT;
        let arrow = match k.code {
            _ if !bare => None,
            KeyCode::Left => Some(Side::Left),
            KeyCode::Right => Some(Side::Right),
            KeyCode::Up => Some(Side::Top),
            KeyCode::Down => Some(Side::Bottom),
            _ => None,
        };
        // (Delete and Backspace only with the focus on a Widget in the Tray: they remove it, as `x` does there)
        let on_widget = matches!(on_tray, Some(TrayStop::Widget(_)));
        let arrange_key = bare
            && (arrow.is_some()
                || matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('t' | 's' | 'x' | '+' | '=' | '-' | ']' | '[' | 'w'))
                || on_widget && matches!(k.code, KeyCode::Delete | KeyCode::Backspace));
        if !arrange_key {
            return self.playback_key(k);
        }
        if self.tile.is_some() {
            return match (k.code, arrow) {
                (_, Some(side)) => self.aim_step(side),
                (KeyCode::Enter, _) => self.key_drop(),
                (KeyCode::Esc, _) => self.tile_cancel(),
                _ if !arrange_key => self.playback_key(k),
                _ => {}
            };
        }
        // `s` waits for the arrow saying which way; any other key ends the wait and does nothing else
        if self.arranging.as_mut().is_some_and(|a| std::mem::take(&mut a.split_next)) {
            if let Some(side) = arrow {
                self.split_focused(side);
            }
            return;
        }
        // the focus in the Tray: ← → move along it, ↑ goes back up into the Panes, Enter picks a Widget up (or, on
        // "+ Widget", opens the Widget picker for the Tray), `x` (or Delete, Backspace) removes the Widget, as its `×`
        if let Some(stop) = on_tray {
            return match (k.code, arrow) {
                (_, Some(side @ (Side::Left | Side::Right))) => self.tray_focus_step(stop, side == Side::Right),
                (_, Some(Side::Top)) => self.tray_focus_up(stop),
                (KeyCode::Enter, _) => match stop {
                    TrayStop::Widget(w) => self.pick_up_from_tray(w),
                    TrayStop::Add => self.open_tray_picker(),
                },
                (KeyCode::Char('x') | KeyCode::Delete | KeyCode::Backspace, _) => match stop {
                    TrayStop::Widget(w) => self.tray_remove_focused(w),
                    TrayStop::Add => {}
                },
                (KeyCode::Char('w'), _) => self.open_tray_picker(),
                (KeyCode::Esc, _) => self.arrange_esc(),
                // (the other keys for a Pane have none here)
                _ => {}
            };
        }
        let focused = self.focused_pane();
        match (k.code, arrow, focused) {
            (_, Some(side), _) => self.focus_step(side),
            (KeyCode::Esc, ..) => self.arrange_esc(),
            (KeyCode::Enter, _, Some(id)) => self.pick_up(id),
            (KeyCode::Char('t'), _, Some(id)) => match self.layout.to_tray(id) {
                Ok(a) => self.commit_layout(a),
                Err(e) => self.refused(e),
            },
            (KeyCode::Char('s'), _, Some(_)) => {
                if let Some(a) = self.arranging.as_mut() {
                    a.split_next = true;
                }
            }
            (KeyCode::Char('x'), _, Some(id)) => self.close_pane(id),
            (KeyCode::Char('+' | '='), _, Some(id)) => self.resize_step(id, true, Axis::Horizontal),
            (KeyCode::Char('-'), _, Some(id)) => self.resize_step(id, false, Axis::Horizontal),
            (KeyCode::Char(']'), _, Some(id)) => self.resize_step(id, true, Axis::Vertical),
            (KeyCode::Char('['), _, Some(id)) => self.resize_step(id, false, Axis::Vertical),
            (KeyCode::Char('w'), ..) => self.open_tray_picker(),
            _ if !arrange_key => self.playback_key(k),
            _ => {}
        }
    }

    /// The playback keys, which work while arranging (the player bar stays, the Tray under it).
    fn playback_key(&mut self, k: KeyEvent) {
        use Action::*;
        if let Some(a) = self.keymap.action(k).filter(|a| matches!(a, TogglePlay | Next | Previous | SeekForward | SeekBack | SeekForwardFine | SeekBackFine | VolumeUp | VolumeDown | Shuffle | Repeat | Like)) {
            self.run(a);
        }
    }

    /// The Tray strip scrolled `by` Widgets (the wheel, its ‹ ›).
    pub fn tray_scroll(&mut self, by: i32) {
        let Some(now) = self.arranging.as_ref().map(|a| a.tray_first) else { return };
        let want = (now as i64 + by as i64).max(0) as usize;
        // no further than the last start that still fills the strip
        let first = tray_layout(self, self.tray_rect(), want).first;
        if let Some(a) = self.arranging.as_mut() {
            a.tray_first = first;
        }
    }

    /// The `×` of Widget `w` in the Tray: it is removed (Home never; one already gone: nothing).
    fn tray_remove(&mut self, w: Widget) {
        if !self.layout.tray().contains(&w) {
            return;
        }
        match self.layout.remove_from_tray(w) {
            Ok(a) => self.commit_layout(a),
            Err(e) => self.refused(e),
        }
    }

    /// `x` (or Delete, Backspace) on Widget `w` focused in the Tray: removed as by its `×` (Home is refused the same
    /// way); the focus goes to the next Widget in the Tray, else the one before, else back up to the Panes.
    fn tray_remove_focused(&mut self, w: Widget) {
        let x = self.tray_chip_x(w);
        self.tray_remove(w);
        // (the edit moved the focus on: `settle_tray_focus`; back in the Panes, onto the one above the chip)
        if !self.layout.tray().contains(&w) && self.arranging.as_ref().is_some_and(|a| a.tray_focus.is_none()) {
            self.focus = self.pane_above_tray(x).or_else(|| fallback_pane(&self.layout));
        }
    }

    /// After the layout changed (an edit, an undo or redo, the mouse's `×` in the Tray): a keyboard focus on a Widget
    /// that has left the Tray (`old` was the Tray before) moves to the next Widget still there, else the one before,
    /// else back to the Panes.
    fn settle_tray_focus(&mut self, old: &[Widget]) {
        let Some(TrayStop::Widget(w)) = self.arranging.as_ref().and_then(|a| a.tray_focus) else { return };
        let tray = self.layout.tray().to_vec();
        if tray.contains(&w) {
            return;
        }
        let i = old.iter().position(|t| *t == w).unwrap_or(old.len());
        let after = old.iter().skip(i + 1);
        let before = old.iter().take(i).rev();
        let next = after.chain(before).find(|t| tray.contains(t)).copied();
        if let Some(a) = self.arranging.as_mut() {
            a.tray_focus = next.map(TrayStop::Widget);
        }
        match next {
            Some(n) => {
                self.tray_chip_x(n);
            }
            None => {
                if !self.focus.is_some_and(|p| self.layout.contains(p)) {
                    self.focus = fallback_pane(&self.layout);
                }
            }
        }
    }

    /// Clear Tray: what's in it is removed, but Home.
    fn clear_tray(&mut self) {
        let a = self.layout.clear_tray();
        if a != self.layout {
            self.commit_layout(a);
        }
    }

    /// A mouse event in Arrange mode (on top of everything but popups): title drags, dividers, a Pane's split and
    /// close buttons, the Tray and Done; a right click opens the Pane's menu; the wheel scrolls the Tray. Nothing else
    /// in the Panes takes a click (a click focuses the Pane).
    pub(crate) fn arrange_pointer(&mut self, m: MouseEvent, coarse: bool) {
        // a press ends `s`'s wait for its arrow, and the keyboard's focus in the Tray
        if let (MouseEventKind::Down(_), Some(a)) = (m.kind, self.arranging.as_mut()) {
            a.split_next = false;
            a.tray_focus = None;
        }
        if self.tile.is_some() {
            return self.tile_pointer(m);
        }
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => match self.hit_at(x, y) {
                Some(Hit::Title(p) | Hit::TitleName(p)) => self.tile_down(p, x, y),
                Some(Hit::Close(p)) => self.close_pane(p),
                Some(Hit::SplitPane(p)) => {
                    let side = self.default_split(p);
                    self.split_pane(p, side);
                }
                Some(Hit::Divider(i)) => self.grab_divider(i, x, y),
                Some(Hit::TrayItem(w)) => self.tray_down(w, x, y),
                Some(Hit::TrayRemove(w)) => self.tray_remove(w),
                Some(Hit::ClearTray) => self.clear_tray(),
                Some(Hit::TrayAdd) => self.open_tray_picker(),
                Some(Hit::TrayScroll(d)) => self.tray_scroll(d),
                Some(Hit::ArrangeDone) => self.arrange_done(),
                Some(Hit::Action(a @ (Action::UndoLayout | Action::RedoLayout))) => self.run(a),
                _ => {
                    if let Some(p) = self.pane_under(x, y) {
                        self.focus = Some(p);
                    }
                }
            },
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(p) = self.pane_under(x, y) {
                    self.open_pane_menu(p, Some((x, y)));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(Hit::Divider(i)) = self.drag.clone()
                    && let Some(d) = self.dividers.get(i).cloned()
                {
                    let pos = if d.axis == crate::layout::Axis::Vertical { y } else { x };
                    self.drag_divider(&d, (pos as i32 + self.drag_offset).max(0) as u16);
                }
            }
            MouseEventKind::Up(_) => {
                if self.drag.take().is_some() {
                    self.divider_released();
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let rows = if m.kind == MouseEventKind::ScrollUp { -1 } else { 1 };
                self.modal_scroll(Modal::Arrange, rows * if coarse { 10 } else { 1 });
            }
            _ => {}
        }
    }

    /// A press on divider `i`'s grip at (x, y): it follows the pointer until the release.
    pub(crate) fn grab_divider(&mut self, i: usize, x: u16, y: u16) {
        self.divider_from = Some(self.layout.clone());
        if let Some(d) = self.dividers.get(i) {
            let (boundary, grab) = match d.axis {
                crate::layout::Axis::Horizontal => (d.grip.x + 1, x),
                crate::layout::Axis::Vertical => (d.grip.y + 1, y),
            };
            self.drag_offset = boundary as i32 - grab as i32;
        }
        self.drag = Some(Hit::Divider(i));
    }

    /// Esc on the naming prompt: keep the edit as the one unnamed "<Design> (edited)", replacing the last.
    pub fn keep_edited(&mut self) {
        let o = self.shown.keep_edited(&self.layout, &self.theme.id);
        self.shown_did(o);
    }

    /// "Reset design": the built-in Design's own layout again. A Design of the user's stays in the picker as it was
    /// last saved (its file isn't touched); the unnamed edit of a built-in one is dropped.
    pub fn reset_design(&mut self) {
        let before = self.snapshot();
        let theme = self.theme.id.clone();
        let o = self.shown.reset();
        self.set_design(self.shown.design());
        self.set_theme(&theme);
        self.shown_did(o);
        if before.tree != self.layout || before.shown.file().is_some() {
            self.remember(before);
        }
    }

    // ── close and split ───────────────────────────────────────────────

    pub fn close_pane(&mut self, id: PaneId) {
        let Some(pane) = self.layout.get(id) else { return };
        let name = self.name_of(pane);
        let area = self.pane_area();
        let tree = match self.layout.close(id, area) {
            Ok(t) => t,
            Err(Refused::Required(_)) => return self.say(format!("{name} can be moved, but never closed")),
            Err(Refused::NoRoom | Refused::LastPane) => return self.say(format!("Closing {name} would squash the others")),
            Err(e) => return self.refused(e),
        };
        if self.focus == Some(id) {
            self.focus = fallback_pane(&tree);
        }
        // (the bar's one line: what the edit did, and where a built-in Design's edit went)
        match self.commit_kept(tree) {
            Some(copy) => self.say(format!("Closed {name} · kept as {copy}")),
            None => self.say(format!("Closed {name}")),
        }
    }

    /// What Pane `p` is called on screen (its Widget's title; an empty Pane is "an empty pane").
    pub fn name_of(&self, p: Pane) -> String {
        p.widget.map_or_else(|| "an empty pane".into(), |w| crate::ui::pane_name(self.shown.design(), w))
    }

    pub fn split_focused(&mut self, side: Side) {
        if let Some(target) = self.focused_pane() {
            self.split_pane(target, side);
        }
    }

    /// A new empty Pane on `side` of Pane `target`, taking half its room, and the Widget picker on it at once (Esc
    /// leaves it empty). Refused with the reason where it would leave a Pane under the minimum.
    pub fn split_pane(&mut self, target: PaneId, side: Side) {
        let Some(pane) = self.layout.get(target) else { return };
        let area = self.pane_area();
        let tree = match self.layout.split(target, side, None, area) {
            Ok(t) => t,
            Err(Refused::NoRoom) => return self.say(format!("No room for a new Pane {} {}", side.word_beside(), self.name_of(pane))),
            Err(e) => return self.refused(e),
        };
        let Some(new) = tree.panes().into_iter().map(|p| p.id).find(|id| !self.layout.contains(*id)) else { return };
        self.commit_layout(tree);
        self.focus = Some(new);
        self.open_widget_picker(new);
        if let Some(p) = self.widget_picker_mut() {
            p.after_split = true;
        }
    }

    /// The side a split of Pane `id` takes unless told otherwise: along its longer side in real proportions (wider than
    /// tall: to the right, else below), or the other way if only that one has room.
    pub fn default_split(&self, id: PaneId) -> Side {
        let (area, aspect) = (self.pane_area(), self.cell_aspect());
        let rect = self.layout.compute(area).panes.into_iter().find(|(p, _)| p.id == id).map(|(_, r)| r);
        let wide = rect.is_none_or(|r| r.width as f64 >= r.height as f64 * aspect);
        let (long, other) = if wide { (Side::Right, Side::Bottom) } else { (Side::Bottom, Side::Right) };
        let fits = |side| self.layout.split(id, side, None, area).is_ok();
        if !fits(long) && fits(other) { other } else { long }
    }

    /// An arrow of the Pane menu's Split row.
    pub fn menu_split(&mut self, side: Side) {
        if let Some(Menu { target: Target::Pane(id), .. }) = self.take_menu() {
            self.split_pane(id, side);
        }
    }

    /// The Pane's menu (D87): Replace widget… first (not on Home's), then the floor plan's edits.
    pub fn open_pane_menu(&mut self, id: PaneId, at: Option<(u16, u16)>) {
        let mut items = vec![];
        if self.can_replace(id) {
            items.push(("Replace widget…".to_string(), MenuAct::ReplaceWidget));
        }
        items.push((SPLIT_ROW.to_string(), MenuAct::Split(self.default_split(id))));
        if !crate::arrangement::required(self.layout.widget_in(id)) {
            items.push(("Close pane".into(), MenuAct::ClosePane));
        }
        if self.arranging.is_none() {
            items.push(("Arrange".into(), MenuAct::Arrange));
        }
        self.focus = Some(id);
        self.open_popup(crate::app::Popup::Menu(Menu { at: at.unwrap_or((0, 0)), target: Target::Pane(id), items, selected: 0 }));
    }

    pub fn pane_menu_run(&mut self, id: PaneId, act: MenuAct) {
        self.focus = Some(id);
        match act {
            MenuAct::ReplaceWidget => self.open_widget_picker(id),
            MenuAct::Split(side) => self.split_pane(id, side),
            MenuAct::Arrange => self.enter_arrange(),
            MenuAct::ClosePane => self.close_pane(id),
            _ => {}
        }
    }

    // ── the keyboard (#43) ────────────────────────────────────────────

    /// An arrow: the focus goes to the Pane next to the focused one that way (none there: it stays).
    /// ↓ from a Pane with none below it: the focus goes onto the Tray's Widget nearest under it (an empty Tray: its
    /// "+ Widget").
    fn focus_step(&mut self, dir: Side) {
        let Some(id) = self.focused_pane() else { return };
        let panes = self.layout.compute(self.pane_area()).panes;
        match neighbour(&panes, id, dir) {
            Some(n) => self.focus = Some(n),
            None if dir == Side::Bottom => {
                let mid = panes.iter().find(|(p, _)| p.id == id).map_or(0, |(_, r)| r.x + r.width / 2);
                let stop = self.tray_nearest(mid).map(TrayStop::Widget).or(self.tray_add_x().map(|_| TrayStop::Add));
                if let Some(stop) = stop {
                    self.focus = None;
                    if let Some(a) = self.arranging.as_mut() {
                        a.tray_focus = Some(stop);
                    }
                }
            }
            None => {}
        }
    }

    /// The Widget in the Tray whose chip (as the strip shows it now) is nearest column `x`.
    fn tray_nearest(&self, x: u16) -> Option<Widget> {
        let first = self.arranging.as_ref().map_or(0, |a| a.tray_first);
        let tray = self.layout.tray();
        let l = tray_layout(self, self.tray_rect(), first);
        l.items.iter().min_by_key(|(_, r, _)| (r.x + r.width / 2).abs_diff(x)).map(|(i, ..)| tray[*i]).or_else(|| tray.first().copied())
    }

    /// The middle column of Widget `w`'s chip in the Tray strip, scrolling the strip to it first if it isn't shown.
    fn tray_chip_x(&mut self, w: Widget) -> u16 {
        let Some(i) = self.layout.tray().iter().position(|x| *x == w) else { return self.tray_rect().x };
        let first = self.arranging.as_ref().map_or(0, |a| a.tray_first);
        let mut l = tray_layout(self, self.tray_rect(), first);
        if !l.items.iter().any(|(j, ..)| *j == i) {
            l = tray_layout(self, self.tray_rect(), if i < first { i } else { i.saturating_sub(l.items.len().saturating_sub(1)) });
            if let Some(a) = self.arranging.as_mut() {
                a.tray_first = l.first;
            }
        }
        l.items.iter().find(|(j, ..)| *j == i).map_or(self.tray_rect().x, |(_, r, _)| r.x + r.width / 2)
    }

    /// The middle column of the Tray's "+ Widget" (None: the strip has no room for it).
    fn tray_add_x(&self) -> Option<u16> {
        let first = self.arranging.as_ref().map_or(0, |a| a.tray_first);
        tray_layout(self, self.tray_rect(), first).add.map(|r| r.x + r.width / 2)
    }

    /// ← or → with the focus in the Tray: the stop before or after (none there: it stays), shown. "+ Widget" is the
    /// stop after the last Widget.
    fn tray_focus_step(&mut self, stop: TrayStop, right: bool) {
        let tray = self.layout.tray().to_vec();
        let mut stops: Vec<TrayStop> = tray.into_iter().map(TrayStop::Widget).collect();
        if self.tray_add_x().is_some() {
            stops.push(TrayStop::Add);
        }
        let Some(i) = stops.iter().position(|s| *s == stop) else { return };
        let Some(&n) = (if right { stops.get(i + 1) } else { i.checked_sub(1).and_then(|j| stops.get(j)) }) else { return };
        if let Some(a) = self.arranging.as_mut() {
            a.tray_focus = Some(n);
        }
        if let TrayStop::Widget(w) = n {
            self.tray_chip_x(w);
        }
    }

    /// ↑ from the Tray: the focus goes back to the Pane above the stop (a Widget's chip, or "+ Widget").
    fn tray_focus_up(&mut self, stop: TrayStop) {
        let x = match stop {
            TrayStop::Widget(w) => self.tray_chip_x(w),
            TrayStop::Add => self.tray_add_x().unwrap_or(self.tray_rect().x),
        };
        let up = self.pane_above_tray(x);
        if let Some(a) = self.arranging.as_mut() {
            a.tray_focus = None;
        }
        self.focus = up.or_else(|| fallback_pane(&self.layout));
    }

    /// The bottom Pane over column `x` (the nearest one where none is).
    fn pane_above_tray(&self, x: u16) -> Option<PaneId> {
        let area = self.pane_area();
        let panes = self.layout.compute(area).panes;
        let bottom = panes.iter().filter(|(_, r)| r.bottom() == area.bottom());
        bottom.min_by_key(|(_, r)| if x >= r.x && x < r.right() { 0 } else { x.abs_diff(r.x + r.width / 2) as u32 + 1 }).map(|(p, _)| p.id)
    }

    /// Enter on Widget `w` in the Tray: it is picked up (the chip), aimed at the Tray (where it is) until ↑ aims into
    /// the Panes.
    fn pick_up_from_tray(&mut self, w: Widget) {
        let x = self.tray_chip_x(w);
        let pane = Pane { id: PaneId::UNSET, widget: Some(w) };
        let on = Some(self.layout.clone());
        self.tile = Some(TileDrag { pane, from: (0, 0), at: (0, 0), live: true, over: None, tray: false, from_tray: true, aim: Some(Aim::Tray(x)), on, label: None });
    }

    /// Enter on Pane `id`: its Widget is picked up (lit, with the chip), aimed at its own Pane until an arrow says
    /// otherwise. An empty Pane has nothing to pick up.
    fn pick_up(&mut self, id: PaneId) {
        let Some(pane) = self.layout.get(id).filter(|p| p.widget.is_some()) else { return };
        let on = Some(self.layout.clone());
        self.tile = Some(TileDrag { pane, from: (0, 0), at: (0, 0), live: true, over: None, tray: false, from_tray: false, aim: Some(Aim::Pane(id)), on, label: None });
    }

    /// An arrow with a Widget picked up: the aim goes to the next Pane that way, lit with what Enter would do (back on
    /// its own Pane: nothing lit). ↓ from a bottom Pane aims at the Tray (lit for a Pane's Widget, as for the mouse);
    /// ↑ from the Tray aims at the Pane above.
    fn aim_step(&mut self, dir: Side) {
        let Some((own, from_tray, at)) = self.tile.as_ref().and_then(|t| Some((t.pane.id, t.from_tray, t.aim?))) else { return };
        let area = self.pane_area();
        let panes = self.layout.compute(area).panes;
        let to = match at {
            Aim::Pane(p) => match neighbour(&panes, p, dir) {
                Some(n) => Aim::Pane(n),
                None if dir == Side::Bottom => Aim::Tray(panes.iter().find(|(q, _)| q.id == p).map_or(area.x, |(_, r)| r.x + r.width / 2)),
                None => return,
            },
            Aim::Tray(x) if dir == Side::Top => match self.pane_above_tray(x) {
                Some(n) => Aim::Pane(n),
                None => return,
            },
            Aim::Tray(_) => return,
        };
        if let Some(t) = self.tile.as_mut() {
            t.aim = Some(to);
            t.over = match to {
                Aim::Pane(n) => (n != own).then_some(n),
                Aim::Tray(_) => None,
            };
            t.tray = matches!(to, Aim::Tray(_)) && !from_tray;
        }
        // a Pane's Widget aimed at the Tray lands after its last Widget: the strip shows its end, with room for the
        // chip there
        if matches!(to, Aim::Tray(_)) && !from_tray {
            self.tray_to_end();
        }
    }

    /// A Widget picked up with Enter is put back once anything else edits the layout (Ctrl+K Close pane, Replace
    /// widget…, a Design change…): its Pane, or what that Pane holds, may be gone.
    pub fn pick_up_check(&mut self) {
        if self.tile.as_ref().is_some_and(|t| t.on.as_ref().is_some_and(|on| *on != self.layout)) {
            self.tile = None;
        }
    }

    /// Enter with a Widget picked up: it swaps with the Widget of the Pane aimed at (an empty one is filled), and the
    /// focus goes with it; from the Tray, it takes the Pane (its Widget goes into the Tray in its place). A Pane's
    /// Widget aimed at the Tray waits there, its Pane left empty. Aimed where it came from: it is put back.
    fn key_drop(&mut self) {
        let Some(t) = self.tile.take() else { return };
        let edit = match (t.from_tray, t.pane.widget, t.aim, t.over) {
            (true, Some(w), _, Some(to)) => self.layout.out_of_tray(w, to).map(|a| (a, Some(to))),
            (false, _, Some(Aim::Tray(_)), _) => self.layout.to_tray(t.pane.id).map(|a| (a, None)),
            (false, _, _, Some(to)) => self.layout.swap(t.pane.id, to).map(|a| (a, Some(to))),
            _ => return,
        };
        match edit {
            Ok((a, to)) => {
                if to.is_some() {
                    self.focus = to;
                    if let Some(a) = self.arranging.as_mut() {
                        a.tray_focus = None;
                    }
                }
                self.commit_layout(a);
                // set aside: the strip shows where it went
                if to.is_none() {
                    self.tray_to_end();
                }
            }
            Err(e) => self.refused(e),
        }
    }

    /// The Tray strip scrolled to its end (as far as it goes).
    fn tray_to_end(&mut self) {
        let end = tray_layout(self, self.tray_rect(), usize::MAX).first;
        if let Some(a) = self.arranging.as_mut() {
            a.tray_first = end;
        }
    }

    /// `+` `-` (`axis` Horizontal: its border left or right of it) or `]` `[` (Vertical: above or below it): the
    /// focused Pane a step bigger or smaller that way.
    fn resize_step(&mut self, id: PaneId, grow: bool, axis: Axis) {
        let area = self.pane_area();
        match self.layout.step_line(id, grow, area, axis) {
            Ok(tree) => self.commit_layout(tree),
            Err(_) => self.say(if grow { "Can't grow it more" } else { "Can't shrink it more" }),
        }
    }
}

// ── drawing ───────────────────────────────────────────────────────────

/// The Pane a release would drop on: its frame lit in the accent, and a label saying what happens at `at` (from
/// `App::place_label`: in its middle, clear of the chip).
pub fn draw_target(buf: &mut Buffer, r: Rect, label: &str, at: Option<Rect>, th: &Theme) {
    let lit = th.s(th.accent).add_modifier(Modifier::BOLD);
    let (right, bottom) = (r.right().saturating_sub(1), r.bottom().saturating_sub(1));
    let edge = (r.x..r.right()).flat_map(|x| [(x, r.y), (x, bottom)]).chain((r.y..r.bottom()).flat_map(|y| [(r.x, y), (right, y)]));
    for (x, y) in edge {
        if let Some(c) = buf.cell_mut((x, y))
            && c.symbol().chars().next().is_some_and(|ch| ('\u{2500}'..='\u{257f}').contains(&ch))
        {
            c.set_style(lit);
        }
    }
    if let Some(l) = at {
        text(buf, l.x, l.y, l.width, &fit(label, l.width), th.chip(th.accent));
    }
}

/// Where a target's label goes: centred in it, cut to fit inside its frame, on row `stay` (where it was last frame)
/// while the chip (`chip`) leaves that free, else on the middle row if free, else on the row nearest the middle at
/// least two rows from the chip (a move of one row doesn't reach it again), else the nearest free one.
pub fn label_rect(r: Rect, label: &str, chip: Option<Rect>, stay: Option<u16>) -> Option<Rect> {
    let w = (label.chars().count() as u16 + 2).min(r.width.saturating_sub(2));
    if r.height < 3 || w < 3 {
        return None;
    }
    let mid = r.y + r.height / 2;
    let at = |y: u16| Rect::new(r.x + (r.width - w) / 2, y, w, 1);
    let free = |l: &Rect| chip.is_none_or(|c| !c.intersects(*l));
    let inside = |y: u16| y > r.y && y < r.bottom() - 1;
    if let Some(l) = stay.filter(|&y| inside(y)).or(Some(mid)).map(at).filter(free) {
        return Some(l);
    }
    let rows = || (r.y + 1..r.bottom() - 1).map(at).filter(free);
    let clear = |l: &Rect| chip.is_none_or(|c| l.y.abs_diff(c.y) >= 2);
    rows().filter(clear).min_by_key(|l| l.y.abs_diff(mid)).or_else(|| rows().min_by_key(|l| l.y.abs_diff(mid))).or(Some(at(mid)))
}

/// ` text ` in `w` columns: cut with an ellipsis if it is longer.
fn fit(s: &str, w: u16) -> String {
    let padded = format!(" {s} ");
    if padded.chars().count() <= w as usize {
        return padded;
    }
    format!(" {}…", s.chars().take((w as usize).saturating_sub(2)).collect::<String>())
}

/// The chip by the pointer: the held Widget's name with its grip.
pub fn draw_chip(buf: &mut Buffer, r: Rect, label: &str, th: &Theme) {
    blank(buf, r.x, r.y, r.width, th.chip(th.accent2));
    text(buf, r.x, r.y, r.width, &fit(label, r.width), th.chip(th.accent2));
}

/// Where the parts of the Tray strip go: each Widget shown (its place in the Tray, its chip, its × unless it is
/// Home), the ‹ › while they don't all fit, "+ Widget", Clear Tray, and the first Widget shown.
pub struct TrayLayout {
    pub items: Vec<(usize, Rect, Option<Rect>)>,
    pub prev: Option<Rect>,
    pub next: Option<Rect>,
    pub add: Option<Rect>,
    pub clear: Option<Rect>,
    pub first: usize,
}

/// The Tray's title on its frame, and its "+ Widget" button beside it (#42).
const TRAY_TITLE: &str = " Tray ";
const TRAY_ADD: &str = " + Widget ";

/// A Widget's chip in the Tray: its grip and name.
fn tray_chip(name: &str) -> String {
    format!(" ⠿ {name} ")
}

/// The Tray strip at `r` (three rows: its frame round one row of Widgets), from Widget `first` on, or as near it as
/// still fills the row: when they don't all fit, ‹ › at the ends scroll it.
pub fn tray_layout(app: &App, r: Rect, first: usize) -> TrayLayout {
    let tray = app.layout.tray();
    let y = r.y + 1;
    let (x0, x1) = (r.x + 2, r.right().saturating_sub(2));
    let width = |w: Widget| tray_chip(&crate::ui::pane_name(app.shown.design(), w)).chars().count() as u16 + if crate::arrangement::required(Some(w)) { 0 } else { 2 };
    let widths: Vec<u16> = tray.iter().map(|w| width(*w)).collect();
    // a Pane's Widget aimed here with the keys: room for its chip after the last one, where it lands
    let reserve = app.tile.as_ref().filter(|t| t.live && t.tray && t.aim.is_some()).and_then(|t| t.pane.widget).map_or(0, |w| {
        (crate::ui::pane_name(app.shown.design(), w).chars().count() as u16 + 4).min(CHIP_MAX) + 1
    });
    let all: u16 = widths.iter().map(|w| w + 1).sum::<u16>() + reserve;
    let over = all > x1.saturating_sub(x0) + 1;
    let (from, to) = if over { (x0 + 2, x1.saturating_sub(2)) } else { (x0, x1) };
    // the last place to start from that still fills the row
    let mut last = tray.len().saturating_sub(1);
    let mut used = reserve;
    for (i, w) in widths.iter().enumerate().rev() {
        used += w + 1;
        if used > to.saturating_sub(from) + 1 {
            break;
        }
        last = i;
    }
    let first = if over { first.min(last) } else { 0 };
    let mut items = vec![];
    let mut x = from;
    for (i, w) in tray.iter().enumerate().skip(first) {
        let cw = width(*w);
        if x + cw > to {
            break;
        }
        let chip = tray_chip(&crate::ui::pane_name(app.shown.design(), *w)).chars().count() as u16;
        let close = (!crate::arrangement::required(Some(*w))).then(|| Rect::new(x + chip, y, 2, 1));
        items.push((i, Rect::new(x, y, chip, 1), close));
        x += cw + 1;
    }
    let shown_to = items.last().map_or(first, |(i, ..)| *i + 1);
    let prev = (over && first > 0).then(|| Rect::new(x0, y, 1, 1));
    let next = (over && shown_to < tray.len()).then(|| Rect::new(x1.saturating_sub(1), y, 1, 1));
    let clear_label = " Clear Tray ";
    let clear = tray.iter().any(|w| !crate::arrangement::required(Some(*w))).then(|| Rect::new(r.right().saturating_sub(clear_label.chars().count() as u16 + 2), r.y, clear_label.chars().count() as u16, 1));
    // "+ Widget" on the frame after the title (where it fits before Clear Tray)
    let add = Rect::new(r.x + 1 + TRAY_TITLE.chars().count() as u16 + 1, r.y, TRAY_ADD.chars().count() as u16, 1);
    let add = (add.right() < clear.map_or(r.right().saturating_sub(1), |c| c.x)).then_some(add);
    TrayLayout { items, prev, next, add, clear, first }
}

/// The Tray strip (Arrange mode, under the player bar): what waits there, each with its ×, Clear Tray, and while a
/// Pane's Widget is held over it, lit with what a release does.
pub fn draw_tray(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    use ratatui::widgets::{Block, Borders, Widget as _};
    if r.height < 3 || r.width < 20 {
        return;
    }
    let lit = app.tile.as_ref().is_some_and(|t| t.live && t.tray);
    let border = if lit { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.border) };
    Block::default().borders(Borders::ALL).border_set(th.borders).border_style(border).title(ratatui::text::Span::styled(TRAY_TITLE, th.s(th.accent))).style(th.base()).render(r, buf);
    let first = app.arranging.as_ref().map_or(0, |a| a.tray_first);
    let l = tray_layout(app, r, first);
    if let Some(a) = app.arranging.as_mut() {
        a.tray_first = l.first;
    }
    let tray = app.layout.tray().to_vec();
    // (a Widget aimed here with the keys shows where it lands, in its place)
    let landing = app.tile.as_ref().is_some_and(|t| t.live && t.tray && t.aim.is_some());
    if tray.is_empty() && !landing {
        let s = "drop a Widget here to set it aside";
        text(buf, r.x + 2, r.y + 1, r.width.saturating_sub(4), s, th.s(th.dim));
    }
    let focus = app.arranging.as_ref().and_then(|a| a.tray_focus);
    let focus_on = |w: Widget| focus == Some(TrayStop::Widget(w));
    for (i, chip, close) in &l.items {
        let name = crate::ui::pane_name(app.shown.design(), tray[*i]);
        // (the one the keyboard's focus is on: in the focus colour)
        let st = if focus_on(tray[*i]) { th.chip(th.accent) } else { th.chip(th.accent2) };
        text(buf, chip.x, chip.y, chip.width, &tray_chip(&name), st);
        app.hit(*chip, Hit::TrayItem(tray[*i]));
        if let Some(c) = close {
            text(buf, c.x, c.y, c.width, "× ", th.s(th.dim));
            app.hit(Rect::new(c.x, c.y, 1, 1), Hit::TrayRemove(tray[*i]));
        }
    }
    for (a, glyph, d) in [(l.prev, "‹", -1), (l.next, "›", 1)] {
        if let Some(a) = a {
            text(buf, a.x, a.y, 1, glyph, th.s(th.accent));
            app.hit(a, Hit::TrayScroll(d));
        }
    }
    if let Some(a) = l.add {
        // (the keyboard's focus on it: in the focus colour, like a Widget's chip)
        let st = if focus == Some(TrayStop::Add) { th.chip(th.accent) } else { th.s(th.dim) };
        text(buf, a.x, a.y, a.width, TRAY_ADD, st);
        app.hit(a, Hit::TrayAdd);
    }
    if let Some(c) = l.clear {
        text(buf, c.x, c.y, c.width, " Clear Tray ", th.s(th.dim));
        app.hit(c, Hit::ClearTray);
    }
    if lit {
        let label = " set aside here ";
        let w = label.chars().count() as u16;
        text(buf, r.x + r.width.saturating_sub(w) / 2, r.bottom() - 1, w, label, th.chip(th.accent));
    }
}

/// Arrange mode's line at the bottom: its keys and gestures, then Done (unavailable, saying why, while Home waits in
/// the Tray).
pub fn draw_legend(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    blank(buf, r.x, r.y, r.width, th.base());
    let tag = " ARRANGE ";
    let tw = tag.chars().count() as u16;
    text(buf, r.x + 1, r.y, tw, tag, th.chip(th.accent));
    let done = " Done ";
    let dw = done.chars().count() as u16;
    let dx = r.right().saturating_sub(dw + 1);
    let blocked = app.done_blocked();
    let mut end = dx;
    match &blocked {
        Some(why) => {
            let ww = why.chars().count() as u16;
            let wx = dx.saturating_sub(ww + 2);
            text(buf, wx, r.y, ww, why, th.s(th.accent2));
            text(buf, dx, r.y, dw, done, th.s(th.dim));
            end = wx;
        }
        None => text(buf, dx, r.y, dw, done, th.chip(th.accent)),
    }
    app.hit(Rect::new(dx, r.y, dw, 1), Hit::ArrangeDone);
    let key = |a| app.keymap.label(a);
    // the keys of the moment: a Widget picked up, `s` waiting for its arrow, or the lot
    let held = app.tile.as_ref().is_some_and(|t| t.aim.is_some());
    // each with how long it stays when the line is short: `w` goes first, then the mouse's gesture, then undo / redo,
    // then the keys (the arrows and Enter last)
    let mut parts: Vec<(u8, String)> = if held {
        vec![(9, "←↑↓→ aim".into()), (9, "Enter drop".into()), (9, "Esc put back".into())]
    } else if app.arranging.as_ref().is_some_and(|a| a.split_next) {
        vec![(9, "←↑↓→ split that way".into())]
    } else {
        // (the focus on a Widget in the Tray: `x` removes it instead of closing a Pane)
        let on_widget = app.arranging.as_ref().is_some_and(|a| matches!(a.tray_focus, Some(TrayStop::Widget(w)) if app.layout.tray().contains(&w)));
        let x = if on_widget { "x remove" } else { "× x close" };
        let mut parts = vec![(1, "drag a title onto a Pane or the Tray".to_string()), (8, "←↑↓→ Enter pick up".into()), (6, "t Tray".into()), (5, "⊞ s+arrow split".into()), (5, x.into()), (4, "+−[]size".into()), (0, "w + Widget".into())];
        for (a, what) in [(Action::UndoLayout, "undo"), (Action::RedoLayout, "redo")] {
            let k = key(a);
            if !k.is_empty() {
                parts.push((2, format!("{k} {what}")));
            }
        }
        parts.push((7, "Esc done".into()));
        parts
    };
    let x = r.x + tw + 2;
    let room = end.saturating_sub(x + 1) as usize;
    // as many as fit, whole, in their order; the first to go is the one that stays the shortest
    let join = |parts: &[(u8, String)]| parts.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join(" · ");
    let mut line = join(&parts);
    while line.chars().count() > room && parts.len() > 1 {
        let least = parts.iter().enumerate().min_by_key(|(_, (keep, _))| *keep).map(|(i, _)| i).unwrap_or(0);
        parts.remove(least);
        line = join(&parts);
    }
    if line.chars().count() <= room {
        text(buf, x, r.y, room as u16, &line, th.s(th.dim));
    }
}

/// For a few seconds after a layout edit, the bar's right end offers to undo it (after an undo, to redo it).
pub fn draw_undo_chip(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let Some((at, a)) = app.undo_offer else { return };
    let can = if a == Action::UndoLayout { app.history.can_undo() } else { app.history.can_redo() };
    if at.elapsed() > OFFER || !can || r.width < 40 {
        return;
    }
    let key = app.keymap.label(a);
    let label = format!(" {} {} {key} ", if a == Action::UndoLayout { "↶ undo" } else { "↷ redo" }, if key.is_empty() { "" } else { "·" });
    let w = label.chars().count() as u16;
    let x = r.right().saturating_sub(w + 1);
    blank(buf, x.saturating_sub(1), r.y, w + 2, th.base());
    text(buf, x, r.y, w, &label, th.chip(th.accent));
    app.hit(Rect::new(x, r.y, w, 1), Hit::Action(a));
}

impl Side {
    /// "right of", "below"… for a title.
    pub fn word_beside(self) -> &'static str {
        match self {
            Side::Left => "left of",
            Side::Right => "right of",
            Side::Top => "above",
            Side::Bottom => "below",
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod paths;
#[cfg(test)]
mod legend;
