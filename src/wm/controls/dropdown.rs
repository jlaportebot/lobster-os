//! Open/closed state of a dropdown (select).
//!
//! The list itself lives in [`crate::wm::popup_list`], which also owns the
//! highlight and keyboard handling; this type only records whether the
//! owning page wants the list showing.

/// Whether one dropdown's popup list is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dropdown {
    open: bool,
}

impl Dropdown {
    /// A closed dropdown.
    #[must_use]
    pub const fn new() -> Self {
        Self { open: false }
    }

    /// Whether the popup list is showing.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Close without choosing anything (Escape, outside click, a pick).
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Open when closed, close when open.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn starts_closed() {
        assert!(!Dropdown::new().is_open());
    }

    #[test]
    fn toggle_opens_then_closes() {
        let mut menu = Dropdown::new();
        menu.toggle();
        assert!(menu.is_open());
        menu.toggle();
        assert!(!menu.is_open());
    }

    #[test]
    fn close_is_idempotent() {
        let mut menu = Dropdown::new();
        menu.toggle();
        menu.close();
        menu.close();
        assert!(!menu.is_open());
    }
}
