//! What the terminal can do, and turning its raw mouse reports into cells plus a fractional column.
//!
//! With SGR-Pixels (DECSET 1016) the terminal reports pointer positions in pixels, in the same SGR format crossterm
//! already parses, so crossterm's `column`/`row` hold pixels and we divide by the cell size ourselves.

use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
use std::io::Write;
use std::time::{Duration, Instant};

/// Terminals known to implement SGR-Pixels mouse reporting. `SINGULARITY_PIXEL_MOUSE=0|1` overrides.
pub fn pixel_mouse_supported() -> bool {
    match std::env::var("SINGULARITY_PIXEL_MOUSE") {
        Ok(v) => v == "1",
        Err(_) => modern_terminal(),
    }
}

/// The pixel render tier (kitty graphics protocol). `SINGULARITY_PIXEL=0|1` overrides.
pub fn kitty_graphics_supported() -> bool {
    match std::env::var("SINGULARITY_PIXEL") {
        Ok(v) => v == "1",
        Err(_) => modern_terminal(),
    }
}

/// Images can sit under cell backgrounds (kitty itself; `SINGULARITY_BELOW_BG=1|0` decides for tests).
pub fn below_backgrounds() -> bool {
    match std::env::var("SINGULARITY_BELOW_BG") {
        Ok(v) => v == "1",
        Err(_) => std::env::var("KITTY_WINDOW_ID").is_ok() || std::env::var("TERM").is_ok_and(|t| t == "xterm-kitty"),
    }
}

/// Pixels can go through shared memory: kitty itself, on this machine.
pub fn kitty_shm() -> bool {
    std::env::var("KITTY_WINDOW_ID").is_ok() && std::env::var("SSH_CONNECTION").is_err() && std::env::var("SINGULARITY_NO_SHM").is_err()
}

/// One cell's size in pixels, if the terminal reports its pixel size.
pub fn cell_px() -> Option<(f64, f64)> {
    match MouseUnits::measure()? {
        MouseUnits::Pixels { cell_w, cell_h } => Some((cell_w, cell_h)),
        MouseUnits::Cells => None,
    }
}

/// kitty, Ghostty and WezTerm, not inside tmux/screen (which re-encode mouse reports and drop graphics).
fn modern_terminal() -> bool {
    let var = |k: &str| std::env::var(k).unwrap_or_default().to_lowercase();
    if std::env::var("TMUX").is_ok() || var("TERM").starts_with("screen") || var("TERM").starts_with("tmux") {
        return false;
    }
    var("TERM").contains("kitty")
        || var("TERM").contains("ghostty")
        || var("TERM_PROGRAM").contains("ghostty")
        || var("TERM_PROGRAM").contains("wezterm")
        || std::env::var("KITTY_WINDOW_ID").is_ok()
        || std::env::var("GHOSTTY_RESOURCES_DIR").is_ok()
}

/// How mouse reports are encoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseUnits {
    Cells,
    /// pixel reports; the size of one cell in pixels
    Pixels { cell_w: f64, cell_h: f64 },
}

impl MouseUnits {
    /// Pixel units if the terminal supports them and reports its pixel size (TIOCGWINSZ), else cells.
    pub fn detect() -> MouseUnits {
        if !pixel_mouse_supported() {
            return MouseUnits::Cells;
        }
        MouseUnits::measure().unwrap_or(MouseUnits::Cells)
    }

    /// Re-read the cell size (after a resize or font change).
    pub fn measure() -> Option<MouseUnits> {
        let ws = ratatui::crossterm::terminal::window_size().ok()?;
        (ws.width > 0 && ws.height > 0 && ws.columns > 0 && ws.rows > 0)
            .then(|| MouseUnits::Pixels { cell_w: ws.width as f64 / ws.columns as f64, cell_h: ws.height as f64 / ws.rows as f64 })
    }

    pub fn enable(self, out: &mut impl Write) -> std::io::Result<()> {
        if matches!(self, MouseUnits::Pixels { .. }) {
            out.write_all(b"\x1b[?1016h")?;
            out.flush()?;
        }
        Ok(())
    }

    pub fn disable(self, out: &mut impl Write) -> std::io::Result<()> {
        if matches!(self, MouseUnits::Pixels { .. }) {
            out.write_all(b"\x1b[?1016l")?;
            out.flush()?;
        }
        Ok(())
    }

