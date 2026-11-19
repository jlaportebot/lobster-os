//! Back/forward navigation history.

use alloc::string::String;
use alloc::vec::Vec;

/// Entries kept in the history.
pub const MAX_HISTORY: usize = 64;

/// Back/forward history. `pos` is the index of the current entry.
#[derive(Default)]
pub struct History {
    entries: Vec<String>,
    pos: usize,
}

impl History {
    /// Empty history.
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            pos: 0,
        }
    }

    /// All entries, oldest first.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// The current entry.
    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.pos).map(|s| s.as_str())
    }

    /// True when `back` would move.
    pub fn can_back(&self) -> bool {
        self.pos > 0 && !self.entries.is_empty()
    }

    /// True when `forward` would move.
    pub fn can_forward(&self) -> bool {
        self.pos + 1 < self.entries.len()
    }

    /// Record a fresh navigation: drops any forward entries.
    pub fn visit(&mut self, url: &str) {
        if !self.entries.is_empty() {
            self.entries.truncate(self.pos + 1);
        }
        self.entries.push(String::from(url));
        if self.entries.len() > MAX_HISTORY {
            self.entries.remove(0);
        }
        self.pos = self.entries.len() - 1;
    }

    /// Step back; returns the URL to load, or `None` at the start.
    pub fn back(&mut self) -> Option<String> {
        if !self.can_back() {
            return None;
        }
        self.pos -= 1;
        Some(self.entries[self.pos].clone())
    }

    /// Step forward; returns the URL to load, or `None` at the end.
    pub fn forward(&mut self) -> Option<String> {
        if !self.can_forward() {
            return None;
        }
        self.pos += 1;
        Some(self.entries[self.pos].clone())
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn back_forward_keeps_entries() {
        let mut h = History::new();
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), None);
        h.visit("A");
        h.visit("B");
        h.visit("C");
        assert_eq!(h.back().as_deref(), Some("B"));
        assert_eq!(h.back().as_deref(), Some("A"));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward().as_deref(), Some("B"));
        assert_eq!(h.forward().as_deref(), Some("C"));
        assert_eq!(h.forward(), None);
        assert_eq!(h.entries().len(), 3);
    }

    #[test]
    fn visit_after_back_drops_forward() {
        let mut h = History::new();
        h.visit("A");
        h.visit("B");
        h.visit("C");
        h.back();
        h.back();
        h.visit("D");
        assert_eq!(h.entries(), ["A", "D"]);
        assert_eq!(h.current(), Some("D"));
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn is_bounded() {
        let mut h = History::new();
        for i in 0..(MAX_HISTORY * 3) {
            h.visit(&format!("u{i}"));
        }
        assert_eq!(h.entries().len(), MAX_HISTORY);
        assert_eq!(
            h.current(),
            Some(format!("u{}", MAX_HISTORY * 3 - 1).as_str())
        );
    }

    #[test]
    fn can_flags_follow_position() {
        let mut h = History::new();
        assert!(!h.can_back() && !h.can_forward());
        h.visit("A");
        h.visit("B");
        assert!(h.can_back() && !h.can_forward());
        h.back();
        assert!(!h.can_back() && h.can_forward());
    }
}
