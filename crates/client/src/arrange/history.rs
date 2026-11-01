//! Layout undo and redo: two stacks of what was on screen. Generic, so the tree property test can drive it without an
//! App.

/// How many edits back undo reaches (per session).
pub const CAP: usize = 50;

pub struct History<T> {
    undo: Vec<T>,
    redo: Vec<T>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        History { undo: vec![], redo: vec![] }
    }
}

impl<T> History<T> {
    /// An edit is about to replace `before`: it can be undone, and what was undone before can't be redone any more.
    pub fn push(&mut self, before: T) {
        if self.undo.len() == CAP {
            self.undo.remove(0);
        }
        self.undo.push(before);
        self.redo.clear();
    }

    /// The state to go back to, if `ok` accepts it; `now` (what's on screen) becomes redoable. Refused: both stay.
    pub fn undo(&mut self, now: T, ok: impl FnOnce(&T) -> bool) -> Option<T> {
        Self::step(&mut self.undo, &mut self.redo, now, ok)
    }

    /// The state an undo left, again.
    pub fn redo(&mut self, now: T, ok: impl FnOnce(&T) -> bool) -> Option<T> {
        Self::step(&mut self.redo, &mut self.undo, now, ok)
    }

    fn step(from: &mut Vec<T>, to: &mut Vec<T>, now: T, ok: impl FnOnce(&T) -> bool) -> Option<T> {
        if !ok(from.last()?) {
            return None;
        }
        let back = from.pop()?;
        to.push(now);
        Some(back)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_walk_back_and_forth_and_a_new_edit_drops_the_redos() {
        let mut h = History::default();
        let mut now = 0;
        for next in 1..=3 {
            h.push(now);
            now = next;
        }
        now = h.undo(now, |_| true).unwrap();
        assert_eq!(now, 2);
        now = h.undo(now, |_| true).unwrap();
        assert_eq!(now, 1);
        now = h.redo(now, |_| true).unwrap();
        assert_eq!(now, 2);
        // refused: nothing moves
        assert_eq!(h.undo(now, |&s| s != 1), None);
        assert!(h.can_undo() && h.can_redo());
        // a new edit: the redo is gone
        h.push(now);
        assert!(!h.can_redo());
        assert_eq!(h.redo(9, |_| true), None);
        assert_eq!(h.undo(4, |_| true), Some(2));
        assert_eq!(h.undo(2, |_| true), Some(1));
        assert_eq!(h.undo(1, |_| true), Some(0));
        assert_eq!(h.undo(0, |_| true), None);
    }

    #[test]
    fn it_keeps_the_last_fifty() {
        let mut h = History::default();
        for i in 0..80 {
            h.push(i);
        }
        let mut n = 0;
        let mut now = 80;
        while let Some(s) = h.undo(now, |_| true) {
            now = s;
            n += 1;
        }
        assert_eq!((n, now), (CAP, 30));
    }
}
