//! Back / forward navigation history.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Visited folders with a cursor, like a browser history.
#[derive(Clone, Debug, Default)]
pub struct History {
    back: Vec<String>,
    forward: Vec<String>,
    current: String,
}

impl History {
    /// History positioned at `start`.
    #[must_use]
    pub fn new(start: &str) -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
            current: start.to_string(),
        }
    }

    /// Folder being shown.
    #[must_use]
    pub fn current(&self) -> &str {
        &self.current
    }

    /// Go to `path`, remembering where we were and dropping the forward
    /// list. Returns false when `path` is already current.
    pub fn navigate(&mut self, path: &str) -> bool {
        if path == self.current {
            return false;
        }
        let from = core::mem::replace(&mut self.current, path.to_string());
        self.back.push(from);
        self.forward.clear();
        true
    }

    /// Step back; returns the new current folder.
    pub fn go_back(&mut self) -> Option<&str> {
        let target = self.back.pop()?;
        let from = core::mem::replace(&mut self.current, target);
        self.forward.push(from);
        Some(&self.current)
    }

    /// Step forward; returns the new current folder.
    pub fn go_forward(&mut self) -> Option<&str> {
        let target = self.forward.pop()?;
        let from = core::mem::replace(&mut self.current, target);
        self.back.push(from);
        Some(&self.current)
    }

    /// Whether there is a folder to go back to.
    #[must_use]
    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Whether there is a folder to go forward to.
    #[must_use]
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Replace the current folder without recording history (used when it
    /// disappears and we fall back to a parent).
    pub fn replace_current(&mut self, path: &str) {
        self.current = path.to_string();
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn starts_with_nothing_to_go_back_or_forward_to() {
        let h = History::new("/home");
        assert_eq!(h.current(), "/home");
        assert!(!h.can_back() && !h.can_forward());
    }

    #[test]
    fn back_and_forward_walk_the_visited_folders() {
        let mut h = History::new("/a");
        assert!(h.navigate("/b"));
        assert!(h.navigate("/c"));
        assert_eq!(h.go_back(), Some("/b"));
        assert_eq!(h.go_back(), Some("/a"));
        assert_eq!(h.go_back(), None);
        assert_eq!(h.go_forward(), Some("/b"));
        assert_eq!(h.go_forward(), Some("/c"));
        assert_eq!(h.go_forward(), None);
        assert_eq!(h.current(), "/c");
    }

    #[test]
    fn navigating_clears_the_forward_list() {
        let mut h = History::new("/a");
        h.navigate("/b");
        h.go_back();
        assert!(h.can_forward());
        h.navigate("/z");
        assert!(!h.can_forward());
        assert_eq!(h.go_back(), Some("/a"));
    }

    #[test]
    fn navigating_to_the_current_folder_changes_nothing() {
        let mut h = History::new("/a");
        assert!(!h.navigate("/a"));
        assert!(!h.can_back());
    }

    #[test]
    fn replace_current_keeps_the_trail() {
        let mut h = History::new("/a");
        h.navigate("/a/b");
        h.replace_current("/a");
        assert_eq!(h.current(), "/a");
        assert_eq!(h.go_back(), Some("/a"));
    }
}
