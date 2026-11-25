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
    /// the built-in Design (its Widgets' names)
    design: usize,
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
        Bench { a: Arrange::default(), design: d, layout, focus, window, body: crate::ui::rows(window, false)[1], keymap: Keymap::default(), said: vec![], asked: vec![], commits: 0 }
    }

    /// The dividers as a frame of this layout draws them.
    fn dividers(&self) -> Vec<Divider> {
        self.layout.compute(self.body).dividers
    }

    /// `f` with the Scene as it stands, and what came out carried out.
    fn run(&mut self, f: impl FnOnce(&mut Arrange, &Scene) -> Vec<Out>) {
        let outs = self.call(f);
        self.apply(outs);
    }

    /// `f` with the Scene as it stands.
    fn call(&mut self, f: impl FnOnce(&mut Arrange, &Scene) -> Vec<Out>) -> Vec<Out> {
        let dividers = self.dividers();
        let sc = Scene { layout: &self.layout, focus: self.focus, window: self.window, body: self.body, dividers: &dividers, aspect: 2.0, design: self.design, keymap: &self.keymap, mark: Default::default() };
        f(&mut self.a, &sc)
    }

    /// What came out, carried out as the App does (plainly).
    fn apply(&mut self, outs: Vec<Out>) {
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
                Out::Body(r) => self.body = r,
                Out::Restore { snap, said } => {
                    // (as `App::restore`: the layout replaced, so the Tray's focus settles, then the focus it had)
                    let (old, was) = (self.layout.tray().to_vec(), self.focused());
                    self.layout = snap.tree;
                    let more = self.call(|a, sc| a.layout_replaced(sc, &old, was));
                    self.apply(more);
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
        self.run(|a, sc| a.route(sc, m, hit, false));
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

// ── moved from the App's tests (D89): what Arrange mode itself does ───

impl Bench {
    /// The window takes a new size (the Panes' area follows, arranging or not).
    fn resize(&mut self, w: u16, h: u16) {
        self.window = Rect::new(0, 0, w, h);
        self.run(|a, sc| a.resized(sc));
    }

    fn split(&mut self, side: Side) {
        self.run(|a, sc| a.split_pane(sc, None, side));
    }

    fn close(&mut self, w: Widget) {
        let id = self.pid(w);
        self.run(|a, sc| a.close_pane(sc, id));
    }

    /// Where Pane `w`'s title can be grabbed, and what is drawn there.
    fn title(&self, w: Widget) -> ((u16, u16), Option<Hit>) {
        let r = self.rect(w);
        ((r.x + 3, r.y), Some(Hit::Title(self.pid(w))))
    }

    fn opened_picker(&self) -> bool {
        self.asked.iter().any(|o| matches!(o, Out::OpenWidgetPicker { .. }))
    }
}

#[test]
fn a_split_with_no_room_says_so() {
    let mut b = Bench::new();
    b.resize(50, 14);
    b.focus_on(Widget::Library);
    let before = b.layout.clone();
    // 50 columns: Singularity's four columns of 12 take 48 of them, no room for a fifth
    b.split(Side::Right);
    assert!(!b.opened_picker(), "no picker");
    assert_eq!(b.layout, before, "nothing changed");
    assert!(b.said.last().is_some_and(|t| t.contains("No room for a new Pane right of")), "{:?}", b.said);
}

#[test]
fn the_focus_always_lands_on_a_real_pane() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Home);
    b.key(KeyCode::Char('t'));
    assert!(b.layout.tray().contains(&Widget::Home));
    b.focus_on(Widget::Fft);
    b.key(KeyCode::Char('x'));
    assert!(b.focus.is_some_and(|p| b.layout.contains(p)), "{:?}", b.focus);
    for code in [KeyCode::Up, KeyCode::Down, KeyCode::Left] {
        b.key(code);
        assert!(b.focus.is_some_and(|p| b.layout.contains(p)));
    }
    // a focus left on a Pane that's gone (another path): the next key acts from a real one
    b.focus = Some(PaneId::UNSET);
    b.key(KeyCode::Enter);
    assert!(b.a.tile.as_ref().is_some_and(|t| b.layout.contains(t.pane.id)), "picked up from a real Pane");
    b.key(KeyCode::Esc);
    // undo back past the close: still a real Pane
    b.undo(true);
    b.undo(true);
    assert!(b.focus.is_some_and(|p| b.layout.contains(p)));
}

#[test]
fn the_pointer_leaves_a_picked_up_widget_alone_until_a_press() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Queue);
    let before = b.layout.clone();
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    let (x, y) = centre(b.rect(Widget::Lyrics));
    b.mouse(MouseEventKind::Moved, x, y, None);
    b.release((x, y));
    assert!(b.a.carrying(), "still picked up");
    assert!(!b.a.moving_now(), "nothing follows the pointer");
    b.press((x, y), None);
    assert!(b.a.tile.is_none() && b.focused() == Some(Widget::Lyrics));
    assert_eq!(b.layout, before);
}

