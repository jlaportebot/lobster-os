//! Pure path and file-name helpers for the Files app.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The filesystem root.
pub const ROOT: &str = "/";
/// Longest file name the Files app accepts (ext2 limit).
pub const MAX_NAME_LEN: usize = 255;

/// Why a proposed file name was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    HasSlash,
    Reserved,
    TooLong,
    Exists,
}

impl NameError {
    /// Message shown to the user.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            NameError::Empty => "The name cannot be empty.",
            NameError::HasSlash => "The name cannot contain '/'.",
            NameError::Reserved => "That name is reserved.",
            NameError::TooLong => "The name is too long.",
            NameError::Exists => "An item with that name already exists.",
        }
    }
}

/// One clickable segment of the breadcrumb bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    /// Text shown for the segment.
    pub label: String,
    /// Absolute path the segment navigates to.
    pub path: String,
}

/// `dir` and `name` joined with exactly one separator.
#[must_use]
pub fn join(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

/// Resolve `.` and `..` and collapse repeated separators.
#[must_use]
pub fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Parent directory of `path` (the root is its own parent).
#[must_use]
pub fn parent(path: &str) -> String {
    let norm = normalize(path);
    match norm.rfind('/') {
        Some(0) | None => ROOT.to_string(),
        Some(at) => norm[..at].to_string(),
    }
}

/// Last component of `path` (empty for the root).
#[must_use]
pub fn file_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or("")
}

/// Whether `path` equals `ancestor` or lies below it.
#[must_use]
pub fn is_same_or_inside(path: &str, ancestor: &str) -> bool {
    let (path, ancestor) = (path.trim_end_matches('/'), ancestor.trim_end_matches('/'));
    ancestor.is_empty()
        || path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Validate a new name; `taken` says whether a sibling already has it.
pub fn check_name(name: &str, taken: &dyn Fn(&str) -> bool) -> Result<(), NameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.contains('/') {
        return Err(NameError::HasSlash);
    }
    if name == "." || name == ".." {
        return Err(NameError::Reserved);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(NameError::TooLong);
    }
    if taken(name) {
        return Err(NameError::Exists);
    }
    Ok(())
}

/// Split `name.ext` into its stem and extension (with the dot); names
/// without a real extension keep everything in the stem.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) if at > 0 && at + 1 < name.len() => name.split_at(at),
        _ => (name, ""),
    }
}

