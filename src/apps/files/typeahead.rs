//! Type-to-select: typed letters jump to the next item whose name starts
//! with what has been typed recently.

use super::model::Entry;
use alloc::string::String;

/// Idle time after which the typed prefix starts over.
pub const RESET_AFTER_MS: u64 = 1000;

/// The prefix typed so far and when the last key arrived.
#[derive(Clone, Debug, Default)]
pub struct TypeAhead {
    prefix: String,
    last_ms: u64,
}

impl TypeAhead {
    /// Nothing typed.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            prefix: String::new(),
            last_ms: 0,
        }
    }

    /// The prefix currently being matched.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Record `c` typed at `now_ms` and return the item to select, searching
    /// from `current`. A repeated single letter cycles through matches; a
    /// longer prefix narrows from the current item.
    pub fn type_char(
        &mut self,
        c: char,
        now_ms: u64,
        entries: &[Entry],
        current: Option<usize>,
    ) -> Option<usize> {
        if now_ms.saturating_sub(self.last_ms) > RESET_AFTER_MS {
            self.prefix.clear();
        }
        self.last_ms = now_ms;
        let cycling = self.prefix.chars().all(|p| p.eq_ignore_ascii_case(&c));
        self.prefix.push(c.to_ascii_lowercase());
        let single_run = cycling && !self.prefix.is_empty();
        let probe = if single_run {
            self.prefix.chars().next().map(String::from)?
        } else {
            self.prefix.clone()
        };
        let skip_current = single_run && current.is_some();
        find(entries, &probe, current, skip_current)
    }
}

fn find(entries: &[Entry], prefix: &str, from: Option<usize>, skip_current: bool) -> Option<usize> {
    let n = entries.len();
    let start = from.map_or(0, |i| if skip_current { i + 1 } else { i });
    (0..n)
        .map(|k| (start + k) % n)
        .find(|i| entries[*i].name.to_ascii_lowercase().starts_with(prefix))
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> alloc::vec::Vec<Entry> {
        list.iter()
            .map(|n| Entry {
                name: (*n).into(),
                is_dir: false,
                size: 0,
                mtime: 0,
            })
            .collect()
    }

    #[test]
    fn first_letter_jumps_to_the_first_match() {
        let e = names(&["alpha", "beta", "bravo", "charlie"]);
        let mut t = TypeAhead::new();
        assert_eq!(t.type_char('b', 10, &e, None), Some(1));
    }

    #[test]
    fn matching_ignores_case() {
        let e = names(&["Alpha", "Beta"]);
        let mut t = TypeAhead::new();
        assert_eq!(t.type_char('B', 10, &e, None), Some(1));
    }

    #[test]
    fn more_letters_narrow_the_prefix_from_the_current_item() {
        let e = names(&["beta", "bravo", "brick"]);
        let mut t = TypeAhead::new();
        let first = t.type_char('b', 10, &e, Some(2));
        assert_eq!(first, Some(0));
        let second = t.type_char('r', 300, &e, first);
        assert_eq!(second, Some(1));
        assert_eq!(t.prefix(), "br");
        assert_eq!(t.type_char('i', 500, &e, second), Some(2));
    }

    #[test]
    fn repeating_one_letter_cycles_through_matches() {
        let e = names(&["bar", "baz", "foo"]);
        let mut t = TypeAhead::new();
        let a = t.type_char('b', 10, &e, None);
        let b = t.type_char('b', 200, &e, a);
        let c = t.type_char('b', 400, &e, b);
        assert_eq!((a, b, c), (Some(0), Some(1), Some(0)));
    }

    #[test]
    fn the_prefix_resets_after_a_pause() {
        let e = names(&["apple", "banana", "berry"]);
        let mut t = TypeAhead::new();
        t.type_char('b', 10, &e, None);
        let again = t.type_char('a', 10 + RESET_AFTER_MS + 1, &e, Some(1));
        assert_eq!(t.prefix(), "a");
        assert_eq!(again, Some(0));
    }

    #[test]
    fn no_match_or_no_items_gives_none() {
        let e = names(&["apple"]);
        let mut t = TypeAhead::new();
        assert_eq!(t.type_char('z', 10, &e, None), None);
        assert_eq!(TypeAhead::new().type_char('a', 10, &[], None), None);
    }
}