#[test]
fn a_short_wide_pane_is_swapped_with_wherever_the_pointer_is_on_it() {
    // the instruments in a short row under Home: fft dropped anywhere on loudness swaps the two
    use crate::layout::{Axis::*, Size::*};
    let mut b = Bench::new();
    let s = Node::pane;
    let row = Node::split(Horizontal, Ratio(0.5), s(Widget::Fft), s(Widget::Loudness));
    b.layout = Node::split(Horizontal, Cells(30), s(Widget::Library), Node::split(Vertical, Ratio(0.7), Node::split(Vertical, Ratio(0.6), s(Widget::Home), s(Widget::Queue)), Node::split(Horizontal, Ratio(0.8), row, s(Widget::Lyrics)))).into();
    // (arranging already, so the Panes are where they stay for every drag)
    b.enter();
    let before = b.layout.clone();
    let l = b.rect(Widget::Loudness);
    let loudness = b.pid(Widget::Loudness);
    for p in [(l.x, l.y), (l.x + l.width / 2, l.bottom() - 1), (l.right() - 1, l.y + l.height / 2)] {
        let (at, hit) = b.title(Widget::Fft);
        b.press(at, hit);
        b.drag(p);
        assert_eq!(b.a.tile.as_ref().unwrap().over, Some(loudness), "{p:?} in {l:?}");
        b.key(KeyCode::Esc);
        b.release(p);
    }
    b.title_drag(Widget::Fft, (l.x + l.width / 2, l.y));
    assert_eq!(b.layout, before.swap(b.pid(Widget::Loudness), b.pid(Widget::Fft)).unwrap());
}

#[test]
fn a_picked_up_widget_aimed_down_onto_the_tray_waits_there() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Queue);
    let spot = b.pid(Widget::Queue);
    b.key(KeyCode::Enter);
    b.key(KeyCode::Down);
    assert!(b.a.tile.as_ref().is_some_and(|t| t.tray && t.over.is_none()) && b.a.tray_lit());
    // ↑ aims back into the Panes, ↓ at the Tray again
    b.key(KeyCode::Up);
    assert!(b.a.tile.as_ref().is_some_and(|t| !t.tray));
    b.key(KeyCode::Down);
    b.key(KeyCode::Enter);
    assert!(b.layout.tray().contains(&Widget::Queue) && b.layout.widget_in(spot).is_none());
    assert!(b.a.tile.is_none() && b.focus == Some(spot));
}

#[test]
fn the_x_on_one_widget_in_the_tray_removes_it_and_undo_redo_cover_tray_edits() {
    let mut b = Bench::new();
    let start = b.layout.clone();
    let t = b.tray_rect();
    let tray = (t.x + t.width / 2, t.y + 1);
    b.title_drag(Widget::Stream, tray);
    b.title_drag(Widget::Fft, tray);
    let two = b.layout.clone();
    b.press((0, 0), Some(Hit::TrayRemove(Widget::Stream)));
    // a second click in the same burst (no frame between) lands on the same spot: nothing more goes
    b.press((0, 0), Some(Hit::TrayRemove(Widget::Stream)));
    assert_eq!(b.layout.tray(), [Widget::Fft], "only that one went");
    assert!(!b.layout.has(Widget::Stream));
    b.undo(true);
    assert_eq!(b.layout, two);
    b.undo(true);
    b.undo(true);
    assert_eq!(b.layout, start);
    b.undo(false);
    assert_eq!(b.layout.tray(), [Widget::Stream]);
    assert!(b.a.is_on(), "undo and redo work in Arrange mode");
}

#[test]
fn closing_sends_the_widget_to_the_tray_and_clear_tray_never_removes_home() {
    let mut b = Bench::new();
    b.enter();
    let fft = b.pid(Widget::Fft);
    b.press((0, 0), Some(Hit::Close(fft)));
    assert!(!b.layout.contains(fft), "the neighbour took the room");
    assert_eq!(b.layout.tray(), [Widget::Fft]);
    // Home into the Tray: Done is unavailable
    let t = b.tray_rect();
    b.title_drag(Widget::Home, (t.x + t.width / 2, t.y + 1));
    assert_eq!(b.layout.tray(), [Widget::Fft, Widget::Home]);
    b.key(KeyCode::Esc);
    b.press((0, 0), Some(Hit::ArrangeDone));
    assert!(b.a.is_on(), "neither Esc nor Done ends it");
    // Clear Tray removes the rest, never Home; nor does Home's own removal
    b.press((0, 0), Some(Hit::TrayRemove(Widget::Home)));
    assert_eq!(b.layout.tray(), [Widget::Fft, Widget::Home]);
    b.press((0, 0), Some(Hit::ClearTray));
    assert_eq!(b.layout.tray(), [Widget::Home]);
    // Home back into its (empty) Pane: Done ends Arrange mode
    let empty = b.layout.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    let to = centre(b.layout.compute(b.body).rect(empty).unwrap());
    b.press((t.x + 3, t.y + 1), Some(Hit::TrayItem(Widget::Home)));
    b.drag(to);
    b.release(to);
    b.key(KeyCode::Esc);
    assert!(!b.a.is_on() && b.layout.has(Widget::Home));
}

