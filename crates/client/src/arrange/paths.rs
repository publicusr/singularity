//! Pointer paths (#21): whole gestures replayed the way a real hand makes them, with a test clock. Events arrive at
//! their own times; frames come when the Client's pacer says they are due (`pace::Pacer`: the motion rate while
//! something moves, input bringing a frame forward), and pointer moves between two frames are combined as the main
//! loop combines them (`pace::Moves`), or not. Every frame is recorded (whether a Pane is held, the zone shown, whether
//! the layout and the preview fit), and every path is checked for the same things: the layout always fits, what's
//! dropped is what was previewed, the preview doesn't flicker, and nothing is left held once the button is up.
//!
//! What replaying them found first (each is a test below that was red):
//! - a fling, an overshoot let go at once, a release over the top bar or the player, a release right after a resize:
//!   the Pane landed where it was let go, a place the preview never showed (or a different one than it showed);
//! - a release the Client never heard (let go over another window, the focus gone): the Pane stayed held for good and
//!   swallowed every click; a divider stayed held too (and kept the Client at the motion frame rate);
//! - a right click while holding a Pane dropped it on the spot, without its landing, though the left button was down;
//! - a divider flung past its limit at the well stopped where the last report that fitted left it, short of the limit;
//! - after a resize mid-drag the preview kept a zone of the old window (found in review, once the replayer drew the
//!   new window as the Client does).
//!
//! Not found: flicker. The dwell (120 ms, 40 ms creeping) and the sticky zone borders keep every preview up for at
//! least 100 ms on every path tried, jitter included.
//!
//! Not modelled: the terminal's pixel reports (`term::MouseUnits`; the Client gets cells either way) and the wheel.

use super::*;
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
    zone: Option<Zone>,
    /// whether the layout, and the preview if any, fit the window they are drawn in
    fits: bool,
}

/// Where the Panes go in a window of this size (under the top bar, over the player and the hints).
fn body((w, h): (u16, u16)) -> Rect {
    Rect::new(0, 1, w, h.saturating_sub(5))
}

/// A Client being played a path: the App, the test clock, the frame pacer and what each frame showed.
struct Replay {
    a: App,
    /// the test clock's zero: far ahead of the real one, so the frame's own `Instant::now()` (the UI settles the held
    /// Pane at it) never counts as time that passed
    t0: Instant,
    combine: bool,
    size: (u16, u16),
    frames: Vec<Frame>,
    /// draw every frame (slow: Panes in motion are drawn), else only when the window or the layout changed
    draw: bool,
    /// the layout last drawn
    drawn: Option<Node>,
    /// for every hold that ended: the layout before, what the preview showed then (None: nothing, or the hold ended
    /// some other way than a release) and the layout after
    drops: Vec<(Node, Option<Node>, Node)>,
    /// the layout before the path
    start: Node,
    /// the divider a press grabbed
    divider: Option<usize>,
    moves: Moves,
    pacer: Pacer,
}

