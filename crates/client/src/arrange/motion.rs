//! Panes in motion (owner, 2026-11-01: "tiles that move around in real time, like iOS apps"). Every Pane springs
//! from where it is drawn to where the layout puts it whenever that changes (a drop's candidate, a commit, an undo);
//! the Pane held by the mouse floats over the rest and lands in its slot. Only rectangles live here: what to draw in
//! them is the UI's. Nothing allocates per frame once the set of Panes is known.
//!
//! Each edge of each rectangle is a critically damped spring (position and velocity), solved exactly for the time
//! that really passed: the same moment looks the same at 30, 120 or 240 frames a second, and a new target mid-flight
//! bends the path from the speed it already has instead of starting a fresh curve (the owner's "teleporting" when a
//! fast drag changed the target every few frames, 2026-11-01).

use ratatui::layout::Rect;
use std::time::Instant;
#[cfg(test)]
use std::time::Duration;

/// The springs' angular frequency (1/s). A critically damped spring covers half the way in 1.68/ω (47 ms) and is
/// within a cell of a 40-cell move after 5.6/ω (155 ms): a single move settles in about 150–200 ms.
pub const OMEGA: f32 = 36.0;
/// An edge this close to its target (cells), and this slow (cells/s), is there: it snaps exactly onto it.
const REST_X: f32 = 0.35;
const REST_V: f32 = 10.0;
/// Longer than any move takes: tests skip this far ahead to see where things end.
#[cfg(test)]
pub const SETTLE: Duration = Duration::from_millis(400);
/// A frame later than this (a stall, a suspended terminal) counts as this long: things arrive, they don't vanish.
const MAX_DT: f32 = 1.0;

/// One coordinate on its way to a target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub x: f32,
    pub v: f32,
}

impl Spring {
    pub fn at(x: f32) -> Spring {
        Spring { x, v: 0.0 }
    }

    /// `dt` seconds on toward `to`: the exact solution x(t) = to + (d + (v + ωd)t)e^(−ωt), so it doesn't matter how
    /// the time is cut into frames. Close enough and slow enough, it is exactly there.
    pub fn step(&mut self, to: f32, dt: f32) {
        if self.x == to && self.v == 0.0 {
            return;
        }
        let dt = if dt.is_finite() { dt.clamp(0.0, MAX_DT) } else { 0.0 };
        let (d, w) = (self.x - to, OMEGA);
        let e = (-w * dt).exp();
        let c = self.v + w * d;
        let (nd, nv) = ((d + c * dt) * e, (self.v - w * c * dt) * e);
        if nd.abs() < REST_X && nv.abs() < REST_V || !nd.is_finite() || !nv.is_finite() {
            *self = Spring::at(to);
        } else {
            *self = Spring { x: to + nd, v: nv };
        }
    }

    pub fn rests_at(&self, to: f32) -> bool {
        self.x == to && self.v == 0.0
    }
}

/// A rectangle on its way somewhere: its four edges (left, top, right, bottom) spring on their own, so a Pane inside
/// an area stays inside it and grows or shrinks as it goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectSpring {
    edges: [Spring; 4],
    pub to: Rect,
}

fn edges(r: Rect) -> [f32; 4] {
    [r.x as f32, r.y as f32, r.right() as f32, r.bottom() as f32]
}

impl RectSpring {
    pub fn still(r: Rect) -> RectSpring {
        RectSpring { edges: edges(r).map(Spring::at), to: r }
    }

    /// From `from`, at rest, heading for `to`.
    pub fn from_to(from: Rect, to: Rect) -> RectSpring {
        RectSpring { edges: edges(from).map(Spring::at), to }
    }

    /// Head for `to` from wherever it is, keeping its speed.
    pub fn retarget(&mut self, to: Rect) {
        self.to = to;
    }

    pub fn step(&mut self, dt: f32) {
        let to = edges(self.to);
        for (s, t) in self.edges.iter_mut().zip(to) {
            s.step(t, dt);
        }
    }

    /// Where it is, in whole cells (each edge rounded on its own), never smaller than one cell.
    pub fn rect(&self) -> Rect {
        let [l, t, r, b] = self.edges.map(|e| e.x.round().max(0.0).min(u16::MAX as f32) as u16);
        let (x, y) = (l.min(r), t.min(b));
        Rect::new(x, y, l.max(r).saturating_sub(x).max(1), t.max(b).saturating_sub(y).max(1))
    }

    pub fn done(&self) -> bool {
        let to = edges(self.to);
        self.edges.iter().zip(to).all(|(s, t)| s.rests_at(t))
    }
}