#[test]
fn a_widget_from_the_tray_fills_an_empty_pane_or_swaps_with_what_is_there() {
    let mut b = Bench::new();
    let (stream, queue) = (b.pid(Widget::Stream), b.pid(Widget::Queue));
    let t = b.tray_rect();
    let tray = (t.x + t.width / 2, t.y + 1);
    b.title_drag(Widget::Stream, tray);
    // onto the queue: the queue goes into the Tray in its place
    let to = centre(b.rect(Widget::Queue));
    let from = (t.x + 4, t.y + 1);
    b.press(from, Some(Hit::TrayItem(Widget::Stream)));
    b.drag((from.0 + 3, from.1 - 3));
    b.drag(to);
    assert_eq!(b.a.drag_marks(&b.layout, 0).unwrap().0.map(|(_, l)| l), Some("swap with queue".into()));
    b.release(to);
    assert_eq!((b.layout.widget_in(queue), b.layout.tray()), (Some(Widget::Stream), &[Widget::Queue][..]));
    // onto the empty Pane: it fills it, and the Tray is empty
    let to = centre(b.layout.compute(b.body).rect(stream).unwrap());
    b.press(from, Some(Hit::TrayItem(Widget::Queue)));
    b.drag(to);
    b.release(to);
    assert_eq!(b.layout.widget_in(stream), Some(Widget::Queue));
    assert!(b.layout.tray().is_empty());
    // a click on a Widget in the Tray, or a drop back on the Tray, changes nothing
    b.title_drag(Widget::Fft, tray);
    let now = b.layout.clone();
    b.press(from, Some(Hit::TrayItem(Widget::Fft)));
    b.release(from);
    b.press(from, Some(Hit::TrayItem(Widget::Fft)));
    b.drag((from.0 + 20, from.1));
    b.release((from.0 + 20, from.1));
    assert_eq!(b.layout, now);
    assert!(!b.asked.iter().any(|o| matches!(o, Out::TitleClick { .. })), "a Tray click is no title click");
}

/// ↓ onto the Tray from the queue, then ← (or →) until the focus is on `w`.
fn to_tray_widget(b: &mut Bench, w: Widget) {
    b.focus_on(Widget::Queue);
    for _ in 0..10 {
        if b.a.tray_focus().is_some() {
            break;
        }
        b.key(KeyCode::Down);
    }
    for _ in 0..6 {
        if b.a.tray_focus() == Some(TrayStop::Widget(w)) {
            return;
        }
        let i = b.layout.tray().iter().position(|x| *x == w).unwrap();
        let at = match b.a.tray_focus() {
            Some(TrayStop::Widget(x)) => b.layout.tray().iter().position(|y| *y == x).unwrap(),
            _ => usize::MAX,
        };
        b.key(if i < at { KeyCode::Left } else { KeyCode::Right });
    }
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(w)), "the focus reached it");
}

#[test]
fn x_delete_or_backspace_removes_the_focused_widget_from_the_tray() {
    let mut b = Bench::new();
    b.enter();
    for w in [Widget::Fft, Widget::Loudness, Widget::Lyrics] {
        b.focus_on(w);
        b.key(KeyCode::Char('t'));
    }
    assert_eq!(b.layout.tray(), &[Widget::Fft, Widget::Loudness, Widget::Lyrics]);
    to_tray_widget(&mut b, Widget::Loudness);
    // `x`: gone, the focus on the next one
    b.key(KeyCode::Char('x'));
    assert_eq!(b.layout.tray(), &[Widget::Fft, Widget::Lyrics]);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(Widget::Lyrics)));
    // Delete on the last one: the one before
    b.key(KeyCode::Delete);
    assert_eq!(b.layout.tray(), &[Widget::Fft]);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(Widget::Fft)));
    // Backspace on the only one: back to the Panes
    b.key(KeyCode::Backspace);
    assert!(b.layout.tray().is_empty() && b.a.tray_focus().is_none() && b.focused().is_some());
    // an undo brings it back, as for the `×`
    b.undo(true);
    assert_eq!(b.layout.tray(), &[Widget::Fft]);
    // on a Pane, Delete and Backspace do nothing
    let before = b.layout.clone();
    b.key(KeyCode::Delete);
    b.key(KeyCode::Backspace);
    assert_eq!(b.layout, before);
    assert!(b.a.is_on());
    // Home cannot be removed (it has no `×`): refused with the words a removal of it gets, the focus stays on it
    let mut b = Bench::new();
    b.run(|a, sc| a.new_design(sc, "theme"));
    b.key(KeyCode::Down);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(Widget::Home)));
    for k in [KeyCode::Char('x'), KeyCode::Delete, KeyCode::Backspace] {
        b.said.clear();
        b.key(k);
        assert_eq!(b.layout.tray(), &[Widget::Home]);
        assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(Widget::Home)));
        assert!(b.said.last().is_some_and(|t| t.contains("never removed")), "{:?}", b.said);
    }
}

