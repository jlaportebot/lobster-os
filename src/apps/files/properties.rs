//! What the Properties panel shows: aggregation over a capped folder walk
//! and the text of each row.

use super::fsops::Fs;
use super::model::{self, size_text};
use super::paths;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Most items a recursive size walk visits before giving up.
pub const WALK_LIMIT: usize = 4000;

/// Totals of a file or folder tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    /// The walk stopped at the limit, so the totals are lower bounds.
    pub truncated: bool,
}

impl Summary {
    /// Add another summary.
    pub fn merge(&mut self, other: Summary) {
        self.files += other.files;
        self.folders += other.folders;
        self.bytes += other.bytes;
        self.truncated |= other.truncated;
    }

    /// Files plus folders.
    #[must_use]
    pub fn items(&self) -> u64 {
        self.files + self.folders
    }
}

/// Totals for `path`: a file counts itself; a folder counts everything
/// below it (not itself), visiting at most `limit` items.
pub fn summarize(fs: &dyn Fs, path: &str, limit: usize) -> Summary {
    let mut total = Summary::default();
    let mut visited = 0usize;
    match fs.stat(path) {
        Some(e) if !e.is_dir => {
            total.files = 1;
            total.bytes = e.size;
        }
        Some(_) => walk(fs, path, limit, &mut visited, &mut total),
        None => {}
    }
    total
}

fn walk(fs: &dyn Fs, dir: &str, limit: usize, visited: &mut usize, total: &mut Summary) {
    for child in fs.list(dir).unwrap_or_default() {
        if *visited >= limit {
            total.truncated = true;
            return;
        }
        *visited += 1;
        if child.is_dir {
            total.folders += 1;
            walk(fs, &paths::join(dir, &child.name), limit, visited, total);
        } else {
            total.files += 1;
            total.bytes += child.size;
        }
    }
}

/// `drwxr-xr-x` style text of a mode.
#[must_use]
pub fn mode_text(mode: u32, is_dir: bool) -> String {
    let mut out = String::from(if is_dir { "d" } else { "-" });
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    out
}

/// Everything known about one item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    pub name: String,
    pub kind: &'static str,
    pub path: String,
    pub is_dir: bool,
    pub summary: Summary,
    /// Modified time as local epoch seconds (0 = unknown).
    pub modified: u64,
    pub owner: String,
    pub group: String,
    pub mode: u32,
}

fn size_row(d: &Details) -> String {
    if !d.is_dir {
        return format!("{} ({} bytes)", size_text(d.summary.bytes), d.summary.bytes);
    }
    let more = if d.summary.truncated { "+" } else { "" };
    let count = d.summary.items();
    let noun = if count == 1 && more.is_empty() {
        "item"
    } else {
        "items"
    };
    format!(
        "{count}{more} {noun}, {}{}",
        size_text(d.summary.bytes),
        more
    )
}

/// Label / value rows for a single item.
#[must_use]
pub fn rows(d: &Details) -> Vec<(String, String)> {
    let row = |k: &str, v: String| (k.to_string(), v);
    alloc::vec![
        row("Name", d.name.clone()),
        row("Type", d.kind.to_string()),
        row("Location", d.path.clone()),
        row("Size", size_row(d)),
        row("Modified", model::date_text(d.modified)),
        row("Owner", format!("{} / {}", d.owner, d.group)),
        row(
            "Mode",
            format!("{} ({:04o})", mode_text(d.mode, d.is_dir), d.mode & 0o7777)
        ),
    ]
}

