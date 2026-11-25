//! Arrange mode's state and what input does to it (D89, #102), like the Soloist session (D76): keys and pointer
//! moves come in together with what the module needs to see of the screen (a [`Scene`]: the layout, the focus, the
//! window and the Panes' area, the dividers drawn, the Design's names, the keymap), and what happened comes out as
//! [`Out`]s, in the order the App carries them out (an edit committed, a line for the bar, the Widget picker, Done's
//! name prompt…). The layout changes through one door: [`Out::Commit`] for an edit (one undo step, already
//! remembered here), [`Out::Show`] for a layout shown without being one (a divider on its way, a New design… begun).
//! Design-file effects are the Shown Design's: the App runs them for an `Out::Commit`.
//!
//! The module holds Arrange mode itself (`on`: since when, whether anything was edited, the Tray's scroll and focus,
//! `s` waiting for its arrow, a New design… being built), the Widget held (by the mouse or picked up with Enter), the
//! divider held, and layout undo. Its behaviour is described in `arrange.rs`.

use super::history::History;
use crate::app::{Action, Hit};
use crate::arrangement::{Arrangement, Refused};
use crate::keymap::Keymap;
use crate::layout::{Axis, Divider, Node, Pane, PaneId, Widget};
use crate::tiling::{Side, neighbour};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::borrow::Cow;
use std::time::Instant;

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
    pub(crate) on: Option<Arrangement>,
    /// the row the target's label was drawn on last frame (and its Pane): it stays there while the chip leaves it free
    label: Option<(PaneId, u16)>,
}

/// What a Widget picked up with Enter is aimed at: a Pane, or the Tray (↑ leaves it into the Pane above column `x`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Pane(PaneId),
    Tray(u16),
}

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
    history: History<Snapshot>,
    /// the Theme on screen when it began (a Theme picked meanwhile goes too when it's thrown away)
    theme: String,
}

/// What undo goes back to: the layout, the Design it was (its file, and what that said about it) and the focus.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub(crate) tree: Arrangement,
    pub(crate) shown: crate::shown::Mark,
    pub(crate) focus: Option<PaneId>,
}

/// A divider held by the mouse: which (in the last frame's `dividers`), the boundary minus the grab position (so it
/// moves exactly with the pointer whichever grip cell was grabbed), and the layout when it was grabbed (a changed
/// one is an edit on release).
pub struct DividerDrag {
    pub i: usize,
    offset: i32,
    from: Arrangement,
}

/// What Arrange mode sees of the App with each input.
pub struct Scene<'a> {
    pub layout: &'a Arrangement,
    pub focus: Option<PaneId>,
    /// the whole window as last drawn or resized to (0 wide before the first frame)
    pub window: Rect,
    /// where the Panes were last laid out (0 wide before the first frame)
    pub body: Rect,
    /// the dividers of the last frame
    pub dividers: &'a [Divider],
    /// a cell's height over its width
    pub aspect: f64,
    /// the built-in Design shown (its Widgets' names)
    pub design: usize,
    pub keymap: &'a Keymap,
    /// the Shown Design's mark, for undo
    pub mark: crate::shown::Mark,
}

/// What happened, for the App to carry out, in order.
#[derive(Debug)]
pub enum Out {
    /// An edit: one undo step (remembered already). `tree` replaces the layout (None: the one on screen is the edit, a
    /// divider let go); the Design's file follows (`App::edited`); `note` is the bar's line ("Closed queue"), with
    /// where a built-in Design's edit was kept.
    Commit { tree: Option<Arrangement>, note: Option<String> },
    /// The layout shown, not an edit (a divider on its way; the blank of a New design…).
    Show(Arrangement),
    Focus(Option<PaneId>),
    Say(String),
    /// An action of the App's (playback keys, undo and redo from the bar, the hole's next look).
    Run(Action),
    OpenWidgetPicker { pane: PaneId, after_split: bool },
    /// "+ Widget": the Widget picker for the Tray.
    OpenTrayPicker,
    OpenPaneMenu { pane: PaneId, at: (u16, u16) },
    /// A press on a title let go without a drag, at the press's cell: the Widget's name opens the Widget picker; the
    /// hole's Pane changes its look.
    TitleClick { pane: PaneId, at: (u16, u16), widget: Option<Widget> },
    /// Where the Panes are laid out now (Arrange mode on or off takes or gives back the Tray's rows; a new window).
    Body(Rect),
    /// Undo or redo: this goes back on screen (its Design's file with it); `said` is the bar's line unless the
    /// restore said something.
    Restore { snap: Snapshot, said: String },
    /// Done on a New design…: its name prompt (unless one is open).
    AskNewName,
    /// Done after changes: the name prompt for the "<Design> (edited)" they went to (unless one is open).
    AskForkName,
    /// A New design… begins: the Shown Design starts a blank one (the layout came with `Show`).
    BuildNew,
    /// A New design… thrown away: the Design that was on screen comes back as it was, with its Theme.
    Resume { snap: Snapshot, theme: String },
}

/// Layout undo, the Widget held, the divider held and Arrange mode itself.
#[derive(Default)]
pub struct Arrange {
    pub(crate) on: Option<Arranging>,
    pub(crate) tile: Option<TileDrag>,
    pub(crate) divider: Option<DividerDrag>,
    pub(crate) history: History<Snapshot>,
    pub(crate) undo_offer: Option<(Instant, Action)>,
}

/// Where the Panes are laid out (`body`, the last frame's), or a roomy guess before the first frame.
pub fn pane_area(body: Rect) -> Rect {
    if body.width > 0 { body } else { Rect::new(0, 1, 200, 55) }
}

/// The screen's rows (`ui::rows`) in `window`, arranging or not; before the first frame, around a roomy guess.
pub fn screen_rows(window: Rect, arranging: bool) -> [Rect; 5] {
    let w = if window.width > 0 { window } else { Rect::new(0, 0, 200, 60) };
    crate::ui::rows(w, arranging)
}

/// A cell's height over its width (2 until the terminal says).
pub fn aspect(cell_px: Option<(f64, f64)>) -> f64 {
    cell_px.map(|(w, h)| h / w).unwrap_or(2.0)
}

/// The Pane to act on: the focused one, or (the focus is on none of `layout`'s) `fallback_pane`.
pub fn focused_pane(layout: &Arrangement, focus: Option<PaneId>) -> Option<PaneId> {
    focus.filter(|p| layout.contains(*p)).or_else(|| fallback_pane(layout))
}

/// What Pane `p` is called on screen (its Widget's title; an empty Pane is "an empty pane").
pub fn pane_called(design: usize, p: Pane) -> String {
    p.widget.map_or_else(|| "an empty pane".into(), |w| crate::ui::pane_name(design, w))
}

/// Why an edit was refused, in the bar's words.
pub fn refusal(e: Refused, design: usize) -> String {
    e.why(|w| crate::ui::pane_name(design, w))
}

/// The Pane the focus falls back on when it names none of `layout`'s: Home's, or (Home waiting in the Tray) the first.
pub fn fallback_pane(layout: &Arrangement) -> Option<PaneId> {
    layout.pane_of(Widget::Home).or_else(|| layout.panes().first().map(|p| p.id))
}