    /// The event in cell coordinates, plus the pointer's fractional column (the cell's centre in cell units)
    /// and whether that fraction is real.
    pub fn to_cells(self, mut m: MouseEvent) -> (MouseEvent, f64, bool) {
        match self {
            MouseUnits::Cells => (m, m.column as f64 + 0.5, false),
            MouseUnits::Pixels { cell_w, cell_h } => {
                let fx = m.column as f64 / cell_w;
                m.column = fx as u16;
                m.row = (m.row as f64 / cell_h) as u16;
                (m, fx, true)
            }
        }
    }
}

/// Wheel reports into steps (D86, #32). A terminal reports a mouse wheel and a touchpad alike, as up or down with no
/// distance:
///
/// - Ghostty sends one wheel notch as `mouse-scroll-multiplier` reports at once (3 by default): one step;
/// - a touchpad sends a report for every row it travels, at uneven gaps, a few at once on a fast flick: a step each
///   (dropping the ones close together made a fast flick move fewer rows than a slow drag).
///
/// Reports in one direction within [`Self::TOGETHER`] of the first are a group, decided on its own: exactly three are
/// a notch, one step; any other number, a step each. The first report is a step at once; the next two wait to see
/// whether a fourth comes (then they are steps too) or the group ends at three.
#[derive(Default)]
pub struct WheelSteps {
    /// the group under way: its first report, when it came, how many there are
    group: Option<(MouseEvent, Instant, usize)>,
}

impl WheelSteps {
    pub const TOGETHER: Duration = Duration::from_millis(8);

    /// The events to act on for `m`, which came at `at`: anything but the wheel as it is; a wheel report as its steps,
    /// after the ones the group before it held back.
    pub fn push(&mut self, m: MouseEvent, at: Instant) -> Vec<MouseEvent> {
        use MouseEventKind::*;
        if !matches!(m.kind, ScrollUp | ScrollDown | ScrollLeft | ScrollRight) {
            return vec![m];
        }
        let same = self.group.is_some_and(|(g, t, _)| g.kind == m.kind && at.saturating_duration_since(t) < Self::TOGETHER);
        let mut out = if same { vec![] } else { self.end() };
        let (_, _, n) = self.group.get_or_insert((m, at, 0));
        *n += 1;
        match *n {
            1 => out.push(m),
            // not a notch: the two held back, and this one
            4 => out.extend([m; 3]),
            2 | 3 => {}
            _ => out.push(m),
        }
        out
    }

    /// When the group under way ends, if there is one.
    pub fn deadline(&self) -> Option<Instant> {
        self.group.map(|(_, t, _)| t + Self::TOGETHER)
    }