#[test]
fn the_trays_focus_follows_an_undo_or_the_mouse_taking_its_widget_away() {
    let mut b = Bench::new();
    b.enter();
    for w in [Widget::Fft, Widget::Loudness] {
        b.focus_on(w);
        b.key(KeyCode::Char('t'));
    }
    to_tray_widget(&mut b, Widget::Loudness);
    // undo of the last `t`: Loudness is back in its Pane, the focus on the one before it in the Tray
    b.undo(true);
    assert_eq!(b.layout.tray(), &[Widget::Fft]);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(Widget::Fft)));
    let panes = b.layout.panes().len();
    b.key(KeyCode::Char('x'));
    assert!(b.layout.tray().is_empty() && b.layout.panes().len() == panes, "x removed it from the Tray, no Pane closed");
    // the removal undone, then Fft's own `t` undone with the focus on it: the focus is back in the Panes
    b.undo(true);
    assert_eq!(b.layout.tray(), &[Widget::Fft]);
    to_tray_widget(&mut b, Widget::Fft);
    b.undo(true);
    assert!(b.layout.tray().is_empty() && b.a.tray_focus().is_none() && b.focused().is_some());
    // the mouse's × on the focused chip: the focus goes on to what's left (or back to the Panes)
    let mut b = Bench::new();
    b.enter();
    for w in [Widget::Fft, Widget::Loudness] {
        b.focus_on(w);
        b.key(KeyCode::Char('t'));
    }
    to_tray_widget(&mut b, Widget::Fft);
    b.run(|a, sc| a.route(sc, MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 0, row: 0, modifiers: KeyModifiers::NONE }, Some(Hit::TrayRemove(Widget::Fft)), false));
    assert_eq!(b.layout.tray(), &[Widget::Loudness]);
    assert!(b.a.tray_focus().is_none(), "a press ends the keyboard's focus in the Tray");
}

#[test]
fn the_keyboard_reaches_plus_widget_on_the_tray_even_when_it_is_empty() {
    let mut b = Bench::new();
    b.run(|a, sc| a.new_design(sc, "theme"));
    // Home waits: ↓ lands on it, → goes on to "+ Widget"
    b.key(KeyCode::Down);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(Widget::Home)));
    b.key(KeyCode::Right);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Add));
    b.key(KeyCode::Right);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Add), "nothing after it");
    b.key(KeyCode::Left);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(Widget::Home)));
    // Home into the Pane: Enter, ↑, Enter; the Tray is empty now
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    b.key(KeyCode::Enter);
    assert!(b.layout.tray().is_empty() && b.focused() == Some(Widget::Home));
    // ↓ from the bottom Pane: "+ Widget" (the only stop); Enter opens the picker for the Tray
    b.key(KeyCode::Down);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Add));
    b.key(KeyCode::Enter);
    assert!(matches!(b.asked.last(), Some(Out::OpenTrayPicker)));
    // a pick joins the Tray (the App commits it): the focus stays on "+ Widget"
    let tree = b.layout.add_to_tray(Widget::Queue).unwrap();
    b.run(|a, sc| a.commit(sc, tree));
    assert_eq!((b.layout.tray(), b.a.tray_focus()), (&[Widget::Queue][..], Some(TrayStop::Add)));
    // ↑: back to the Pane above
    b.key(KeyCode::Up);
    assert!(b.a.tray_focus().is_none() && b.focused() == Some(Widget::Home));
}

#[test]
fn the_keyboard_takes_widgets_out_of_the_tray_and_the_panes_keys_do_nothing_there() {
    let mut b = Bench::new();
    b.enter();
    // an empty Tray: ↓ from a bottom Pane goes to its "+ Widget" (#42), ↑ back
    b.focus_on(Widget::Queue);
    b.key(KeyCode::Down);
    assert!(b.focus.is_none() && b.a.tray_focus() == Some(TrayStop::Add));
    b.key(KeyCode::Up);
    assert!(b.focused().is_some() && b.a.tray_focus().is_none());
    for w in [Widget::Fft, Widget::Loudness] {
        b.focus_on(w);
        b.key(KeyCode::Char('t'));
    }
    to_tray_widget(&mut b, Widget::Fft);
    b.key(KeyCode::Left);
    assert_eq!(b.a.tray_focus(), Some(TrayStop::Widget(Widget::Fft)), "nothing before it");
    b.key(KeyCode::Right);
    // the keys for a Pane do nothing here
    let before = b.layout.clone();
    for c in ['t', 's', '+', '-'] {
        b.key(KeyCode::Char(c));
    }
    assert_eq!(b.layout, before);
    // Enter picks it up, Esc puts it back and the focus stays in the Tray
    b.key(KeyCode::Enter);
    assert!(b.a.carrying());
    b.key(KeyCode::Esc);
    assert!(b.a.tile.is_none() && b.a.tray_focus() == Some(TrayStop::Widget(Widget::Loudness)) && b.a.is_on());
    // picked up again, ↑ aims at the Pane above it, Enter puts it there: that Pane's Widget goes into the Tray
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    let to = b.a.tile.as_ref().and_then(|t| t.over).expect("aimed at a Pane");
    let was = b.layout.widget_in(to);
    b.key(KeyCode::Enter);
    assert_eq!(b.layout.widget_in(to), Some(Widget::Loudness));
    assert!(was.is_none_or(|w| b.layout.tray().contains(&w)) && !b.layout.tray().contains(&Widget::Loudness));
    // ↓ to the Tray and ↑ back: the focus is on a bottom Pane again
    b.key(KeyCode::Down);
    assert!(b.a.tray_focus().is_some());
    b.key(KeyCode::Up);
    assert!(b.a.tray_focus().is_none() && b.focus.is_some_and(|p| b.layout.contains(p)));
}

