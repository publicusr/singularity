//! Arrange mode without an App (D89, #102): keys and pointer moves in, outcomes out. A `Bench` plays the App's part
//! as plainly as it can (it carries out the layout, focus and bar outcomes, keeps the Panes' area, and records what
//! it was asked to open), so these say what the module does; what the App makes of it (drawing, popups, the Design's
//! files) is `arrange/tests.rs`'s.

use super::*;
use crate::arrangement::outline;
use crate::design::DESIGNS;

const W: u16 = 160;
const H: u16 = 48;

struct Bench {
    a: Arrange,
    layout: Arrangement,
    focus: Option<PaneId>,
    window: Rect,
    body: Rect,
    keymap: Keymap,
    /// the bar's lines, in order
    said: Vec<String>,
    /// what the App was asked for besides the layout, the focus and the bar
    asked: Vec<Out>,
    /// edits committed
    commits: usize,
}

impl Bench {
    fn new() -> Bench {
        Bench::design(0)
    }

    fn design(d: usize) -> Bench {
        let layout: Arrangement = (DESIGNS[d].layout)().into();
        let window = Rect::new(0, 0, W, H);
        let focus = layout.pane_of(Widget::Home);
        Bench { a: Arrange::default(), layout, focus, window, body: crate::ui::rows(window, false)[1], keymap: Keymap::default(), said: vec![], asked: vec![], commits: 0 }
    }

    /// The dividers as a frame of this layout draws them.
    fn dividers(&self) -> Vec<Divider> {
        self.layout.compute(self.body).dividers
    }

    /// `f` with the Scene as it stands, and what came out carried out.
    fn run(&mut self, f: impl FnOnce(&mut Arrange, &Scene) -> Vec<Out>) {
        let dividers = self.dividers();
        let outs = {
            let sc = Scene { layout: &self.layout, focus: self.focus, window: self.window, body: self.body, dividers: &dividers, aspect: 2.0, design: 0, keymap: &self.keymap, mark: Default::default() };
            f(&mut self.a, &sc)
        };
        for o in outs {
            match o {
                Out::Commit { tree, note } => {
                    if let Some(t) = tree {
                        self.layout = t;
                    }
                    self.said.extend(note);
                    self.commits += 1;
                }
                Out::Show(t) => self.layout = t,
                Out::Focus(f) => self.focus = f,
                Out::Say(s) => self.said.push(s),
                Out::Relayout => self.body = crate::ui::rows(self.window, self.a.is_on())[1],
                Out::Restore { snap, said } => {
                    self.layout = snap.tree;
                    self.focus = snap.focus.filter(|p| self.layout.contains(*p)).or_else(|| fallback_pane(&self.layout));
                    self.said.push(said);
                }
                Out::Resume { snap, .. } => {
                    self.layout = snap.tree;
                    self.focus = snap.focus;
                }
                other => self.asked.push(other),
            }
        }
    }

    fn enter(&mut self) {
        self.run(|a, sc| a.enter(sc));
    }

