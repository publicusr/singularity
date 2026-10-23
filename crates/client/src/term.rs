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

/// One wheel notch can arrive as a burst of reports: Ghostty sends `mouse-scroll-multiplier` of them (3 by default)
/// at once, which moved lists three rows a notch. Wheel reports in the same direction that arrive within `BURST` of
/// the first one are one step; separate notches come tens of milliseconds apart even on a fast spin.
#[derive(Default)]
pub struct WheelBursts {
    first: Option<(MouseEventKind, Instant)>,
}

impl WheelBursts {
    pub const BURST: Duration = Duration::from_millis(8);

    /// Whether to act on this event: false only for a repeat inside a wheel burst.
    pub fn keep(&mut self, kind: MouseEventKind, at: Instant) -> bool {
        use MouseEventKind::*;
        if !matches!(kind, ScrollUp | ScrollDown | ScrollLeft | ScrollRight) {
            return true;
        }
        if let Some((k, t)) = self.first
            && k == kind
            && at.saturating_duration_since(t) < Self::BURST
        {
            return false;
        }
        self.first = Some((kind, at));
        true
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

    #[test]
    fn a_wheel_burst_is_one_step() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut w = WheelBursts::default();
        // Ghostty: three reports for one notch, a microsecond apart
        let kept: Vec<bool> = (0..3).map(|i| w.keep(MouseEventKind::ScrollDown, t + Duration::from_micros(i))).collect();
        assert_eq!(kept, [true, false, false]);
        // the next notch, and one the other way inside the window, both count
        assert!(w.keep(MouseEventKind::ScrollDown, ms(40)));
        assert!(w.keep(MouseEventKind::ScrollUp, ms(41)));
        assert!(!w.keep(MouseEventKind::ScrollUp, ms(42)));
        // other events pass and don't end a burst
        assert!(w.keep(MouseEventKind::Moved, ms(43)));
        assert!(!w.keep(MouseEventKind::ScrollUp, ms(44)));
        // one report per notch (kitty), notches 20 ms apart: every one counts
        assert!((0..5).all(|i| w.keep(MouseEventKind::ScrollDown, ms(100 + 20 * i))));
    }
}