    /// The group under way, if it has ended by `now`: the steps it held back (none for a notch).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn settle(&mut self, now: Instant) -> Vec<MouseEvent> {
        match self.deadline() {
            Some(end) if now >= end => self.end(),
            _ => vec![],
        }
    }

    /// The group under way ends (no report came in its time): the steps it held back (two reports: the second).
    pub fn end(&mut self) -> Vec<MouseEvent> {
        match self.group.take() {
            Some((m, _, 2)) => vec![m],
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

    fn ev(x: u16, y: u16) -> MouseEvent {
        MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn pixel_reports_become_cells_and_fractions() {
        let u = MouseUnits::Pixels { cell_w: 10.0, cell_h: 20.0 };
        let (m, fx, precise) = u.to_cells(ev(457, 41));
        assert_eq!((m.column, m.row), (45, 2));
        assert!((fx - 45.7).abs() < 1e-9 && precise);
        let (m, fx, precise) = MouseUnits::Cells.to_cells(ev(45, 2));
        assert_eq!((m.column, m.row, fx, precise), (45, 2, 45.5, false));
    }

    fn wheel(kind: MouseEventKind) -> MouseEvent {
        MouseEvent { kind, column: 3, row: 4, modifiers: KeyModifiers::NONE }
    }

    /// The steps for reports this many µs after `t`, the last group settled as the next frame would.
    fn steps(w: &mut WheelSteps, t: Instant, reports: &[(u64, MouseEventKind)]) -> usize {
        let mut n = 0;
        for &(us, kind) in reports {
            n += w.push(wheel(kind), t + Duration::from_micros(us)).len();
        }
        n + w.settle(t + Duration::from_micros(reports.last().map_or(0, |r| r.0)) + WheelSteps::TOGETHER).len()
    }

    #[test]
    fn a_wheel_notch_is_one_step() {
        use MouseEventKind::*;
        let t = Instant::now();
        let mut w = WheelSteps::default();
        // Ghostty: three reports for one notch, a microsecond apart; the first is the step, at once
        assert_eq!(w.push(wheel(ScrollDown), t).len(), 1);
        assert!(w.push(wheel(ScrollDown), t + Duration::from_micros(1)).is_empty());
        assert!(w.push(wheel(ScrollDown), t + Duration::from_micros(2)).is_empty());
        assert!(w.settle(t + Duration::from_millis(5)).is_empty());
        // notches 10 ms apart (a fast spin), and one the other way: one step each
        let notch = |at: u64, k| [(at, k), (at + 1, k), (at + 2, k)];
        let spin: Vec<_> = (0..6).flat_map(|i| notch(100_000 + i * 10_000, ScrollDown)).chain(notch(170_000, ScrollUp)).collect();
        assert_eq!(steps(&mut w, t, &spin), 7);
        // other events pass, and don't end a group
        assert_eq!(w.push(wheel(ScrollDown), t + Duration::from_millis(300)).len(), 1);
        assert_eq!(w.push(wheel(Moved), t + Duration::from_micros(300_001)).len(), 1);
        assert!(w.push(wheel(ScrollDown), t + Duration::from_micros(300_002)).is_empty());
        // one report a notch (kitty, or Ghostty with a multiplier of 1): every one counts
        let mut w = WheelSteps::default();
        assert_eq!(steps(&mut w, t, &(0..5).map(|i| (i * 20_000, ScrollDown)).collect::<Vec<_>>()), 5);
    }

    /// The owner's touchpad in Ghostty (`tests/fixtures/wheel-touchpad-ghostty.txt`): every report is a step, so a fast
    /// flick moves as far as its reports say, where dropping the close ones made it move less than a slow drag. The one
    /// exception: three reports at the same instant can't be told from a wheel's notch, and are one step (once in the
    /// recording, in "mixed").
    #[test]
    fn a_touchpad_moves_a_step_a_report() {
        let fixture = include_str!("../tests/fixtures/wheel-touchpad-ghostty.txt");
        let mut phases: Vec<(&str, Vec<(u64, MouseEventKind)>)> = vec![];
        for l in fixture.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let f: Vec<&str> = l.split_whitespace().collect();
            let us = (f[1].parse::<f64>().unwrap() * 1000.0) as u64;
            let kind = if f[2] == "up" { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown };
            match phases.last_mut() {
                Some((p, r)) if *p == f[0] => r.push((us, kind)),
                _ => phases.push((f[0], vec![(us, kind)])),
            }
        }
        assert_eq!(phases.iter().map(|p| p.0).collect::<Vec<_>>(), ["slow", "flick", "nudges", "mixed"]);
        for ((phase, reports), lost) in phases.into_iter().zip([0, 0, 0, 2]) {
            // each phase after a pause, as it was recorded
            let mut w = WheelSteps::default();
            assert_eq!(steps(&mut w, Instant::now(), &reports), reports.len() - lost, "{phase}: a step a report");
        }
    }

    /// The steps a group of two held back come when it ends; each group is decided on its own, so a notch right after a
    /// swipe is still one step, and a fourth report releases the two held back at once.
    #[test]
    fn a_touchpads_held_steps_come_when_its_group_ends() {
        use MouseEventKind::*;
        let t = Instant::now();
        let mut w = WheelSteps::default();
        assert_eq!(w.push(wheel(ScrollUp), t).len(), 1);
        assert!(w.push(wheel(ScrollUp), t).is_empty());
        assert_eq!(w.deadline(), Some(t + WheelSteps::TOGETHER));
        assert!(w.settle(t + Duration::from_millis(1)).is_empty(), "not yet");
        assert_eq!(w.settle(t + WheelSteps::TOGETHER).len(), 1);
        assert_eq!(w.deadline(), None);
        // right after: three at once are a notch, one step (nothing sticks from the swipe)
        let soon = t + Duration::from_millis(20);
        assert_eq!((0..3).map(|i| w.push(wheel(ScrollUp), soon + Duration::from_micros(i)).len()).sum::<usize>(), 1);
        // four together: the fourth brings the two held back with it
        let later = t + Duration::from_millis(60);
        let got: Vec<usize> = (0..4).map(|i| w.push(wheel(ScrollUp), later + Duration::from_micros(i)).len()).collect();
        assert_eq!(got, [1, 0, 0, 3]);
        // and the next notch is one step again
        let next = t + Duration::from_millis(100);
        assert_eq!((0..3).map(|i| w.push(wheel(ScrollUp), next + Duration::from_micros(i)).len()).sum::<usize>(), 1);
        assert!(w.settle(next + WheelSteps::TOGETHER).is_empty());
    }
}
