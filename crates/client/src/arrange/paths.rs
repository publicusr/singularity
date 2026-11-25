//! Pointer paths (#21, retargeted for D87): whole gestures replayed the way a real hand makes them, with a test clock.
//! Events arrive at their own times; frames come when the Client's pacer says they are due (`pace::Pacer`: the motion
//! rate while something moves, input bringing a frame forward), and pointer moves between two frames are combined as
//! the main loop combines them (`pace::Moves`), or not. Every frame is recorded (whether a Widget is held, the Pane
//! highlighted, where every Pane is, whether the floor plan is valid), and every path is checked for the same things:
//! the floor plan is always valid, nothing but the highlight and the chip changes while a Widget is held (every Pane's
//! rectangle the same, frame after frame), the highlight is always the Pane under the pointer (no dwell), a release
//! drops on the Pane under it (the one highlighted at release) by swapping the two Widgets and nothing else, and
//! nothing is left held once the button is up.
//!
//! What replaying them found under D66 (each was a test that was red; the behaviours stay covered):
//! - a fling, an overshoot let go at once, a release over the top bar or the player, a release right after a resize:
//!   the Pane landed somewhere the preview never showed;
//! - a release the Client never heard (let go over another window, the focus gone): the Pane stayed held for good and
//!   swallowed every click; a divider stayed held too (and kept the Client at the motion frame rate);
//! - a right click while holding a Pane dropped it on the spot, though the left button was down;
//! - a divider flung past its limit at the well stopped where the last report that fitted left it, short of the limit;
//! - after a resize mid-drag the drop used a Pane of the old window.
//!
//! Not modelled: the terminal's pixel reports (`term::MouseUnits`; the Client gets cells either way) and the wheel.

use super::*;
use crate::layout::Node;
use crate::design::DESIGNS;
use ratatui::crossterm::event::KeyModifiers;
use crate::arrangement::outline;
use crate::pace::{Moves, Pacer};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::Event;

const W: u16 = 160;
const H: u16 = 48;

/// What the hand (or the window) does at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ev {
    Press(u16, u16),
    /// moved with the button down
    Drag(u16, u16),
    Release(u16, u16),
    /// moved with no button down (the terminal only says so: a release we never heard, outside the window)
    Hover(u16, u16),
    /// the right button, pressed and let go, while the left one is held
    RightClick(u16, u16),
    /// the terminal window took a new size
    Resize(u16, u16),
    /// the window lost the focus (another app took the pointer)
    FocusLost,
}

/// A path: each event and when it happens (ms after the start, fractions allowed).
type Path = Vec<(f64, Ev)>;

/// One frame as drawn.
#[derive(Clone, Debug)]
struct Frame {
    /// ms after the start
    t: f64,
    held: bool,
    /// the Pane highlighted, and the one under the pointer by the geometry (other than the held Widget's own)
    over: Option<Spot>,
    under: Option<Spot>,
    /// where every Pane is drawn, and the size it is drawn at
    rects: Vec<(PaneId, Rect)>,
    body: Rect,
    /// whether the floor plan is valid at the size it is drawn at
    fits: bool,
}

/// Where a release drops: a Pane, or the Tray.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Spot {
    Pane(PaneId),
    Tray,
}

impl Spot {
    /// What a held Widget highlights.
    fn of(t: &crate::arrange::TileDrag) -> Option<Spot> {
        if t.tray { Some(Spot::Tray) } else { t.over.map(Spot::Pane) }
    }
}

/// One hold that ended: the Arrangement before, how it ended (a release heard at a cell, and the Pane under that
/// cell other than the held one's own, by the geometry; or not a release), the Pane held, and the Arrangement after.
struct Drop {
    before: Arrangement,
    released: Option<Option<Spot>>,
    /// the highlight as the release found it, if the release came on the cell of the last move
    shown: Option<Option<Spot>>,
    held: PaneId,
    /// the Widget, when it was picked up from the Tray (no Pane of its own: `held` is `PaneId::UNSET`)
    from_tray: Option<Widget>,
    after: Arrangement,
}

/// Where the Panes go in a window of this size (under the top bar, over the player and the hints).
fn body((w, h): (u16, u16), arranging: bool) -> Rect {
    crate::ui::rows(Rect::new(0, 0, w, h), arranging)[1]
}

/// A Client being played a path: the App, the test clock, the frame pacer and what each frame showed.
struct Replay {
    a: App,
    /// the test clock's zero
    t0: Instant,
    combine: bool,
    size: (u16, u16),
    frames: Vec<Frame>,
    /// draw every frame, else only when the window or the layout changed
    draw: bool,
    /// the layout last drawn
    drawn: Option<Arrangement>,
    drops: Vec<Drop>,
    /// the layout before the path
    start: Arrangement,
    /// the divider a press grabbed
    divider: Option<usize>,
    /// the last cell the pointer was reported at with the button down
    last_drag: Option<(u16, u16)>,
    moves: Moves,
    pacer: Pacer,
}

impl Replay {
    /// Design `design` at 160×48, drawing at `fps` while something moves, pointer moves combined or not.
    fn new(design: usize, fps: u64, combine: bool) -> Replay {
        let mut a = App::new(None);
        a.set_design(design);
        a.config.visuals.motion_fps = fps as _;
        let t0 = Instant::now() + Duration::from_secs(3600);
        let start = a.layout.clone();
        let mut r = Replay { a, t0, combine, size: (W, H), frames: vec![], draw: false, drawn: None, drops: vec![], start, divider: None, last_drag: None, moves: Moves::default(), pacer: Pacer::new(t0) };
        r.render();
        r
    }