    fn key(&mut self, code: KeyCode) {
        self.run(|a, sc| a.key(sc, KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn undo(&mut self, back: bool) {
        self.run(|a, sc| a.undo(sc, back));
    }

    /// A mouse event at (x, y) with `hit` drawn there, routed as the App routes it.
    fn mouse(&mut self, kind: MouseEventKind, x: u16, y: u16, hit: Option<Hit>) {
        let m = MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
        self.run(|a, sc| a.unheard(sc, m.kind));
        self.run(|a, sc| a.route(sc, m, hit));
    }

    fn press(&mut self, (x, y): (u16, u16), hit: Option<Hit>) {
        self.mouse(MouseEventKind::Down(MouseButton::Left), x, y, hit);
    }

    fn drag(&mut self, (x, y): (u16, u16)) {
        self.mouse(MouseEventKind::Drag(MouseButton::Left), x, y, None);
    }

    fn release(&mut self, (x, y): (u16, u16)) {
        self.mouse(MouseEventKind::Up(MouseButton::Left), x, y, None);
    }

    /// Pane `w` dragged by its title to (x, y) and let go there.
    fn title_drag(&mut self, w: Widget, to: (u16, u16)) {
        let id = self.pid(w);
        let r = self.rect(w);
        self.press((r.x + 3, r.y), Some(Hit::Title(id)));
        self.drag(to);
        self.release(to);
    }

    fn pid(&self, w: Widget) -> PaneId {
        self.layout.pane_of(w).unwrap_or_else(|| panic!("no Pane shows {w}"))
    }

    fn rect(&self, w: Widget) -> Rect {
        self.layout.compute(self.body).rect_of(w).unwrap_or_else(|| panic!("no {w}"))
    }

    fn focused(&self) -> Option<Widget> {
        self.focus.and_then(|p| self.layout.widget_in(p))
    }

    fn focus_on(&mut self, w: Widget) {
        self.focus = self.layout.pane_of(w);
    }

    fn tray_rect(&self) -> Rect {
        crate::ui::rows(self.window, true)[3]
    }
}

fn centre(r: Rect) -> (u16, u16) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn same_floor(a: &Arrangement, b: &Arrangement) -> bool {
    outline(a.plan()) == outline(b.plan())
}

// ── the mouse ─────────────────────────────────────────────────────────

#[test]
fn a_title_press_without_movement_is_a_click() {
    let mut b = Bench::new();
    let before = b.layout.clone();
    let q = b.pid(Widget::Queue);
    let r = b.rect(Widget::Queue);
    b.press((r.x + 3, r.y), Some(Hit::Title(q)));
    b.release((r.x + 4, r.y));
    assert_eq!(b.focused(), Some(Widget::Queue), "the press focused it");
    assert_eq!(b.layout, before);
    assert!(b.a.tile.is_none() && !b.a.is_on() && b.commits == 0);
    assert!(matches!(b.asked[..], [Out::TitleClick { pane, at, widget: Some(Widget::Queue) }] if pane == q && at == (r.x + 3, r.y)), "{:?}", b.asked);
}

#[test]
fn dragging_a_title_onto_another_pane_swaps_their_widgets_and_turns_arrange_mode_on() {
    let mut b = Bench::new();
    let (stream, queue) = (b.pid(Widget::Stream), b.pid(Widget::Queue));
    let before = b.layout.clone();
    let r = b.rect(Widget::Stream);
    b.press((r.x + 3, r.y), Some(Hit::Title(stream)));
    b.drag((r.x + 7, r.y + 3));
    assert!(b.a.is_on(), "the drag turned Arrange mode on");
    // where the Panes are now (they made room for the Tray once, as it started, Q39)
    let floor = b.layout.compute(b.body).panes;
    let q = centre(b.rect(Widget::Queue));
    for p in [(q.0 - 10, q.1), q] {
        b.drag(p);
        assert_eq!(b.layout.compute(b.body).panes, floor, "nothing moves during the drag");
        assert_eq!(b.layout, before, "nothing is committed yet");
    }
    assert_eq!(b.a.tile.as_ref().unwrap().over, Some(queue));
    assert_eq!(b.a.drag_marks(&b.layout, 0).unwrap().0.map(|(_, l)| l), Some("swap with queue".into()));
    b.release(q);
    assert!(b.a.tile.is_none() && b.a.is_on(), "a quick swap is one gesture: the mode stays on");
    assert_eq!((b.layout.widget_in(queue), b.layout.widget_in(stream)), (Some(Widget::Stream), Some(Widget::Queue)));
    assert!(same_floor(&b.layout, &before));
    assert_eq!(b.focused(), Some(Widget::Stream), "the focus goes with the Widget");
    assert_eq!(b.commits, 1);
}

#[test]
fn a_drop_on_an_empty_pane_fills_it_and_an_empty_panes_title_carries_nothing() {
    let mut b = Bench::new();
    b.layout = b.layout.split(b.pid(Widget::Queue), Side::Right, None, b.body).unwrap();
    b.enter();
    let empty = b.layout.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    let fft = b.pid(Widget::Fft);
    let target = centre(b.layout.compute(b.body).rect(empty).unwrap());
    b.title_drag(Widget::Fft, target);
    assert_eq!((b.layout.widget_in(empty), b.layout.widget_in(fft)), (Some(Widget::Fft), None));
    let before = b.layout.clone();
    let f = b.layout.compute(b.body).rect(fft).unwrap();
    b.press((f.x + 3, f.y), Some(Hit::Title(fft)));
    b.drag(target);
    assert!(!b.a.tile.as_ref().is_some_and(|t| t.live), "no drag");
    b.release(target);
    assert_eq!(b.layout, before);
}

#[test]
fn esc_a_drop_on_its_own_pane_or_off_the_panes_changes_nothing() {
    let mut b = Bench::new();
    b.enter();
    let before = b.layout.clone();
    let q = centre(b.rect(Widget::Queue));
    let s = b.rect(Widget::Stream);
    let id = b.pid(Widget::Stream);
    // Esc
    b.press((s.x + 3, s.y), Some(Hit::Title(id)));
    b.drag(q);
    assert!(b.a.tile.as_ref().unwrap().over.is_some());
    b.key(KeyCode::Esc);
    assert!(b.a.tile.is_none() && b.a.is_on(), "Esc lets it go, Arrange mode stays");
    b.release(q);
    // its own Pane, the top bar, the legend
    for at in [(s.x + 5, s.y + 4), (q.0, 0), (q.0, H - 1)] {
        b.press((s.x + 3, s.y), Some(Hit::Title(id)));
        b.drag(q);
        b.drag(at);
        b.release(at);
        assert_eq!(b.layout, before, "let go at {at:?}");
    }
    assert!(b.commits == 0 && !b.a.history.can_undo());
}

#[test]
fn every_drop_is_allowed_even_into_a_pane_smaller_than_the_widget_likes() {
    let mut b = Bench::new();
    b.enter();
    let before = b.layout.clone();
    let (well, fft) = (b.pid(Widget::Well), b.pid(Widget::Fft));
    b.title_drag(Widget::Well, centre(b.rect(Widget::Fft)));
    assert_eq!((b.layout.widget_in(fft), b.layout.widget_in(well)), (Some(Widget::Well), Some(Widget::Fft)));
    assert!(same_floor(&b.layout, &before) && b.layout.fits(b.body));
}

#[test]
fn a_drop_on_the_tray_sets_the_widget_aside_and_one_from_the_tray_takes_a_pane() {
    let mut b = Bench::new();
    let fft = b.pid(Widget::Fft);
    let r = b.rect(Widget::Fft);
    b.press((r.x + 3, r.y), Some(Hit::Title(fft)));
    b.drag((r.x + 8, r.y + 4));
    let t = b.tray_rect();
    b.drag((t.x + 30, t.y + 1));
    assert!(b.a.tray_lit(), "the Tray is lit");
    b.release((t.x + 30, t.y + 1));
    assert_eq!((b.layout.tray(), b.layout.widget_in(fft)), (&[Widget::Fft][..], None));
    // from the Tray onto the queue's Pane: it takes it, the queue waits in its place
    let q = b.pid(Widget::Queue);
    b.press((t.x + 4, t.y + 1), Some(Hit::TrayItem(Widget::Fft)));
    let to = centre(b.rect(Widget::Queue));
    b.drag(to);
    assert_eq!(b.a.tile.as_ref().unwrap().over, Some(q));
    b.release(to);
    assert_eq!((b.layout.widget_in(q), b.layout.tray()), (Some(Widget::Fft), &[Widget::Queue][..]));
}

#[test]
fn a_release_never_heard_lets_go_and_a_divider_is_let_go_where_it_is() {
    let mut b = Bench::new();
    let before = b.layout.clone();
    let r = b.rect(Widget::Stream);
    b.press((r.x + 3, r.y), Some(Hit::Title(b.pid(Widget::Stream))));
    b.drag(centre(b.rect(Widget::Queue)));
    // a bare move: the release went elsewhere
    b.mouse(MouseEventKind::Moved, 5, 5, None);
    assert!(b.a.tile.is_none() && b.layout == before);
    // a divider dragged, then a fresh press: it stays where it got to, as one edit
    let d = b.dividers()[0].clone();
    let at = (d.grip.x, d.grip.y);
    b.press(at, Some(Hit::Divider(0)));
    let to = if d.axis == Axis::Horizontal { (at.0 + 4, at.1) } else { (at.0, at.1 + 2) };
    b.drag(to);
    let moved = b.layout.clone();
    assert!(moved != before && b.a.moving_now());
    b.press((1, 1), None);
    assert!(b.a.divider.is_none() && !b.a.moving_now());
    assert_eq!((b.layout.clone(), b.commits), (moved, 1));
    b.undo(true);
    assert_eq!(b.layout, before, "one undo step");
}

#[test]
fn letting_go_of_a_divider_is_one_edit_and_the_keys_wait_meanwhile() {
    let mut b = Bench::new();
    b.enter();
    let before = b.layout.clone();
    let d = b.dividers()[0].clone();
    let at = (d.grip.x, d.grip.y);
    b.press(at, Some(Hit::Divider(0)));
    let to = if d.axis == Axis::Horizontal { (at.0 + 4, at.1) } else { (at.0, at.1 + 2) };
    b.drag(to);
    // an arrange key while the mouse holds the divider: nothing
    b.focus_on(Widget::Queue);
    b.key(KeyCode::Char('t'));
    assert!(b.a.divider.is_some() && b.layout.tray().is_empty());
    assert_eq!(b.commits, 0, "a divider on its way is no edit");
    b.release(to);
    assert!(b.layout != before && b.commits == 1);
    assert!(b.a.divider.is_none());
}

#[test]
fn a_close_button_closes_and_a_split_button_splits_and_asks_for_a_widget() {
    let mut b = Bench::new();
    b.enter();
    let fft = b.pid(Widget::Fft);
    let r = b.rect(Widget::Fft);
    b.press((r.right() - 3, r.y), Some(Hit::Close(fft)));
    assert!(b.layout.tray().contains(&Widget::Fft) && !b.layout.contains(fft));
    assert_eq!(b.said.last(), Some(&format!("Closed {}", crate::ui::pane_name(0, Widget::Fft))));
    let n = b.layout.panes().len();
    let q = b.pid(Widget::Queue);
    let r = b.rect(Widget::Queue);
    b.press((r.right() - 6, r.y), Some(Hit::SplitPane(q)));
    assert_eq!(b.layout.panes().len(), n + 1);
    let new = b.layout.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    assert_eq!(b.focus, Some(new));
    assert!(matches!(b.asked.last(), Some(Out::OpenWidgetPicker { pane, after_split: true }) if *pane == new));
    // Home's Pane never closes
    let h = b.pid(Widget::Home);
    b.run(|a, sc| a.close_pane(sc, h));
    assert!(b.layout.contains(h) && b.said.last().unwrap().contains("never closed"));
}

// ── the keyboard ──────────────────────────────────────────────────────

#[test]
fn arrows_move_the_focus_between_panes_and_nothing_else() {
    let mut b = Bench::new();
    b.enter();
    let before = b.layout.clone();
    b.focus_on(Widget::Queue);
    b.key(KeyCode::Up);
    assert_eq!(b.focused(), Some(Widget::Home));
    b.key(KeyCode::Down);
    assert_eq!(b.focused(), Some(Widget::Queue));
    b.focus_on(Widget::Lyrics);
    b.key(KeyCode::Right);
    assert_eq!(b.focused(), Some(Widget::Lyrics), "nothing that way: it stays");
    b.focus = None;
    b.key(KeyCode::Down);
    assert_eq!(b.focused(), Some(Widget::Queue), "nothing focused: from Home's Pane");
    assert_eq!(b.layout, before);
}

#[test]
fn enter_picks_up_arrows_aim_and_enter_swaps_as_one_undo_step() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Queue);
    let before = b.layout.clone();
    let (q, h) = (b.pid(Widget::Queue), b.pid(Widget::Home));
    b.key(KeyCode::Enter);
    assert_eq!(b.a.moving(), Some(q));
    assert!(b.a.carrying());
    b.key(KeyCode::Up);
    assert_eq!(b.a.drag_marks(&b.layout, 0).unwrap().0.map(|(p, _)| p), Some(h));
    assert_eq!(b.layout, before, "nothing moved");
    b.key(KeyCode::Enter);
    assert_eq!((b.layout.widget_in(h), b.layout.widget_in(q)), (Some(Widget::Queue), Some(Widget::Home)));
    assert!(b.a.tile.is_none() && b.focused() == Some(Widget::Queue) && b.a.is_on());
    b.undo(true);
    assert_eq!(b.layout, before);
}

#[test]
fn esc_puts_a_picked_up_widget_back_and_with_nothing_picked_up_is_done() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Queue);
    let before = b.layout.clone();
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    b.key(KeyCode::Esc);
    assert!(b.a.tile.is_none() && b.a.is_on());
    // aimed back at its own Pane: nothing lit, Enter puts it back
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    b.key(KeyCode::Down);
    assert!(b.a.drag_marks(&b.layout, 0).is_some_and(|(t, _)| t.is_none()));
    b.key(KeyCode::Enter);
    assert!(b.a.tile.is_none() && b.layout == before);
    b.key(KeyCode::Esc);
    assert!(!b.a.is_on() && b.asked.is_empty(), "nothing changed: Done, no name asked");
}