/// Rows for a multi-item selection.
#[must_use]
pub fn multi_rows(count: usize, total: Summary, location: &str) -> Vec<(String, String)> {
    let more = if total.truncated { "+" } else { "" };
    alloc::vec![
        ("Selection".to_string(), format!("{count} items")),
        ("Location".to_string(), location.to_string()),
        (
            "Contains".to_string(),
            format!("{} files, {} folders{more}", total.files, total.folders),
        ),
        (
            "Total size".to_string(),
            format!("{}{more}", size_text(total.bytes))
        ),
    ]
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::apps::files::testfs::MemFs;

    fn tree() -> MemFs {
        MemFs::with(&[
            ("/a", None),
            ("/a/one.txt", Some("1234")),
            ("/a/sub", None),
            ("/a/sub/two.txt", Some("123456")),
            ("/a/sub/deep", None),
            ("/lone.bin", Some("xx")),
        ])
    }

    #[test]
    fn a_file_counts_itself() {
        let s = summarize(&tree(), "/lone.bin", WALK_LIMIT);
        assert_eq!((s.files, s.folders, s.bytes), (1, 0, 2));
    }

    #[test]
    fn a_folder_totals_everything_below_it() {
        let s = summarize(&tree(), "/a", WALK_LIMIT);
        assert_eq!(
            (s.files, s.folders, s.bytes, s.truncated),
            (2, 2, 10, false)
        );
        assert_eq!(s.items(), 4);
    }

    #[test]
    fn the_walk_is_capped_and_flagged() {
        let s = summarize(&tree(), "/a", 2);
        assert!(s.truncated);
        assert_eq!(s.items(), 2);
    }

    #[test]
    fn a_missing_path_summarizes_to_nothing() {
        assert_eq!(summarize(&tree(), "/gone", WALK_LIMIT), Summary::default());
    }

    #[test]
    fn summaries_merge() {
        let mut a = summarize(&tree(), "/a", WALK_LIMIT);
        a.merge(summarize(&tree(), "/lone.bin", WALK_LIMIT));
        assert_eq!((a.files, a.folders, a.bytes), (3, 2, 12));
    }

    #[test]
    fn modes_print_like_ls() {
        assert_eq!(mode_text(0o755, true), "drwxr-xr-x");
        assert_eq!(mode_text(0o644, false), "-rw-r--r--");
        assert_eq!(mode_text(0o000, false), "----------");
    }

    fn details(is_dir: bool, summary: Summary) -> Details {
        Details {
            name: "a".into(),
            kind: if is_dir { "Folder" } else { "Text" },
            path: "/home/user/a".into(),
            is_dir,
            summary,
            modified: 0,
            owner: "user".into(),
            group: "users".into(),
            mode: 0o755,
        }
    }

    #[test]
    fn file_rows_show_exact_bytes() {
        let s = Summary {
            files: 1,
            bytes: 1536,
            ..Summary::default()
        };
        let r = rows(&details(false, s));
        assert_eq!(
            r[3],
            ("Size".to_string(), "1.5 KB (1536 bytes)".to_string())
        );
        assert_eq!(r[2].1, "/home/user/a");
        assert_eq!(r[6].1, "-rwxr-xr-x (0755)");
    }

    #[test]
    fn folder_rows_show_count_and_total_with_a_cap_marker() {
        let s = Summary {
            files: 3,
            folders: 1,
            bytes: 2048,
            truncated: true,
        };
        let r = rows(&details(true, s));
        assert_eq!(r[3].1, "4+ items, 2.0 KB+");
    }

    #[test]
    fn a_folder_with_one_item_says_item_not_items() {
        let s = Summary {
            files: 1,
            folders: 0,
            bytes: 9,
            truncated: false,
        };
        let r = rows(&details(true, s));
        assert_eq!(r[3].1, "1 item, 9 B");
    }

    #[test]
    fn multi_rows_give_count_and_total() {
        let s = Summary {
            files: 5,
            folders: 2,
            bytes: 3072,
            truncated: false,
        };
        let r = multi_rows(3, s, "/home/user");
        assert_eq!(r[0].1, "3 items");
        assert_eq!(r[2].1, "5 files, 2 folders");
        assert_eq!(r[3].1, "3.0 KB");
    }
}
