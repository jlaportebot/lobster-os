//! Directory entries, sorting and the text shown in the columns.

use crate::display::icons::{self, Icon};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;

/// Bytes per binary unit step.
const UNIT: u64 = 1024;
/// Size units above bytes.
const UNIT_NAMES: [&str; 3] = ["KB", "MB", "GB"];

/// One item of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// Size in bytes (0 for folders).
    pub size: u64,
    /// Modification time in seconds since the epoch (0 = unknown).
    pub mtime: u64,
}

/// List column a sort can be keyed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
    Type,
}

/// A sort key plus direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

impl Sort {
    /// Ascending by name, the default.
    pub const DEFAULT: Sort = Sort {
        key: SortKey::Name,
        descending: false,
    };

    /// Click on a column header: same column flips direction, another
    /// column starts ascending.
    #[must_use]
    pub fn after_click(self, key: SortKey) -> Sort {
        if key == self.key {
            Sort {
                key,
                descending: !self.descending,
            }
        } else {
            Sort {
                key,
                descending: false,
            }
        }
    }
}

/// Items shown as a table or as a grid of large icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Icons,
}

fn name_order(a: &Entry, b: &Entry) -> Ordering {
    a.name
        .to_ascii_lowercase()
        .cmp(&b.name.to_ascii_lowercase())
        .then_with(|| a.name.cmp(&b.name))
}

fn key_order(a: &Entry, b: &Entry, key: SortKey) -> Ordering {
    match key {
        SortKey::Name => name_order(a, b),
        SortKey::Size => a.size.cmp(&b.size),
        SortKey::Modified => a.mtime.cmp(&b.mtime),
        SortKey::Type => type_label(a).cmp(type_label(b)),
    }
}

/// Sort in place: folders first, then by the key (ties by name), with the
/// direction applied inside each group.
pub fn sort_entries(entries: &mut [Entry], sort: Sort) {
    entries.sort_by(|a, b| {
        let by_key = key_order(a, b, sort.key);
        let by_key = if sort.descending {
            by_key.reverse()
        } else {
            by_key
        };
        b.is_dir
            .cmp(&a.is_dir)
            .then(by_key)
            .then_with(|| name_order(a, b))
    });
}

/// Short description of the kind of item, also the Type sort key.
#[must_use]
pub fn type_label(entry: &Entry) -> &'static str {
    match icons::for_file_name(&entry.name, entry.is_dir) {
        Icon::FileFolder => "Folder",
        Icon::FileText => "Text",
        Icon::FileCode => "Source",
        Icon::FileImage => "Image",
        Icon::FileAudio => "Audio",
        Icon::FileVideo => "Video",
        Icon::FileArchive => "Archive",
        Icon::FileExecutable => "Program",
        Icon::FilePdf => "PDF",
        Icon::FileLink => "Link",
        _ => "File",
    }
}

/// Human size: `812 B`, `1.5 KB`, `2.0 MB`, `1.3 GB`.
#[must_use]
pub fn size_text(bytes: u64) -> String {
    if bytes < UNIT {
        return format!("{bytes} B");
    }
    let mut unit = UNIT;
    let mut name = 0;
    while bytes / unit >= UNIT && name + 1 < UNIT_NAMES.len() {
        unit *= UNIT;
        name += 1;
    }
    let tenths = bytes * 10 / unit;
    format!("{}.{} {}", tenths / 10, tenths % 10, UNIT_NAMES[name])
}

/// `YYYY-MM-DD HH:MM` for a local epoch time, or an em dash when unknown.
#[must_use]
pub fn date_text(local_secs: u64) -> String {
    if local_secs == 0 {
        return "\u{2014}".to_string();
    }
    let t = crate::clock::DateTime::from_epoch_secs(local_secs);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute
    )
}

/// Names of every entry in `entries`, in order.
#[must_use]
pub fn names(entries: &[Entry]) -> Vec<String> {
    entries.iter().map(|e| e.name.clone()).collect()
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, mtime: u64) -> Entry {
        Entry {
            name: name.into(),
            is_dir: false,
            size,
            mtime,
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            name: name.into(),
            is_dir: true,
            size: 0,
            mtime: 0,
        }
    }

    fn sorted(mut v: Vec<Entry>, sort: Sort) -> Vec<String> {
        sort_entries(&mut v, sort);
        names(&v)
    }

    fn sample() -> Vec<Entry> {
        vec![
            file("b.txt", 300, 20),
            dir("Zeta"),
            file("A.png", 100, 30),
            dir("alpha"),
            file("c.rs", 200, 10),
        ]
    }

    #[test]
    fn name_sort_puts_folders_first_and_ignores_case() {
        assert_eq!(
            sorted(sample(), Sort::DEFAULT),
            ["alpha", "Zeta", "A.png", "b.txt", "c.rs"]
        );
    }

    #[test]
    fn reversing_keeps_folders_first() {
        let sort = Sort {
            key: SortKey::Name,
            descending: true,
        };
        assert_eq!(
            sorted(sample(), sort),
            ["Zeta", "alpha", "c.rs", "b.txt", "A.png"]
        );
    }

    #[test]
    fn size_sort_orders_files_and_breaks_ties_by_name() {
        let mut v = sample();
        v.push(file("d.txt", 100, 5));
        let sort = Sort {
            key: SortKey::Size,
            descending: false,
        };
        assert_eq!(
            sorted(v, sort),
            ["alpha", "Zeta", "A.png", "d.txt", "c.rs", "b.txt"]
        );
    }

    #[test]
    fn modified_sort_orders_by_time() {
        let sort = Sort {
            key: SortKey::Modified,
            descending: true,
        };
        assert_eq!(
            sorted(sample(), sort),
            ["alpha", "Zeta", "A.png", "b.txt", "c.rs"]
        );
    }

    #[test]
    fn type_sort_groups_by_kind() {
        let sort = Sort {
            key: SortKey::Type,
            descending: false,
        };
        let out = sorted(sample(), sort);
        assert_eq!(&out[..2], ["alpha", "Zeta"]);
        assert_eq!(out[2], "A.png");
    }

    #[test]
    fn header_click_toggles_then_resets_direction() {
        let s = Sort::DEFAULT.after_click(SortKey::Name);
        assert!(s.descending);
        let s = s.after_click(SortKey::Size);
        assert_eq!((s.key, s.descending), (SortKey::Size, false));
    }

    #[test]
    fn type_labels_follow_the_extension() {
        assert_eq!(type_label(&dir("x")), "Folder");
        assert_eq!(type_label(&file("a.TXT", 0, 0)), "Text");
        assert_eq!(type_label(&file("a.png", 0, 0)), "Image");
        assert_eq!(type_label(&file("a", 0, 0)), "File");
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(size_text(0), "0 B");
        assert_eq!(size_text(812), "812 B");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(2 * 1024 * 1024), "2.0 MB");
        assert_eq!(size_text(1_400_000_000), "1.3 GB");
    }

    #[test]
    fn dates_are_formatted_or_dashed() {
        assert_eq!(date_text(0), "\u{2014}");
        assert_eq!(date_text(1_700_000_000), "2023-11-14 22:13");
    }
}