#[test]
fn t_sends_the_focused_widget_to_the_tray_and_an_empty_pane_has_nothing_to_pick_up() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Queue);
    let spot = b.pid(Widget::Queue);
    b.key(KeyCode::Char('t'));
    assert!(b.layout.tray().contains(&Widget::Queue) && b.layout.widget_in(spot).is_none());
    b.focus = Some(spot);
    b.key(KeyCode::Enter);
    assert!(b.a.tile.is_none(), "nothing to pick up");
    // Home picked up and dropped on the empty Pane below it fills it
    b.key(KeyCode::Up);
    let was = b.pid(Widget::Home);
    b.key(KeyCode::Enter);
    b.key(KeyCode::Down);
    assert_eq!(b.a.drag_marks(&b.layout, 0).unwrap().0.map(|(_, l)| l), Some("put here".into()));
    b.key(KeyCode::Enter);
    assert_eq!(b.layout.pane_of(Widget::Home), Some(spot));
    assert!(b.layout.widget_in(was).is_none() && b.focus == Some(spot));
}

#[test]
fn s_then_an_arrow_splits_x_closes_and_plus_minus_move_the_lines() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Queue);
    let size = |b: &Bench| (b.rect(Widget::Queue).width, b.rect(Widget::Queue).height);
    let (w0, h0) = size(&b);
    b.key(KeyCode::Char('-'));
    assert_eq!(size(&b), (w0 - 2, h0));
    b.key(KeyCode::Char('+'));
    b.key(KeyCode::Char('='));
    assert_eq!(size(&b), (w0 + 2, h0));
    b.key(KeyCode::Char('['));
    assert_eq!(size(&b), (w0 + 2, h0 - 1));
    b.key(KeyCode::Char(']'));
    b.key(KeyCode::Char(']'));
    assert_eq!(size(&b), (w0 + 2, h0 + 1));
    let (q, n) = (b.rect(Widget::Queue), b.layout.panes().len());
    b.key(KeyCode::Char('s'));
    assert!(b.a.split_waiting() && b.layout.panes().len() == n);
    b.key(KeyCode::Down);
    assert_eq!(b.layout.panes().len(), n + 1);
    let new = match b.asked.last() {
        Some(Out::OpenWidgetPicker { pane, after_split: true }) => *pane,
        o => panic!("{o:?}"),
    };
    let r = b.layout.compute(b.body).rect(new).unwrap();
    assert!(r.y > b.rect(Widget::Queue).y && r.x == q.x, "below the queue");
    // any other key after s ends the wait and does nothing else
    let before = b.layout.clone();
    b.focus_on(Widget::Fft);
    b.key(KeyCode::Char('s'));
    b.key(KeyCode::Char('x'));
    b.key(KeyCode::Left);
    assert_eq!(b.layout, before);
    b.focus_on(Widget::Fft);
    b.key(KeyCode::Char('x'));
    assert!(b.layout.pane_of(Widget::Fft).is_none() && b.layout.tray().contains(&Widget::Fft));
    b.focus_on(Widget::Home);
    b.key(KeyCode::Char('x'));
    assert!(b.layout.pane_of(Widget::Home).is_some() && b.said.last().unwrap().contains("never closed"));
}

