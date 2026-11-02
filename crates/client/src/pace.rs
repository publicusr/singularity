//! Frame pacing (owner, 2026-11-01: motion at the monitor's rate, up to 240 Hz). Frames are due on a steady grid,
//! one period after the last one was due rather than one period after its work ended, so the spacing doesn't drift
//! by however long a frame took. Input never waits for a slow idle frame: it brings the next frame forward to one
//! period of the input rate after the last. The loop sleeps until a frame is due or input arrives (no busy-wait), and
//! after falling behind by a whole period it starts the grid again from now instead of drawing a burst to catch up.

use std::time::{Duration, Instant};

pub fn period(fps: u64) -> Duration {
    Duration::from_secs_f64(1.0 / fps.clamp(1, 1000) as f64)
}

pub struct Pacer {
    /// when the last frame was due (its place on the grid)
    last: Instant,
    /// input asked for a frame by then
    hurry: Option<Instant>,
}

impl Pacer {
    pub fn new(now: Instant) -> Pacer {
        Pacer { last: now, hurry: None }
    }

    /// When the next frame is due at `fps`: a period after the last one was due, or sooner if input asked.
    pub fn due(&self, fps: u64) -> Instant {
        let d = self.last + period(fps);
        self.hurry.map_or(d, |h| h.min(d))
    }

    /// Input arrived: the next frame comes at most a period of `fps` after the last one.
    pub fn input(&mut self, fps: u64) {
        let h = self.last + period(fps);
        self.hurry = Some(self.hurry.map_or(h, |x| x.min(h)));
    }

    /// A frame due at `due` (at `fps`) starts at `now`: the next one is timed from `due` while we keep up, from `now`
    /// once we have fallen a whole period behind.
    pub fn begin(&mut self, now: Instant, due: Instant, fps: u64) {
        self.hurry = None;
        self.last = if now.saturating_duration_since(due) >= period(fps) { now } else { due };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// Frames drawn over `secs` at `fps` when each takes `work`: their start times.
    fn frames(fps: u64, work: Duration, secs: u64) -> Vec<Instant> {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0);
        let mut now = t0;
        let mut starts = vec![];
        while now < t0 + Duration::from_secs(secs) {
            let due = p.due(fps);
            // sleep until due (never before), then work
            now = now.max(due);
            p.begin(now, due, fps);
            starts.push(now);
            now += work;
        }
        starts
    }

    #[test]
    fn frames_keep_a_steady_grid_whatever_the_work_takes() {
        for (fps, work) in [(240u64, MS), (240, 3 * MS), (120, 7 * MS), (30, 20 * MS)] {
            let f = frames(fps, work, 1);
            let gaps: Vec<Duration> = f.windows(2).map(|w| w[1] - w[0]).collect();
            assert!(gaps.iter().all(|g| *g == period(fps)), "{fps} fps with {work:?} of work: {gaps:?}");
            assert!((f.len() as i64 - fps as i64).abs() <= 1, "{fps}: {} frames in a second", f.len());
        }
        // sleeping after the work instead would have drifted: 240 fps with 3 ms of work is then only ~140 fps
        let naive = (1000.0 / (1000.0 / 240.0 + 3.0)) as u64;
        assert!(naive < 150);
    }

    #[test]
    fn a_frame_that_takes_too_long_restarts_the_grid_instead_of_a_burst() {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0);
        let due = p.due(240);
        // this frame started 30 ms late (a stall)
        p.begin(due + 30 * MS, due, 240);
        assert_eq!(p.due(240), due + 30 * MS + period(240), "the next is a period later, not at once");
        // slightly late (less than a period): the grid holds
        let due = p.due(240);
        p.begin(due + MS, due, 240);
        assert_eq!(p.due(240), due + period(240));
    }

    #[test]
    fn input_brings_a_slow_frame_forward_but_never_back() {
        let t0 = Instant::now();
        let mut p = Pacer::new(t0);
        assert_eq!(p.due(30), t0 + period(30));
        p.input(120);
        assert_eq!(p.due(30), t0 + period(120), "a key at 30 fps is drawn within 8 ms, not 33");
        // more input doesn't push it later, a slower input rate doesn't either
        p.input(60);
        assert_eq!(p.due(30), t0 + period(120));
        // the frame after that is back on the slow grid, from the hurried one
        let due = p.due(30);
        p.begin(due, due, 30);
        assert_eq!(p.due(30), due + period(30));
        // a fast rate's own grid is never slowed by input at a lower rate
        p.input(60);
        assert_eq!(p.due(240), due + period(240));
    }

    #[test]
    fn rates_switch_at_once() {
        let t0 = Instant::now();
        let p = Pacer::new(t0);
        // idle at 30, a drag starts: the next frame is a 240 fps period after the last, not a 30 fps one
        assert_eq!(p.due(240), t0 + period(240));
        assert!(period(240) < Duration::from_micros(4200) && period(30) > Duration::from_millis(33));
    }
}