/// Where everything is drawn: one spring per Pane on screen, the floating Pane's, and the area they live in.
pub struct Motion {
    /// off: everything is drawn where the layout puts it (tests, unless they ask)
    pub on: bool,
    area: Rect,
    panes: Vec<(String, RectSpring)>,
    /// the last frame, and the time since the one before it (s)
    last: Option<Instant>,
    dt: f32,
    /// time a test let pass
    skipped: f32,
    /// a Pane held by the mouse grows from its place in the layout to its floating size: 0 → 1
    pub lift: Spring,
    /// the Pane flying from where it was let go into its slot
    pub float: Option<RectSpring>,
    /// that Pane's name: drawn raised until it lands
    pub landing: Option<String>,
}

impl Motion {
    pub fn new(on: bool) -> Motion {
        Motion { on, area: Rect::default(), panes: vec![], last: None, dt: 0.0, skipped: 0.0, lift: Spring::at(1.0), float: None, landing: None }
    }

    /// A new frame at `now`: the time since the last one moves the held Pane's lift and a landing on. (Under test,
    /// time passes only when a test says so: `skip`.)
    pub fn frame(&mut self, now: Instant) {
        let dt = self.last.map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
        let dt = if cfg!(test) { std::mem::take(&mut self.skipped) } else { dt };
        self.last = Some(now);
        self.frame_dt(dt);
    }

    /// The same with the time given (tests).
    pub fn frame_dt(&mut self, dt: f32) {
        self.dt = dt;
        if !self.on {
            self.lift = Spring::at(1.0);
            self.float = None;
            self.landing = None;
            return;
        }
        self.lift.step(1.0, dt);
        if let Some(f) = self.float.as_mut() {
            f.step(dt);
            if f.done() {
                self.float = None;
                self.landing = None;
            }
        }
    }

    /// A Pane was picked up: it starts growing (or shrinking) from its place toward its floating size.
    pub fn lifted(&mut self) {
        self.float = None;
        self.landing = None;
        self.lift = Spring::at(if self.on { 0.0 } else { 1.0 });
    }

    /// The held Pane let go at `from`: it flies to `to`, drawn over the rest until it is there.
    pub fn land(&mut self, slot: &str, from: Rect, to: Rect) {
        if self.on && from != to {
            self.float = Some(RectSpring::from_to(from, to));
            self.landing = Some(slot.into());
        } else {
            self.float = None;
            self.landing = None;
        }
    }

    /// How far the held Pane has lifted (0: in its place, 1: at its floating size).
    pub fn lifted_by(&self) -> f32 {
        if self.on { self.lift.x.clamp(0.0, 1.0) } else { 1.0 }
    }

    /// The landing Pane's rectangle this frame.
    pub fn float_at(&self) -> Option<Rect> {
        self.float.map(|f| clamp(f.rect(), self.area))
    }

    /// This frame's rectangles for `targets` (where the layout puts each Pane) into `out`, index for index. A Pane
    /// whose target changed springs there from where it was, at the speed it had; a new Pane appears in place; `snap`
    /// (a divider being dragged, a new terminal size) puts everything where it belongs at once.
    pub fn step(&mut self, targets: &[(String, Rect)], area: Rect, snap: bool, out: &mut Vec<Rect>) {
        let moved = area != self.area;
        let snap = snap || !self.on || moved;
        self.area = area;
        // forget Panes that left (no allocation unless the set changed)
        if self.panes.len() != targets.len() || self.panes.iter().any(|(s, _)| !targets.iter().any(|(t, _)| t == s)) {
            self.panes.retain(|(s, _)| targets.iter().any(|(t, _)| t == s));
        }
        out.clear();
        for (name, to) in targets {
            let r = match self.panes.iter_mut().find(|(s, _)| s == name) {
                Some((_, sp)) if snap => {
                    *sp = RectSpring::still(*to);
                    *to
                }
                Some((_, sp)) => {
                    sp.retarget(*to);
                    sp.step(self.dt);
                    sp.rect()
                }
                None => {
                    self.panes.push((name.clone(), RectSpring::still(*to)));
                    *to
                }
            };
            out.push(clamp(r, area));
        }
        // each frame's time is spent once
        self.dt = 0.0;
        if moved {
            self.float = None;
            self.landing = None;
        }
    }

    /// Whether anything is still on its way (the Client then draws at the motion frame rate).
    pub fn busy(&self) -> bool {
        self.on && (self.float.is_some() || !self.lift.rests_at(1.0) || self.panes.iter().any(|(_, s)| !s.done()))
    }

    #[cfg(test)]
    /// Let `by` pass before the next frame.
    pub fn skip(&mut self, by: Duration) {
        self.skipped += by.as_secs_f32();
    }
}

