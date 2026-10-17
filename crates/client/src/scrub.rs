//! Scrubbing the seek and volume bars (#24). Pure math, so the precision is unit-tested.
//!
//! Terminals report the mouse per cell, and a 60-cell seek bar makes one cell ≈ 4 s of a song. Two ways out:
//! - *pixel* mode (SGR-Pixels, kitty/Ghostty/WezTerm): the pointer's fractional column maps absolutely onto the bar;
//! - *cell* mode (anywhere else): the press jumps to the cell, then the drag moves relatively, one cell per step
//!   (1 s or 1 %), and a held modifier switches back to coarse absolute mapping.
//!
//! The target is shown while dragging and the seek is sent once, on release.

use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    Seek,
    Volume,
}

impl Bar {
    /// one fine step: a second of the track, or a percent of volume
    pub fn step(self) -> u64 {
        match self {
            Bar::Seek => 1000,
            Bar::Volume => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scrub {
    pub bar: Bar,
    pub rect: Rect,
    /// largest value: the track's duration in ms, or 100 %
    pub max: u64,
    /// the target the bar shows while dragging; sent on release
    pub value: u64,
    /// the pointer reports sub-cell positions, so absolute mapping is already precise
    precise: bool,
    /// relative mode: the pointer column and value the fine drag is measured from
    anchor: (f64, u64),
}

impl Scrub {
    /// A press at fractional column `x` jumps the target there.
    pub fn press(bar: Bar, rect: Rect, max: u64, x: f64, precise: bool) -> Scrub {
        let value = absolute(bar, rect, max, x);
        Scrub { bar, rect, max, value, precise, anchor: (x, value) }
    }

    /// The pointer moved to fractional column `x`; `coarse` is a held modifier (cell mode only).
    pub fn drag(&mut self, x: f64, coarse: bool) {
        if self.precise || coarse {
            self.value = absolute(self.bar, self.rect, self.max, x);
            // letting go of the modifier continues finely from here
            self.anchor = (x, self.value);
            return;
        }
        let (ax, av) = self.anchor;
        let steps = (x - ax).round() as i64;
        self.value = (av as i64 + steps * self.bar.step() as i64).clamp(0, self.max as i64) as u64;
    }
}

/// Absolute mapping: the first cell's centre is 0, the last cell's centre is `max`; seeks land on whole seconds.
pub fn absolute(bar: Bar, rect: Rect, max: u64, x: f64) -> u64 {
    let span = (rect.width.max(2) - 1) as f64;
    let f = ((x - rect.x as f64 - 0.5) / span).clamp(0.0, 1.0);
    let step = bar.step() as f64;
    (((f * max as f64) / step).round() * step).min(max as f64) as u64
}

/// Bars draw as a run of cells: how many are full at `value`, and whether a half-cell tip follows.
pub fn fill(value: u64, max: u64, width: u16) -> (u16, bool) {
    if max == 0 {
        return (0, false);
    }
    let cells = value.min(max) as f64 / max as f64 * width as f64;
    (cells.floor() as u16, cells.fract() >= 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: u64 = 4 * 60 * 1000;
    // a typical seek bar: 60 cells, ~4 s per cell
    const BAR: Rect = Rect { x: 20, y: 0, width: 60, height: 1 };

    #[test]
    fn absolute_ends_and_whole_seconds() {
        assert_eq!(absolute(Bar::Seek, BAR, TRACK, 20.0), 0);
        assert_eq!(absolute(Bar::Seek, BAR, TRACK, 80.0), TRACK);
        assert_eq!(absolute(Bar::Seek, BAR, TRACK, 50.5) % 1000, 0);
        assert_eq!(absolute(Bar::Volume, Rect::new(0, 0, 12, 1), 100, 11.5), 100);
        assert_eq!(absolute(Bar::Volume, Rect::new(0, 0, 12, 1), 100, 0.5), 0);
    }

    #[test]
    fn pixel_mode_reaches_every_second_of_a_four_minute_track() {
        // 60 cells at 10 px each: the pointer moves in tenths of a cell
        let mut seen = std::collections::BTreeSet::new();
        let mut s = Scrub::press(Bar::Seek, BAR, TRACK, 20.5, true);
        for px in 0..=600 {
            s.drag(20.0 + px as f64 / 10.0, false);
            seen.insert(s.value);
        }
        assert!((0..=240).all(|sec| seen.contains(&(sec * 1000))), "missed seconds: {:?}", (0..=240).filter(|s| !seen.contains(&(s * 1000))).collect::<Vec<_>>());
    }

    #[test]
    fn cell_mode_drags_one_second_per_cell() {
        let mut s = Scrub::press(Bar::Seek, BAR, TRACK, 50.5, false);
        let start = s.value;
        s.drag(51.5, false);
        assert_eq!(s.value, start + 1000);
        s.drag(53.5, false);
        assert_eq!(s.value, start + 3000);
        s.drag(48.5, false);
        assert_eq!(s.value, start - 2000);
        // any whole second is reachable: jump near, then walk cell by cell
        let target = 97_000;
        let mut s = Scrub::press(Bar::Seek, BAR, TRACK, 20.5 + (target as f64 / TRACK as f64) * 59.0, false);
        let off = (target as i64 - s.value as i64) / 1000;
        s.drag(20.5 + (target as f64 / TRACK as f64) * 59.0 + off as f64, false);
        assert_eq!(s.value, target);
    }

    #[test]
    fn coarse_modifier_maps_absolutely_and_fine_re_anchors() {
        let mut s = Scrub::press(Bar::Seek, BAR, TRACK, 20.5, false);
        s.drag(79.5, true);
        assert_eq!(s.value, TRACK);
        s.drag(78.5, false); // let go of the modifier: fine again from here
        assert_eq!(s.value, TRACK - 1000);
    }

    #[test]
    fn volume_moves_in_one_percent_steps_and_clamps() {
        let r = Rect::new(0, 0, 12, 1);
        let mut s = Scrub::press(Bar::Volume, r, 100, 5.5, false);
        let v = s.value;
        s.drag(6.5, false);
        assert_eq!(s.value, v + 1);
        s.drag(500.0, false);
        assert_eq!(s.value, 100);
        let mut p = Scrub::press(Bar::Volume, r, 100, 0.5, true);
        p.drag(0.5 + 11.0 * 0.37, false);
        assert_eq!(p.value, 37);
    }

    #[test]
    fn fill_cells_and_tip() {
        assert_eq!(fill(0, 0, 10), (0, false));
        assert_eq!(fill(50, 100, 10), (5, false));
        assert_eq!(fill(56, 100, 10), (5, true));
        assert_eq!(fill(200, 100, 10), (10, false));
    }
}