    fn at(&self, ms: f64) -> Instant {
        self.t0 + Duration::from_secs_f64(ms.max(0.0) / 1000.0)
    }

    fn ms(&self, at: Instant) -> f64 {
        at.saturating_duration_since(self.t0).as_secs_f64() * 1000.0
    }

    fn render(&mut self) {
        let mut t = Terminal::new(TestBackend::new(self.size.0, self.size.1)).unwrap();
        t.draw(|f| crate::ui::draw(f, &mut self.a)).unwrap();
        self.drawn = Some(self.a.layout.clone());
    }

    /// What is under (x, y) by the geometry alone: a Pane other than `own`, or the Tray (Arrange mode's strip under
    /// the player bar).
    fn under(&self, x: u16, y: u16, own: PaneId) -> Option<Spot> {
        let t = self.a.tray_rect();
        if self.a.arrange.on.is_some() && own != PaneId::UNSET && x < t.right() && y >= t.y && y < t.bottom() {
            return Some(Spot::Tray);
        }
        let c = self.a.layout.compute(self.a.pane_area());
        c.panes.iter().find(|(_, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()).map(|(p, _)| p.id).filter(|p| *p != own).map(Spot::Pane)
    }

    /// A frame due at `due`: the move that waited for it, then it is drawn.
    fn frame(&mut self, due: Instant) {
        if let Some((at, ev)) = self.moves.flush() {
            self.handle(at, ev);
        }
        if self.draw || self.a.pane_area() != body(self.size, self.a.arrange.on.is_some()) || self.drawn.as_ref() != Some(&self.a.layout) {
            self.render();
        }
        let area = self.a.pane_area();
        let fits = self.a.layout.fits(area);
        // drawn this frame: the Panes' own hit boxes say where; else where the layout puts them
        let mut rects: Vec<(PaneId, Rect)> = if self.draw {
            self.a.hits.iter().filter_map(|(r, h)| if let Hit::Focus(p) = h { Some((*p, *r)) } else { None }).collect()
        } else {
            self.a.layout.compute(area).panes.into_iter().map(|(p, r)| (p.id, r)).collect()
        };
        rects.sort_by_key(|(p, _)| *p);
        let t = self.a.arrange.tile.as_ref().filter(|t| t.live);
        let under = t.and_then(|t| self.under(t.at.0, t.at.1, t.pane.id));
        let ms = self.ms(due);
        self.frames.push(Frame { t: ms, held: t.is_some(), over: t.and_then(Spot::of), under, rects, body: area, fits });
    }

    /// Frames up to `until`, each when the pacer says it is due.
    fn frames_until(&mut self, until: Instant) {
        loop {
            let fps = self.a.frame_rate(true);
            let due = self.pacer.due(fps);
            if due > until {
                return;
            }
            self.frame(due);
            self.pacer.begin(due, due, fps);
        }
    }

    /// One event handled, as `main.rs`'s `handle` does; a hold it ends is recorded with how it ended.
    fn handle(&mut self, at: Instant, ev: Event) {
        self.a.input_at = Some(at);
        let held = self.a.arrange.tile.as_ref().filter(|t| t.live).map(|t| (t.pane.id, Spot::of(t), t.pane.widget.filter(|_| t.from_tray)));
        let before = self.a.layout.clone();
        match ev {
            Event::Mouse(m) => {
                let up = m.kind == MouseEventKind::Up(MouseButton::Left);
                let released = held.filter(|_| up).map(|(own, ..)| self.under(m.column, m.row, own));
                let shown = held.filter(|_| up && self.last_drag == Some((m.column, m.row))).map(|(_, over, _)| over);
                if m.kind == MouseEventKind::Drag(MouseButton::Left) {
                    self.last_drag = Some((m.column, m.row));
                }
                self.a.mouse(m);
                if let (MouseEventKind::Down(MouseButton::Left), Some(i)) = (m.kind, self.a.arrange.divider_held()) {
                    self.divider = Some(i);
                }
                if let Some((own, _, from_tray)) = held.filter(|_| self.a.arrange.tile.is_none()) {
                    self.drops.push(Drop { before, released, shown, held: own, from_tray, after: self.a.layout.clone() });
                }
            }
            Event::FocusLost => self.a.pointer_gone(),
            Event::Resize(w, h) => {
                self.a.pointer_gone();
                self.a.window_resized(w, h);
            }
            _ => {}
        }
        self.a.input_at = None;
    }

    /// An event arrives at `ms`: it brings the next frame forward, as input does in the main loop.
    fn deliver(&mut self, ms: f64, ev: Event) {
        let at = self.at(ms);
        let hover = matches!(&ev, Event::Mouse(m) if m.kind == MouseEventKind::Moved) && !self.a.moving_now();
        self.pacer.input(if hover { 60.min(self.a.motion_fps()) } else { self.a.motion_fps() });
        if self.combine {
            for (at, ev) in self.moves.push(at, ev).into_iter().flatten() {
                self.handle(at, ev);
            }
        } else {
            self.handle(at, ev);
        }
    }

    /// Play `path`, then let `after` ms of frames pass.
    fn play(&mut self, path: &Path, after: f64) {
        let mouse = |kind, x, y| Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
        for &(ms, ev) in path {
            self.frames_until(self.at(ms));
            match ev {
                Ev::Press(x, y) => self.deliver(ms, mouse(MouseEventKind::Down(MouseButton::Left), x, y)),
                Ev::Drag(x, y) => self.deliver(ms, mouse(MouseEventKind::Drag(MouseButton::Left), x, y)),
                Ev::Release(x, y) => self.deliver(ms, mouse(MouseEventKind::Up(MouseButton::Left), x, y)),
                Ev::Hover(x, y) => self.deliver(ms, mouse(MouseEventKind::Moved, x, y)),
                Ev::RightClick(x, y) => {
                    self.deliver(ms, mouse(MouseEventKind::Down(MouseButton::Right), x, y));
                    self.deliver(ms, mouse(MouseEventKind::Up(MouseButton::Right), x, y));
                }
                Ev::Resize(w, h) => {
                    self.deliver(ms, Event::Resize(w, h));
                    self.size = (w, h);
                }
                Ev::FocusLost => self.deliver(ms, Event::FocusLost),
            }
        }
        let end = path.last().map_or(0.0, |p| p.0) + after;
        self.frames_until(self.at(end));
        self.render();
    }

    /// The Pane's rectangle in the layout as it stands.
    fn rect(&self, w: Widget) -> Rect {
        self.a.layout.compute(self.a.pane_area()).rect_of(w).unwrap_or_else(|| panic!("no {w}"))
    }

    /// A spot on the Pane's title bar to grab it by.
    fn title(&self, w: Widget) -> (u16, u16) {
        let r = self.rect(w);
        (r.x + 3, r.y)
    }

    /// Every check that holds on every path. `what` names the path in a failure.
    fn check(&self, what: &str) {
        let area = self.a.pane_area();
        assert!(self.a.layout.fits(area), "{what}: the floor plan isn't valid: {:?}", self.a.layout);
        // nothing is left held once the button is up (every path ends with it up)
        assert!(self.a.arrange.tile.is_none(), "{what}: a Widget is still held");
        assert!(self.a.arrange.divider.is_none(), "{what}: a divider is still held");
        assert!(!self.a.moving_now(), "{what}: still moving after the path");
        // a release drops on the Pane under it, which is the one highlighted at release: the two Widgets swap and the
        // floor plan stays; a hold that ended any other way (Esc, unheard, on its own Pane, off the Panes) changes nothing
        for (i, d) in self.drops.iter().enumerate() {
            let want = match (d.released, d.from_tray) {
                // from the Tray onto a Pane: it takes the Pane, whose Widget waits in its place
                (Some(Some(Spot::Pane(to))), Some(w)) => d.before.out_of_tray(w, to).unwrap_or_else(|e| panic!("{what}: drop {i} refused: {e:?}")),
                (_, Some(_)) => d.before.clone(),
                (Some(Some(Spot::Pane(to))), None) => d.before.swap(d.held, to).unwrap_or_else(|e| panic!("{what}: drop {i} refused: {e:?}")),
                (Some(Some(Spot::Tray)), None) => d.before.to_tray(d.held).unwrap_or_else(|e| panic!("{what}: drop {i} refused: {e:?}")),
                _ => d.before.clone(),
            };
            assert_eq!(d.after, want, "{what}: drop {i} landed somewhere other than the Pane under the release");
            assert_eq!(outline(d.after.plan()), outline(d.before.plan()), "{what}: drop {i} changed the floor plan");
            if let (Some(shown), Some(to)) = (d.shown, d.released) {
                assert_eq!(shown, to, "{what}: drop {i} went to another Pane than the one highlighted");
            }
        }
        for f in &self.frames {
            assert!(f.fits, "{what}: the floor plan isn't valid at {:.0} ms", f.t);
            // no dwell, no lag: the highlight is the Pane under the pointer, every frame
            assert_eq!(f.over, f.under, "{what}: at {:.0} ms the highlight isn't the Pane under the pointer", f.t);
        }
        // while a Widget is held nothing else moves: every Pane where it was the frame before (at the same window size)
        for w in self.frames.windows(2) {
            if w[0].held && w[1].held && w[0].body == w[1].body {
                assert_eq!(w[0].rects, w[1].rects, "{what}: a Pane moved at {:.0} ms during a drag", w[1].t);
            }
        }
    }

    /// How often the highlight changed while a Widget was held.
    fn changes(&self) -> usize {
        self.frames.windows(2).filter(|w| w[1].held && w[0].held && w[0].over != w[1].over).count()
    }
}

// ── hands ─────────────────────────────────────────────────────────────

/// A small seeded random source (splitmix64), so every path is the same on every run.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in [lo, hi).
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f()
    }
}

