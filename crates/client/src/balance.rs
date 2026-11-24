//! Balance (#58, D87): where a Widget's content sits when its Pane has more room than the content fills. The content
//! is one block, centred on the axis with room to spare (an odd cell goes after it), and the parts that can grow
//! (a graph, a chart) step up to a bigger size first. Instead of clinging to the top left and leaving the rest of the
//! Pane empty. Each Widget says what its block is and where it may grow; these say where it goes.

/// Where a block `len` cells long starts in `space` cells from `start`: centred, the odd cell after it; at `start`
/// when it fills the space or doesn't fit.
pub fn centre(start: u16, space: u16, len: u16) -> u16 {
    start + space.saturating_sub(len) / 2
}

/// The row a block `halves` half-rows tall starts at in `rows` rows from `start`, for blocks drawn in half-blocks
/// (a pixel font whose last row is only its upper half): the row nearest the centred position, the higher one on a
/// tie.
pub fn centre_halves(start: u16, rows: u16, halves: u16) -> u16 {
    // the ideal top in quarter rows: (space - block) / 2 half rows
    let quarters = (rows * 2).saturating_sub(halves);
    start + quarters / 4 + u16::from(quarters % 4 == 3)
}

/// The biggest of `steps` (smallest first) that `fits`; the smallest when none does.
pub fn step(steps: &[u16], fits: impl Fn(u16) -> bool) -> u16 {
    steps.iter().copied().filter(|s| fits(*s)).max().unwrap_or(steps[0])
}

/// For tests: the inked part of `r` in `buf` as (first row, last row, first column, last column), None when blank.
#[cfg(test)]
pub fn ink(buf: &ratatui::buffer::Buffer, r: ratatui::layout::Rect) -> Option<(u16, u16, u16, u16)> {
    let inked = |x: u16, y: u16| buf.cell((x, y)).is_some_and(|c| c.symbol() != " ");
    let rows: Vec<u16> = (r.top()..r.bottom()).filter(|&y| (r.left()..r.right()).any(|x| inked(x, y))).collect();
    let cols: Vec<u16> = (r.left()..r.right()).filter(|&x| (r.top()..r.bottom()).any(|y| inked(x, y))).collect();
    Some((*rows.first()?, *rows.last()?, *cols.first()?, *cols.last()?))
}

/// For tests: the rows over and under the inked part of `r`, and the columns left and right of it.
#[cfg(test)]
pub fn margins(buf: &ratatui::buffer::Buffer, r: ratatui::layout::Rect) -> (u16, u16, u16, u16) {
    let (top, bottom, left, right) = ink(buf, r).expect("something drawn");
    (top - r.y, r.bottom() - 1 - bottom, left - r.x, r.right() - 1 - right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_sits_in_the_middle_of_its_room_the_odd_cell_after_it() {
        assert_eq!(centre(10, 20, 10), 15);
        assert_eq!(centre(10, 21, 10), 15, "the odd cell goes after the block");
        assert_eq!(centre(10, 20, 20), 10, "filling the room: where it was");
        assert_eq!(centre(10, 20, 30), 10, "too big: from the start, cut at the end");
        assert_eq!(centre(0, 0, 0), 0);
    }

    #[test]
    fn half_row_blocks_sit_on_the_row_nearest_the_middle() {
        // a 2.5-row title in 4 rows: 0.75 rows over it is nearer 1 than 0
        assert_eq!(centre_halves(0, 4, 5), 1);
        // a whole number of rows: the plain centre, the odd row after it
        assert_eq!(centre_halves(0, 6, 8), 1);
        assert_eq!(centre_halves(3, 7, 8), 4);
        // halfway between two rows: the higher one
        assert_eq!(centre_halves(0, 28, 10), 11);
        assert_eq!(centre_halves(0, 6, 10), 0);
        assert_eq!(centre_halves(5, 2, 9), 5, "too tall: from the start");
    }

    #[test]
    fn a_part_steps_up_to_the_biggest_size_that_fits() {
        let steps = [10, 16, 24];
        assert_eq!(step(&steps, |s| s <= 30), 24);
        assert_eq!(step(&steps, |s| s <= 20), 16);
        assert_eq!(step(&steps, |s| s <= 12), 10);
        assert_eq!(step(&steps, |_| false), 10, "none fits: the smallest");
    }
}