#[test]
fn only_the_bare_keys_arrange_and_the_playback_keys_go_to_the_player() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Queue);
    let before = b.layout.clone();
    b.run(|a, sc| a.key(sc, KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT)));
    b.run(|a, sc| a.key(sc, KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT)));
    assert_eq!(b.layout, before);
    assert_eq!(b.focused(), Some(Widget::Queue));
    assert!(b.asked.iter().any(|o| matches!(o, Out::Run(Action::SeekForward))), "{:?}", b.asked);
    b.key(KeyCode::Char(' '));
    assert!(matches!(b.asked.last(), Some(Out::Run(Action::TogglePlay))), "{:?}", b.asked);
    // `m`, the key that came in, leaves as Esc does (Q70)
    b.key(KeyCode::Char('m'));
    assert!(!b.a.is_on());
}

#[test]
fn a_pick_up_ends_when_something_else_edits_the_layout() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Queue);
    b.key(KeyCode::Enter);
    assert!(b.a.carrying());
    let fft = b.pid(Widget::Fft);
    b.run(|a, sc| a.close_pane(sc, fft));
    b.key(KeyCode::Up);
    assert!(b.a.tile.is_none(), "put back: the layout it was picked up on is gone");
}

#[test]
fn the_keyboard_walks_the_tray_and_takes_widgets_out_of_it() {
    let mut b = Bench::new();
    b.enter();
    for w in [Widget::Fft, Widget::Loudness] {
        b.focus_on(w);
        b.key(KeyCode::Char('t'));
    }
    assert_eq!(b.layout.tray(), &[Widget::Fft, Widget::Loudness]);
    b.focus_on(Widget::Queue);
    for _ in 0..10 {
        if b.a.tray_focus().is_some() {
            break;
        }
        b.key(KeyCode::Down);
    }
    assert!(b.focus.is_none() && matches!(b.a.tray_focus(), Some(TrayStop::Widget(_))), "↓ reached the Tray");
    for _ in 0..5 {
        if b.a.tray_focus() != Some(TrayStop::Widget(Widget::Fft)) {
            b.key(KeyCode::Left);
        }
    }
    b.key(KeyCode::Right);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(Widget::Loudness)));
    b.key(KeyCode::Right);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Add), "+ Widget is the last stop");
    b.key(KeyCode::Enter);
    assert!(matches!(b.asked.last(), Some(Out::OpenTrayPicker)));
    // x removes the focused Widget; the focus moves to the one before
    b.key(KeyCode::Left);
    b.key(KeyCode::Char('x'));
    assert_eq!((b.layout.tray(), b.a.tray_focus()), (&[Widget::Fft][..], Some(TrayStop::Widget(Widget::Fft))));
    // Enter picks it up, ↑ aims into the Panes, Enter puts it there
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    let to = b.a.tile.as_ref().and_then(|t| t.over).expect("aimed at a Pane");
    b.key(KeyCode::Enter);
    assert_eq!(b.layout.widget_in(to), Some(Widget::Fft));
    assert!(b.focus == Some(to) && b.a.tray_focus().is_none() && b.a.tile.is_none());
}