/// A free name for a pasted copy: `name (copy)`, `name (copy 2)`, ... with
/// the marker placed before a file's extension.
#[must_use]
pub fn conflict_name(name: &str, is_dir: bool, taken: &dyn Fn(&str) -> bool) -> String {
    let (stem, ext) = if is_dir {
        (name, "")
    } else {
        split_extension(name)
    };
    let first = format!("{stem} (copy){ext}");
    if !taken(&first) {
        return first;
    }
    (2u32..)
        .map(|n| format!("{stem} (copy {n}){ext}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or(first)
}

/// A free default for a new item: `base`, `base 2`, `base 3`, ...
#[must_use]
pub fn unique_default(base: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2u32..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| base.to_string())
}

/// Breadcrumb segments for `path`, root first.
#[must_use]
pub fn crumbs(path: &str) -> Vec<Crumb> {
    let mut out = alloc::vec![Crumb {
        label: ROOT.to_string(),
        path: ROOT.to_string(),
    }];
    let mut so_far = String::new();
    for part in normalize(path).split('/').filter(|p| !p.is_empty()) {
        so_far.push('/');
        so_far.push_str(part);
        out.push(Crumb {
            label: part.to_string(),
            path: so_far.clone(),
        });
    }
    out
}

/// Index of the first breadcrumb that still fits: everything before it is
/// replaced by one "…" item. The last segment is always kept.
#[must_use]
pub fn first_visible_crumb(widths: &[u32], sep_w: u32, ellipsis_w: u32, avail: u32) -> usize {
    let Some(last) = widths.len().checked_sub(1) else {
        return 0;
    };
    for first in 0..last {
        let shown = &widths[first..];
        let seps = sep_w * (shown.len() as u32 - 1);
        let lead = if first > 0 { ellipsis_w + sep_w } else { 0 };
        if lead + shown.iter().sum::<u32>() + seps <= avail {
            return first;
        }
    }
    last
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn none(_: &str) -> bool {
        false
    }

    #[test]
    fn join_uses_one_separator() {
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/home/user", "a.txt"), "/home/user/a.txt");
        assert_eq!(join("/home/user/", "a"), "/home/user/a");
    }

    #[test]
    fn normalize_resolves_dots() {
        assert_eq!(normalize("/a/./b/../c//d/"), "/a/c/d");
        assert_eq!(normalize("/.."), "/");
        assert_eq!(normalize(""), "/");
    }

    #[test]
    fn parent_of_root_is_root() {
        assert_eq!(parent("/"), "/");
        assert_eq!(parent("/home"), "/");
        assert_eq!(parent("/home/user/x"), "/home/user");
    }

    #[test]
    fn file_name_is_the_last_component() {
        assert_eq!(file_name("/home/user/a.txt"), "a.txt");
        assert_eq!(file_name("/"), "");
    }

    #[test]
    fn inside_detects_self_and_descendants_only() {
        assert!(is_same_or_inside("/a/b", "/a/b"));
        assert!(is_same_or_inside("/a/b/c", "/a/b"));
        assert!(!is_same_or_inside("/a/bc", "/a/b"));
        assert!(!is_same_or_inside("/a", "/a/b"));
        assert!(is_same_or_inside("/anything", "/"));
    }

    #[test]
    fn names_are_validated() {
        let taken = |n: &str| n == "dup";
        assert_eq!(check_name("", &taken), Err(NameError::Empty));
        assert_eq!(check_name("   ", &taken), Err(NameError::Empty));
        assert_eq!(check_name("a/b", &taken), Err(NameError::HasSlash));
        assert_eq!(check_name("..", &taken), Err(NameError::Reserved));
        assert_eq!(check_name(".", &taken), Err(NameError::Reserved));
        assert_eq!(check_name("dup", &taken), Err(NameError::Exists));
        assert_eq!(check_name("fine.txt", &taken), Ok(()));
        let long = "x".repeat(MAX_NAME_LEN + 1);
        assert_eq!(check_name(&long, &taken), Err(NameError::TooLong));
    }

    #[test]
    fn conflict_names_count_up_before_the_extension() {
        assert_eq!(conflict_name("a.txt", false, &none), "a (copy).txt");
        let taken = |n: &str| n == "a (copy).txt";
        assert_eq!(conflict_name("a.txt", false, &taken), "a (copy 2).txt");
        let taken = |n: &str| n == "a (copy).txt" || n == "a (copy 2).txt";
        assert_eq!(conflict_name("a.txt", false, &taken), "a (copy 3).txt");
    }

    #[test]
    fn conflict_names_for_folders_and_dotfiles_append() {
        assert_eq!(conflict_name("docs", true, &none), "docs (copy)");
        assert_eq!(conflict_name("v1.2", true, &none), "v1.2 (copy)");
        assert_eq!(conflict_name(".bashrc", false, &none), ".bashrc (copy)");
        assert_eq!(conflict_name("noext", false, &none), "noext (copy)");
    }

    #[test]
    fn unique_default_numbers_from_two() {
        assert_eq!(unique_default("New Folder", &none), "New Folder");
        let taken = |n: &str| n == "New Folder" || n == "New Folder 2";
        assert_eq!(unique_default("New Folder", &taken), "New Folder 3");
    }

    #[test]
    fn crumbs_start_at_the_root() {
        let c = crumbs("/home/user");
        let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["/", "home", "user"]);
        assert_eq!(c[0].path, "/");
        assert_eq!(c[1].path, "/home");
        assert_eq!(c[2].path, "/home/user");
        assert_eq!(crumbs("/").len(), 1);
    }

    #[test]
    fn everything_visible_when_it_fits() {
        // three crumbs of 20 px, 2 separators of 10 px
        assert_eq!(first_visible_crumb(&[20, 20, 20], 10, 15, 80), 0);
    }

    #[test]
    fn leading_crumbs_are_elided_when_too_wide() {
        // all: 20*4+10*3 = 110; from 1: 15+10 + 20*3+10*2 = 105; from 2: 25+40+10 = 75
        assert_eq!(first_visible_crumb(&[20, 20, 20, 20], 10, 15, 80), 2);
    }

    #[test]
    fn the_last_crumb_is_always_kept() {
        assert_eq!(first_visible_crumb(&[20, 20, 200], 10, 15, 50), 2);
        assert_eq!(first_visible_crumb(&[], 10, 15, 50), 0);
    }
}
