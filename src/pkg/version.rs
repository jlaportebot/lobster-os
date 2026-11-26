//! Package version ordering.
//!
//! Versions are dotted numbers with an optional `-suffix` (`1.10.0`,
//! `2.0.0-rc1`). Numbers compare as numbers, so `1.10.0` is newer than
//! `1.9.0`; a suffix marks a pre-release, which sorts before the plain
//! version.

use core::cmp::Ordering;

/// Compare two dotted version strings.
#[must_use]
pub fn compare(a: &str, b: &str) -> Ordering {
    let (core_a, pre_a) = split_pre_release(a);
    let (core_b, pre_b) = split_pre_release(b);
    compare_cores(core_a, core_b).then_with(|| match (pre_a, pre_b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    })
}

/// Whether `candidate` is strictly newer than `installed`.
#[must_use]
pub fn is_newer(candidate: &str, installed: &str) -> bool {
    compare(candidate, installed) == Ordering::Greater
}

fn split_pre_release(version: &str) -> (&str, Option<&str>) {
    match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    }
}

/// Numeric value of one dotted segment; anything unparsable counts as 0 so
/// a malformed index never panics or reorders the whole catalogue.
fn segment_value(segment: &str) -> u64 {
    let digits_end = segment
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(segment.len());
    segment[..digits_end].parse().unwrap_or(0)
}

fn compare_cores(a: &str, b: &str) -> Ordering {
    let mut left = a.split('.');
    let mut right = b.split('.');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (x, y) => {
                let order = segment_value(x.unwrap_or("0")).cmp(&segment_value(y.unwrap_or("0")));
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn ten_is_newer_than_nine() {
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.9.0", "1.10.0"));
    }

    #[test]
    fn equal_versions_are_not_newer() {
        assert_eq!(compare("1.0.0", "1.0.0"), Ordering::Equal);
        assert!(!is_newer("1.0.0", "1.0.0"));
    }

    #[test]
    fn missing_segments_count_as_zero() {
        assert_eq!(compare("1.2", "1.2.0"), Ordering::Equal);
        assert!(is_newer("1.2.1", "1.2"));
    }

    #[test]
    fn major_dominates_minor() {
        assert!(is_newer("2.0.0", "1.99.99"));
    }

    #[test]
    fn pre_release_sorts_before_release() {
        assert!(is_newer("2.0.0", "2.0.0-rc1"));
        assert!(is_newer("2.0.0-rc2", "2.0.0-rc1"));
    }

    #[test]
    fn garbage_segments_do_not_panic() {
        assert_eq!(compare("x.y", "0.0"), Ordering::Equal);
        assert_eq!(compare("", ""), Ordering::Equal);
        assert!(is_newer("1.0.0", ""));
    }

    #[test]
    fn trailing_letters_in_a_segment_are_ignored() {
        assert_eq!(compare("1.2a", "1.2"), Ordering::Equal);
    }
}