// ── Done, undo and New design… ────────────────────────────────────────

#[test]
fn done_asks_for_a_name_only_after_a_change_and_never_while_home_waits_in_the_tray() {
    let mut b = Bench::new();
    b.enter();
    b.key(KeyCode::Esc);
    assert!(!b.a.is_on() && b.asked.is_empty());
    b.enter();
    b.focus_on(Widget::Home);
    b.key(KeyCode::Char('t'));
    b.run(|a, sc| a.done(sc));
    assert!(b.a.is_on(), "Home still needs a Pane");
    assert_eq!(done_blocked(&b.layout, 0).as_deref(), Some("Home still needs a Pane"));
    b.undo(true);
    b.key(KeyCode::Esc);
    assert!(!b.a.is_on() && b.asked.is_empty(), "changed and changed back: nothing to name");
    b.enter();
    b.focus_on(Widget::Fft);
    b.key(KeyCode::Char('t'));
    b.key(KeyCode::Esc);
    assert!(!b.a.is_on() && matches!(b.asked[..], [Out::AskForkName]));
}

#[test]
fn undo_and_redo_say_so_and_an_undo_that_would_not_fit_is_refused() {
    let mut b = Bench::new();
    b.undo(true);
    assert_eq!(b.said.last().map(String::as_str), Some("Nothing to undo"));
    b.enter();
    b.focus_on(Widget::Fft);
    let before = b.layout.clone();
    b.key(KeyCode::Char('t'));
    let after = b.layout.clone();
    assert!(b.a.undo_offer().is_some_and(|(_, a, can)| a == Action::UndoLayout && can));
    b.undo(true);
    assert!(b.layout == before && b.said.last().unwrap().starts_with("Undone"));
    b.undo(false);
    assert!(b.layout == after && b.said.last().unwrap().starts_with("Redone"));
    // a window too small for the layout undo goes back to
    b.undo(true);
    b.window = Rect::new(0, 0, 40, 14);
    b.body = crate::ui::rows(b.window, true)[1];
    b.undo(false);
    assert!(b.layout == before && b.said.last().unwrap().contains("doesn't fit"), "{:?}", b.said);
}

