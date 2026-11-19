//! Multi-selection model for the item list and grid.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

/// Selected item indices plus the anchor (start of a Shift range) and the
/// lead (the keyboard cursor).
#[derive(Clone, Debug, Default)]
pub struct Selection {
    chosen: BTreeSet<usize>,
    anchor: Option<usize>,
    lead: Option<usize>,
}

impl Selection {
    /// Nothing selected.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            chosen: BTreeSet::new(),
            anchor: None,
            lead: None,
        }
    }

    /// Plain click: select only `i`.
    pub fn click(&mut self, i: usize) {
        self.chosen.clear();
        self.chosen.insert(i);
        self.anchor = Some(i);
        self.lead = Some(i);
    }

    /// Ctrl+click: flip `i` without touching the rest.
    pub fn toggle(&mut self, i: usize) {
        if !self.chosen.remove(&i) {
            self.chosen.insert(i);
        }
        self.anchor = Some(i);
        self.lead = Some(i);
    }

    /// Shift+click: select the range from the anchor to `i`.
    pub fn extend_to(&mut self, i: usize) {
        let from = self.anchor.unwrap_or(i);
        self.chosen = (from.min(i)..=from.max(i)).collect();
        self.anchor = Some(from);
        self.lead = Some(i);
    }

    /// Ctrl+A.
    pub fn select_all(&mut self, len: usize) {
        self.chosen = (0..len).collect();
        self.anchor = (len > 0).then_some(0);
        self.lead = len.checked_sub(1);
    }

    /// Click on empty space.
    pub fn clear(&mut self) {
        self.chosen.clear();
        self.anchor = None;
        self.lead = None;
    }

    /// Keyboard move of the cursor to `target`, extending when Shift is held.
    pub fn move_to(&mut self, target: usize, extend: bool) {
        if extend {
            self.extend_to(target);
        } else {
            self.click(target);
        }
    }

    /// Whether `i` is selected.
    #[must_use]
    pub fn contains(&self, i: usize) -> bool {
        self.chosen.contains(&i)
    }

    /// Number of selected items.
    #[must_use]
    pub fn count(&self) -> usize {
        self.chosen.len()
    }

    /// Selected indices, ascending.
    #[must_use]
    pub fn indices(&self) -> Vec<usize> {
        self.chosen.iter().copied().collect()
    }

    /// The keyboard cursor, if any.
    #[must_use]
    pub fn lead(&self) -> Option<usize> {
        self.lead
    }

    /// Drop indices at or past `len` (the list shrank).
    pub fn retain_below(&mut self, len: usize) {
        self.chosen.retain(|i| *i < len);
        self.anchor = self.anchor.filter(|i| *i < len);
        self.lead = self.lead.filter(|i| *i < len);
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn click_selects_only_that_item() {
        let mut s = Selection::new();
        s.click(2);
        s.click(4);
        assert_eq!(s.indices(), [4]);
        assert_eq!(s.lead(), Some(4));
    }

    #[test]
    fn ctrl_click_toggles_without_losing_others() {
        let mut s = Selection::new();
        s.click(1);
        s.toggle(3);
        assert_eq!(s.indices(), [1, 3]);
        s.toggle(1);
        assert_eq!(s.indices(), [3]);
    }

    #[test]
    fn shift_click_selects_the_range_from_the_anchor() {
        let mut s = Selection::new();
        s.click(2);
        s.extend_to(5);
        assert_eq!(s.indices(), [2, 3, 4, 5]);
        s.extend_to(0);
        assert_eq!(s.indices(), [0, 1, 2]);
    }

    #[test]
    fn shift_click_without_an_anchor_selects_the_item() {
        let mut s = Selection::new();
        s.extend_to(3);
        assert_eq!(s.indices(), [3]);
    }

    #[test]
    fn ctrl_click_moves_the_anchor() {
        let mut s = Selection::new();
        s.click(0);
        s.toggle(4);
        s.extend_to(6);
        assert_eq!(s.indices(), [4, 5, 6]);
    }

    #[test]
    fn select_all_and_clear() {
        let mut s = Selection::new();
        s.select_all(4);
        assert_eq!(s.count(), 4);
        assert!(s.contains(3) && !s.contains(4));
        s.clear();
        assert_eq!(s.count(), 0);
        assert_eq!(s.lead(), None);
    }

    #[test]
    fn keyboard_move_replaces_or_extends() {
        let mut s = Selection::new();
        s.move_to(1, false);
        s.move_to(2, false);
        assert_eq!(s.indices(), [2]);
        s.move_to(4, true);
        assert_eq!(s.indices(), [2, 3, 4]);
        assert_eq!(s.lead(), Some(4));
    }

    #[test]
    fn shrinking_the_list_drops_stale_indices() {
        let mut s = Selection::new();
        s.select_all(6);
        s.retain_below(3);
        assert_eq!(s.indices(), [0, 1, 2]);
    }
}