/// How a pointing device reports.
#[derive(Clone, Copy, Debug)]
struct Device {
    /// reports a second while it moves
    hz: f64,
    /// how far off the hand's ideal line a report can be (cells)
    noise: f64,
    /// how far the pointer slips as the button comes up (a finger lifting off a trackpad), cells
    slip: f64,
    /// while resting, a tremor report every so many ms (0: a mouse at rest is still)
    tremor_ms: f64,
}

const MOUSE: Device = Device { hz: 500.0, noise: 0.3, slip: 0.0, tremor_ms: 0.0 };
const TRACKPAD: Device = Device { hz: 90.0, noise: 0.6, slip: 1.2, tremor_ms: 35.0 };

/// A hand on a device: builds a path from strokes, keeping the time and where the pointer is.
struct Hand {
    dev: Device,
    rng: Rng,
    t: f64,
    at: (f64, f64),
    path: Path,
    /// the last cell reported (a terminal reports a move only when the cell changes)
    last: Option<(u16, u16)>,
    bounds: (u16, u16),
}

impl Hand {
    fn new(dev: Device, seed: u64, t: f64) -> Hand {
        Hand { dev, rng: Rng::new(seed), t, at: (0.0, 0.0), path: vec![], last: None, bounds: (W, H) }
    }