#[test]
fn a_new_design_starts_from_one_empty_pane_and_esc_before_anything_is_placed_goes_back() {
    let mut b = Bench::new();
    let before = b.layout.clone();
    b.focus_on(Widget::Fft);
    b.run(|a, sc| a.new_design(sc, "theme"));
    assert!(b.a.building() && b.a.is_on());
    assert_eq!((b.layout.panes().len(), b.layout.tray()), (1, &[Widget::Home][..]));
    assert!(matches!(b.asked[..], [Out::BuildNew]));
    // Done waits for Home; Esc with nothing placed: back to what was there, untouched
    b.key(KeyCode::Esc);
    assert!(!b.a.is_on() && !b.a.building() && b.layout == before && b.focused() == Some(Widget::Fft));
    // placed something: Esc is Done, which asks for the name
    b.run(|a, sc| a.new_design(sc, "theme"));
    b.key(KeyCode::Down);
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    b.key(KeyCode::Enter);
    assert!(b.layout.pane_of(Widget::Home).is_some(), "{:?}", b.layout);
    b.key(KeyCode::Esc);
    assert!(b.a.building() && matches!(b.asked.last(), Some(Out::AskNewName)));
    b.run(|a, sc| a.new_design_saved(sc));
    assert!(!b.a.is_on() && !b.a.history.can_undo(), "the new Design starts its own history");
}