#[test]
fn only_the_bare_keys_arrange_and_a_press_ends_s_waiting() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Fft);
    let before = b.layout.clone();
    for (c, m) in [(KeyCode::Char('x'), KeyModifiers::CONTROL), (KeyCode::Char('t'), KeyModifiers::ALT), (KeyCode::Enter, KeyModifiers::CONTROL), (KeyCode::Char('='), KeyModifiers::CONTROL)] {
        b.run(|a, sc| a.key(sc, KeyEvent::new(c, m)));
    }
    assert_eq!(b.layout, before);
    assert!(b.a.tile.is_none());
    // `+` as the terminal may send it, with its Shift: grows
    b.focus_on(Widget::Queue);
    b.run(|a, sc| a.key(sc, KeyEvent::new(KeyCode::Char('+'), KeyModifiers::SHIFT)));
    assert_ne!(b.layout, before);
    // s, then a click: the wait is over, and → moves the focus instead of splitting
    let n = b.layout.panes().len();
    b.key(KeyCode::Char('s'));
    let c = centre(b.rect(Widget::Queue));
    b.press(c, None);
    b.release(c);
    b.key(KeyCode::Right);
    assert_eq!(b.layout.panes().len(), n);
    assert!(!b.opened_picker());
}

#[test]
fn an_undo_that_would_not_fit_the_window_is_refused() {
    // Home split twice: three columns where Singularity has one; one closed, the window shrinks, and the undo back
    // to three wouldn't fit
    let mut b = Bench::new();
    let home = b.pid(Widget::Home);
    b.layout = b.layout.split(home, Side::Right, Some(Widget::Scope), b.body).unwrap().split(home, Side::Right, Some(Widget::Rain), b.body).unwrap();
    let three = b.layout.clone();
    b.close(Widget::Rain);
    let two = b.layout.clone();
    let (w, h) = (50..160u16).map(|w| (w, 30u16)).find(|&(w, h)| {
        let body = Rect::new(0, 1, w, h - 5);
        !three.fits(body) && two.fits(body)
    }).expect("such a size");
    b.resize(w, h);
    b.undo(true);
    assert_eq!(b.layout, two);
    assert!(b.said.last().is_some_and(|t| t.contains("doesn't fit this window")), "{:?}", b.said);
    // back at a size it fits, it goes
    b.resize(W, H);
    b.undo(true);
    assert_eq!(b.layout, three);
}

#[test]
fn a_drag_that_turns_arrange_mode_on_drops_on_the_pane_where_it_now_is() {
    let mut tried = 0;
    for (d, design) in DESIGNS.iter().enumerate() {
        let mut b = Bench::design(d);
        if !b.layout.has(Widget::Queue) || b.layout.unplaced().is_some() {
            continue;
        }
        tried += 1;
        let before = b.layout.clone();
        let held = before.widgets().into_iter().find(|w| *w != Widget::Queue).unwrap();
        let old = b.rect(Widget::Queue);
        let (at, hit) = b.title(held);
        b.press(at, hit);
        b.drag((at.0 + 4, at.1 + 2));
        assert!(b.a.is_on(), "{}: the drag turned Arrange mode on", design.id);
        assert_eq!(b.body, crate::ui::rows(b.window, true)[1], "{}: the Panes' area is shorter at once", design.id);
        let new = b.rect(Widget::Queue);
        assert!(new.height < old.height || new.y < old.y, "{}: the queue moved up or shrank", design.id);
        // a cell of the queue now that wasn't before, if any; else its middle
        let inside = |r: Rect, (cx, cy): (u16, u16)| cx >= r.x && cx < r.right() && cy >= r.y && cy < r.bottom();
        let spot = (new.y..new.bottom()).map(|cy| (new.x + new.width / 2, cy)).find(|c| !inside(old, *c)).unwrap_or(centre(new));
        b.drag(spot);
        b.release(spot);
        let want = before.swap(before.pane_of(held).unwrap(), before.pane_of(Widget::Queue).unwrap()).unwrap();
        assert_eq!(b.layout, want, "{}: released at {spot:?} on the queue's new {new:?}", design.id);
    }
    assert!(tried >= 3, "Designs with a queue: {tried}");
}