/// Inside `area`, at least one cell.
pub fn clamp(r: Rect, area: Rect) -> Rect {
    if area.width == 0 || area.height == 0 {
        return r;
    }
    let x = r.x.clamp(area.x, area.right() - 1);
    let y = r.y.clamp(area.y, area.bottom() - 1);
    Rect::new(x, y, r.width.clamp(1, area.right() - x), r.height.clamp(1, area.bottom() - y))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spring run toward `to` for `secs` at `fps`, the target changing to `then` `switch` s in (on a frame
    /// boundary at every rate used here).
    fn run(fps: f32, secs: f32, from: f32, to: f32, switch: Option<(f32, f32)>) -> Spring {
        let mut s = Spring::at(from);
        let frames = (secs * fps).round() as usize;
        let at = switch.map_or(usize::MAX, |(t, _)| (t * fps).round() as usize);
        for i in 0..frames {
            s.step(if i >= at { switch.unwrap().1 } else { to }, 1.0 / fps);
        }
        s
    }

    #[test]
    fn a_single_move_settles_in_about_two_hundred_ms_and_ends_exactly() {
        for dist in [5.0f32, 40.0, 120.0] {
            let mut s = Spring::at(0.0);
            let mut t = 0.0;
            let mut half = None;
            while !s.rests_at(dist) {
                s.step(dist, 1.0 / 240.0);
                t += 1.0 / 240.0;
                if half.is_none() && s.x >= dist / 2.0 {
                    half = Some(t);
                }
                assert!(s.x <= dist + 1e-3, "a critically damped spring from rest never overshoots: {} at {t}", s.x);
                assert!(t < 0.5, "{dist}: never settles");
            }
            assert_eq!(s.x, dist, "exactly there");
            assert!((0.035..0.06).contains(&half.unwrap()), "{dist}: half way at {half:?}");
            assert!((0.09..0.26).contains(&t), "{dist} cells settle in {t:.3} s");
        }
    }

    #[test]
    fn the_frame_rate_does_not_change_the_path() {
        // the same moments at 30, 120 and 240 fps, with a retarget mid-flight; within a hundredth of a cell
        for (secs, switch) in [(0.1, None), (0.2, Some((0.1, -30.0))), (0.6, Some((0.1, -30.0)))] {
            let a = run(30.0, secs, 0.0, 50.0, switch);
            let b = run(120.0, secs, 0.0, 50.0, switch);
            let c = run(240.0, secs, 0.0, 50.0, switch);
            assert!((a.x - b.x).abs() < 0.01 && (b.x - c.x).abs() < 0.01, "{secs}: {a:?} {b:?} {c:?}");
        }
        assert_eq!(run(30.0, 0.6, 0.0, 50.0, Some((0.1, -30.0))).x, -30.0, "settled exactly, whatever the rate");
        assert_eq!(run(240.0, 0.6, 0.0, 50.0, Some((0.1, -30.0))).x, -30.0);
    }

    #[test]
    fn a_new_target_mid_flight_keeps_the_speed_it_had() {
        let mut s = Spring::at(0.0);
        for _ in 0..12 {
            s.step(60.0, 1.0 / 240.0);
        }
        let (x, v) = (s.x, s.v);
        assert!(v > 100.0, "well under way: {v}");
        // the target jumps the other way: no jump in place or speed, it turns
        s.step(-40.0, 0.0);
        assert_eq!((s.x, s.v), (x, v));
        let mut prev = s;
        for _ in 0..4 {
            s.step(-40.0, 1.0 / 240.0);
            assert!((s.x - prev.x).abs() < v / 240.0 * 1.5, "a smooth step: {prev:?} → {s:?}");
            assert!(s.v < prev.v, "slowing, then turning");
            prev = s;
        }
        while !s.rests_at(-40.0) {
            s.step(-40.0, 1.0 / 240.0);
        }
    }

    #[test]
    fn a_rect_moves_every_edge_toward_its_target_and_lands_on_it() {
        let (a, b) = (Rect::new(0, 1, 30, 29), Rect::new(72, 23, 41, 21));
        let mut r = RectSpring::from_to(a, b);
        let mut prev = a;
        let mut frames = 0;
        while !r.done() {
            r.step(1.0 / 120.0);
            let now = r.rect();
            for (p, q, end) in [(prev.x, now.x, b.x), (prev.y, now.y, b.y), (prev.right(), now.right(), b.right()), (prev.bottom(), now.bottom(), b.bottom())] {
                assert!((p <= q && q <= end) || (p >= q && q >= end), "{frames}: {prev:?} → {now:?}");
            }
            assert!(now.width >= 1 && now.height >= 1);
            prev = now;
            frames += 1;
        }
        assert_eq!(r.rect(), b);
        assert!((12..=32).contains(&frames), "{frames} frames at 120 fps");
        // the same end at any rate
        for fps in [30.0, 240.0] {
            let mut r = RectSpring::from_to(a, b);
            for _ in 0..(fps * 0.4) as usize {
                r.step(1.0 / fps);
            }
            assert_eq!(r.rect(), b);
        }
        // a rect of nothing is still a cell
        assert_eq!(RectSpring::still(Rect::new(7, 7, 0, 0)).rect(), Rect::new(7, 7, 1, 1));
    }

    #[test]
    fn panes_spring_to_new_places_appear_in_place_and_snap_when_asked() {
        let area = Rect::new(0, 1, 100, 40);
        let mut m = Motion::new(true);
        let mut out = vec![];
        let a = vec![("x".to_string(), Rect::new(0, 1, 50, 40)), ("y".to_string(), Rect::new(50, 1, 50, 40))];
        m.frame_dt(0.0);
        m.step(&a, area, false, &mut out);
        assert_eq!(out, vec![a[0].1, a[1].1], "the first frame is where they are");
        // swapped: both on their way, then there
        let b = vec![("x".to_string(), Rect::new(50, 1, 50, 40)), ("y".to_string(), Rect::new(0, 1, 50, 40))];
        m.frame_dt(0.0);
        m.step(&b, area, false, &mut out);
        assert_eq!(out, vec![a[0].1, a[1].1]);
        assert!(m.busy());
        m.frame_dt(0.06);
        m.step(&b, area, false, &mut out);
        assert!(out[0].x > 0 && out[0].x < 50 && out[1].x > 0 && out[1].x < 50, "{out:?}");
        // a frame's time is spent once: drawing again without a new frame moves nothing
        let again = out.clone();
        m.step(&b, area, false, &mut out);
        assert_eq!(out, again);
        m.frame_dt(SETTLE.as_secs_f32());
        m.step(&b, area, false, &mut out);
        assert_eq!(out, vec![b[0].1, b[1].1]);
        assert!(!m.busy());
        // a third Pane appears where it belongs; one that left is forgotten
        let c = vec![("x".to_string(), Rect::new(0, 1, 30, 40)), ("z".to_string(), Rect::new(30, 1, 70, 40))];
        m.step(&c, area, false, &mut out);
        assert_eq!(out[1], c[1].1);
        assert_eq!(m.panes.len(), 2);
        // snap
        m.step(&a, area, true, &mut out);
        assert_eq!(out, vec![a[0].1, a[1].1]);
        // a new area (the terminal resized) snaps too, and nothing is ever drawn outside it
        let small = Rect::new(0, 1, 60, 20);
        m.frame_dt(0.02);
        m.step(&b, area, false, &mut out);
        m.step(&[("x".to_string(), Rect::new(30, 1, 30, 20))], small, false, &mut out);
        assert_eq!(out, vec![Rect::new(30, 1, 30, 20)]);
        // off: always where they belong
        let mut off = Motion::new(false);
        off.step(&a, area, false, &mut out);
        off.step(&b, area, false, &mut out);
        assert_eq!(out, vec![b[0].1, b[1].1]);
        assert!(!off.busy());
    }

    #[test]
    fn the_lift_and_a_landing_run_on_the_frames_time() {
        let mut m = Motion::new(true);
        m.step(&[], Rect::new(0, 1, 100, 40), false, &mut vec![]);
        m.lifted();
        assert_eq!(m.lifted_by(), 0.0);
        m.frame_dt(0.05);
        assert!((0.3..0.8).contains(&m.lifted_by()), "{}", m.lifted_by());
        m.frame_dt(SETTLE.as_secs_f32());
        assert_eq!(m.lifted_by(), 1.0);
        m.land("p", Rect::new(40, 10, 20, 8), Rect::new(0, 1, 50, 20));
        assert!(m.busy() && m.landing.is_some());
        m.frame_dt(0.05);
        let mid = m.float_at().unwrap();
        assert!(mid.x < 40 && mid.x > 0, "{mid:?}");
        m.frame_dt(SETTLE.as_secs_f32());
        assert!(m.float_at().is_none() && m.landing.is_none() && !m.busy(), "landed");
    }

    #[test]
    fn clamping_keeps_a_cell_inside_the_area() {
        let area = Rect::new(0, 1, 50, 20);
        assert_eq!(clamp(Rect::new(45, 18, 20, 10), area), Rect::new(45, 18, 5, 3));
        assert_eq!(clamp(Rect::new(80, 0, 0, 0), area), Rect::new(49, 1, 1, 1));
    }
}