// ── ADR 0005: the closed set of edits ─────────────────────────────────

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn cell(&mut self, r: Rect) -> (u16, u16) {
        (r.x + self.below(r.width as usize) as u16, r.y + self.below(r.height as usize) as u16)
    }
}

/// What a gesture or a key was, for what it may do to the floor plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    /// a title or a Tray Widget carried and let go, or Enter's drop: contents only
    Carry,
    /// a divider dragged, or `+` `-` `[` `]`: sizes only
    Line,
    /// `s` then an arrow, or a split button
    Split,
    /// `x`, or a ×
    Close,
    /// a Widget to the Tray (`t`), or out of it (`x` on it, its ×, Clear Tray): contents only
    Tray,
    /// arrows, Esc, Done, undo and redo: no edit of their own (undo and redo restore one)
    Other,
}

/// The floor plan's shape without sizes or contents.
fn shape(n: &Node) -> Node {
    match n {
        Node::Pane(p) => Node::Pane(Pane { id: p.id, widget: None }),
        Node::Split { axis, a, b, .. } => Node::Split { axis: *axis, first: crate::layout::Size::Cells(0), a: Box::new(shape(a)), b: Box::new(shape(b)) },
    }
}

/// Every Widget on screen or waiting in the Tray, sorted.
fn held(a: &Arrangement) -> Vec<Widget> {
    let mut v: Vec<Widget> = a.widgets().into_iter().chain(a.tray().iter().copied()).collect();
    v.sort_by_key(|w| w.name());
    v
}