#[test]
fn a_pick_up_ends_when_something_else_edits_the_layout_even_with_no_frame_between() {
    let mut b = Bench::new();
    b.enter();
    // Close pane on the focused (picked-up) Pane: the pick-up is gone
    b.focus_on(Widget::Fft);
    b.key(KeyCode::Enter);
    b.close(Widget::Fft);
    b.run(|a, sc| {
        a.pick_up_check(sc.layout);
        vec![]
    });
    assert!(b.a.tile.is_none() && b.a.drag_marks(&b.layout, 0).is_none());
    // a pick elsewhere (the Widget picker's, committed by the App): gone too
    b.focus_on(Widget::Loudness);
    b.key(KeyCode::Enter);
    let tree = b.layout.replace(b.pid(Widget::Loudness), Widget::Scope).unwrap();
    b.run(|a, sc| a.commit(sc, tree));
    b.key(KeyCode::Up);
    assert!(b.a.tile.is_none(), "the next key found the pick-up gone");
    // an edit with no frame in between: the next key finds the pick-up gone, and drops nothing
    b.key(KeyCode::Esc);
    b.focus_on(Widget::Queue);
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    b.close(Widget::Stream);
    let after = b.layout.clone();
    b.key(KeyCode::Enter);
    assert_eq!(b.layout, after, "the old pick-up didn't drop");
    assert!(b.a.tile.as_ref().is_some_and(|t| t.on.as_ref() == Some(&b.layout)), "Enter picked up afresh");
}

#[test]
fn m_leaves_arrange_mode_as_esc_does() {
    let mut b = Bench::new();
    b.enter();
    b.key(KeyCode::Char('m'));
    assert!(!b.a.is_on() && b.asked.is_empty(), "nothing changed: Done, no prompt");
    // a Widget picked up: put back, still arranging
    b.enter();
    b.focus_on(Widget::Queue);
    let before = b.layout.clone();
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    b.key(KeyCode::Char('m'));
    assert!(b.a.tile.is_none() && b.a.is_on() && b.layout == before);
    // `s` waiting: the wait ends, nothing else
    b.key(KeyCode::Char('s'));
    b.key(KeyCode::Char('m'));
    assert!(b.a.is_on() && !b.a.split_waiting());
    // from the Tray's focus too
    b.key(KeyCode::Down);
    assert!(b.a.tray_focus().is_some());
    b.key(KeyCode::Char('m'));
    assert!(!b.a.is_on());
    // with a change: Done asks for its name, as Esc does; the `arrange` action does the same
    b.enter();
    b.focus_on(Widget::Fft);
    b.key(KeyCode::Char('x'));
    b.run(|a, sc| a.arrange_action(sc));
    assert!(!b.a.is_on() && matches!(b.asked.last(), Some(Out::AskForkName)));
}

#[test]
fn w_opens_the_trays_widget_picker_anywhere_in_arrange_mode_but_not_while_carrying() {
    let mut b = Bench::new();
    b.enter();
    b.focus_on(Widget::Fft);
    b.key(KeyCode::Char('x'));
    b.focus_on(Widget::Queue);
    b.key(KeyCode::Char('w'));
    assert!(matches!(b.asked.pop(), Some(Out::OpenTrayPicker)));
    b.key(KeyCode::Down);
    assert!(b.a.tray_focus().is_some());
    b.key(KeyCode::Char('w'));
    assert!(matches!(b.asked.pop(), Some(Out::OpenTrayPicker)));
    b.focus_on(Widget::Queue);
    b.key(KeyCode::Up);
    b.key(KeyCode::Enter);
    assert!(b.a.carrying());
    b.key(KeyCode::Char('w'));
    assert!(!b.asked.iter().any(|o| matches!(o, Out::OpenTrayPicker)) && b.a.tile.is_some());
}

#[test]
fn every_kind_of_edit_is_one_undo_step() {
    let mut b = Bench::new();
    let start = b.layout.clone();
    let mut trees = vec![start.clone()];
    // close
    b.close(Widget::Fft);
    trees.push(b.layout.clone());
    // split (an empty Pane) and the Widget picked for it (amended in): one step
    b.focus_on(Widget::Loudness);
    b.split(Side::Right);
    let new = b.layout.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    let tree = b.layout.replace(new, Widget::Scope).unwrap();
    b.run(|a, sc| a.amend(sc, tree));
    trees.push(b.layout.clone());
    // Arrange mode's keys: a pick-up dropped, a size step, to the Tray
    b.focus_on(Widget::Queue);
    b.enter();
    b.key(KeyCode::Enter);
    b.key(KeyCode::Up);
    b.key(KeyCode::Enter);
    trees.push(b.layout.clone());
    b.key(KeyCode::Char('-'));
    trees.push(b.layout.clone());
    b.key(KeyCode::Char('t'));
    trees.push(b.layout.clone());
    b.key(KeyCode::Esc);
    // a divider dragged and let go
    let g = b.dividers()[0].grip;
    b.press((g.x, g.y + 5), Some(Hit::Divider(0)));
    b.drag((g.x + 6, g.y + 5));
    b.drag((g.x + 8, g.y + 5));
    b.release((g.x + 8, g.y + 5));
    trees.push(b.layout.clone());
    for (n, t) in trees.windows(2).rev().enumerate() {
        assert_ne!(t[0], t[1], "step {n} changed something");
        b.undo(true);
        assert_eq!(b.layout, t[0], "undo {n}");
    }
    b.undo(true);
    assert_eq!(b.said.last().map(String::as_str), Some("Nothing to undo"));
    for t in &trees[1..] {
        b.undo(false);
        assert_eq!(&b.layout, t);
    }
}

