//! Panes in motion (owner, 2026-11-01: "tiles that move around in real time, like iOS apps"). Every Pane eases from
//! where it is drawn to where the layout puts it whenever that changes (a drop's candidate, a commit, an undo); the
//! Pane held by the mouse floats over the rest and lands in its slot. Only rectangles live here: what to draw in them
//! is the UI's. Nothing allocates per frame once the set of Panes is known.

use ratatui::layout::Rect;
use std::time::{Duration, Instant};

/// How long a Pane takes to reach a new place.
pub const DURATION: Duration = Duration::from_millis(180);

/// Fast at first, settling gently (cubic ease-out). 0 → 0, 1 → 1, never outside, never backwards.
pub fn ease_out(t: f32) -> f32 {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 1.0 };
    1.0 - (1.0 - t).powi(3)
}

/// `a` a share `k` of the way to `b`: each edge moves on its own, so a Pane inside an area stays inside it. Never
/// smaller than one cell.
pub fn lerp_rect(a: Rect, b: Rect, k: f32) -> Rect {
    let k = k.clamp(0.0, 1.0);
    let mix = |p: u16, q: u16| (p as f32 + (q as f32 - p as f32) * k).round() as u16;
    let (x, y) = (mix(a.x, b.x), mix(a.y, b.y));
    let (r, bot) = (mix(a.right(), b.right()), mix(a.bottom(), b.bottom()));
    Rect::new(x, y, r.saturating_sub(x).max(1), bot.saturating_sub(y).max(1))
}

/// One rectangle on its way somewhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    pub from: Rect,
    pub to: Rect,
    pub start: Instant,
}

impl Tween {
    pub fn still(r: Rect, now: Instant) -> Tween {
        Tween { from: r, to: r, start: now }
    }

    fn progress(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.start).as_secs_f32() / DURATION.as_secs_f32()
    }

    pub fn at(&self, now: Instant) -> Rect {
        if self.from == self.to {
            return self.to;
        }
        let t = self.progress(now);
        if t >= 1.0 { self.to } else { lerp_rect(self.from, self.to, ease_out(t)) }
    }

    pub fn done(&self, now: Instant) -> bool {
        self.from == self.to || self.progress(now) >= 1.0
    }

    /// Head for `to` from wherever it is now (a target that changes midway turns smoothly).
    pub fn retarget(&mut self, to: Rect, now: Instant) {
        if to != self.to {
            self.from = self.at(now);
            self.to = to;
            self.start = now;
        }
    }

    /// Head for `to`, which moves with the pointer, without starting over: the way there keeps its timing and ends
    /// exactly on the pointer.
    pub fn follow(&mut self, to: Rect) {
        self.to = to;
    }
}

/// Where everything is drawn: one Tween per Pane on screen, the floating Pane's, and the area they live in.
#[derive(Default)]
pub struct Motion {
    /// off: everything is drawn where the layout puts it (tests, unless they ask)
    pub on: bool,
    area: Rect,
    panes: Vec<(String, Tween)>,
    /// the Pane held by the mouse, or flying back into its slot after a release
    pub float: Option<Tween>,
    /// the Pane flying into its slot: drawn raised until it lands
    pub landing: Option<String>,
}

impl Motion {
    pub fn new(on: bool) -> Motion {
        Motion { on, ..Default::default() }
    }

