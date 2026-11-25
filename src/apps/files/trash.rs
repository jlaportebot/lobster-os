//! The freedesktop-style Trash: `Trash/files/<name>` plus
//! `Trash/info/<name>.trashinfo` recording where each item came from.

use super::fsops::{self, Fs, OpError};
use super::model::Entry;
use super::paths;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// First line of every info file.
const INFO_HEADER: &str = "[Trash Info]";
const PATH_KEY: &str = "Path=";
const DATE_KEY: &str = "DeletionDate=";
/// Suffix of info files.
const INFO_SUFFIX: &str = ".trashinfo";
/// Date written when an info file has none.
const FALLBACK_DATE: &str = "1970-01-01T00:00:00";

/// Where the trash lives for one user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dirs {
    pub files: String,
    pub info: String,
}

/// Trash folders under the home directory `home`.
#[must_use]
pub fn dirs(home: &str) -> Dirs {
    let base = paths::join(&paths::join(&paths::join(home, ".local"), "share"), "Trash");
    Dirs {
        files: paths::join(&base, "files"),
        info: paths::join(&base, "info"),
    }
}

/// An item in the trash with the details from its info file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashEntry {
    pub name: String,
    pub original: Option<String>,
    pub deleted: Option<String>,
}

fn encode_path(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/._~-".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode_path(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|c| (*c as char).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Contents of an info file.
#[must_use]
pub fn info_text(original: &str, deleted_iso: &str) -> String {
    format!(
        "{INFO_HEADER}\n{PATH_KEY}{}\n{DATE_KEY}{deleted_iso}\n",
        encode_path(original)
    )
}

/// Read an info file: `(original path, deletion date)`.
#[must_use]
pub fn parse_info(text: &str) -> Option<(String, String)> {
    let mut lines = text.lines();
    if lines.next()?.trim() != INFO_HEADER {
        return None;
    }
    let (mut path, mut date) = (None, None);
    for line in lines {
        if let Some(v) = line.strip_prefix(PATH_KEY) {
            path = Some(decode_path(v.trim()));
        } else if let Some(v) = line.strip_prefix(DATE_KEY) {
            date = Some(v.trim().to_string());
        }
    }
    Some((path?, date.unwrap_or_else(|| FALLBACK_DATE.to_string())))
}

/// `YYYY-MM-DDTHH:MM:SS` for a local epoch time.
#[must_use]
pub fn iso_time(local_secs: u64) -> String {
    let t = crate::clock::DateTime::from_epoch_secs(local_secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// Where `original` is restored to: itself when free, else a conflict name
/// beside it.
#[must_use]
pub fn restore_target(original: &str, is_dir: bool, taken: &dyn Fn(&str) -> bool) -> String {
    let dir = paths::parent(original);
    let name = paths::file_name(original);
    if !taken(original) {
        return original.to_string();
    }
    let in_dir = |n: &str| taken(&paths::join(&dir, n));
    paths::join(&dir, &paths::conflict_name(name, is_dir, &in_dir))
}

fn info_path(d: &Dirs, name: &str) -> String {
    paths::join(&d.info, &format!("{name}{INFO_SUFFIX}"))
}

/// Create `path` and any missing parents.
pub fn ensure_dir(fs: &mut dyn Fs, path: &str) -> Result<(), OpError> {
    let mut so_far = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        so_far.push('/');
        so_far.push_str(part);
        if fs.stat(&so_far).is_none() {
            fs.mkdir(&so_far).map_err(|err| OpError::Fs {
                path: so_far.clone(),
                err,
            })?;
        }
    }
    Ok(())
}

/// Move `src` into the trash and record where it came from; returns the
/// name it has in the trash.
pub fn trash_item(
    fs: &mut dyn Fs,
    d: &Dirs,
    src: &str,
    deleted_iso: &str,
    cap: u64,
) -> Result<String, OpError> {
    ensure_dir(fs, &d.files)?;
    ensure_dir(fs, &d.info)?;
    let dest = fsops::move_item(fs, src, &d.files, cap)?;
    let name = paths::file_name(&dest).to_string();
    let text = info_text(src, deleted_iso);
    let _ = fs.write(&info_path(d, &name), text.as_bytes());
    Ok(name)
}

/// Items in the trash with their info.
pub fn list(fs: &dyn Fs, d: &Dirs) -> Vec<TrashEntry> {
    let entries: Vec<Entry> = fs.list(&d.files).unwrap_or_default();
    entries
        .into_iter()
        .map(|e| {
            let info = fs.read(&info_path(d, &e.name)).ok();
            let parsed = info.and_then(|b| parse_info(&String::from_utf8_lossy(&b)));
            TrashEntry {
                name: e.name,
                original: parsed.as_ref().map(|p| p.0.clone()),
                deleted: parsed.map(|p| p.1),
            }
        })
        .collect()
}

/// Put a trashed item back where it came from; returns the restored path.
pub fn restore(fs: &mut dyn Fs, d: &Dirs, name: &str, cap: u64) -> Result<String, OpError> {
    let entry = list(fs, d)
        .into_iter()
        .find(|e| e.name == name)
        .ok_or_else(|| OpError::Missing(name.to_string()))?;
    let original = entry
        .original
        .ok_or_else(|| OpError::Missing(format!("restore information for {name}")))?;
    let src = paths::join(&d.files, name);
    let is_dir = fs.stat(&src).is_some_and(|e| e.is_dir);
    ensure_dir(fs, &paths::parent(&original))?;
    let target = restore_target(&original, is_dir, &|p| fs.stat(p).is_some());
    fsops::move_to_path(fs, &src, &target, cap)?;
    let _ = fs.remove(&info_path(d, name));
    Ok(target)
}

/// Delete one trashed item for good.
pub fn purge(fs: &mut dyn Fs, d: &Dirs, name: &str) -> Result<(), OpError> {
    fsops::remove_tree(fs, &paths::join(&d.files, name))?;
    let _ = fs.remove(&info_path(d, name));
    Ok(())
}

/// Delete everything in the trash.
pub fn empty(fs: &mut dyn Fs, d: &Dirs) -> Result<(), OpError> {
    for e in fs.list(&d.files).unwrap_or_default() {
        purge(fs, d, &e.name)?;
    }
    Ok(())
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::apps::files::testfs::MemFs;

    const CAP: u64 = 1000;
    const HOME: &str = "/home/user";

    fn fs_with_note() -> MemFs {
        MemFs::with(&[
            ("/home", None),
            ("/home/user", None),
            ("/home/user/docs", None),
            ("/home/user/docs/note.txt", Some("hi")),
        ])
    }

    #[test]
    fn trash_dirs_follow_the_freedesktop_layout() {
        let d = dirs(HOME);
        assert_eq!(d.files, "/home/user/.local/share/Trash/files");
        assert_eq!(d.info, "/home/user/.local/share/Trash/info");
    }

    #[test]
    fn info_files_round_trip_awkward_paths() {
        let path = "/home/user/My Docs/résumé 100%.txt";
        let text = info_text(path, "2026-11-19T11:02:00");
        assert!(text.starts_with("[Trash Info]\nPath=/home/user/My%20Docs/"));
        assert_eq!(
            parse_info(&text),
            Some((path.to_string(), "2026-11-19T11:02:00".to_string()))
        );
    }

    #[test]
    fn malformed_info_is_rejected() {
        assert_eq!(parse_info("Path=/x\n"), None);
        assert_eq!(parse_info("[Trash Info]\nDeletionDate=x\n"), None);
    }

    #[test]
    fn iso_time_has_the_trashinfo_shape() {
        assert_eq!(iso_time(1_700_000_000), "2023-11-14T22:13:20");
    }

    #[test]
    fn restore_uses_the_original_path_when_free() {
        assert_eq!(restore_target("/a/b.txt", false, &|_| false), "/a/b.txt");
    }

    #[test]
    fn restore_names_a_conflict_beside_the_original() {
        let taken = |p: &str| p == "/a/b.txt";
        assert_eq!(restore_target("/a/b.txt", false, &taken), "/a/b (copy).txt");
        let taken = |p: &str| p == "/a/b.txt" || p == "/a/b (copy).txt";
        assert_eq!(
            restore_target("/a/b.txt", false, &taken),
            "/a/b (copy 2).txt"
        );
    }

    #[test]
    fn trashing_moves_the_item_and_writes_info() {
        let mut fs = fs_with_note();
        let d = dirs(HOME);
        let name = trash_item(
            &mut fs,
            &d,
            "/home/user/docs/note.txt",
            "2026-11-19T11:02:00",
            CAP,
        );
        assert_eq!(name, Ok("note.txt".to_string()));
        assert!(fs.stat("/home/user/docs/note.txt").is_none());
        assert_eq!(
            fs.read(&format!("{}/note.txt", d.files)),
            Ok(b"hi".to_vec())
        );
        let listed = list(&fs, &d);
        assert_eq!(
            listed[0].original.as_deref(),
            Some("/home/user/docs/note.txt")
        );
        assert_eq!(listed[0].deleted.as_deref(), Some("2026-11-19T11:02:00"));
    }

    #[test]
    fn trashing_two_items_with_one_name_keeps_both() {
        let mut fs = fs_with_note();
        let d = dirs(HOME);
        trash_item(&mut fs, &d, "/home/user/docs/note.txt", "t", CAP).ok();
        fs.write("/home/user/note.txt", b"second").ok();
        let second = trash_item(&mut fs, &d, "/home/user/note.txt", "t", CAP);
        assert_eq!(second, Ok("note (copy).txt".to_string()));
        assert_eq!(list(&fs, &d).len(), 2);
    }

    #[test]
    fn restore_puts_the_item_back_and_drops_its_info() {
        let mut fs = fs_with_note();
        let d = dirs(HOME);
        trash_item(&mut fs, &d, "/home/user/docs/note.txt", "t", CAP).ok();
        let back = restore(&mut fs, &d, "note.txt", CAP);
        assert_eq!(back, Ok("/home/user/docs/note.txt".to_string()));
        assert!(list(&fs, &d).is_empty());
        assert!(fs.stat(&info_path(&d, "note.txt")).is_none());
    }

    #[test]
    fn restore_onto_a_taken_name_is_conflict_named() {
        let mut fs = fs_with_note();
        let d = dirs(HOME);
        trash_item(&mut fs, &d, "/home/user/docs/note.txt", "t", CAP).ok();
        fs.write("/home/user/docs/note.txt", b"new").ok();
        let back = restore(&mut fs, &d, "note.txt", CAP);
        assert_eq!(back, Ok("/home/user/docs/note (copy).txt".to_string()));
        assert_eq!(fs.read("/home/user/docs/note.txt"), Ok(b"new".to_vec()));
    }

    #[test]
    fn restore_recreates_a_deleted_parent_folder() {
        let mut fs = fs_with_note();
        let d = dirs(HOME);
        trash_item(&mut fs, &d, "/home/user/docs/note.txt", "t", CAP).ok();
        fs.remove("/home/user/docs").ok();
        assert!(restore(&mut fs, &d, "note.txt", CAP).is_ok());
        assert!(fs.stat("/home/user/docs/note.txt").is_some());
    }

    #[test]
    fn trashing_a_folder_and_purging_it_removes_everything() {
        let mut fs = fs_with_note();
        let d = dirs(HOME);
        trash_item(&mut fs, &d, "/home/user/docs", "t", CAP).ok();
        assert!(purge(&mut fs, &d, "docs").is_ok());
        assert!(list(&fs, &d).is_empty());
        assert!(fs.paths().iter().all(|p| !p.contains("note.txt")));
    }

    #[test]
    fn emptying_clears_every_item() {
        let mut fs = fs_with_note();
        let d = dirs(HOME);
        trash_item(&mut fs, &d, "/home/user/docs/note.txt", "t", CAP).ok();
        trash_item(&mut fs, &d, "/home/user/docs", "t", CAP).ok();
        assert!(empty(&mut fs, &d).is_ok());
        assert!(list(&fs, &d).is_empty());
    }

    #[test]
    fn restoring_something_not_in_the_trash_reports_it() {
        let mut fs = fs_with_note();
        let d = dirs(HOME);
        ensure_dir(&mut fs, &d.files).ok();
        assert_eq!(
            restore(&mut fs, &d, "ghost", CAP),
            Err(OpError::Missing("ghost".into()))
        );
    }
}