#[test]
fn a_fast_sweep_highlights_each_pane_at_once_and_drops_on_the_last() {
    // (arranging already, so the Panes are where they stay for the whole drag)
    let arranging = || {
        let mut b = Bench::new();
        b.enter();
        b
    };
    let mut b = arranging();
    let before = b.layout.clone();
    let floor = b.layout.compute(b.body).panes;
    let (at, hit) = b.title(Widget::Stream);
    b.press(at, hit);
    let names = [Widget::Queue, Widget::Home, Widget::Library, Widget::Lyrics, Widget::Fft];
    let spots: Vec<(u16, u16)> = names.iter().map(|s| centre(b.rect(*s))).collect();
    for (i, &p) in spots.iter().enumerate() {
        b.drag(p);
        assert_eq!(b.a.tile.as_ref().unwrap().over, Some(b.pid(names[i])), "{}", names[i]);
        assert_eq!(b.layout.compute(b.body).panes, floor, "nothing rearranges");
    }
    // a flick on to the queue and let go there at once: the queue it is
    let q = spots[0];
    b.drag(q);
    b.release(q);
    let swap = |l: &Arrangement, x: Widget, y: Widget| l.swap(l.pane_of(x).unwrap(), l.pane_of(y).unwrap()).unwrap();
    assert_eq!(b.layout, swap(&before, Widget::Stream, Widget::Queue));
    // a release on another cell than the last move (a finger slipping as it lifts): the Pane under the release
    let mut c = arranging();
    let (at, hit) = c.title(Widget::Stream);
    c.press(at, hit);
    c.drag(q);
    let l = centre(c.rect(Widget::Library));
    c.release(l);
    assert_eq!(c.layout, swap(&before, Widget::Stream, Widget::Library));
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
    /// a title or a Tray Widget carried and let go, or Enter's drop: contents only, of the two places involved
    Carry,
    /// a divider dragged, or `+` `=` `-` `[` `]`: sizes only
    Line,
    /// `s` then an arrow, or a split button
    Split,
    /// `x`, or a ×
    Close,
    /// `t` on the focused Pane (it was focused): only that Pane is emptied, its Widget at the Tray's end
    ToTray,
    /// out of the Tray (`x`, Delete or Backspace on it, its ×, Clear Tray): only the Tray loses Widgets
    TrayRemove,
    /// undo or redo: back to a state seen before
    Undo,
    /// arrows, Esc, Done, `w`, a `t` with nothing focused…: no edit of their own
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

/// The Panes (of `before`) whose Widget is another in `after`.
fn changed_panes(before: &Arrangement, after: &Arrangement) -> Vec<PaneId> {
    before.panes().into_iter().filter(|p| after.widget_in(p.id) != p.widget).map(|p| p.id).collect()
}

/// One random input to the Bench, and what kind it was.
fn random_input(b: &mut Bench, rng: &mut Rng) -> Option<Kind> {
    use KeyCode::*;
    let keys = [Left, Right, Up, Down, Enter, Esc, Char('t'), Char('s'), Char('x'), Char('+'), Char('='), Char('-'), Char('['), Char(']'), Char('w'), Delete, Backspace];
    let panes = b.layout.compute(b.body).panes;
    let pick = |rng: &mut Rng| panes[rng.below(panes.len())];
    Some(match rng.below(11) {
        0..=3 => {
            if !b.a.is_on() {
                b.enter();
            }
            let k = keys[rng.below(keys.len())];
            let split = b.a.split_waiting() && matches!(k, Left | Right | Up | Down);
            let carrying = b.a.carrying();
            let panes = b.layout.panes().len();
            b.key(k);
            // (a focus in the Tray on a Widget that has left it is let go first: `x` then closes a Pane)
            match k {
                _ if split => Kind::Split,
                Enter if carrying => Kind::Carry,
                Char('t') if !carrying => Kind::ToTray,
                Char('x') if !carrying && b.layout.panes().len() < panes => Kind::Close,
                Char('x') | Delete | Backspace if !carrying => Kind::TrayRemove,
                Char('+' | '=' | '-' | '[' | ']') if !carrying => Kind::Line,
                _ => Kind::Other,
            }
        }
        4 | 5 => {
            // a title carried somewhere (onto a Pane, the Tray, off the Panes)
            let (p, r) = pick(rng);
            b.press((r.x + 3, r.y), Some(Hit::Title(p.id)));
            let area = if rng.below(4) == 0 { b.tray_rect() } else { b.window };
            for _ in 0..rng.below(4) + 1 {
                let c = rng.cell(area);
                b.drag(c);
            }
            let c = rng.cell(area);
            b.release(c);
            Kind::Carry
        }
        6 => {
            // a Widget from the Tray onto the Panes
            let &w = b.layout.tray().get(rng.below(b.layout.tray().len()))?;
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
                return None;
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
        8 => {
            b.undo(rng.below(3) > 0);
            Kind::Undo
        }
        9 => {
            if b.a.is_on() {
                b.press((0, 0), Some(Hit::ArrangeDone));
            }
            Kind::Other
        }
        _ => {
            if !b.a.is_on() {
                b.enter();
            }
            let (p, r) = pick(rng);
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
                    Kind::TrayRemove
                }
                _ => {
                    b.press((0, 0), Some(Hit::ClearTray));
                    Kind::TrayRemove
                }
            }
        }
    })
}