    /// This frame's rectangles for `targets` (where the layout puts each Pane) into `out`, index for index. A Pane
    /// whose target changed eases there from where it was; a new Pane appears in place; `snap` (a divider being
    /// dragged, a new terminal size) puts everything where it belongs at once.
    pub fn step(&mut self, targets: &[(String, Rect)], area: Rect, now: Instant, snap: bool, out: &mut Vec<Rect>) {
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
                Some((_, tw)) if snap => {
                    *tw = Tween::still(*to, now);
                    *to
                }
                Some((_, tw)) => {
                    tw.retarget(*to, now);
                    tw.at(now)
                }
                None => {
                    self.panes.push((name.clone(), Tween::still(*to, now)));
                    *to
                }
            };
            out.push(clamp(r, area));
        }
        if moved {
            self.float = None;
            self.landing = None;
        }
    }

    /// The floating Pane's rectangle this frame; a landing that has arrived ends.
    pub fn float_at(&mut self, now: Instant) -> Option<Rect> {
        let t = self.float?;
        if self.landing.is_some() && t.done(now) {
            self.float = None;
            self.landing = None;
            return None;
        }
        Some(clamp(t.at(now), self.area))
    }

    /// Whether anything is still on its way (the Client then draws at full speed).
    pub fn busy(&self, now: Instant) -> bool {
        self.on && (self.float.is_some() || self.panes.iter().any(|(_, t)| !t.done(now)))
    }

    #[cfg(test)]
    /// Pretend `by` has passed since every Tween started.
    pub fn age(&mut self, by: Duration) {
        let back = |t: &mut Tween| t.start = t.start.checked_sub(by).unwrap_or(t.start);
        self.panes.iter_mut().for_each(|(_, t)| back(t));
        if let Some(t) = self.float.as_mut() {
            back(t);
        }
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

    #[test]
    fn easing_is_monotone_and_ends_exactly() {
        let mut last = 0.0;
        for i in 0..=100 {
            let e = ease_out(i as f32 / 100.0);
            assert!(e >= last && (0.0..=1.0).contains(&e), "{i}: {e}");
            last = e;
        }
        assert_eq!((ease_out(0.0), ease_out(1.0), ease_out(7.0), ease_out(-1.0), ease_out(f32::NAN)), (0.0, 1.0, 1.0, 0.0, 1.0));
        // ease-out: more than half way at the half
        assert!(ease_out(0.5) > 0.8);
    }

    #[test]
    fn a_tween_moves_every_edge_monotonically_and_lands_on_its_target() {
        let (a, b) = (Rect::new(0, 1, 30, 29), Rect::new(72, 23, 41, 21));
        let t0 = Instant::now();
        let t = Tween { from: a, to: b, start: t0 };
        let mut prev = a;
        for ms in (0..=240).step_by(10) {
            let r = t.at(t0 + Duration::from_millis(ms));
            for (p, q, end) in [(prev.x, r.x, b.x), (prev.y, r.y, b.y), (prev.right(), r.right(), b.right()), (prev.bottom(), r.bottom(), b.bottom())] {
                assert!((p <= q && q <= end) || (p >= q && q >= end), "{ms} ms: {prev:?} → {r:?}");
            }
            assert!(r.width >= 1 && r.height >= 1);
            prev = r;
        }
        assert_eq!(t.at(t0), a);
        assert_eq!(t.at(t0 + DURATION), b, "exactly there at the end");
        assert!(t.done(t0 + DURATION) && !t.done(t0 + DURATION / 2));
        // a rect shrinking to nothing stays a cell
        assert_eq!(lerp_rect(Rect::new(5, 5, 4, 4), Rect::new(7, 7, 0, 0), 1.0), Rect::new(7, 7, 1, 1));
    }

    #[test]
    fn a_new_target_midway_turns_from_where_it_is() {
        let t0 = Instant::now();
        let mut t = Tween::still(Rect::new(0, 0, 20, 10), t0);
        t.retarget(Rect::new(40, 0, 20, 10), t0);
        let mid = t0 + DURATION / 2;
        let here = t.at(mid);
        t.retarget(Rect::new(0, 20, 20, 10), mid);
        assert_eq!(t.at(mid), here, "no jump");
        assert_eq!(t.at(mid + DURATION), Rect::new(0, 20, 20, 10));
    }

    #[test]
    fn panes_ease_to_new_places_appear_in_place_and_snap_when_asked() {
        let area = Rect::new(0, 1, 100, 40);
        let mut m = Motion::new(true);
        let mut out = vec![];
        let t0 = Instant::now();
        let a = vec![("x".to_string(), Rect::new(0, 1, 50, 40)), ("y".to_string(), Rect::new(50, 1, 50, 40))];
        m.step(&a, area, t0, false, &mut out);
        assert_eq!(out, vec![a[0].1, a[1].1], "the first frame is where they are");
        // swapped: both on their way, then there
        let b = vec![("x".to_string(), Rect::new(50, 1, 50, 40)), ("y".to_string(), Rect::new(0, 1, 50, 40))];
        m.step(&b, area, t0, false, &mut out);
        assert_eq!(out, vec![a[0].1, a[1].1]);
        assert!(m.busy(t0));
        m.step(&b, area, t0 + DURATION / 3, false, &mut out);
        assert!(out[0].x > 0 && out[0].x < 50 && out[1].x > 0 && out[1].x < 50, "{out:?}");
        m.step(&b, area, t0 + DURATION, false, &mut out);
        assert_eq!(out, vec![b[0].1, b[1].1]);
        assert!(!m.busy(t0 + DURATION));
        // a third Pane appears where it belongs; one that left is forgotten
        let c = vec![("x".to_string(), Rect::new(0, 1, 30, 40)), ("z".to_string(), Rect::new(30, 1, 70, 40))];
        m.step(&c, area, t0 + DURATION, false, &mut out);
        assert_eq!(out[1], c[1].1);
        assert_eq!(m.panes.len(), 2);
        // snap
        m.step(&a, area, t0 + DURATION, true, &mut out);
        assert_eq!(out, vec![a[0].1, a[1].1]);
        // a new area (the terminal resized) snaps too, and nothing is ever drawn outside it
        let small = Rect::new(0, 1, 60, 20);
        m.step(&b, area, t0 + DURATION * 2, false, &mut out);
        m.step(&[("x".to_string(), Rect::new(30, 1, 30, 20))], small, t0 + DURATION * 2, false, &mut out);
        assert_eq!(out, vec![Rect::new(30, 1, 30, 20)]);
        // off: always where they belong
        let mut off = Motion::new(false);
        off.step(&a, area, t0, false, &mut out);
        off.step(&b, area, t0, false, &mut out);
        assert_eq!(out, vec![b[0].1, b[1].1]);
        assert!(!off.busy(t0));
    }

    #[test]
    fn clamping_keeps_a_cell_inside_the_area() {
        let area = Rect::new(0, 1, 50, 20);
        assert_eq!(clamp(Rect::new(45, 18, 20, 10), area), Rect::new(45, 18, 5, 3));
        assert_eq!(clamp(Rect::new(80, 0, 0, 0), area), Rect::new(49, 1, 1, 1));
    }
}
