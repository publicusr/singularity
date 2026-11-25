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
//! state whose floor plan no longer fits the window is refused with a notice.//!
//! The state and what input does to it are `mode`'s ([`Arrange`]: keys and pointer moves in, outcomes out, D89); this
//! file is how the App routes input to it and carries out what comes back (`App::arrange_did`, the one door the
//! layout changes through), and how Arrange mode is drawn.

pub mod history;
pub mod mode;

pub use mode::{Arrange, Out, Scene, Snapshot, TrayStop, done_blocked, fallback_pane, tray_layout};
#[cfg(test)]
pub use mode::{CHIP_MAX, TileDrag};
#[cfg(test)]
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};
#[cfg(test)]
use std::time::Instant;

use crate::app::{Action, App, Hit, Menu, MenuAct, Modal, NameFor, Naming, Target};
use crate::arrangement::{Arrangement, Refused};
use crate::layout::{Pane, PaneId, Widget, is_hole};
use crate::theme::Theme;
use crate::tiling::Side;
use crate::ui::{blank, text};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use std::time::Duration;

/// How long the bar offers "undo" after an edit.
pub const OFFER: Duration = Duration::from_secs(4);

/// The Tray strip's height: the rows the Panes give up while arranging (Q39).
pub const TRAY_H: u16 = 3;

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

/// The side after (`right`) or before `side` on the Split row, round at the ends.
pub fn next_side(side: Side, right: bool) -> Side {
    let i = SPLIT_SIDES.iter().position(|s| *s == side).unwrap_or(0);
    SPLIT_SIDES[(i + if right { 1 } else { 3 }) % 4]
}

/// What Arrange mode sees of `app` (its fields, so `app.arrange` can be borrowed beside it).
macro_rules! scene {
    ($app:expr) => {
        crate::arrange::Scene {
            layout: &$app.layout,
            focus: $app.focus,
            window: $app.window,
            body: $app.body,
            dividers: &$app.dividers,
            aspect: $app.cell_px.map(|(w, h)| h / w).unwrap_or(2.0),
            design: $app.shown.design(),
            keymap: &$app.keymap,
            mark: $app.shown.mark(),
        }
    };
}