impl Replay {
    /// Design `design` at 160×48, drawing at `fps` while something moves, pointer moves combined or not.
    fn new(design: usize, fps: u64, combine: bool) -> Replay {
        let mut a = App::new(None);
        a.set_design(design);
        a.motion.on = true;
        a.config.visuals.motion_fps = fps as _;
        let t0 = Instant::now() + Duration::from_secs(3600);
        let mut r = Replay { a, t0, combine, size: (W, H), frames: vec![], draw: false, drawn: None, drops: vec![], start: Node::Slot(String::new()), divider: None, moves: Moves::default(), pacer: Pacer::new(t0) };
        r.render();
        r.start = r.a.layout.clone();
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

    /// A frame due at `due`: the move that waited for it, then the held Pane settles on the frame's clock, and it is
    /// drawn (the springs and the lift take the time since the last frame).
    fn frame(&mut self, due: Instant, since: Duration) {
        if let Some((at, ev)) = self.moves.flush() {
            self.handle(at, ev);
        }
        self.a.tile_settle(due);
        self.a.motion.skip(since);
        if self.draw || self.a.pane_area() != body(self.size) || self.drawn.as_ref() != Some(&self.a.layout) {
            self.render();
        }
        let (area, aspect) = (self.a.pane_area(), self.a.cell_aspect());
        let t = self.a.tile.as_ref();
        let fits = self.a.layout.valid(area, aspect) && t.and_then(|t| t.preview.as_ref()).is_none_or(|p| p.valid(area, aspect));
        let ms = self.ms(due);
        // a screen edge's share follows the window's size: the same edge, whatever its share, is the same zone to the eye
        let zone = t.and_then(|t| t.zone.clone()).map(|z| if let Zone::Edge(s, _) = z { Zone::Edge(s, 0.0) } else { z });
        self.frames.push(Frame { t: ms, held: t.is_some_and(|t| t.live), zone, fits });
    }

    /// Frames up to `until`, each when the pacer says it is due.
    fn frames_until(&mut self, until: Instant) {
        loop {
            let fps = self.a.frame_rate(true);
            let due = self.pacer.due(fps);
            if due > until {
                return;
            }
            let last = self.frames.last().map_or(self.t0, |f| self.at(f.t));
            self.frame(due, due.saturating_duration_since(last));
            self.pacer.begin(due, due, fps);
        }
    }

    /// One event handled, as `main.rs`'s `handle` does; a hold it ends is recorded with what was shown.
    fn handle(&mut self, at: Instant, ev: Event) {
        self.a.input_at = Some(at);
        let held = self.a.tile.as_ref().filter(|t| t.live).map(|t| t.preview.clone());
        let before = self.a.layout.clone();
        match ev {
            Event::Mouse(m) => {
                self.a.mouse(m);
                if let (MouseEventKind::Down(MouseButton::Left), Some(Hit::Divider(i))) = (m.kind, &self.a.drag) {
                    self.divider = Some(*i);
                }
                if let Some(shown) = held.filter(|_| self.a.tile.is_none()) {
                    let shown = if m.kind == MouseEventKind::Up(MouseButton::Left) { shown } else { None };
                    self.drops.push((before, shown, self.a.layout.clone()));
                }
            }
            Event::FocusLost => self.a.pointer_gone(),
            Event::Resize(..) => self.a.pointer_gone(),
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
        // whatever was still landing is down (the first frame takes the skipped time, the second sees it rest)
        self.a.motion.skip(motion::SETTLE);
        self.render();
        self.render();
    }

    /// The Pane's rectangle in the layout as it stands.
    fn rect(&self, slot: &str) -> Rect {
        self.a.layout.compute_shaped(self.a.pane_area(), self.a.cell_aspect()).slots.into_iter().find(|(s, _)| s == slot).map(|(_, r)| r).unwrap_or_else(|| panic!("no {slot}"))
    }

    /// A spot on the Pane's title bar to grab it by.
    fn title(&self, slot: &str) -> (u16, u16) {
        let r = self.rect(slot);
        (r.x + 3, r.y)
    }

    /// Every check that holds on every path. `what` names the path in a failure.
    fn check(&self, what: &str) {
        let (area, aspect) = (self.a.pane_area(), self.a.cell_aspect());
        assert!(self.a.layout.valid(area, aspect), "{what}: the layout doesn't fit: {:?}", self.a.layout);
        // nothing is left held once the button is up (every path ends with it up)
        assert!(self.a.tile.is_none(), "{what}: a Pane is still held");
        assert!(self.a.drag.is_none() && self.a.divider_from.is_none(), "{what}: a divider is still held");
        assert!(!self.a.moving_now(), "{what}: still moving after the path");
        // what's dropped is what was previewed (nothing previewed, or a hold ended without a release: nothing changes)
        for (i, (before, preview, after)) in self.drops.iter().enumerate() {
            assert_eq!(after, preview.as_ref().unwrap_or(before), "{what}: drop {i} landed somewhere other than its preview");
        }
        // the layout (a divider's too, as it is dragged) and every preview fit, frame by frame
        for f in &self.frames {
            assert!(f.fits, "{what}: a layout or preview that doesn't fit the window at {:.0} ms", f.t);
        }
        // no flicker: a preview never comes back to the one it left within a person's reaction time
        let shown: Vec<(f64, Option<Zone>)> = self.frames.iter().filter(|f| f.held).fold(vec![], |mut v: Vec<(f64, Option<Zone>)>, f| {
            if v.last().is_none_or(|l| l.1 != f.zone) {
                v.push((f.t, f.zone.clone()));
            }
            v
        });
        for w in shown.windows(3) {
            assert!(!(w[0].1 == w[2].1 && w[2].0 - w[1].0 < FOLLOW), "{what}: the preview flickered {:?} → {:?} → back within {:.0} ms", w[0].1, w[1].1, w[2].0 - w[1].0);
        }
        // …and no zone is shown for less than a dwell's shortest, however many it passes
        for w in shown.windows(2) {
            assert!(w[1].0 - w[0].0 >= QUICK_DWELL.as_secs_f64() * 1000.0 - 1.0 || w[0].1.is_none(), "{what}: {:?} shown for only {:.0} ms", w[0].1, w[1].0 - w[0].0);
        }
    }

    /// How often the shown zone changed while a Pane was held.
    fn changes(&self) -> usize {
        self.frames.windows(2).filter(|w| w[1].held && w[0].held && w[0].zone != w[1].zone).count()
    }
}

/// A preview a person can follow: one gone again sooner than this is flicker. (The dwell makes 100–150 ms the
/// shortest a random hand gets: a zone it rested in for a dwell and then left.)
const FOLLOW: f64 = 100.0;

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
fn held_in(layout: &Node) -> String {
    let names = layout.slot_names();
    if names.iter().any(|s| s == "stream") { "stream".into() } else { names.into_iter().find(|s| !["queue", "library", "home"].contains(&s.as_str())).unwrap() }
}
/// The Pane a path picks up now.
fn held(r: &Replay) -> String {
    held_in(&r.a.layout)
}

/// `shape` played on every Design with a mouse and a trackpad, at every frame rate and with moves combined or not,
/// each checked; the replays come back for what a test checks on top.
fn every_way(name: &str, shape: impl Fn(&Replay, &mut Hand)) -> Vec<(String, Replay)> {
    let mut out = vec![];
    for (design, d) in DESIGNS.iter().enumerate() {
        for (dev, kind) in [(MOUSE, "mouse"), (TRACKPAD, "trackpad")] {
            for (seed, &(fps, combine)) in RIGS.iter().enumerate() {
                let mut r = Replay::new(design, fps, combine);
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

/// What dropping the held Pane on `zone` makes of the layout the path started from.
fn dropped_on(r: &Replay, zone: Zone) -> Node {
    r.start.moved(&held_in(&r.start), &zone).unwrap()
}

#[test]
fn a_fast_fling_across_panes_shows_nothing_and_puts_the_pane_back() {
    for (what, r) in every_way("fling", |r, h| {
        let me = held(r);
        let (x, y) = r.title(&me);
        h.press(x, y);
        for s in r.a.layout.slot_names().iter().filter(|s| **s != me) {
            let c = centre(r.rect(s));
            h.glide(c.0, c.1, 50.0);
        }
        h.release();
    }) {
        assert_eq!(r.changes(), 0, "{what}: a preview while flinging");
        assert_eq!(r.a.layout, r.start, "{what}: nothing was shown, so nothing moved");
    }
}

#[test]
fn a_slow_creep_into_a_pane_shows_it_and_drops_there() {
    for (what, r) in every_way("creep", |r, h| {
        let (x, y) = r.title(&held(r));
        let q = r.rect("queue");
        let (qx, qy) = centre(q);
        h.press(x, y).glide(q.x as f64 - 6.0, qy, 150.0);
        let mut cx = q.x as f64 - 6.0;
        while cx < qx {
            cx += 1.0;
            h.glide(cx, qy, 110.0);
        }
        h.rest(60.0).release();
    }) {
        assert_eq!(r.a.layout, dropped_on(&r, Zone::Swap("queue".into())), "{what}");
    }
}

#[test]
fn jitter_on_a_border_between_two_zones_does_not_flicker() {
    // on the border between the queue's left band and its centre, and on the border between two Panes
    for (name, band) in [("jitter on a band", true), ("jitter on a border", false)] {
        for (what, r) in every_way(name, |r, h| {
            let (x, y) = r.title(&held(r));
            let q = r.rect("queue");
            let qy = centre(q).1;
            let at = if band { q.x as f64 + (q.width as f64 * 0.2).round() } else { q.x as f64 };
            h.press(x, y).glide(at, qy, 200.0).jitter(if band { 2.0 } else { -3.0 }, 0.0, 10, 30.0).rest(60.0).release();
        }) {
            assert!(r.changes() <= 2, "{what}: the preview changed {} times", r.changes());
        }
    }
}

#[test]
fn overshooting_a_pane_and_coming_back_drops_on_it() {
    every_way("overshoot, let go at once", |r, h| {
        let (x, y) = r.title(&held(r));
        let (qx, qy) = centre(r.rect("queue"));
        h.press(x, y).glide(qx + 30.0, qy + 4.0, 160.0).glide(qx, qy, 120.0).rest(20.0).release();
    });
    for (what, r) in every_way("overshoot and rest", |r, h| {
        let (x, y) = r.title(&held(r));
        let (qx, qy) = centre(r.rect("queue"));
        h.press(x, y).glide(qx + 30.0, qy + 4.0, 160.0).glide(qx, qy, 120.0).rest(250.0).release();
    }) {
        assert_eq!(r.a.layout, dropped_on(&r, Zone::Swap("queue".into())), "{what}");
    }
}

#[test]
fn a_pause_on_one_pane_then_going_on_to_another_drops_on_the_second() {
    for (what, r) in every_way("pause, then on", |r, h| {
        let (x, y) = r.title(&held(r));
        let (qx, qy) = centre(r.rect("queue"));
        let lib = centre(r.rect("library"));
        h.press(x, y).glide(qx, qy, 200.0).rest(300.0).glide(lib.0, lib.1, 250.0).rest(200.0).release();
    }) {
        assert_eq!(r.a.layout, dropped_on(&r, Zone::Swap("library".into())), "{what}");
        assert!(r.changes() <= 3, "{what}: {} changes for two pauses", r.changes());
    }
}

/// Picked up, rested on the queue, then out of the window to the right.
fn out_of_the_window(r: &Replay, h: &mut Hand) {
    let (x, y) = r.title(&held(r));
    let (qx, qy) = centre(r.rect("queue"));
    h.press(x, y).glide(qx, qy, 200.0).rest(200.0).glide(W as f64 + 20.0, qy, 120.0);
}

#[test]
fn letting_go_outside_the_window_drops_what_was_shown() {
    // the terminal reports the release at its edge
    every_way("let go outside the window", |r, h| {
        out_of_the_window(r, h);
        h.rest(150.0).release();
    });
}

#[test]
fn a_release_the_client_never_heard_puts_the_pane_back() {
    // the release went to another window: the next we hear is the pointer moving with no button down
    for (what, r) in every_way("released unheard", |r, h| {
        out_of_the_window(r, h);
        h.then(300.0, Ev::Hover(W - 1, 20)).then(20.0, Ev::Hover(W - 5, 21));
    }) {
        assert_eq!(r.a.layout, r.start, "{what}: put back where it was");
    }
    // another app took the focus mid-drag
    every_way("focus lost", |r, h| {
        out_of_the_window(r, h);
        h.then(5.0, Ev::FocusLost).then(400.0, Ev::Hover(5, 5));
    });
    // …or the next thing is a fresh press: the Pane goes back and the press is a press
    for (what, r) in every_way("pressed again", |r, h| {
        out_of_the_window(r, h);
        let (lx, ly) = r.title("library");
        h.then(300.0, Ev::Press(lx, ly)).then(30.0, Ev::Release(lx, ly));
    }) {
        assert_eq!(r.a.layout, r.start, "{what}: put back where it was");
        assert_eq!(r.a.focus, "library", "{what}: the press clicked the library's title");
    }
}

#[test]
fn letting_go_over_the_top_bar_or_the_player_drops_what_was_shown() {
    for top in [true, false] {
        let y_out = if top { 0.0 } else { H as f64 - 2.0 };
        // from the queue straight out and let go: the queue was shown, so the queue it is
        for (what, r) in every_way(if top { "let go over the top bar" } else { "let go over the player" }, |r, h| {
            let (x, y) = r.title(&held(r));
            let (qx, qy) = centre(r.rect("queue"));
            let hx = centre(r.rect("home")).0;
            h.press(x, y).glide(qx, qy, 200.0).rest(200.0).glide(hx, y_out, 150.0).rest(20.0).release();
        }) {
            assert_eq!(r.a.layout, dropped_on(&r, Zone::Swap("queue".into())), "{what}");
        }
        // resting there: whatever it shows (a whole row at that edge, or the Pane next to it, whose zone holds on
        // for a row) is the drop
        every_way(if top { "rest over the top bar" } else { "rest over the player" }, |r, h| {
            let (x, y) = r.title(&held(r));
            let hx = centre(r.rect("home")).0;
            h.press(x, y).glide(hx, y_out, 200.0).rest(250.0).release();
        });
    }
}

#[test]
fn a_resize_mid_drag_keeps_every_preview_and_the_drop_in_the_new_window() {
    let grab = |r: &Replay, h: &mut Hand| {
        let (x, y) = r.title(&held(r));
        let (qx, qy) = centre(r.rect("queue"));
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
    // resting on a spot inside both windows, then the window shrinks and it's let go at once: what drops is the zone
    // under the pointer in the new window (or nothing), never the one the old window had there
    for (x, y) in [(20u16, 8u16), (40, 15), (60, 20), (80, 10), (30, 22), (85, 24)] {
        for (what, r) in every_way(&format!("rest at {x},{y}, resized, let go"), |r, h| {
            let (tx, ty) = r.title(&held(r));
            h.press(tx, ty).glide(x as f64, y as f64, 200.0).rest(250.0).then(1.0, Ev::Drag(x, y)).then(5.0, Ev::Resize(100, 30)).then(20.0, Ev::Release(x, y));
        }) {
            let me = held_in(&r.start);
            let (area, aspect) = (r.a.pane_area(), r.a.cell_aspect());
            let here = crate::tiling::zone_at(&r.start.compute_shaped(area, aspect).slots, area, &me, x, y);
            let new = here.and_then(|z| r.start.moved(&me, &z));
            assert!(r.a.layout == r.start || Some(&r.a.layout) == new.as_ref(), "{what}: dropped on a zone of the old window");
        }
    }
}

#[test]
fn a_right_click_mid_drag_does_not_drop_the_pane() {
    for (what, r) in every_way("right click", |r, h| {
        let (x, y) = r.title(&held(r));
        let (qx, qy) = centre(r.rect("queue"));
        let lib = centre(r.rect("library"));
        h.press(x, y).glide(qx, qy, 200.0).rest(200.0).then(5.0, Ev::RightClick(qx as u16, qy as u16)).glide(lib.0, lib.1, 150.0).rest(200.0).release();
    }) {
        assert_eq!(r.a.layout, dropped_on(&r, Zone::Swap("library".into())), "{what}: still held, dropped on the library");
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
            assert!(!r.frames.iter().any(|f| f.held), "{what}: a press on a divider picked up a Pane");
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
            let (area, aspect) = (r.a.pane_area(), r.a.cell_aspect());
            let i = r.divider.unwrap();
            let d = r.a.layout.compute_shaped(area, aspect).dividers[i].clone();
            let at = r.a.layout.boundary(&d).unwrap();
            let mut further = r.a.layout.clone();
            further.drag(&d, if low { at.saturating_sub(1) } else { at + 1 });
            assert!(further == r.a.layout || !further.valid(area, aspect), "{what}: stopped short at {at}");
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
        let names = r.a.layout.slot_names();
        let me = names[(rng.next() % names.len() as u64) as usize].clone();
        let (x, y) = r.title(&me);
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

/// One path drawn frame by frame with Panes in motion: what's on screen stays inside the layout all the way.
#[test]
fn a_drag_drawn_frame_by_frame_stays_inside_the_layout() {
    for (dev, combine) in [(MOUSE, true), (TRACKPAD, false)] {
        let mut r = Replay::new(0, 120, combine);
        r.draw = true;
        let (x, y) = r.title("stream");
        let (qx, qy) = centre(r.rect("queue"));
        let lib = centre(r.rect("library"));
        let mut h = Hand::new(dev, 5, 10.0);
        h.press(x, y).glide(qx, qy, 200.0).rest(200.0).glide(lib.0, lib.1, 150.0).rest(200.0).release();
        let path = h.done();
        r.play(&path, 400.0);
        r.check("drawn");
        let body = r.a.pane_area();
        assert!(r.a.drawn.iter().all(|d| d.x >= body.x && d.y >= body.y && d.right() <= body.right() && d.bottom() <= body.bottom()));
        assert_eq!(r.a.layout, dropped_on(&r, Zone::Swap("library".into())));
    }
}

