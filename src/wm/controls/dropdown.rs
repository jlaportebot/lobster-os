//! Dropdown (select) state machine: closed / open with a highlighted item.
//!
//! The popup list itself is drawn by [`super::Ui::dropdown_popup`]; this
//! type only decides what keyboard and pointer input mean.

/// Open/closed state and highlighted row of one dropdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dropdown {
    open: bool,
    highlight: usize,
    len: usize,
}

impl Dropdown {
    /// A closed dropdown over `len` items.
    #[must_use]
    pub const fn new(len: usize) -> Self {
        Self {
            open: false,
            highlight: 0,
            len,
        }
    }

    /// Whether the popup list is showing.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Row the keyboard cursor is on.
    #[must_use]
    pub const fn highlight(&self) -> usize {
        self.highlight
    }

    /// Open the list with the currently selected item highlighted.
    pub fn open(&mut self, selected: usize) {
        self.open = true;
        self.highlight = selected.min(self.len.saturating_sub(1));
    }

    /// Close without choosing anything (Escape, outside click).
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Open when closed, close when open.
    pub fn toggle(&mut self, selected: usize) {
        if self.open {
            self.close();
        } else {
            self.open(selected);
        }
    }

    /// Move the highlight by `delta` rows, clamped; ignored while closed.
    pub fn move_highlight(&mut self, delta: i32) {
        if !self.open {
            return;
        }
        let last = self.len.saturating_sub(1) as i64;
        self.highlight = (self.highlight as i64 + i64::from(delta)).clamp(0, last) as usize;
    }

    /// Choose the highlighted row and close (Enter / Space).
    pub fn activate(&mut self) -> Option<usize> {
        if !self.open {
            return None;
        }
        self.open = false;
        Some(self.highlight)
    }

    /// Choose row `index` and close; out-of-range clicks are ignored.
    pub fn click_item(&mut self, index: usize) -> Option<usize> {
        if !self.open || index >= self.len {
            return None;
        }
        self.open = false;
        Some(index)
    }
}
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn starts_closed() {
        assert!(!Dropdown::new(4).is_open());
    }

    #[test]
    fn open_highlights_the_selected_item() {
        let mut menu = Dropdown::new(6);
        menu.open(3);
        assert!(menu.is_open());
        assert_eq!(menu.highlight(), 3);
    }

    #[test]
    fn navigation_clamps_at_both_ends() {
        let mut menu = Dropdown::new(3);
        menu.open(0);
        menu.move_highlight(-1);
        assert_eq!(menu.highlight(), 0);
        menu.move_highlight(5);
        assert_eq!(menu.highlight(), 2);
    }

    #[test]
    fn activate_returns_the_highlight_and_closes() {
        let mut menu = Dropdown::new(4);
        menu.open(1);
        menu.move_highlight(2);
        assert_eq!(menu.activate(), Some(3));
        assert!(!menu.is_open());
    }

    #[test]
    fn closing_never_selects() {
        let mut menu = Dropdown::new(4);
        menu.open(1);
        menu.close();
        assert!(!menu.is_open());
        assert_eq!(menu.activate(), None);
    }

    #[test]
    fn click_item_selects_it_and_closes() {
        let mut menu = Dropdown::new(4);
        menu.open(0);
        assert_eq!(menu.click_item(2), Some(2));
        assert!(!menu.is_open());
    }

    #[test]
    fn click_item_out_of_range_is_ignored() {
        let mut menu = Dropdown::new(2);
        menu.open(0);
        assert_eq!(menu.click_item(9), None);
        assert!(menu.is_open());
    }

    #[test]
    fn navigation_is_ignored_while_closed() {
        let mut menu = Dropdown::new(4);
        menu.move_highlight(2);
        assert_eq!(menu.highlight(), 0);
    }

    #[test]
    fn toggle_opens_then_closes() {
        let mut menu = Dropdown::new(4);
        menu.toggle(2);
        assert!(menu.is_open());
        menu.toggle(2);
        assert!(!menu.is_open());
    }
}