    /// Inside a window of this size (a pointer outside it is reported at its edge).
    fn within(mut self, w: u16, h: u16) -> Hand {
        self.bounds = (w, h);
        self
    }

    fn cell(&self, (x, y): (f64, f64)) -> (u16, u16) {
        (x.round().clamp(0.0, self.bounds.0 as f64 - 1.0) as u16, y.round().clamp(0.0, self.bounds.1 as f64 - 1.0) as u16)
    }

    fn report(&mut self, p: (f64, f64)) {
        let c = self.cell(p);
        if self.last != Some(c) {
            self.path.push((self.t, Ev::Drag(c.0, c.1)));
            self.last = Some(c);
        }
    }

    fn press(&mut self, x: u16, y: u16) -> &mut Hand {
        self.at = (x as f64, y as f64);
        self.last = Some((x, y));
        self.path.push((self.t, Ev::Press(x, y)));
        self
    }

    /// A stroke to (x, y) taking `ms`, the way a hand moves: slow, fast, slow (minimum jerk), with the device's noise.
    fn glide(&mut self, x: f64, y: f64, ms: f64) -> &mut Hand {
        let (x0, y0) = self.at;
        let n = ((ms / 1000.0 * self.dev.hz).ceil() as usize).max(1);
        let t0 = self.t;
        for i in 1..=n {
            let u = i as f64 / n as f64;
            let s = u * u * u * (10.0 - 15.0 * u + 6.0 * u * u);
            let k = if i == n { 0.0 } else { self.dev.noise };
            let p = (x0 + (x - x0) * s + self.rng.range(-k, k), y0 + (y - y0) * s + self.rng.range(-k, k) / 2.0);
            self.t = t0 + ms * u;
            self.report(p);
        }
        self.at = (x, y);
        self
    }

    /// Resting for `ms`: a mouse stays still, a finger on a trackpad trembles a cell now and then.
    fn rest(&mut self, ms: f64) -> &mut Hand {
        let end = self.t + ms;
        if self.dev.tremor_ms > 0.0 {
            let home = self.at;
            while self.t + self.dev.tremor_ms < end {
                self.t += self.dev.tremor_ms;
                let p = (home.0 + self.rng.range(-0.9, 0.9), home.1 + self.rng.range(-0.6, 0.6));
                self.report(p);
            }
        }
        self.t = end;
        self
    }

    /// Back and forth `n` times between here and (`dx`, `dy`) cells away, `ms` a leg (a hand hesitating on a border).
    fn jitter(&mut self, dx: f64, dy: f64, n: usize, ms: f64) -> &mut Hand {
        let home = self.at;
        for _ in 0..n {
            self.glide(home.0 + dx, home.1 + dy, ms);
            self.glide(home.0, home.1, ms);
        }
        self
    }

    fn release(&mut self) -> &mut Hand {
        let s = self.dev.slip;
        let p = (self.at.0 + self.rng.range(-s, s), self.at.1 + self.rng.range(-s, s) / 2.0);
        let (x, y) = self.cell(p);
        self.t += 8.0;
        self.path.push((self.t, Ev::Release(x, y)));
        self
    }

    fn then(&mut self, ms: f64, ev: Ev) -> &mut Hand {
        self.t += ms;
        self.path.push((self.t, ev));
        self
    }

    fn done(&mut self) -> Path {
        std::mem::take(&mut self.path)
    }
}