/// ADR 0005: a drag only moves contents; the floor plan changes only by its own explicit edits (dragging a line,
/// splitting, closing); no Widget is lost but by removing it from the Tray. Random keys and pointer gestures (on
/// titles, the Tray, dividers, the split and close buttons) in every built-in Design, each edit checked against what
/// the input was.
#[test]
fn every_edit_is_one_of_adr_0005s_closed_set() {
    use KeyCode::*;
    let keys = [Left, Right, Up, Down, Enter, Esc, Char('t'), Char('s'), Char('x'), Char('+'), Char('-'), Char('['), Char(']'), Delete];
    // how many edits of each kind were checked
    let mut seen: std::collections::HashMap<Kind, usize> = Default::default();
    for d in 0..DESIGNS.len() {
        for seed in 1..=12u64 {
            let mut rng = Rng(seed * 0x9E37_79B9 + d as u64 + 1);
            let mut b = Bench::design(d);
            let mut carried = false;
            for step in 0..120 {
                let before = b.layout.clone();
                let commits = b.commits;
                let panes = b.layout.compute(b.body).panes;
                let pick = |rng: &mut Rng| panes[rng.below(panes.len())];
                let kind = match rng.below(9) {
                    0..=3 => {
                        if !b.a.is_on() {
                            b.enter();
                        }
                        let k = keys[rng.below(keys.len())];
                        let split = b.a.split_waiting() && matches!(k, Left | Right | Up | Down);
                        let carrying = b.a.carrying();
                        let on_tray = b.a.tray_focus().is_some();
                        b.key(k);
                        match k {
                            _ if split => Kind::Split,
                            Enter if carrying => Kind::Carry,
                            Char('t') => Kind::Tray,
                            Char('x') | Delete if on_tray => Kind::Tray,
                            Char('x') => Kind::Close,
                            Char('+' | '-' | '[' | ']') => Kind::Line,
                            _ => Kind::Other,
                        }
                    }
                    4 | 5 => {
                        // a title carried somewhere (onto a Pane, the Tray, off the Panes)
                        let (p, r) = pick(&mut rng);
                        b.press((r.x + 3, r.y), Some(Hit::Title(p.id)));
                        let area = if rng.below(4) == 0 { b.tray_rect() } else { b.window };
                        for _ in 0..rng.below(4) + 1 {
                            let c = rng.cell(area);
                            b.drag(c);
                        }
                        let c = rng.cell(area);
                        b.release(c);
                        carried = true;
                        Kind::Carry
                    }
                    6 => {
                        // a Widget from the Tray onto the Panes
                        let Some(&w) = b.layout.tray().get(rng.below(b.layout.tray().len())) else { continue };
                        if !b.a.is_on() {
                            b.enter();
                        }
                        let t = b.tray_rect();
                        b.press((t.x + 3, t.y + 1), Some(Hit::TrayItem(w)));
                        let c = rng.cell(b.body);
                        b.drag(c);
                        b.release(c);
                        Kind::Carry
                    }
                    7 => {
                        let ds = b.dividers();
                        if ds.is_empty() {
                            continue;
                        }
                        let i = rng.below(ds.len());
                        let g = ds[i].grip;
                        b.press((g.x, g.y), Some(Hit::Divider(i)));
                        for _ in 0..rng.below(3) + 1 {
                            let c = rng.cell(b.window);
                            b.drag(c);
                        }
                        let c = rng.cell(b.window);
                        b.release(c);
                        Kind::Line
                    }
                    _ => {
                        if !b.a.is_on() {
                            b.enter();
                        }
                        let (p, r) = pick(&mut rng);
                        let tray = b.layout.tray().to_vec();
                        match rng.below(4) {
                            0 => {
                                b.press((r.right() - 3, r.y), Some(Hit::Close(p.id)));
                                Kind::Close
                            }
                            1 => {
                                b.press((r.right() - 6, r.y), Some(Hit::SplitPane(p.id)));
                                Kind::Split
                            }
                            2 if !tray.is_empty() => {
                                b.press((0, 0), Some(Hit::TrayRemove(tray[rng.below(tray.len())])));
                                Kind::Tray
                            }
                            _ => {
                                b.press((0, 0), Some(Hit::ClearTray));
                                Kind::Tray
                            }
                        }
                    }
                };
                let what = format!("design {d} seed {seed} step {step} ({kind:?})");
                assert!(b.layout.fits(b.body) || !before.fits(b.body), "{what}: the floor plan isn't valid");
                assert!(b.a.tile.is_none() || b.a.carrying(), "{what}: a Widget left held by the pointer");
                assert!(b.a.divider.is_none(), "{what}: a divider left held");
                if b.layout == before {
                    continue;
                }
                assert!(b.commits == commits + 1, "{what}: the layout changed by {} edits", b.commits - commits);
                *seen.entry(kind).or_default() += 1;
                let (was, now) = (held(&before), held(&b.layout));
                match kind {
                    Kind::Carry => {
                        assert!(same_floor(&b.layout, &before), "{what}: a carried Widget changed the floor plan");
                        assert_eq!(now, was, "{what}: a carried Widget lost or added one");
                    }
                    Kind::Tray => {
                        assert!(same_floor(&b.layout, &before), "{what}: a Tray edit changed the floor plan");
                        assert!(now.iter().all(|w| was.contains(w)) && b.layout.widgets() == before.widgets() || now == was, "{what}: a Tray edit did more than set aside or remove");
                    }
                    Kind::Line => {
                        assert_eq!(shape(b.layout.plan()), shape(before.plan()), "{what}: a line moved more than a line");
                        assert_eq!((b.layout.widgets(), b.layout.tray()), (before.widgets(), before.tray()), "{what}: a line moved contents");
                    }
                    Kind::Split => {
                        assert_eq!(b.layout.panes().len(), before.panes().len() + 1, "{what}");
                        assert_eq!(now, was, "{what}: a split moved contents");
                        assert!(before.panes().iter().all(|p| b.layout.widget_in(p.id) == p.widget), "{what}: a split moved a Widget");
                    }
                    Kind::Close => {
                        assert_eq!(b.layout.panes().len() + 1, before.panes().len(), "{what}");
                        assert_eq!(now, was, "{what}: a close lost a Widget (it waits in the Tray)");
                    }
                    Kind::Other => panic!("{what}: {:?} became {:?} without an edit", before, b.layout),
                }
            }
            assert!(carried, "design {d} seed {seed}: no title was carried");
        }
    }
    for k in [Kind::Carry, Kind::Line, Kind::Split, Kind::Close, Kind::Tray] {
        assert!(seen.get(&k).is_some_and(|n| *n >= 20), "too few {k:?} edits checked: {seen:?}");
    }
}