/// ADR 0005: a drag only moves contents; the floor plan changes only by its own explicit edits (dragging a line,
/// splitting, closing); no Widget is lost but by removing it from the Tray. Random keys and pointer gestures (on
/// titles, the Tray, dividers, the split and close buttons, Done), undo and redo, in every built-in Design, each
/// change checked against what the input was.
#[test]
fn every_edit_is_one_of_adr_0005s_closed_set() {
    // how many changes of each kind were checked
    let mut seen: std::collections::HashMap<Kind, usize> = Default::default();
    for d in 0..DESIGNS.len() {
        for seed in 1..=12u64 {
            let mut rng = Rng(seed * 0x9E37_79B9 + d as u64 + 1);
            let mut b = Bench::design(d);
            // every layout there has been (undo and redo only go back to one of them)
            let mut states = vec![b.layout.clone()];
            for step in 0..150 {
                let before = b.layout.clone();
                let focused = focused_pane(&before, b.focus);
                let commits = b.commits;
                let Some(kind) = random_input(&mut b, &mut rng) else { continue };
                let what = format!("design {d} seed {seed} step {step} ({kind:?})");
                assert!(b.a.tile.is_none() || b.a.carrying(), "{what}: a Widget left held by the pointer");
                assert!(b.a.divider.is_none(), "{what}: a divider left held");
                if b.layout == before {
                    continue;
                }
                *seen.entry(kind).or_default() += 1;
                let (was, now) = (held(&before), held(&b.layout));
                let moved = changed_panes(&before, &b.layout);
                let tray_moved = b.layout.tray() != before.tray();
                if kind == Kind::Undo {
                    assert!(states.contains(&b.layout), "{what}: undo or redo went to a layout never shown");
                    assert_eq!(b.commits, commits, "{what}: undo is no edit of its own");
                    continue;
                }
                assert_eq!(b.commits, commits + 1, "{what}: the layout changed by {} edits", b.commits - commits);
                states.push(b.layout.clone());
                match kind {
                    Kind::Carry => {
                        assert!(same_floor(&b.layout, &before), "{what}: a carried Widget changed the floor plan");
                        assert_eq!(now, was, "{what}: a carried Widget lost or added one");
                        assert!(moved.len() + tray_moved as usize <= 2, "{what}: a drop changed {moved:?} (and the Tray: {tray_moved})");
                    }
                    Kind::ToTray => {
                        assert!(same_floor(&b.layout, &before), "{what}");
                        let f = focused.expect("t with a focused Pane");
                        assert_eq!(moved, vec![f], "{what}: t emptied another Pane");
                        assert_eq!(b.layout.widget_in(f), None, "{what}");
                        let w = before.widget_in(f).unwrap();
                        assert_eq!(b.layout.tray().last(), Some(&w), "{what}: it waits at the Tray's end");
                        assert_eq!(now, was, "{what}");
                    }
                    Kind::TrayRemove => {
                        assert!(same_floor(&b.layout, &before) && moved.is_empty(), "{what}: a Tray edit touched a Pane");
                        assert!(b.layout.tray().iter().all(|w| before.tray().contains(w)) && b.layout.tray().len() < before.tray().len(), "{what}");
                    }
                    Kind::Line => {
                        assert_eq!(shape(b.layout.plan()), shape(before.plan()), "{what}: a line moved more than a line");
                        assert!(moved.is_empty() && !tray_moved, "{what}: a line moved contents");
                    }
                    Kind::Split => {
                        assert_eq!(b.layout.panes().len(), before.panes().len() + 1, "{what}");
                        assert!(moved.is_empty() && !tray_moved, "{what}: a split moved contents");
                        assert!(b.layout.panes().iter().filter(|p| !before.contains(p.id)).all(|p| p.widget.is_none()), "{what}: the new Pane isn't empty");
                    }
                    Kind::Close => {
                        assert_eq!(b.layout.panes().len() + 1, before.panes().len(), "{what}");
                        let gone: Vec<Pane> = before.panes().into_iter().filter(|p| !b.layout.contains(p.id)).collect();
                        assert_eq!(gone.len(), 1, "{what}: one Pane closes");
                        assert!(b.layout.panes().iter().all(|p| before.widget_in(p.id) == p.widget), "{what}: a survivor changed Widget");
                        if let Some(w) = gone[0].widget {
                            assert!(b.layout.tray().contains(&w) && !before.tray().contains(&w), "{what}: the closed Widget isn't in the Tray");
                        }
                        assert_eq!(now, was, "{what}: a close lost a Widget");
                    }
                    Kind::Undo | Kind::Other => panic!("{what}: {:?} became {:?} without an edit", before, b.layout),
                }
            }
        }
    }
    for k in [Kind::Carry, Kind::Line, Kind::Split, Kind::Close, Kind::ToTray, Kind::TrayRemove, Kind::Undo] {
        assert!(seen.get(&k).is_some_and(|n| *n >= 20), "too few {k:?} changes checked: {seen:?}");
    }
}