/// Arrange mode's `method` with the App's Scene (and `args`), and what came out carried out.
macro_rules! arrange {
    ($app:expr, $m:ident $(, $arg:expr)*) => {{
        let o = {
            let sc = scene!($app);
            $app.arrange.$m(&sc $(, $arg)*)
        };
        $app.arrange_did(o)
    }};
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
        arrange!(self, layout_replaced, &tray);
        self.focus = was.and_then(|w| self.layout.pane_of(w));
        if self.arrange.is_on() && self.focus.is_none() {
            self.focus = fallback_pane(&self.layout);
        }
    }

    /// The Pane whose Widget is being moved (held by the mouse, or picked up with Enter): drawn highlighted.
    pub fn moving(&self) -> Option<PaneId> {
        self.arrange.moving()
    }

    /// Where the Tray strip is in Arrange mode: under the player bar, which is under the Panes (Q39).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn tray_rect(&self) -> Rect {
        self.rows(true)[3]
    }

    /// Where the player bar is: under the Panes.
    pub fn player_rect(&self) -> Rect {
        self.rows(self.arrange.is_on())[2]
    }

    /// The screen's rows (`ui::rows`) in this window, arranging or not; before the first frame, around a roomy guess.
    fn rows(&self, arranging: bool) -> [Rect; 5] {
        let w = if self.window.width > 0 { self.window } else { Rect::new(0, 0, 200, 60) };
        crate::ui::rows(w, arranging)
    }

    /// The Panes' area again, for the window's size and whether Arrange mode is on (it takes the Tray's rows), at once:
    /// a release in the same batch of input finds the Pane under it where the next frame draws it.
    fn relayout(&mut self) {
        self.relayout_body();
        self.tile_resized();
    }

    fn relayout_body(&mut self) {
        if self.window.width > 0 {
            self.body = crate::ui::rows(self.window, self.arrange.is_on())[1];
        }
    }

    /// Esc while a Widget is held: nothing changes.
    pub fn tile_cancel(&mut self) {
        self.arrange.drop_held();
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
        let sc = scene!(self);
        self.arrange.resized(&sc);
    }

    /// While a Widget is held over another Pane: that Pane, what its label says, and the chip's text.
    pub fn drag_marks(&self) -> Option<(Option<(PaneId, String)>, String)> {
        self.arrange.drag_marks(&self.layout, self.shown.design())
    }

    /// The chip by the pointer for `text` (`Arrange::chip_rect`).
    pub fn chip_rect(&self, area: Rect, text: &str) -> Option<Rect> {
        self.arrange.chip_rect(&scene!(self), area, text)
    }

    /// Where the label of target `id` (at `r`) goes this frame, clear of the chip (`Arrange::place_label`).
    pub fn place_label(&mut self, id: PaneId, r: Rect, label: &str, chip: Option<Rect>) -> Option<Rect> {
        self.arrange.place_label(id, r, label, chip)
    }

    /// The row target `id`'s label was drawn on last.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn label_row(&self, id: PaneId) -> Option<u16> {
        self.arrange.label_row(id)
    }

    /// The divider to light up: the one being dragged, else the one under the pointer.
    pub fn grip_lit(&self) -> Option<usize> {
        match self.arrange.divider_held() {
            Some(i) => Some(i),
            _ if self.arrange.tile.is_none() && matches!(self.modal(), None | Some(Modal::Arrange)) => self.grip_hover,
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

    // ── the one door: what Arrange mode's input did ───────────────────

    /// Carry out what Arrange mode said happened, in order.
    pub(crate) fn arrange_did(&mut self, outs: Vec<Out>) {
        for o in outs {
            match o {
                Out::Commit { tree, note } => {
                    if let Some(t) = tree {
                        self.layout = t;
                        self.want_widget_data();
                    }
                    let kept = self.edited_files();
                    if let Some(n) = note {
                        self.say(match kept {
                            Some(copy) => format!("{n} · kept as {copy}"),
                            None => n,
                        });
                    }
                }
                Out::Show(t) => self.layout = t,
                Out::Focus(f) => self.focus = f,
                Out::Say(s) => self.say(s),
                Out::Run(a) => self.run(a),
                Out::OpenWidgetPicker { pane, after_split } => {
                    self.open_widget_picker(pane);
                    if after_split && let Some(p) = self.widget_picker_mut() {
                        p.after_split = true;
                    }
                }
                Out::OpenTrayPicker => self.open_tray_picker(),
                Out::OpenPaneMenu { pane, at } => self.open_pane_menu(pane, Some(at)),
                Out::TitleClick { pane, at, widget } => {
                    // a click on the Widget's name: the Widget picker (D87)
                    if self.hit_at(at.0, at.1) == Some(Hit::TitleName(pane)) {
                        self.open_widget_picker(pane);
                    // a click elsewhere on the title: the focus moved at the press; the hole's Pane also changes its
                    // look
                    } else if is_hole(widget) {
                        self.run(Action::NextLook);
                    }
                }
                Out::Relayout => self.relayout_body(),
                Out::Restore { snap, said } => {
                    self.toast = None;
                    self.restore(snap);
                    if self.toast.is_none() {
                        self.say(said);
                    }
                }
                Out::AskNewName => {
                    if self.naming().is_none() {
                        self.open_popup(crate::app::Popup::Naming(Naming::new_design()));
                    }
                }
                Out::AskForkName => {
                    if self.naming().is_none() && let Some((name, problems)) = self.shown.done_prompt() {
                        if let Some(p) = problems {
                            self.say(p);
                        }
                        self.open_popup(crate::app::Popup::Naming(Naming::fork(name)));
                    }
                }
                Out::BuildNew => {
                    self.shown.build_new();
                    self.want_widget_data();
                }
                Out::Resume { snap, theme } => {
                    self.layout = snap.tree;
                    self.shown.resume(snap.shown);
                    self.focus = snap.focus.filter(|p| self.layout.contains(*p)).or_else(|| self.layout.pane_of(Widget::Home));
                    if theme != self.theme.id && self.set_theme(&theme) {
                        self.save_ui();
                    }
                    self.want_widget_data();
                }
            }
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn edited(&mut self) -> Option<String> {
        self.arrange.note_edit();
        self.edited_files()
    }

    /// What an edit (noted by Arrange mode already) does to the Design's files: none while a New design… is built.
    fn edited_files(&mut self) -> Option<String> {
        if self.arrange.building() {
            return None;
        }
        let mut o = self.shown.edited(&self.layout, &self.theme.id, self.arrange.is_on());
        let kept = o.kept_as.take();
        self.shown_did(o);
        kept
    }

    /// `tree` replaces the layout as the rest of the last edit (a split's pick): one undo takes both back.
    pub(crate) fn amend_layout(&mut self, tree: Arrangement) {
        let tray = self.layout.tray().to_vec();
        self.layout = tree;
        arrange!(self, amended, &tray);
        self.want_widget_data();
        self.edited_files();
    }

    /// `tree` replaces the layout as an edit (one undo step).
    pub(crate) fn commit_layout(&mut self, tree: Arrangement) {
        arrange!(self, commit, tree)
    }

    pub fn undo_layout(&mut self) {
        arrange!(self, undo, true)
    }

    pub fn redo_layout(&mut self) {
        arrange!(self, undo, false)
    }

    // ── Arrange mode ──────────────────────────────────────────────────

    /// Arrange mode on (the palette's and the Pane menu's "Arrange", a title drag, Esc in a split's picker).
    pub fn enter_arrange(&mut self) {
        arrange!(self, enter)
    }

    /// "New design…" (#42).
    pub fn new_design(&mut self) {
        let theme = self.theme.id.clone();
        arrange!(self, new_design, &theme)
    }

    /// Whether a New design… is being built.
    pub fn building(&self) -> bool {
        self.arrange.building()
    }

    /// The saved Design that was on screen when a New design… began (config.toml keeps naming it meanwhile).
    pub fn building_from(&self) -> Option<String> {
        self.arrange.building_from()
    }

    /// Esc on a New design… before anything is placed, or Discard: the Design that was on screen, as it was, with its
    /// Theme and its undo history.
    pub fn cancel_new_design(&mut self) {
        arrange!(self, cancel_new)
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
        arrange!(self, new_design_saved);
        true
    }

    /// The Tray's "+ Widget": the Widget picker, adding the pick to the Tray (#42).
    pub fn open_tray_picker(&mut self) {
        self.open_popup(crate::app::Popup::WidgetPicker(crate::panemenu::WidgetPicker { target: PaneId::UNSET, query: String::new(), sel: 0, after_split: false, tray: true }));
    }

    /// Before each frame: Home always has a Pane outside Arrange mode (`Arrange::homeless`).
    pub fn arrange_if_homeless(&mut self) {
        let previewing = self.picker().is_some();
        arrange!(self, homeless, previewing)
    }

    /// A Widget picked up with Enter is put back once anything else edits the layout (`Arrange::pick_up_check`).
    pub fn pick_up_check(&mut self) {
        self.arrange.pick_up_check(&self.layout);
    }

    /// What keeps Done unavailable: "Home still needs a Pane", while it waits in the Tray.
    pub fn done_blocked(&self) -> Option<String> {
        done_blocked(&self.layout, self.shown.design())
    }

    /// Done (or Esc): Arrange mode ends, once Home has a Pane.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn arrange_done(&mut self) {
        arrange!(self, done)
    }

    /// Keys in Arrange mode (`Arrange::key`).
    pub fn arrange_key(&mut self, k: KeyEvent) {
        arrange!(self, key, k)
    }

    /// The Tray strip scrolled `by` Widgets (the wheel, its ‹ ›).
    pub fn tray_scroll(&mut self, by: i32) {
        let sc = scene!(self);
        self.arrange.tray_scroll(&sc, by);
    }

    /// A mouse event in Arrange mode (`Arrange::pointer`), with what the last frame drew under it.
    pub(crate) fn arrange_pointer(&mut self, m: MouseEvent, coarse: bool) {
        let hit = self.hit_at(m.column, m.row);
        arrange!(self, pointer, m, hit, coarse)
    }

    /// A bare move or a fresh press while something is held: its release was never heard (`Arrange::unheard`).
    pub fn release_unheard(&mut self, kind: ratatui::crossterm::event::MouseEventKind) {
        arrange!(self, unheard, kind)
    }

    /// Mouse events while a Widget is held (`Arrange::tile_pointer`).
    pub fn arrange_tile_pointer(&mut self, m: MouseEvent) {
        arrange!(self, tile_pointer, m)
    }

    /// A press on a Pane's title bar outside Arrange mode (`Arrange::title_down`).
    pub fn title_down(&mut self, id: PaneId, x: u16, y: u16) {
        arrange!(self, title_down, id, x, y)
    }

    /// A press on divider `i`'s grip at (x, y): it follows the pointer until the release.
    pub(crate) fn grab_divider(&mut self, i: usize, x: u16, y: u16) {
        let sc = scene!(self);
        self.arrange.grab_divider(&sc, i, x, y);
    }

    /// The pointer at (x, y) with the button down while a divider is held.
    pub(crate) fn divider_to(&mut self, x: u16, y: u16) {
        arrange!(self, divider_to, x, y)
    }

    /// The button let go: a divider held is let go there (a changed layout is an edit).
    pub(crate) fn divider_up(&mut self) {
        arrange!(self, divider_up)
    }

    /// Esc on the naming prompt: keep the edit as the one unnamed "<Design> (edited)", replacing the last.
    pub fn keep_edited(&mut self) {
        let o = self.shown.keep_edited(&self.layout, &self.theme.id);
        self.shown_did(o);
    }

    /// "Reset design": the built-in Design's own layout again. A Design of the user's stays in the picker as it was
    /// last saved (its file isn't touched); the unnamed edit of a built-in one is dropped.
    pub fn reset_design(&mut self) {
        let before = Snapshot { tree: self.layout.clone(), shown: self.shown.mark(), focus: self.focus };
        let theme = self.theme.id.clone();
        let o = self.shown.reset();
        self.set_design(self.shown.design());
        self.set_theme(&theme);
        self.shown_did(o);
        if before.tree != self.layout || before.shown.file().is_some() {
            self.arrange.remember(before);
        }
    }

    // ── close and split ───────────────────────────────────────────────

    pub fn close_pane(&mut self, id: PaneId) {
        arrange!(self, close_pane, id)
    }

    /// What Pane `p` is called on screen (its Widget's title; an empty Pane is "an empty pane").
    pub fn name_of(&self, p: Pane) -> String {
        p.widget.map_or_else(|| "an empty pane".into(), |w| crate::ui::pane_name(self.shown.design(), w))
    }

    pub fn split_focused(&mut self, side: Side) {
        arrange!(self, split_pane, None, side)
    }

    /// A new empty Pane on `side` of Pane `target`, taking half its room, and the Widget picker on it at once (Esc
    /// leaves it empty). Refused with the reason where it would leave a Pane under the minimum.
    pub fn split_pane(&mut self, target: PaneId, side: Side) {
        arrange!(self, split_pane, Some(target), side)
    }

    /// The side a split of Pane `id` takes unless told otherwise (`Arrange::default_split`).
    pub fn default_split(&self, id: PaneId) -> Side {
        self.arrange.default_split(&scene!(self), id)
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
        if !self.arrange.is_on() {
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

/// The Tray strip (Arrange mode, under the player bar): what waits there, each with its ×, Clear Tray, and while a
/// Pane's Widget is held over it, lit with what a release does.
pub fn draw_tray(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    use ratatui::widgets::{Block, Borders, Widget as _};
    if r.height < 3 || r.width < 20 {
        return;
    }
    let lit = app.arrange.tray_lit();
    let border = if lit { th.s(th.accent).add_modifier(Modifier::BOLD) } else { th.s(th.border) };
    Block::default().borders(Borders::ALL).border_set(th.borders).border_style(border).title(ratatui::text::Span::styled(mode::TRAY_TITLE, th.s(th.accent))).style(th.base()).render(r, buf);
    let design = app.shown.design();
    let first = app.arrange.tray_first().unwrap_or(0);
    let l = tray_layout(app.layout.tray(), design, app.arrange.tray_reserve(design), r, first);
    app.arrange.tray_drawn_from(l.first);
    let tray = app.layout.tray().to_vec();
    // (a Widget aimed here with the keys shows where it lands, in its place)
    let landing = app.arrange.landing_in_tray();
    if tray.is_empty() && !landing {
        let s = "drop a Widget here to set it aside";
        text(buf, r.x + 2, r.y + 1, r.width.saturating_sub(4), s, th.s(th.dim));
    }
    let focus = app.arrange.tray_focus();
    let focus_on = |w: Widget| focus == Some(TrayStop::Widget(w));
    for (i, chip, close) in &l.items {
        let name = crate::ui::pane_name(design, tray[*i]);
        // (the one the keyboard's focus is on: in the focus colour)
        let st = if focus_on(tray[*i]) { th.chip(th.accent) } else { th.chip(th.accent2) };
        text(buf, chip.x, chip.y, chip.width, &mode::tray_chip(&name), st);
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
        text(buf, a.x, a.y, a.width, mode::TRAY_ADD, st);
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
    let held = app.arrange.carrying();
    // each with how long it stays when the line is short: `w` goes first, then the mouse's gesture, then undo / redo,
    // then the keys (the arrows and Enter last)
    let mut parts: Vec<(u8, String)> = if held {
        vec![(9, "←↑↓→ aim".into()), (9, "Enter drop".into()), (9, "Esc put back".into())]
    } else if app.arrange.split_waiting() {
        vec![(9, "←↑↓→ split that way".into())]
    } else {
        // (the focus on a Widget in the Tray: `x` removes it instead of closing a Pane)
        let on_widget = matches!(app.arrange.tray_focus(), Some(TrayStop::Widget(w)) if app.layout.tray().contains(&w));
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
    let Some((at, a, can)) = app.arrange.undo_offer() else { return };
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
