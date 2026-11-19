//! Bookmarks and history persisted under `/sd/etc/browser/`.

use super::history::History;
use alloc::string::String;
use alloc::vec::Vec;

const DIR: &str = "/sd/etc/browser";
const BOOKMARKS_FILE: &str = "/sd/etc/browser/bookmarks";
const HISTORY_FILE: &str = "/sd/etc/browser/history";
/// History entries written to disk.
const PERSISTED_HISTORY: usize = 20;

/// Parse a one-URL-per-line file, skipping blanks.
#[must_use]
pub fn parse_lines(data: &[u8]) -> Vec<String> {
    core::str::from_utf8(data)
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Serialise URLs one per line.
#[must_use]
pub fn join_lines(items: &[String]) -> String {
    let mut body = String::new();
    for item in items {
        body.push_str(item);
        body.push('\n');
    }
    body
}

/// Load the saved bookmarks (empty when none).
#[must_use]
pub fn load() -> Vec<String> {
    crate::fs::vfs::read_file(BOOKMARKS_FILE)
        .map(|d| parse_lines(&d))
        .unwrap_or_default()
}

/// Save the bookmarks; failures are logged, never fatal.
pub fn save(items: &[String]) {
    let _ = crate::fs::vfs::create_dir(DIR);
    if let Err(e) = crate::fs::vfs::write_file(BOOKMARKS_FILE, join_lines(items).as_bytes()) {
        crate::println!("[browser] bookmark save failed: {:?}", e);
    }
}

/// Persist the most recent history entries.
pub fn save_history(history: &History) {
    let entries = history.entries();
    let start = entries.len().saturating_sub(PERSISTED_HISTORY);
    let _ = crate::fs::vfs::create_dir(DIR);
    let _ = crate::fs::vfs::write_file(HISTORY_FILE, join_lines(&entries[start..]).as_bytes());
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip_and_skip_blanks() {
        let items = alloc::vec![String::from("http://a"), String::from("https://b/c")];
        let text = join_lines(&items);
        assert_eq!(parse_lines(text.as_bytes()), items);
        assert_eq!(parse_lines(b"\n  \nhttp://x \n"), ["http://x"]);
        assert!(parse_lines(&[0xff, 0xfe]).is_empty());
    }
}