fn centre(r: Rect) -> (f64, f64) {
    ((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64)
}

/// Every way the Client can be run: a frame rate and whether moves are combined.
const RIGS: [(u64, bool); 3] = [(60, true), (144, true), (60, false)];

// ── the paths ─────────────────────────────────────────────────────────

/// The Pane a path picks up in `layout`: the stream if there is one, else the first that isn't the queue, the
/// library or Home.
fn held_in(layout: &Node) -> Widget {
    let names = layout.widgets();
    if names.contains(&Widget::Stream) { Widget::Stream } else { names.into_iter().find(|s| ![Widget::Queue, Widget::Library, Widget::Home].contains(s)).unwrap() }
}
/// The Pane a path picks up now.
fn held(r: &Replay) -> Widget {
    held_in(&r.a.layout)
}

/// `shape` played on every Design with a mouse and a trackpad, at every frame rate and with moves combined or not,
/// each checked; the replays come back for what a test checks on top.
fn every_way(name: &str, shape: impl Fn(&Replay, &mut Hand)) -> Vec<(String, Replay)> {
    every_way_from(name, |_| {}, shape)
}

/// `every_way`, from a start `setup` makes (the Design on screen before the path).
fn every_way_from(name: &str, setup: impl Fn(&mut Replay), shape: impl Fn(&Replay, &mut Hand)) -> Vec<(String, Replay)> {
    let mut out = vec![];
    for (design, d) in DESIGNS.iter().enumerate() {
        for (dev, kind) in [(MOUSE, "mouse"), (TRACKPAD, "trackpad")] {
            for (seed, &(fps, combine)) in RIGS.iter().enumerate() {
                let mut r = Replay::new(design, fps, combine);
                setup(&mut r);
                let mut h = Hand::new(dev, seed as u64 + 1, 10.0);
                shape(&r, &mut h);
                let path = h.done();
                r.play(&path, 400.0);
                let what = format!("{name} · {} · {kind} · {fps} fps{}", d.id, if combine { "" } else { " uncombined" });
                r.check(&what);
                out.push((what, r));
            }
        }
    }
    out
}

/// The layout the path started from with the held Widget and `w` swapped.
fn swapped_with(r: &Replay, w: Widget) -> Arrangement {
    let s = &r.start;
    s.swap(pid(s, held_in(s)), pid(s, w)).unwrap()
}

/// The Pane showing `w` in `n`.
fn pid(n: &Node, w: Widget) -> PaneId {
    n.pane_of(w).unwrap_or_else(|| panic!("no Pane shows {w}"))
}

/// Arrange mode on, with the Widget a path would hold waiting in the Tray (its Pane left empty).
fn from_the_tray(r: &mut Replay) {
    let w = held(r);
    r.a.enter_arrange();
    r.a.layout = r.a.layout.to_tray(pid(&r.a.layout, w)).unwrap();
    r.start = r.a.layout.clone();
    r.render();
}

/// Where the chip of the Widget waiting in the Tray was drawn.
fn tray_chip(r: &Replay) -> (f64, f64) {
    let rect = r.a.hits.iter().find_map(|(rect, h)| matches!(h, Hit::TrayItem(_)).then_some(*rect)).expect("a chip in the Tray");
    (rect.x as f64 + rect.width as f64 / 2.0, rect.y as f64)
}

#[test]
fn a_widget_dragged_from_the_tray_lands_on_the_pane_under_the_release() {
    // onto an occupied Pane (its Widget goes into the Tray), onto its own empty Pane (it fills it), and back onto the
    // Tray (nothing changes)
    for target in ["occupied", "empty", "tray"] {
        for (what, r) in every_way_from(&format!("from the Tray onto {target}"), from_the_tray, |r, h| {
            let (x, y) = tray_chip(r);
            let to = match target {
                "occupied" => centre(r.rect(Widget::Home)),
                "empty" => {
                    let empty = r.a.layout.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
                    let rect = r.a.layout.compute(r.a.pane_area()).rect(empty).unwrap();
                    centre(rect)
                }
                _ => (x + 30.0, y),
            };
            h.press(x as u16, y as u16).glide(to.0, to.1, 250.0).rest(60.0).release();
        }) {
            let w = r.start.tray()[0];
            match target {
                "occupied" => assert!(r.a.layout.pane_of(w).is_some() && r.a.layout.tray() == [Widget::Home], "{what}: {:?}", r.a.layout),
                "empty" => assert!(r.a.layout.pane_of(w).is_some() && r.a.layout.tray().is_empty(), "{what}"),
                _ => assert_eq!(r.a.layout, r.start, "{what}: nothing changes"),
            }
        }
    }
}

#[test]
fn a_fast_fling_across_panes_highlights_each_at_once_and_drops_where_it_is_let_go() {
    for (what, r) in every_way("fling", |r, h| {
        let me = held(r);
        let (x, y) = r.title(me);
        h.press(x, y);
        for s in r.a.layout.widgets().into_iter().filter(|s| *s != me) {
            let c = centre(r.rect(s));
            h.glide(c.0, c.1, 50.0);
        }
        h.release();
    }) {
        assert!(r.changes() >= 2, "{what}: the highlight followed the pointer across the Panes");
        let last = *r.start.widgets().iter().rfind(|s| **s != held_in(&r.start)).unwrap();
        assert_eq!(r.a.layout, swapped_with(&r, last), "{what}");
    }
}

#[test]
fn a_slow_creep_into_a_pane_drops_there() {
    for (what, r) in every_way("creep", |r, h| {
        let (x, y) = r.title(held(r));
        let q = r.rect(Widget::Queue);
        let (qx, qy) = centre(q);
        h.press(x, y).glide(q.x as f64 - 6.0, qy, 150.0);
        let mut cx = q.x as f64 - 6.0;
        while cx < qx {
            cx += 1.0;
            h.glide(cx, qy, 110.0);
        }
        h.rest(60.0).release();
    }) {
        assert_eq!(r.a.layout, swapped_with(&r, Widget::Queue), "{what}");
    }
}

#[test]
fn jitter_on_the_border_between_two_panes_drops_on_the_one_under_the_release() {
    // on the border between two Panes and well inside one: the release's own cell decides (the checks)
    for (name, inside) in [("jitter on a border", false), ("jitter inside", true)] {
        every_way(name, |r, h| {
            let (x, y) = r.title(held(r));
            let q = r.rect(Widget::Queue);
            let qy = centre(q).1;
            let at = if inside { q.x as f64 + (q.width as f64 * 0.2).round() } else { q.x as f64 };
            h.press(x, y).glide(at, qy, 200.0).jitter(if inside { 2.0 } else { -3.0 }, 0.0, 10, 30.0).rest(60.0).release();
        });
    }
}

#[test]
fn overshooting_a_pane_and_coming_back_drops_on_it() {
    for rest in [20.0, 250.0] {
        for (what, r) in every_way(&format!("overshoot, rest {rest} ms"), |r, h| {
            let (x, y) = r.title(held(r));
            let (qx, qy) = centre(r.rect(Widget::Queue));
            h.press(x, y).glide(qx + 30.0, qy + 4.0, 160.0).glide(qx, qy, 120.0).rest(rest).release();
        }) {
            assert_eq!(r.a.layout, swapped_with(&r, Widget::Queue), "{what}");
        }
    }
}

#[test]
fn a_pause_on_one_pane_then_going_on_to_another_drops_on_the_second() {
    for (what, r) in every_way("pause, then on", |r, h| {
        let (x, y) = r.title(held(r));
        let (qx, qy) = centre(r.rect(Widget::Queue));
        let lib = centre(r.rect(Widget::Library));
        h.press(x, y).glide(qx, qy, 200.0).rest(300.0).glide(lib.0, lib.1, 250.0).rest(200.0).release();
    }) {
        assert_eq!(r.a.layout, swapped_with(&r, Widget::Library), "{what}");
    }
}

/// Picked up, rested on the queue, then out of the window to the right.
fn out_of_the_window(r: &Replay, h: &mut Hand) {
    let (x, y) = r.title(held(r));
    let (qx, qy) = centre(r.rect(Widget::Queue));
    h.press(x, y).glide(qx, qy, 200.0).rest(200.0).glide(W as f64 + 20.0, qy, 120.0);
}

#[test]
fn letting_go_outside_the_window_drops_on_the_pane_at_its_edge() {
    // the terminal reports the release at its edge: whatever Pane is there (the checks)
    every_way("let go outside the window", |r, h| {
        out_of_the_window(r, h);
        h.rest(150.0).release();
    });
}

#[test]
fn a_release_the_client_never_heard_changes_nothing() {
    // the release went to another window: the next we hear is the pointer moving with no button down
    for (what, r) in every_way("released unheard", |r, h| {
        out_of_the_window(r, h);
        h.then(300.0, Ev::Hover(W - 1, 20)).then(20.0, Ev::Hover(W - 5, 21));
    }) {
        assert_eq!(r.a.layout, r.start, "{what}: nothing moved");
    }
    // another app took the focus mid-drag
    every_way("focus lost", |r, h| {
        out_of_the_window(r, h);
        h.then(5.0, Ev::FocusLost).then(400.0, Ev::Hover(5, 5));
    });
    // …or the next thing is a fresh press: the drag is over and the press is a press
    for (what, r) in every_way("pressed again", |r, h| {
        out_of_the_window(r, h);
        let (lx, ly) = r.title(Widget::Library);
        h.then(300.0, Ev::Press(lx, ly)).then(30.0, Ev::Release(lx, ly));
    }) {
        assert_eq!(r.a.layout, r.start, "{what}: nothing moved");
        assert_eq!(r.a.focused(), Some(Widget::Library), "{what}: the press clicked the library's title");
    }
}

#[test]
fn letting_go_over_the_top_bar_changes_nothing_and_over_the_tray_sets_it_aside() {
    for (y_out, name) in [(0.0, "the top bar"), (H as f64 - 3.0, "the Tray")] {
        for (what, r) in every_way(&format!("let go over {name}"), |r, h| {
            let (x, y) = r.title(held(r));
            let (qx, qy) = centre(r.rect(Widget::Queue));
            let hx = centre(r.rect(Widget::Home)).0;
            h.press(x, y).glide(qx, qy, 200.0).rest(200.0).glide(hx, y_out, 150.0).rest(20.0).release();
        }) {
            // (Arrange mode began with the drag, so the Tray is under the player bar)
            match name {
                "the Tray" => {
                    let w = r.start.widget_in(r.drops[0].held).unwrap();
                    assert_eq!(r.a.layout.tray(), [w], "{what}: it waits in the Tray");
                    assert_eq!(r.a.layout.widget_in(r.drops[0].held), None, "{what}: its Pane is left empty");
                }
                _ => assert_eq!(r.a.layout, r.start, "{what}: no Pane there, no drop"),
            }
        }
    }
}

#[test]
fn a_resize_mid_drag_drops_on_a_pane_of_the_new_window() {
    let grab = |r: &Replay, h: &mut Hand| {
        let (x, y) = r.title(held(r));
        let (qx, qy) = centre(r.rect(Widget::Queue));
        h.press(x, y).glide(qx, qy, 200.0).rest(200.0).then(5.0, Ev::Resize(100, 30));
        *h = std::mem::replace(h, Hand::new(MOUSE, 0, 0.0)).within(100, 30);
        h.at = (qx.min(90.0), qy.min(25.0));
    };
    every_way("resized, then on", |r, h| {
        grab(r, h);
        h.glide(40.0, 15.0, 100.0).rest(200.0).release();
    });
    every_way("resized, let go at once", |r, h| {
        grab(r, h);
        let (x, y) = (h.at.0 as u16, h.at.1 as u16);
        h.then(20.0, Ev::Release(x, y));
    });
    // resting on a spot inside both windows, then the window shrinks and it's let go at once: what drops is the Pane
    // under the pointer in the new window, never the one the old window had there
    // (the release 20 ms after the resize, or in the same batch of input: before any frame is drawn at the new size)
    for ((x, y), gap) in [(20u16, 8u16), (40, 15), (60, 20), (80, 10), (30, 22), (85, 24)].into_iter().flat_map(|p| [(p, 20.0), (p, 0.0)]) {
        for (what, r) in every_way(&format!("rest at {x},{y}, resized, let go {gap} ms later"), |r, h| {
            let (tx, ty) = r.title(held(r));
            h.press(tx, ty).glide(x as f64, y as f64, 200.0).rest(250.0).then(1.0, Ev::Drag(x, y)).then(5.0, Ev::Resize(100, 30)).then(gap, Ev::Release(x, y));
        }) {
            let me = pid(&r.start, held_in(&r.start));
            let area = r.a.pane_area();
            let here = r.start.compute(area).panes.into_iter().find(|(_, q)| x >= q.x && x < q.right() && y >= q.y && y < q.bottom()).map(|(p, _)| p.id).filter(|p| *p != me);
            let new = here.map(|p| r.start.swap(me, p).unwrap());
            assert!(r.a.layout == r.start || Some(&r.a.layout) == new.as_ref(), "{what}: dropped on a Pane of the old window");
        }
    }
}

#[test]
fn a_right_click_mid_drag_does_not_drop_the_widget() {
    for (what, r) in every_way("right click", |r, h| {
        let (x, y) = r.title(held(r));
        let (qx, qy) = centre(r.rect(Widget::Queue));
        let lib = centre(r.rect(Widget::Library));
        h.press(x, y).glide(qx, qy, 200.0).rest(200.0).then(5.0, Ev::RightClick(qx as u16, qy as u16)).glide(lib.0, lib.1, 150.0).rest(200.0).release();
    }) {
        assert_eq!(r.a.layout, swapped_with(&r, Widget::Library), "{what}: still held, dropped on the library");
    }
}

/// The cell of divider `i`'s grip nearest its middle that a press really grabs (not a title or a button on it).
fn grip(r: &Replay, i: usize) -> Option<(u16, u16)> {
    let d = r.a.dividers.get(i)?;
    let mut cells: Vec<(u16, u16)> = (d.grip.y..d.grip.bottom()).flat_map(|y| (d.grip.x..d.grip.right()).map(move |x| (x, y))).filter(|&(x, y)| r.a.hit_at(x, y) == Some(Hit::Divider(i))).collect();
    cells.sort_by_key(|&(x, y)| x.abs_diff(d.grip.x + d.grip.width / 2) + y.abs_diff(d.grip.y + d.grip.height / 2));
    cells.first().copied()
}

/// Every divider of every Design dragged by `shape(hand, the grip, whether it moves left and right)`.
fn every_divider(name: &str, shape: impl Fn(&mut Hand, (u16, u16), bool)) -> Vec<(String, Replay)> {
    let most = (0..DESIGNS.len()).map(|d| Replay::new(d, 60, true).a.dividers.len()).max().unwrap();
    let mut out = vec![];
    for i in 0..most {
        for (what, r) in every_way(&format!("{name}, divider {i}"), |r, h| {
            if let Some(at) = grip(r, i) {
                shape(h, at, r.a.dividers[i].axis == crate::layout::Axis::Horizontal);
            }
        }) {
            assert!(!r.frames.iter().any(|f| f.held), "{what}: a press on a divider picked up a Widget");
            if r.divider.is_some() {
                out.push((what, r));
            }
        }
    }
    out
}

#[test]
fn a_divider_swung_both_ways_and_back_never_squashes_a_pane() {
    let out = every_divider("swing", |h, (x, y), horiz| {
        let (a, b) = if horiz { ((0.0, y as f64), (W as f64, y as f64)) } else { ((x as f64, 1.0), (x as f64, H as f64)) };
        h.press(x, y).glide(a.0, a.1, 40.0).glide(b.0, b.1, 60.0).glide(x as f64 + 3.0, y as f64 + 1.0, 80.0).rest(50.0).release();
    });
    assert!(out.len() > 100, "{} divider drags", out.len());
}

#[test]
fn a_divider_flung_past_its_limit_stops_at_the_limit() {
    for low in [true, false] {
        for (what, r) in every_divider(if low { "flung left or up" } else { "flung right or down" }, |h, (x, y), horiz| {
            let to = match (horiz, low) {
                (true, true) => (-10.0, y as f64),
                (true, false) => (W as f64 + 10.0, y as f64),
                (false, true) => (x as f64, -5.0),
                (false, false) => (x as f64, H as f64 + 5.0),
            };
            h.press(x, y).glide(to.0, to.1, 40.0).release();
        }) {
            // one cell further that way and something would no longer fit (or it can't go further at all)
            let area = r.a.pane_area();
            let i = r.divider.unwrap();
            let d = r.a.layout.compute(area).dividers[i].clone();
            let at = r.a.layout.boundary(&d).unwrap();
            let mut further = r.a.layout.plan().clone();
            further.drag(&d, if low { at.saturating_sub(1) } else { at + 1 });
            assert!(further == r.a.layout || !Arrangement::from(further).fits(area), "{what}: stopped short at {at}");
        }
    }
}

#[test]
fn a_divider_released_unheard_is_let_go() {
    every_divider("released unheard", |h, (x, y), horiz| {
        let to = if horiz { (x as f64 - 10.0, y as f64) } else { (x as f64, y as f64 - 4.0) };
        h.press(x, y).glide(to.0, to.1, 100.0).then(200.0, Ev::Hover(3, 3));
    });
}

/// Random strokes a person might make: a press on a title, then glides of every speed with rests, hesitations and
/// overshoots, on a random Design, device and frame rate, let go anywhere (now and then unheard, or after a resize).
#[test]
fn random_human_paths_keep_every_property() {
    let mut rng = Rng::new(21);
    for n in 0..300 {
        let design = (rng.next() % DESIGNS.len() as u64) as usize;
        let (fps, combine) = RIGS[(rng.next() % RIGS.len() as u64) as usize];
        let dev = if rng.next().is_multiple_of(2) { MOUSE } else { TRACKPAD };
        let mut r = Replay::new(design, fps, combine);
        let names = r.a.layout.widgets();
        let me = names[(rng.next() % names.len() as u64) as usize];
        let (x, y) = r.title(me);
        let mut h = Hand::new(dev, rng.next(), 10.0);
        h.press(x, y);
        for _ in 0..1 + rng.next() % 6 {
            let to = (rng.range(-5.0, W as f64 + 5.0), rng.range(-2.0, H as f64 + 2.0));
            match rng.next() % 5 {
                // a fling
                0 => {
                    h.glide(to.0, to.1, rng.range(30.0, 90.0));
                }
                // an overshoot: there and back
                1 => {
                    let back = h.at;
                    h.glide(to.0, to.1, rng.range(80.0, 200.0)).glide(back.0, back.1, rng.range(80.0, 200.0));
                }
                // a hesitation
                2 => {
                    h.glide(to.0, to.1, rng.range(150.0, 600.0)).jitter(rng.range(-3.0, 3.0), rng.range(-1.5, 1.5), 1 + (rng.next() % 5) as usize, rng.range(15.0, 60.0));
                }
                _ => {
                    h.glide(to.0, to.1, rng.range(100.0, 500.0));
                }
            }
            h.rest(rng.range(0.0, 350.0));
        }
        match rng.next() % 8 {
            0 => {
                let (x, y) = h.cell(h.at);
                h.then(200.0, Ev::Hover(x, y));
            }
            1 => {
                let (w, hh) = (100 + (rng.next() % 100) as u16, 30 + (rng.next() % 30) as u16);
                h.then(10.0, Ev::Resize(w, hh));
                let (x, y) = h.cell(h.at);
                h.then(100.0, Ev::Release(x.min(w - 1), y.min(hh - 1)));
            }
            _ => {
                h.release();
            }
        }
        let path = h.done();
        r.play(&path, 400.0);
        // the whole path, shown only if it fails
        println!("random path {n}: {path:?}");
        r.check(&format!("random path {n} ({}, {fps} fps{}, {})", DESIGNS[design].id, if combine { "" } else { " uncombined" }, if dev.hz > 100.0 { "mouse" } else { "trackpad" }));
    }
}

/// One path drawn frame by frame: every Pane's drawn rectangle (its hit box) is the same in every frame of the drag,
/// and the chip and the highlight are what changes.
#[test]
fn a_drag_drawn_frame_by_frame_moves_nothing_but_the_chip_and_the_highlight() {
    for (dev, combine) in [(MOUSE, true), (TRACKPAD, false)] {
        let mut r = Replay::new(0, 120, combine);
        r.draw = true;
        let (x, y) = r.title(Widget::Stream);
        let (qx, qy) = centre(r.rect(Widget::Queue));
        let lib = centre(r.rect(Widget::Library));
        let mut h = Hand::new(dev, 5, 10.0);
        h.press(x, y).glide(qx, qy, 200.0).rest(200.0).glide(lib.0, lib.1, 150.0).rest(200.0).release();
        let path = h.done();
        r.play(&path, 400.0);
        r.check("drawn");
        let held: Vec<&Frame> = r.frames.iter().filter(|f| f.held).collect();
        // (the drag turned Arrange mode on: the Panes made room for the Tray once, at its start, Q39)
        assert!(held.len() > 20 && held.iter().all(|f| f.rects == held[0].rects), "every frame of the drag drew every Pane where it was");
        // and that is the floor plan in the Panes' area less the Tray's rows; before the press, in the whole area
        assert_eq!((r.frames[0].body, held[0].body), (body(r.size, false), body(r.size, true)));
        assert_eq!(held[0].body.height + crate::arrange::TRAY_H, r.frames[0].body.height);
        let mut arranged: Vec<(PaneId, Rect)> = r.start.compute(held[0].body).panes.into_iter().map(|(p, r)| (p.id, r)).collect();
        arranged.sort_by_key(|(p, _)| *p);
        assert_eq!(held[0].rects, arranged, "the held frames show the arranged floor plan");
        assert!(r.changes() >= 2, "the highlight went from Pane to Pane");
        assert_eq!(r.a.layout, swapped_with(&r, Widget::Library));
    }
}