/// What keeps Done unavailable: "Home still needs a Pane", while it waits in the Tray.
pub fn done_blocked(layout: &Arrangement, design: usize) -> Option<String> {
    let w = layout.unplaced()?;
    let name = crate::ui::pane_name(design, w);
    let mut c = name.chars();
    let name = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
    Some(format!("{name} still needs a Pane"))
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
pub const TRAY_TITLE: &str = " Tray ";
pub const TRAY_ADD: &str = " + Widget ";

/// A Widget's chip in the Tray: its grip and name.
pub fn tray_chip(name: &str) -> String {
    format!(" ⠿ {name} ")
}

/// The Tray strip at `r` (three rows: its frame round one row of Widgets) holding `tray`, from Widget `first` on, or
/// as near it as still fills the row: when they don't all fit, ‹ › at the ends scroll it. `reserve` columns after the
/// last one are kept for a Widget aimed there with the keys.
pub fn tray_layout(tray: &[Widget], design: usize, reserve: u16, r: Rect, first: usize) -> TrayLayout {
    let y = r.y + 1;
    let (x0, x1) = (r.x + 2, r.right().saturating_sub(2));
    let width = |w: Widget| tray_chip(&crate::ui::pane_name(design, w)).chars().count() as u16 + if crate::arrangement::required(Some(w)) { 0 } else { 2 };
    let widths: Vec<u16> = tray.iter().map(|w| width(*w)).collect();
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
        let chip = tray_chip(&crate::ui::pane_name(design, *w)).chars().count() as u16;
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

/// One input being handled: the Scene it came with, and the layout, focus and Panes' area as the outcomes so far
/// leave them (an edit committed mid-way is what the rest of the input sees).
struct Work<'s, 'a> {
    sc: &'s Scene<'a>,
    layout: Cow<'s, Arrangement>,
    focus: Option<PaneId>,
    body: Rect,
    out: Vec<Out>,
}

impl<'s, 'a> Work<'s, 'a> {
    fn new(sc: &'s Scene<'a>) -> Self {
        Work { sc, layout: Cow::Borrowed(sc.layout), focus: sc.focus, body: sc.body, out: vec![] }
    }

    /// Where the Panes are laid out, or a roomy guess before the first frame.
    fn area(&self) -> Rect {
        pane_area(self.body)
    }

    /// The screen's rows (`ui::rows`) in this window, arranging or not; before the first frame, around a roomy guess.
    fn rows(&self, arranging: bool) -> [Rect; 5] {
        screen_rows(self.sc.window, arranging)
    }

    fn tray_rect(&self) -> Rect {
        self.rows(true)[3]
    }

    /// What Pane `p` is called on screen (its Widget's title; an empty Pane is "an empty pane").
    fn name_of(&self, p: Pane) -> String {
        pane_called(self.sc.design, p)
    }

    fn say(&mut self, s: impl Into<String>) {
        self.out.push(Out::Say(s.into()));
    }

    fn refused(&mut self, e: Refused) {
        let why = refusal(e, self.sc.design);
        self.say(why);
    }

    fn focus_to(&mut self, f: Option<PaneId>) {
        self.focus = f;
        self.out.push(Out::Focus(f));
    }

    fn fallback(&self) -> Option<PaneId> {
        fallback_pane(&self.layout)
    }

    /// The Pane to act on: the focused one, or (the focus is on none of this layout's) `fallback_pane`.
    fn focused_pane(&self) -> Option<PaneId> {
        focused_pane(&self.layout, self.focus)
    }

    /// The Pane under (x, y), in the layout as it is drawn.
    fn pane_under(&self, x: u16, y: u16) -> Option<PaneId> {
        let area = self.area();
        self.layout.compute(area).panes.into_iter().find(|(_, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()).map(|(p, _)| p.id)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { tree: self.layout.clone().into_owned(), shown: self.sc.mark.clone(), focus: self.focus }
    }

    fn done(self) -> Vec<Out> {
        self.out
    }
}

impl Arrange {
    /// Whether Arrange mode is on.
    pub fn is_on(&self) -> bool {
        self.on.is_some()
    }

    /// Whether a New design… is being built.
    pub fn building(&self) -> bool {
        self.on.as_ref().is_some_and(|a| a.fresh.is_some())
    }

    /// The saved Design that was on screen when a New design… began (config.toml keeps naming it meanwhile).
    pub fn building_from(&self) -> Option<String> {
        self.on.as_ref()?.fresh.as_ref()?.before.shown.file().map(String::from)
    }

    /// The Pane whose Widget is being moved (held by the mouse, or picked up with Enter): drawn highlighted.
    pub fn moving(&self) -> Option<PaneId> {
        self.tile.as_ref().filter(|t| t.live).map(|t| t.pane.id)
    }

    /// Whether something on screen moves with the pointer (a Widget picked up with Enter moves with the keys only).
    pub fn moving_now(&self) -> bool {
        self.tile.as_ref().is_some_and(|t| t.live && t.aim.is_none()) || self.divider.is_some()
    }

    /// Whether anything is held: a Widget (either way) or a divider.
    pub fn holding(&self) -> bool {
        self.tile.is_some() || self.divider.is_some()
    }

    /// The divider held by the mouse, if one is.
    pub fn divider_held(&self) -> Option<usize> {
        self.divider.as_ref().map(|d| d.i)
    }

    /// A held Widget is let go, and nothing changes (Esc, a popup opening, the mini player).
    pub fn drop_held(&mut self) {
        self.tile = None;
    }

    /// A release that came while a popup was open: what was held ends without doing anything.
    pub fn let_go(&mut self) {
        self.tile = None;
        self.divider = None;
    }

    /// Another Design is shown: the undo history was the last one's.
    pub fn forget_history(&mut self) {
        self.history.clear();
    }

    /// What the Tray strip shows first (Arrange mode on).
    pub fn tray_first(&self) -> Option<usize> {
        self.on.as_ref().map(|a| a.tray_first)
    }

    /// The Tray strip was drawn from Widget `first` on (as near the one asked for as still fills it).
    pub fn tray_drawn_from(&mut self, first: usize) {
        if let Some(a) = self.on.as_mut() {
            a.tray_first = first;
        }
    }

    /// The keyboard's focus in the Tray.
    pub fn tray_focus(&self) -> Option<TrayStop> {
        self.on.as_ref().and_then(|a| a.tray_focus)
    }

    /// Whether `s` waits for the arrow saying which way to split.
    pub fn split_waiting(&self) -> bool {
        self.on.as_ref().is_some_and(|a| a.split_next)
    }

    /// Whether a Widget is picked up with Enter (the legend shows the keys for it).
    pub fn carrying(&self) -> bool {
        self.tile.as_ref().is_some_and(|t| t.aim.is_some())
    }

    /// Whether the Tray is lit: a Pane's Widget is held over it.
    pub fn tray_lit(&self) -> bool {
        self.tile.as_ref().is_some_and(|t| t.live && t.tray)
    }

    /// Whether a Pane's Widget is aimed at the Tray with the keys (it is shown where it lands).
    pub fn landing_in_tray(&self) -> bool {
        self.tile.as_ref().is_some_and(|t| t.live && t.tray && t.aim.is_some())
    }

    /// The columns the Tray keeps after its last Widget for a Pane's Widget aimed there with the keys.
    pub fn tray_reserve(&self, design: usize) -> u16 {
        self.tile.as_ref().filter(|t| t.live && t.tray && t.aim.is_some()).and_then(|t| t.pane.widget).map_or(0, |w| (crate::ui::pane_name(design, w).chars().count() as u16 + 4).min(CHIP_MAX) + 1)
    }

    fn tray_layout_in(&self, w: &Work, first: usize) -> TrayLayout {
        tray_layout(w.layout.tray(), w.sc.design, self.tray_reserve(w.sc.design), w.tray_rect(), first)
    }

    /// The bar's offer to undo (or redo) the last edit: since when, and which.
    pub fn undo_offer(&self) -> Option<(Instant, Action, bool)> {
        let (at, a) = self.undo_offer?;
        let can = if a == Action::UndoLayout { self.history.can_undo() } else { self.history.can_redo() };
        Some((at, a, can))
    }

    /// Remember `was` as what undo goes back to (Reset design), and offer it.
    pub fn remember(&mut self, was: Snapshot) {
        self.history.push(was);
        self.undo_offer = Some((Instant::now(), Action::UndoLayout));
    }

    // ── what the App asks of it ───────────────────────────────────────

    /// Arrange mode on (the palette's and the Pane menu's "Arrange", a title drag, Esc in a split's picker).
    pub fn enter(&mut self, sc: &Scene) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.enter_in(&mut w);
        w.done()
    }

    /// Before each frame: Home always has a Pane outside Arrange mode. A Design whose Home waits in the Tray (one left
    /// that way when the app quit) comes up in Arrange mode, where Done waits for it (not while the Design picker
    /// previews one: a preview isn't a Design you have).
    pub fn homeless(&mut self, sc: &Scene, previewing: bool) -> Vec<Out> {
        let mut w = Work::new(sc);
        if self.on.is_none() && !previewing && w.layout.unplaced().is_some() {
            self.enter_in(&mut w);
        }
        w.done()
    }

    /// The Panes' area changed (a new window size, Arrange mode on or off): the Pane under a held Widget is found
    /// again (#21: a release dropped on a Pane of the old window).
    pub fn resized(&mut self, sc: &Scene) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.relayout(&mut w);
        w.done()
    }

    /// A Widget picked up with Enter is put back once anything else edits the layout (Ctrl+K Close pane, Replace
    /// widget…, a Design change…): its Pane, or what that Pane holds, may be gone.
    pub fn pick_up_check(&mut self, layout: &Arrangement) {
        if self.tile.as_ref().is_some_and(|t| t.on.as_ref().is_some_and(|on| on != layout)) {
            self.tile = None;
        }
    }

    /// The layout was replaced from outside (a Design, an undo): a keyboard focus on a Widget that left the Tray
    /// (`old` was the Tray before) moves on.
    pub fn layout_replaced(&mut self, sc: &Scene, old: &[Widget], was: Option<Widget>) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.settle_tray_focus(&mut w, old);
        // the Panes of another tree are other Panes: the focus stays on the Widget it was on (arranging, on a real Pane)
        let mut f = was.and_then(|x| w.layout.pane_of(x));
        if self.on.is_some() && f.is_none() {
            f = w.fallback();
        }
        w.focus_to(f);
        w.done()
    }

    /// `tree` replaces the layout as an edit (one undo step): the Widget picker's pick, a Tray pick.
    pub fn commit(&mut self, sc: &Scene, tree: Arrangement) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.commit_in(&mut w, tree, None);
        w.done()
    }

    /// `tree` replaces the layout as the rest of the last edit (a split's pick): one undo takes both back.
    pub fn amend(&mut self, sc: &Scene, tree: Arrangement) -> Vec<Out> {
        let mut w = Work::new(sc);
        let old = w.layout.tray().to_vec();
        w.layout = Cow::Owned(tree.clone());
        w.out.push(Out::Commit { tree: Some(tree), note: None });
        self.settle_tray_focus(&mut w, &old);
        self.undo_offer = Some((Instant::now(), Action::UndoLayout));
        self.note_edit();
        w.done()
    }

    /// What's on screen now, for undo to come back to (Reset design takes it before it resets).
    pub fn snapshot(&self, sc: &Scene) -> Snapshot {
        Work::new(sc).snapshot()
    }

    /// Reset design replaced `before` with `now`: one undo step, if it changed the layout or left a file.
    pub fn reset_done(&mut self, before: Snapshot, now: &Arrangement) {
        if before.tree != *now || before.shown.file().is_some() {
            self.remember(before);
        }
    }

    /// A key while a Widget is held by the pointer (or any held Widget behind the mini player): Esc lets it go and
    /// nothing changes; every other key waits for the release. True: the key was taken.
    pub fn holds_keys(&mut self, k: KeyEvent, mini: bool) -> bool {
        if !self.tile.as_ref().is_some_and(|t| t.aim.is_none() || mini) {
            return false;
        }
        if k.code == KeyCode::Esc {
            self.tile = None;
        }
        true
    }

    /// The `arrange` action (its key, also one bound with Ctrl or Alt) while arranging: it leaves as Esc does (Q70).
    pub fn arrange_action(&mut self, sc: &Scene) -> Vec<Out> {
        self.key(sc, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
    }

    /// An edit was made: Done will ask about it, and `s` stops waiting for its arrow. False while a New design… is
    /// built (it is written when Done names it), else the Design's file follows.
    pub fn note_edit(&mut self) -> bool {
        if let Some(a) = self.on.as_mut() {
            a.edited = true;
            a.split_next = false;
            if a.fresh.is_some() {
                return false;
            }
        }
        true
    }

    pub fn close_pane(&mut self, sc: &Scene, id: PaneId) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.close_in(&mut w, id);
        w.done()
    }

    /// A new empty Pane on `side` of Pane `target` (or the focused one), and the Widget picker on it at once.
    pub fn split_pane(&mut self, sc: &Scene, target: Option<PaneId>, side: Side) -> Vec<Out> {
        let mut w = Work::new(sc);
        if let Some(t) = target.or_else(|| w.focused_pane()) {
            self.split_in(&mut w, t, side);
        }
        w.done()
    }

    /// The side a split of Pane `id` takes unless told otherwise: along its longer side in real proportions (wider than
    /// tall: to the right, else below), or the other way if only that one has room.
    pub fn default_split(&self, sc: &Scene, id: PaneId) -> Side {
        default_split_in(&Work::new(sc), id)
    }

    /// Undo (`back`) or redo.
    pub fn undo(&mut self, sc: &Scene, back: bool) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.step_history(&mut w, back);
        w.done()
    }

    /// Done (or Esc).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn done(&mut self, sc: &Scene) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.done_in(&mut w);
        w.done()
    }

    /// "New design…" (#42): Arrange mode from one empty Pane filling the screen, with Home waiting in the Tray. The
    /// Design on screen stays as it is (in its file too) until Done names the new one; Esc before anything is placed
    /// goes back to it. `theme` is the Theme on screen.
    pub fn new_design(&mut self, sc: &Scene, theme: &str) -> Vec<Out> {
        let mut w = Work::new(sc);
        if self.building() {
            return w.done();
        }
        let before = w.snapshot();
        let history = std::mem::take(&mut self.history);
        // (from Arrange mode: what was arranged is kept as it is, and the build starts from the blank one)
        self.on = None;
        self.tile = None;
        self.undo_offer = None;
        let blank = Arrangement::new(Node::empty(), vec![]);
        w.layout = Cow::Owned(blank.clone());
        w.out.push(Out::Show(blank));
        w.out.push(Out::BuildNew);
        let first = w.layout.panes().first().map(|p| p.id);
        w.focus_to(first);
        self.enter_in(&mut w);
        if let Some(a) = self.on.as_mut() {
            a.fresh = Some(Box::new(Fresh { before, history, theme: theme.to_string() }));
        }
        w.done()
    }

    /// Esc on a New design… before anything is placed, or Discard: the Design that was on screen, as it was, with its
    /// Theme and its undo history.
    pub fn cancel_new(&mut self, sc: &Scene) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.cancel_new_in(&mut w);
        w.done()
    }

    /// A New design… was saved under its name: Arrange mode ends, and the new Design starts its own undo history.
    pub fn new_design_saved(&mut self, sc: &Scene) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.on = None;
        self.tile = None;
        self.history = Default::default();
        self.undo_offer = None;
        self.relayout(&mut w);
        w.done()
    }

    /// The Tray strip scrolled `by` Widgets (the wheel, its ‹ ›).
    pub fn tray_scroll(&mut self, sc: &Scene, by: i32) {
        let w = Work::new(sc);
        self.tray_scroll_in(&w, by);
    }

    // ── input ─────────────────────────────────────────────────────────

    /// A bare move or a fresh press while a Widget or a divider is held: its release went somewhere else (another
    /// window, after the focus moved) and was never heard (#21: the Pane stayed held and swallowed every click). A held
    /// Widget stays where it was; a divider is let go where it is. (A Widget picked up with Enter was never the
    /// pointer's: a bare move leaves it be, a press puts it back.)
    pub fn unheard(&mut self, sc: &Scene, kind: MouseEventKind) -> Vec<Out> {
        let mut w = Work::new(sc);
        if !matches!(kind, MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)) {
            return w.done();
        }
        if kind == MouseEventKind::Moved && self.tile.as_ref().is_some_and(|t| t.aim.is_some()) {
            return w.done();
        }
        self.tile = None;
        if self.divider.is_some() {
            self.divider_released(&mut w);
        }
        w.done()
    }

    /// Keys in Arrange mode (the shortcuts with Ctrl or Alt work as everywhere; no Pane takes keys): arrows move the
    /// focus between Panes, Enter picks up the focused Widget, `t` sends it to the Tray, `s` then an arrow splits its
    /// Pane that way, `x` closes it, `+` (or `=`) `-` move its border left or right of it, `]` `[` the one above or
    /// below it, `w` opens "+ Widget"'s picker, Esc (or `m`, the key that came in, Q70) is Done. With a Widget picked
    /// up, arrows aim, Enter drops and Esc (or `m`) puts it back. On a Widget in the Tray, `x` (or Delete, Backspace)
    /// removes it. The playback keys these don't take work as ever (the player bar stays).
    pub fn key(&mut self, sc: &Scene, k: KeyEvent) -> Vec<Out> {
        let mut w = Work::new(sc);
        self.key_in(&mut w, k);
        w.done()
    }

    /// A mouse event that is Arrange mode's, once the App's own gates have let it through (a popup open, the player
    /// bar, a wake, a scrub: `App::pointer`), with what the last frame drew at its cell: Arrange mode's own pointer
    /// while it is on, a held Widget's, and outside Arrange mode a press on a title, a × or a divider, a held
    /// divider's drag and the release. (`unheard` comes first, before the App's gates.)
    pub fn route(&mut self, sc: &Scene, m: MouseEvent, hit: Option<Hit>, coarse: bool) -> Vec<Out> {
        let mut w = Work::new(sc);
        if self.on.is_some() {
            self.pointer_in(&mut w, m, hit, coarse);
        } else if self.tile.is_some() {
            self.tile_pointer_in(&mut w, m);
        } else {
            let (x, y) = (m.column, m.row);
            match (m.kind, hit) {
                (MouseEventKind::Down(MouseButton::Left), Some(Hit::Title(p) | Hit::TitleName(p))) => self.tile_down(&mut w, p, x, y),
                (MouseEventKind::Down(MouseButton::Left), Some(Hit::Close(p))) => self.close_in(&mut w, p),
                (MouseEventKind::Down(MouseButton::Left), Some(Hit::Divider(i))) => self.grab_divider(&w, i, x, y),
                (MouseEventKind::Drag(MouseButton::Left), _) => self.divider_move(&mut w, x, y),
                (MouseEventKind::Up(_), _) if self.divider.is_some() => self.divider_released(&mut w),
                _ => {}
            }
        }
        w.done()
    }

    // ── drawing ───────────────────────────────────────────────────────

    /// While a Widget is held over another Pane: that Pane, what its label says ("swap with queue", "put here"), and
    /// the chip's text ("⠿ stream"). Over its own Pane or none: the chip alone.
    pub fn drag_marks(&self, layout: &Arrangement, design: usize) -> Option<(Option<(PaneId, String)>, String)> {
        let t = self.tile.as_ref().filter(|t| t.live)?;
        let w = t.pane.widget?;
        let target = t.over.map(|o| {
            let label = match layout.widget_in(o) {
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
    pub fn chip_rect(&self, sc: &Scene, area: Rect, text: &str) -> Option<Rect> {
        let wk = Work::new(sc);
        let t = self.tile.as_ref().filter(|t| t.live)?;
        let w = (text.chars().count() as u16 + 2).min(CHIP_MAX).min(area.width);
        if let Some(Aim::Tray(x)) = t.aim {
            let r = wk.tray_rect();
            let (x0, x1) = (r.x + 2, r.right().saturating_sub(2));
            let w = w.min(x1.saturating_sub(x0));
            let first = self.on.as_ref().map_or(0, |a| a.tray_first);
            let l = self.tray_layout_in(&wk, first);
            let at = match (t.from_tray, t.pane.widget) {
                // over its own chip in the strip (the column it was picked up at, if that has scrolled away)
                (true, Some(me)) => {
                    let i = wk.layout.tray().iter().position(|w| *w == me);
                    l.items.iter().find(|(j, ..)| Some(*j) == i).map_or(x.saturating_sub(w / 2), |(_, c, _)| c.x)
                }
                // after its last Widget shown (or at its start, empty)
                _ => l.items.last().map_or(x0, |(_, c, close)| close.unwrap_or(*c).right() + 1),
            };
            return Some(Rect::new(at.min(x1.saturating_sub(w)).max(x0), r.y + 1, w, 1));
        }
        if let Some(Aim::Pane(aim)) = t.aim {
            let r = wk.layout.compute(wk.area()).rect(aim)?;
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
        let l = super::label_rect(r, label, chip, self.label_row(id));
        if let Some(t) = self.tile.as_mut() {
            t.label = l.map(|l| (id, l.y));
        }
        l
    }

    /// The row target `id`'s label was drawn on last.
    pub fn label_row(&self, id: PaneId) -> Option<u16> {
        self.tile.as_ref()?.label.filter(|(p, _)| *p == id).map(|(_, y)| y)
    }

    // ── inside ────────────────────────────────────────────────────────

    /// The Panes' area again, for the window's size and whether Arrange mode is on (it takes the Tray's rows), at once:
    /// a release in the same batch of input finds the Pane under it where the next frame draws it.
    fn relayout(&mut self, w: &mut Work) {
        if w.sc.window.width > 0 {
            w.body = crate::ui::rows(w.sc.window, self.on.is_some())[1];
            w.out.push(Out::Body(w.body));
        }
        self.tile_resized(w);
    }

    fn enter_in(&mut self, w: &mut Work) {
        if self.on.is_none() {
            self.on = Some(Arranging { since: w.layout.clone().into_owned(), edited: false, tray_first: 0, split_next: false, tray_focus: None, fresh: None });
            self.relayout(w);
        }
    }

    /// A press on divider `i`'s grip at (x, y): it follows the pointer until the release.
    fn grab_divider(&mut self, w: &Work, i: usize, x: u16, y: u16) {
        let offset = w.sc.dividers.get(i).map_or(0, |d| {
            let (boundary, grab) = match d.axis {
                Axis::Horizontal => (d.grip.x + 1, x),
                Axis::Vertical => (d.grip.y + 1, y),
            };
            boundary as i32 - grab as i32
        });
        self.divider = Some(DividerDrag { i, offset, from: w.layout.clone().into_owned() });
    }

    fn tile_down(&mut self, w: &mut Work, id: PaneId, x: u16, y: u16) {
        w.focus_to(Some(id));
        let pane = w.layout.get(id).unwrap_or(Pane { id, widget: None });
        self.tile = Some(TileDrag { pane, from: (x, y), at: (x, y), live: false, over: None, tray: false, from_tray: false, aim: None, on: None, label: None });
    }

    /// A press on Widget `wd` in the Tray: a drag takes it onto a Pane.
    fn tray_down(&mut self, w: &Work, wd: Widget, x: u16, y: u16) {
        if !w.layout.tray().contains(&wd) {
            return;
        }
        let pane = Pane { id: PaneId::UNSET, widget: Some(wd) };
        self.tile = Some(TileDrag { pane, from: (x, y), at: (x, y), live: false, over: None, tray: false, from_tray: true, aim: None, on: None, label: None });
    }

    /// What a release at (x, y) would drop on, for a Widget held from Pane `own` (or the Tray): the Pane under it
    /// other than its own, or (for a Pane's Widget) the Tray.
    fn aim(&self, w: &Work, x: u16, y: u16, own: PaneId, from_tray: bool) -> (Option<PaneId>, bool) {
        let t = w.tray_rect();
        if self.on.is_some() && x >= t.x && x < t.right() && y >= t.y && y < t.bottom() {
            return (None, !from_tray);
        }
        (w.pane_under(x, y).filter(|p| *p != own), false)
    }

    fn tile_pointer_in(&mut self, w: &mut Work, m: MouseEvent) {
        if self.tile.as_ref().is_some_and(|t| t.aim.is_some()) {
            return;
        }
        match m.kind {
            MouseEventKind::Drag(MouseButton::Left) => self.tile_move(w, m.column, m.row),
            MouseEventKind::Up(MouseButton::Left) => self.tile_up(w, m.column, m.row),
            _ => {}
        }
    }

    /// The pointer at (x, y) with the button down: past a few cells it is a drag (an empty Pane has nothing to carry),
    /// which turns Arrange mode on, and what is under it is the one highlighted, at once.
    fn tile_move(&mut self, w: &mut Work, x: u16, y: u16) {
        let Some(t) = self.tile.as_ref() else { return };
        if !t.live && (x.abs_diff(t.from.0) < 3 && y.abs_diff(t.from.1) < 2 || t.pane.widget.is_none()) {
            return;
        }
        let (own, from_tray) = (t.pane.id, t.from_tray);
        // on while the Widget is held, and after the drop: a quick swap stays one gesture (D87)
        self.enter_in(w);
        let (over, tray) = self.aim(w, x, y, own, from_tray);
        if let Some(t) = self.tile.as_mut() {
            t.live = true;
            t.at = (x, y);
            t.over = over;
            t.tray = tray;
        }
    }

    fn tile_resized(&mut self, w: &mut Work) {
        let Some((own, from_tray, (x, y))) = self.tile.as_ref().filter(|t| t.live && t.aim.is_none()).map(|t| (t.pane.id, t.from_tray, t.at)) else { return };
        let (over, tray) = self.aim(w, x, y, own, from_tray);
        if let Some(t) = self.tile.as_mut() {
            t.over = over;
            t.tray = tray;
        }
    }

    /// Let go at (x, y): on another Pane the two Widgets swap (an empty one is filled); on the Tray the Widget waits
    /// there; from the Tray onto a Pane it takes the Pane (its Widget goes into the Tray). Anywhere else nothing changes.
    fn tile_up(&mut self, w: &mut Work, x: u16, y: u16) {
        let Some(t) = self.tile.as_ref() else { return };
        if !t.live {
            let (id, from, wd, from_tray) = (t.pane.id, t.from, t.pane.widget, t.from_tray);
            self.tile = None;
            // a click on a Widget in the Tray does nothing
            if !from_tray {
                w.out.push(Out::TitleClick { pane: id, at: from, widget: wd });
            }
            return;
        }
        // the release's own cell is the last word (a finger slips as it lifts; moves can come combined)
        self.tile_move(w, x, y);
        let Some(t) = self.tile.take() else { return };
        let edit = match (t.from_tray, t.pane.widget, t.over) {
            (true, Some(wd), Some(to)) => w.layout.out_of_tray(wd, to).map(|a| (a, Some(to))),
            (false, _, _) if t.tray => w.layout.to_tray(t.pane.id).map(|a| (a, None)),
            (false, _, Some(to)) => w.layout.swap(t.pane.id, to).map(|a| (a, Some(to))),
            _ => return,
        };
        match edit {
            Ok((a, to)) => {
                if to.is_some() {
                    w.focus_to(to);
                }
                self.commit_in(w, a, None);
            }
            Err(e) => w.refused(e),
        }
    }

    fn divider_move(&mut self, w: &mut Work, x: u16, y: u16) {
        let Some(dd) = self.divider.as_ref() else { return };
        let Some(d) = w.sc.dividers.get(dd.i) else { return };
        let pos = if d.axis == Axis::Vertical { y } else { x };
        let pos = (pos as i32 + dd.offset).max(0) as u16;
        let tree = w.layout.move_line(d, pos, w.area());
        w.layout = Cow::Owned(tree.clone());
        w.out.push(Out::Show(tree));
    }

    /// A divider let go: a layout that changed is an edit.
    fn divider_released(&mut self, w: &mut Work) {
        if let Some(dd) = self.divider.take().filter(|d| d.from != *w.layout) {
            let mut was = w.snapshot();
            was.tree = dd.from;
            self.remember(was);
            let _ = self.note_edit();
            w.out.push(Out::Commit { tree: None, note: None });
        }
    }

    /// `tree` replaces the layout as an edit (one undo step); `note` is the bar's line for it.
    fn commit_in(&mut self, w: &mut Work, tree: Arrangement, note: Option<String>) {
        let was = w.snapshot();
        let old = w.layout.tray().to_vec();
        w.layout = Cow::Owned(tree.clone());
        w.out.push(Out::Commit { tree: Some(tree), note });
        self.settle_tray_focus(w, &old);
        self.remember(was);
        let _ = self.note_edit();
    }

    fn step_history(&mut self, w: &mut Work, back: bool) {
        // a held Widget is let go first (one picked up with Enter: Ctrl+Z gets here; the mouse and the bar never do
        // mid-drag, but be sure)
        self.tile = None;
        let area = w.area();
        let now = w.snapshot();
        let fits = |s: &Snapshot| s.tree.fits(area);
        let got = if back { self.history.undo(now, fits) } else { self.history.redo(now, fits) };
        let (any, what) = if back { (self.history.can_undo(), "undo") } else { (self.history.can_redo(), "redo") };
        match got {
            Some(snap) => {
                self.undo_offer = Some((Instant::now(), if back { Action::RedoLayout } else { Action::UndoLayout }));
                let key = w.sc.keymap.label(if back { Action::RedoLayout } else { Action::UndoLayout });
                let again = if key.is_empty() { String::new() } else { format!(" · {key} {}", if back { "redoes" } else { "undoes" }) };
                let said = format!("{}{again}", if back { "Undone" } else { "Redone" });
                w.out.push(Out::Restore { snap, said });
            }
            None if any => w.say(format!("Can't {what}: that layout doesn't fit this window")),
            None => w.say(format!("Nothing to {what}")),
        }
    }

    fn cancel_new_in(&mut self, w: &mut Work) {
        let Some(f) = self.on.take().and_then(|a| a.fresh) else { return };
        let Fresh { before, history, theme } = *f;
        self.tile = None;
        self.undo_offer = None;
        self.history = history;
        w.layout = Cow::Owned(before.tree.clone());
        w.out.push(Out::Resume { snap: before, theme });
        self.relayout(w);
    }

    /// Esc with nothing picked up: Done, but on a New design… with nothing placed yet, back to the Design that was on
    /// screen, untouched (#42).
    fn esc(&mut self, w: &mut Work) {
        if self.on.as_ref().is_some_and(|a| a.fresh.is_some() && a.since == *w.layout) {
            return self.cancel_new_in(w);
        }
        self.done_in(w)
    }

    /// Done (or Esc): Arrange mode ends, once Home has a Pane. If anything changed in it and the changes went to a
    /// built-in Design's "<Design> (edited)", the name prompt comes now (Esc keeps it unnamed).
    fn done_in(&mut self, w: &mut Work) {
        if done_blocked(&w.layout, w.sc.design).is_some() {
            return;
        }
        // a New design…: Done asks for its name, and Arrange mode stays on until it has one (#42)
        if self.building() {
            w.out.push(Out::AskNewName);
            return;
        }
        let Some(a) = self.on.take() else { return };
        self.tile = None;
        self.relayout(w);
        if a.edited && a.since != *w.layout {
            w.out.push(Out::AskForkName);
        }
    }

    fn key_in(&mut self, w: &mut Work, k: KeyEvent) {
        // a divider held by the mouse: the release ends it (and records it); no arrange key meanwhile
        if self.divider.is_some() {
            return;
        }
        // the key that came into Arrange mode leaves it: it does what Esc does (Q70)
        let k = if w.sc.keymap.action(k) == Some(Action::Arrange) { KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE) } else { k };
        self.pick_up_check(&w.layout.clone());
        // (a focus on a Widget that left the Tray, or on no Pane of this layout: the keys act from a real Pane)
        let tray = w.layout.tray().to_vec();
        let add = self.tray_add_x(w).is_some();
        if let Some(a) = self.on.as_mut()
            && a.tray_focus.is_some_and(|s| match s {
                TrayStop::Widget(wd) => !tray.contains(&wd),
                TrayStop::Add => !add,
            })
        {
            a.tray_focus = None;
        }
        let on_tray = self.on.as_ref().and_then(|a| a.tray_focus);
        if on_tray.is_none() && !w.focus.is_some_and(|p| w.layout.contains(p)) {
            let f = w.fallback();
            w.focus_to(f);
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
            return playback_key(w, k);
        }
        if self.tile.is_some() {
            return match (k.code, arrow) {
                (_, Some(side)) => self.aim_step(w, side),
                (KeyCode::Enter, _) => self.key_drop(w),
                (KeyCode::Esc, _) => self.drop_held(),
                _ => {}
            };
        }
        // `s` waits for the arrow saying which way; any other key ends the wait and does nothing else
        if self.on.as_mut().is_some_and(|a| std::mem::take(&mut a.split_next)) {
            if let Some(side) = arrow
                && let Some(t) = w.focused_pane()
            {
                self.split_in(w, t, side);
            }
            return;
        }
        // the focus in the Tray: ← → move along it, ↑ goes back up into the Panes, Enter picks a Widget up (or, on
        // "+ Widget", opens the Widget picker for the Tray), `x` (or Delete, Backspace) removes the Widget, as its `×`
        if let Some(stop) = on_tray {
            return match (k.code, arrow) {
                (_, Some(side @ (Side::Left | Side::Right))) => self.tray_focus_step(w, stop, side == Side::Right),
                (_, Some(Side::Top)) => self.tray_focus_up(w, stop),
                (KeyCode::Enter, _) => match stop {
                    TrayStop::Widget(wd) => self.pick_up_from_tray(w, wd),
                    TrayStop::Add => w.out.push(Out::OpenTrayPicker),
                },
                (KeyCode::Char('x') | KeyCode::Delete | KeyCode::Backspace, _) => match stop {
                    TrayStop::Widget(wd) => self.tray_remove_focused(w, wd),
                    TrayStop::Add => {}
                },
                (KeyCode::Char('w'), _) => w.out.push(Out::OpenTrayPicker),
                (KeyCode::Esc, _) => self.esc(w),
                // (the other keys for a Pane have none here)
                _ => {}
            };
        }
        let focused = w.focused_pane();
        match (k.code, arrow, focused) {
            (_, Some(side), _) => self.focus_step(w, side),
            (KeyCode::Esc, ..) => self.esc(w),
            (KeyCode::Enter, _, Some(id)) => self.pick_up(w, id),
            (KeyCode::Char('t'), _, Some(id)) => match w.layout.to_tray(id) {
                Ok(a) => self.commit_in(w, a, None),
                Err(e) => w.refused(e),
            },
            (KeyCode::Char('s'), _, Some(_)) => {
                if let Some(a) = self.on.as_mut() {
                    a.split_next = true;
                }
            }
            (KeyCode::Char('x'), _, Some(id)) => self.close_in(w, id),
            (KeyCode::Char('+' | '='), _, Some(id)) => self.resize_step(w, id, true, Axis::Horizontal),
            (KeyCode::Char('-'), _, Some(id)) => self.resize_step(w, id, false, Axis::Horizontal),
            (KeyCode::Char(']'), _, Some(id)) => self.resize_step(w, id, true, Axis::Vertical),
            (KeyCode::Char('['), _, Some(id)) => self.resize_step(w, id, false, Axis::Vertical),
            (KeyCode::Char('w'), ..) => w.out.push(Out::OpenTrayPicker),
            _ => {}
        }
    }

    fn tray_scroll_in(&mut self, w: &Work, by: i32) {
        let Some(now) = self.on.as_ref().map(|a| a.tray_first) else { return };
        let want = (now as i64 + by as i64).max(0) as usize;
        // no further than the last start that still fills the strip
        let first = self.tray_layout_in(w, want).first;
        if let Some(a) = self.on.as_mut() {
            a.tray_first = first;
        }
    }

    /// The `×` of Widget `wd` in the Tray: it is removed (Home never; one already gone: nothing).
    fn tray_remove(&mut self, w: &mut Work, wd: Widget) {
        if !w.layout.tray().contains(&wd) {
            return;
        }
        match w.layout.remove_from_tray(wd) {
            Ok(a) => self.commit_in(w, a, None),
            Err(e) => w.refused(e),
        }
    }

    /// `x` (or Delete, Backspace) on Widget `wd` focused in the Tray: removed as by its `×` (Home is refused the same
    /// way); the focus goes to the next Widget in the Tray, else the one before, else back up to the Panes.
    fn tray_remove_focused(&mut self, w: &mut Work, wd: Widget) {
        let x = self.tray_chip_x(w, wd);
        self.tray_remove(w, wd);
        // (the edit moved the focus on: `settle_tray_focus`; back in the Panes, onto the one above the chip)
        if !w.layout.tray().contains(&wd) && self.on.as_ref().is_some_and(|a| a.tray_focus.is_none()) {
            let f = self.pane_above_tray(w, x).or_else(|| w.fallback());
            w.focus_to(f);
        }
    }

    /// After the layout changed (an edit, an undo or redo, the mouse's `×` in the Tray): a keyboard focus on a Widget
    /// that has left the Tray (`old` was the Tray before) moves to the next Widget still there, else the one before,
    /// else back to the Panes.
    fn settle_tray_focus(&mut self, w: &mut Work, old: &[Widget]) {
        let Some(TrayStop::Widget(wd)) = self.on.as_ref().and_then(|a| a.tray_focus) else { return };
        let tray = w.layout.tray().to_vec();
        if tray.contains(&wd) {
            return;
        }
        let i = old.iter().position(|t| *t == wd).unwrap_or(old.len());
        let after = old.iter().skip(i + 1);
        let before = old.iter().take(i).rev();
        let next = after.chain(before).find(|t| tray.contains(t)).copied();
        if let Some(a) = self.on.as_mut() {
            a.tray_focus = next.map(TrayStop::Widget);
        }
        match next {
            Some(n) => {
                self.tray_chip_x(w, n);
            }
            None => {
                if !w.focus.is_some_and(|p| w.layout.contains(p)) {
                    let f = w.fallback();
                    w.focus_to(f);
                }
            }
        }
    }

    /// Clear Tray: what's in it is removed, but Home.
    fn clear_tray(&mut self, w: &mut Work) {
        let a = w.layout.clear_tray();
        if a != *w.layout {
            self.commit_in(w, a, None);
        }
    }

    fn pointer_in(&mut self, w: &mut Work, m: MouseEvent, hit: Option<Hit>, coarse: bool) {
        // a press ends `s`'s wait for its arrow, and the keyboard's focus in the Tray
        if let (MouseEventKind::Down(_), Some(a)) = (m.kind, self.on.as_mut()) {
            a.split_next = false;
            a.tray_focus = None;
        }
        if self.tile.is_some() {
            return self.tile_pointer_in(w, m);
        }
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => match hit {
                Some(Hit::Title(p) | Hit::TitleName(p)) => self.tile_down(w, p, x, y),
                Some(Hit::Close(p)) => self.close_in(w, p),
                Some(Hit::SplitPane(p)) => {
                    let side = default_split_in(w, p);
                    self.split_in(w, p, side);
                }
                Some(Hit::Divider(i)) => self.grab_divider(w, i, x, y),
                Some(Hit::TrayItem(wd)) => self.tray_down(w, wd, x, y),
                Some(Hit::TrayRemove(wd)) => self.tray_remove(w, wd),
                Some(Hit::ClearTray) => self.clear_tray(w),
                Some(Hit::TrayAdd) => w.out.push(Out::OpenTrayPicker),
                Some(Hit::TrayScroll(d)) => self.tray_scroll_in(w, d),
                Some(Hit::ArrangeDone) => self.done_in(w),
                Some(Hit::Action(a @ (Action::UndoLayout | Action::RedoLayout))) => w.out.push(Out::Run(a)),
                _ => {
                    if let Some(p) = w.pane_under(x, y) {
                        w.focus_to(Some(p));
                    }
                }
            },
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(p) = w.pane_under(x, y) {
                    w.out.push(Out::OpenPaneMenu { pane: p, at: (x, y) });
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => self.divider_move(w, x, y),
            MouseEventKind::Up(_) => {
                if self.divider.is_some() {
                    self.divider_released(w);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let rows = if m.kind == MouseEventKind::ScrollUp { -1 } else { 1 };
                self.tray_scroll_in(w, rows * if coarse { 10 } else { 1 });
            }
            _ => {}
        }
    }

    // ── close and split ───────────────────────────────────────────────

    fn close_in(&mut self, w: &mut Work, id: PaneId) {
        let Some(pane) = w.layout.get(id) else { return };
        let name = w.name_of(pane);
        let tree = match w.layout.close(id, w.area()) {
            Ok(t) => t,
            Err(Refused::Required(_)) => return w.say(format!("{name} can be moved, but never closed")),
            Err(Refused::NoRoom | Refused::LastPane) => return w.say(format!("Closing {name} would squash the others")),
            Err(e) => return w.refused(e),
        };
        if w.focus == Some(id) {
            w.focus_to(fallback_pane(&tree));
        }
        // (the bar's one line: what the edit did, and where a built-in Design's edit went)
        self.commit_in(w, tree, Some(format!("Closed {name}")));
    }

    /// A new empty Pane on `side` of Pane `target`, taking half its room, and the Widget picker on it at once (Esc
    /// leaves it empty). Refused with the reason where it would leave a Pane under the minimum.
    fn split_in(&mut self, w: &mut Work, target: PaneId, side: Side) {
        let Some(pane) = w.layout.get(target) else { return };
        let tree = match w.layout.split(target, side, None, w.area()) {
            Ok(t) => t,
            Err(Refused::NoRoom) => return w.say(format!("No room for a new Pane {} {}", side.word_beside(), w.name_of(pane))),
            Err(e) => return w.refused(e),
        };
        let Some(new) = tree.panes().into_iter().map(|p| p.id).find(|id| !w.layout.contains(*id)) else { return };
        self.commit_in(w, tree, None);
        w.focus_to(Some(new));
        w.out.push(Out::OpenWidgetPicker { pane: new, after_split: true });
    }

    // ── the keyboard (#43) ────────────────────────────────────────────

    /// An arrow: the focus goes to the Pane next to the focused one that way (none there: it stays).
    /// ↓ from a Pane with none below it: the focus goes onto the Tray's Widget nearest under it (an empty Tray: its
    /// "+ Widget").
    fn focus_step(&mut self, w: &mut Work, dir: Side) {
        let Some(id) = w.focused_pane() else { return };
        let panes = w.layout.compute(w.area()).panes;
        match neighbour(&panes, id, dir) {
            Some(n) => w.focus_to(Some(n)),
            None if dir == Side::Bottom => {
                let mid = panes.iter().find(|(p, _)| p.id == id).map_or(0, |(_, r)| r.x + r.width / 2);
                let stop = self.tray_nearest(w, mid).map(TrayStop::Widget).or(self.tray_add_x(w).map(|_| TrayStop::Add));
                if let Some(stop) = stop {
                    w.focus_to(None);
                    if let Some(a) = self.on.as_mut() {
                        a.tray_focus = Some(stop);
                    }
                }
            }
            None => {}
        }
    }

    /// The Widget in the Tray whose chip (as the strip shows it now) is nearest column `x`.
    fn tray_nearest(&self, w: &Work, x: u16) -> Option<Widget> {
        let first = self.on.as_ref().map_or(0, |a| a.tray_first);
        let tray = w.layout.tray();
        let l = self.tray_layout_in(w, first);
        l.items.iter().min_by_key(|(_, r, _)| (r.x + r.width / 2).abs_diff(x)).map(|(i, ..)| tray[*i]).or_else(|| tray.first().copied())
    }

    /// The middle column of Widget `wd`'s chip in the Tray strip, scrolling the strip to it first if it isn't shown.
    fn tray_chip_x(&mut self, w: &Work, wd: Widget) -> u16 {
        let Some(i) = w.layout.tray().iter().position(|x| *x == wd) else { return w.tray_rect().x };
        let first = self.on.as_ref().map_or(0, |a| a.tray_first);
        let mut l = self.tray_layout_in(w, first);
        if !l.items.iter().any(|(j, ..)| *j == i) {
            l = self.tray_layout_in(w, if i < first { i } else { i.saturating_sub(l.items.len().saturating_sub(1)) });
            if let Some(a) = self.on.as_mut() {
                a.tray_first = l.first;
            }
        }
        l.items.iter().find(|(j, ..)| *j == i).map_or(w.tray_rect().x, |(_, r, _)| r.x + r.width / 2)
    }

    /// The middle column of the Tray's "+ Widget" (None: the strip has no room for it).
    fn tray_add_x(&self, w: &Work) -> Option<u16> {
        let first = self.on.as_ref().map_or(0, |a| a.tray_first);
        self.tray_layout_in(w, first).add.map(|r| r.x + r.width / 2)
    }

    /// ← or → with the focus in the Tray: the stop before or after (none there: it stays), shown. "+ Widget" is the
    /// stop after the last Widget.
    fn tray_focus_step(&mut self, w: &mut Work, stop: TrayStop, right: bool) {
        let tray = w.layout.tray().to_vec();
        let mut stops: Vec<TrayStop> = tray.into_iter().map(TrayStop::Widget).collect();
        if self.tray_add_x(w).is_some() {
            stops.push(TrayStop::Add);
        }
        let Some(i) = stops.iter().position(|s| *s == stop) else { return };
        let Some(&n) = (if right { stops.get(i + 1) } else { i.checked_sub(1).and_then(|j| stops.get(j)) }) else { return };
        if let Some(a) = self.on.as_mut() {
            a.tray_focus = Some(n);
        }
        if let TrayStop::Widget(wd) = n {
            self.tray_chip_x(w, wd);
        }
    }

    /// ↑ from the Tray: the focus goes back to the Pane above the stop (a Widget's chip, or "+ Widget").
    fn tray_focus_up(&mut self, w: &mut Work, stop: TrayStop) {
        let x = match stop {
            TrayStop::Widget(wd) => self.tray_chip_x(w, wd),
            TrayStop::Add => self.tray_add_x(w).unwrap_or(w.tray_rect().x),
        };
        let up = self.pane_above_tray(w, x);
        if let Some(a) = self.on.as_mut() {
            a.tray_focus = None;
        }
        let f = up.or_else(|| w.fallback());
        w.focus_to(f);
    }

    /// The bottom Pane over column `x` (the nearest one where none is).
    fn pane_above_tray(&self, w: &Work, x: u16) -> Option<PaneId> {
        let area = w.area();
        let panes = w.layout.compute(area).panes;
        let bottom = panes.iter().filter(|(_, r)| r.bottom() == area.bottom());
        bottom.min_by_key(|(_, r)| if x >= r.x && x < r.right() { 0 } else { x.abs_diff(r.x + r.width / 2) as u32 + 1 }).map(|(p, _)| p.id)
    }

    /// Enter on Widget `wd` in the Tray: it is picked up (the chip), aimed at the Tray (where it is) until ↑ aims into
    /// the Panes.
    fn pick_up_from_tray(&mut self, w: &mut Work, wd: Widget) {
        let x = self.tray_chip_x(w, wd);
        let pane = Pane { id: PaneId::UNSET, widget: Some(wd) };
        let on = Some(w.layout.clone().into_owned());
        self.tile = Some(TileDrag { pane, from: (0, 0), at: (0, 0), live: true, over: None, tray: false, from_tray: true, aim: Some(Aim::Tray(x)), on, label: None });
    }

    /// Enter on Pane `id`: its Widget is picked up (lit, with the chip), aimed at its own Pane until an arrow says
    /// otherwise. An empty Pane has nothing to pick up.
    fn pick_up(&mut self, w: &mut Work, id: PaneId) {
        let Some(pane) = w.layout.get(id).filter(|p| p.widget.is_some()) else { return };
        let on = Some(w.layout.clone().into_owned());
        self.tile = Some(TileDrag { pane, from: (0, 0), at: (0, 0), live: true, over: None, tray: false, from_tray: false, aim: Some(Aim::Pane(id)), on, label: None });
    }

    /// An arrow with a Widget picked up: the aim goes to the next Pane that way, lit with what Enter would do (back on
    /// its own Pane: nothing lit). ↓ from a bottom Pane aims at the Tray (lit for a Pane's Widget, as for the mouse);
    /// ↑ from the Tray aims at the Pane above.
    fn aim_step(&mut self, w: &mut Work, dir: Side) {
        let Some((own, from_tray, at)) = self.tile.as_ref().and_then(|t| Some((t.pane.id, t.from_tray, t.aim?))) else { return };
        let area = w.area();
        let panes = w.layout.compute(area).panes;
        let to = match at {
            Aim::Pane(p) => match neighbour(&panes, p, dir) {
                Some(n) => Aim::Pane(n),
                None if dir == Side::Bottom => Aim::Tray(panes.iter().find(|(q, _)| q.id == p).map_or(area.x, |(_, r)| r.x + r.width / 2)),
                None => return,
            },
            Aim::Tray(x) if dir == Side::Top => match self.pane_above_tray(w, x) {
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
            self.tray_to_end(w);
        }
    }

    /// Enter with a Widget picked up: it swaps with the Widget of the Pane aimed at (an empty one is filled), and the
    /// focus goes with it; from the Tray, it takes the Pane (its Widget goes into the Tray in its place). A Pane's
    /// Widget aimed at the Tray waits there, its Pane left empty. Aimed where it came from: it is put back.
    fn key_drop(&mut self, w: &mut Work) {
        let Some(t) = self.tile.take() else { return };
        let edit = match (t.from_tray, t.pane.widget, t.aim, t.over) {
            (true, Some(wd), _, Some(to)) => w.layout.out_of_tray(wd, to).map(|a| (a, Some(to))),
            (false, _, Some(Aim::Tray(_)), _) => w.layout.to_tray(t.pane.id).map(|a| (a, None)),
            (false, _, _, Some(to)) => w.layout.swap(t.pane.id, to).map(|a| (a, Some(to))),
            _ => return,
        };
        match edit {
            Ok((a, to)) => {
                if to.is_some() {
                    w.focus_to(to);
                    if let Some(a) = self.on.as_mut() {
                        a.tray_focus = None;
                    }
                }
                self.commit_in(w, a, None);
                // set aside: the strip shows where it went
                if to.is_none() {
                    self.tray_to_end(w);
                }
            }
            Err(e) => w.refused(e),
        }
    }

    /// The Tray strip scrolled to its end (as far as it goes).
    fn tray_to_end(&mut self, w: &Work) {
        let end = self.tray_layout_in(w, usize::MAX).first;
        if let Some(a) = self.on.as_mut() {
            a.tray_first = end;
        }
    }

    /// `+` `-` (`axis` Horizontal: its border left or right of it) or `]` `[` (Vertical: above or below it): the
    /// focused Pane a step bigger or smaller that way.
    fn resize_step(&mut self, w: &mut Work, id: PaneId, grow: bool, axis: Axis) {
        match w.layout.step_line(id, grow, w.area(), axis) {
            Ok(tree) => self.commit_in(w, tree, None),
            Err(_) => w.say(if grow { "Can't grow it more" } else { "Can't shrink it more" }),
        }
    }
}

/// The playback keys, which work while arranging (the player bar stays, the Tray under it).
fn playback_key(w: &mut Work, k: KeyEvent) {
    use Action::*;
    if let Some(a) = w.sc.keymap.action(k).filter(|a| matches!(a, TogglePlay | Next | Previous | SeekForward | SeekBack | SeekForwardFine | SeekBackFine | VolumeUp | VolumeDown | Shuffle | Repeat | Like)) {
        w.out.push(Out::Run(a));
    }
}

fn default_split_in(w: &Work, id: PaneId) -> Side {
    let (area, aspect) = (w.area(), w.sc.aspect);
    let rect = w.layout.compute(area).panes.into_iter().find(|(p, _)| p.id == id).map(|(_, r)| r);
    let wide = rect.is_none_or(|r| r.width as f64 >= r.height as f64 * aspect);
    let (long, other) = if wide { (Side::Right, Side::Bottom) } else { (Side::Bottom, Side::Right) };
    let fits = |side| w.layout.split(id, side, None, area).is_ok();
    if !fits(long) && fits(other) { other } else { long }
}

#[cfg(test)]
mod tests;
