//! File operations behind a small filesystem trait: copy planning (with the
//! size cap checked up front), recursive copy with rollback, move and delete.
//! The kernel VFS implements [`Fs`] in production; tests use an in-memory one.

use super::model::Entry;
use super::paths;
use crate::fs::vfs::{self, VfsError};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Mount point of the host shared folder.
const SHARE_ROOT: &str = "/share";
/// Whether `path` is the host shared folder or inside it.
#[must_use]
pub fn is_share(path: &str) -> bool {
    path == SHARE_ROOT || path.starts_with("/share/")
}

/// Alert text for an operation the host share device cannot do.
#[must_use]
pub fn share_unsupported(what: &str) -> String {
    format!("The shared folder does not support {what}.")
}

/// Deepest folder nesting a recursive operation follows (guards cycles).
const MAX_DEPTH: usize = 32;

/// The filesystem operations the Files app needs.
pub trait Fs {
    /// Entries of folder `dir`.
    fn list(&self, dir: &str) -> Result<Vec<Entry>, VfsError>;
    /// The item at `path`, if it exists.
    fn stat(&self, path: &str) -> Option<Entry>;
    /// Whole contents of the file at `path`.
    fn read(&self, path: &str) -> Result<Vec<u8>, VfsError>;
    /// Create or replace the file at `path`.
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError>;
    /// Create an empty folder.
    fn mkdir(&mut self, path: &str) -> Result<(), VfsError>;
    /// Remove a file or an empty folder.
    fn remove(&mut self, path: &str) -> Result<(), VfsError>;
    /// Move or rename within the filesystem.
    fn rename(&mut self, from: &str, to: &str) -> Result<(), VfsError>;
}

/// Why an operation was refused or failed.
#[derive(Clone, Debug, PartialEq)]
pub enum OpError {
    /// A folder cannot be copied or moved into itself.
    IntoSelf(String),
    /// A file is bigger than the VFS per-file cap.
    TooLarge { name: String, size: u64, cap: u64 },
    /// The source disappeared.
    Missing(String),
    /// The folder tree is nested too deeply to follow.
    TooDeep(String),
    /// The filesystem refused a step.
    Fs { path: String, err: VfsError },
}

impl OpError {
    /// Message shown in an alert.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            OpError::IntoSelf(name) => format!("Cannot copy or move \"{name}\" into itself."),
            OpError::TooLarge { name, size, cap } => {
                format!("\"{name}\" is {size} bytes; files over {cap} bytes are not supported.")
            }
            OpError::Missing(name) => format!("\"{name}\" no longer exists."),
            OpError::TooDeep(name) => format!("\"{name}\" is nested too deeply."),
            OpError::Fs { path, err } if is_share(path) && *err == VfsError::ReadOnly => {
                share_unsupported("creating folders, renaming or deleting items")
            }
            OpError::Fs { path, err } => format!("{path}: {err}"),
        }
    }
}

fn fs_err(path: &str, err: VfsError) -> OpError {
    OpError::Fs {
        path: path.to_string(),
        err,
    }
}

/// One step of a copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Mkdir(String),
    Copy { from: String, to: String },
}

/// Everything a copy will do, decided before anything is written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
}

/// Work out the steps to copy `src` to the new path `dest`, refusing
/// self-copies and over-cap files before anything is written.
pub fn plan_copy(fs: &dyn Fs, src: &str, dest: &str, cap: u64) -> Result<Plan, OpError> {
    let entry = fs
        .stat(src)
        .ok_or_else(|| OpError::Missing(paths::file_name(src).to_string()))?;
    if entry.is_dir && paths::is_same_or_inside(dest, src) {
        return Err(OpError::IntoSelf(entry.name));
    }
    let mut plan = Plan::default();
    plan_into(fs, src, dest, cap, 0, &mut plan)?;
    Ok(plan)
}

fn plan_into(
    fs: &dyn Fs,
    src: &str,
    dest: &str,
    cap: u64,
    depth: usize,
    plan: &mut Plan,
) -> Result<(), OpError> {
    let name = paths::file_name(src).to_string();
    if depth > MAX_DEPTH {
        return Err(OpError::TooDeep(name));
    }
    let entry = fs.stat(src).ok_or_else(|| OpError::Missing(name.clone()))?;
    if !entry.is_dir {
        if entry.size > cap {
            return Err(OpError::TooLarge {
                name,
                size: entry.size,
                cap,
            });
        }
        plan.steps.push(Step::Copy {
            from: src.to_string(),
            to: dest.to_string(),
        });
        return Ok(());
    }
    plan.steps.push(Step::Mkdir(dest.to_string()));
    for child in fs.list(src).map_err(|e| fs_err(src, e))? {
        let child_src = paths::join(src, &child.name);
        let child_dest = paths::join(dest, &child.name);
        plan_into(fs, &child_src, &child_dest, cap, depth + 1, plan)?;
    }
    Ok(())
}

fn run_step(fs: &mut dyn Fs, step: &Step) -> Result<String, OpError> {
    match step {
        Step::Mkdir(path) => fs.mkdir(path).map_err(|e| fs_err(path, e))?,
        Step::Copy { from, to } => {
            let data = fs.read(from).map_err(|e| fs_err(from, e))?;
            fs.write(to, &data).map_err(|e| fs_err(to, e))?;
        }
    }
    Ok(match step {
        Step::Mkdir(path) => path.clone(),
        Step::Copy { to, .. } => to.clone(),
    })
}

/// Run a plan. When a step fails, everything this run created is removed
/// again (best effort, newest first) so no partial copy is left behind.
pub fn execute(fs: &mut dyn Fs, plan: &Plan) -> Result<(), OpError> {
    let mut created: Vec<String> = Vec::new();
    for step in &plan.steps {
        match run_step(fs, step) {
            Ok(path) => created.push(path),
            Err(err) => {
                for path in created.iter().rev() {
                    let _ = fs.remove(path);
                }
                if let Step::Copy { to, .. } = step {
                    let _ = fs.remove(to);
                }
                return Err(err);
            }
        }
    }
    Ok(())
}

/// Free destination path for `src` inside folder `dest_dir`: the plain name
/// when free, else `name (copy)`, `name (copy 2)`, ...
pub fn destination_for(fs: &dyn Fs, src: &str, dest_dir: &str) -> String {
    let name = paths::file_name(src);
    let is_dir = fs.stat(src).is_some_and(|e| e.is_dir);
    let taken = |n: &str| fs.stat(&paths::join(dest_dir, n)).is_some();
    if !taken(name) {
        return paths::join(dest_dir, name);
    }
    paths::join(dest_dir, &paths::conflict_name(name, is_dir, &taken))
}

/// Copy `src` into `dest_dir` under a free name; returns the new path.
pub fn copy_item(fs: &mut dyn Fs, src: &str, dest_dir: &str, cap: u64) -> Result<String, OpError> {
    let dest = destination_for(fs, src, dest_dir);
    let plan = plan_copy(fs, src, &dest, cap)?;
    execute(fs, &plan)?;
    Ok(dest)
}

/// Remove `path` and everything below it.
pub fn remove_tree(fs: &mut dyn Fs, path: &str) -> Result<(), OpError> {
    remove_depth(fs, path, 0)
}

fn remove_depth(fs: &mut dyn Fs, path: &str, depth: usize) -> Result<(), OpError> {
    let name = paths::file_name(path).to_string();
    if depth > MAX_DEPTH {
        return Err(OpError::TooDeep(name));
    }
    let entry = fs.stat(path).ok_or(OpError::Missing(name))?;
    if entry.is_dir {
        for child in fs.list(path).map_err(|e| fs_err(path, e))? {
            remove_depth(fs, &paths::join(path, &child.name), depth + 1)?;
        }
    }
    fs.remove(path).map_err(|e| fs_err(path, e))
}

/// Move `src` to the exact path `dest` with a rename, falling back to copy
/// + delete. Folders cannot move into themselves.
pub fn move_to_path(fs: &mut dyn Fs, src: &str, dest: &str, cap: u64) -> Result<(), OpError> {
    if is_share(src) {
        return Err(fs_err(src, VfsError::ReadOnly));
    }
    let is_dir = fs.stat(src).is_some_and(|e| e.is_dir);
    if is_dir && paths::is_same_or_inside(dest, src) {
        return Err(OpError::IntoSelf(paths::file_name(src).to_string()));
    }
    if fs.rename(src, dest).is_ok() {
        return Ok(());
    }
    let plan = plan_copy(fs, src, dest, cap)?;
    execute(fs, &plan)?;
    remove_tree(fs, src)
}

/// Move `src` into `dest_dir` (a free name is chosen on conflict). Uses a
/// rename and falls back to copy + delete; returns the new path.
pub fn move_item(fs: &mut dyn Fs, src: &str, dest_dir: &str, cap: u64) -> Result<String, OpError> {
    if paths::parent(src) == paths::normalize(dest_dir) {
        return Ok(src.to_string());
    }
    let dest = destination_for(fs, src, dest_dir);
    move_to_path(fs, src, &dest, cap)?;
    Ok(dest)
}

/// The kernel VFS as an [`Fs`].
pub struct VfsFs;

fn entry_of(name: &str, path: &str, is_dir: bool, size: u64) -> Entry {
    let mtime = vfs::stat(path).map_or(0, |m| m.modified_time);
    Entry {
        name: name.to_string(),
        is_dir,
        size,
        mtime,
    }
}

/// The host shared folder is not part of the VFS node tree: `/share` is a
/// folder when the device is present and its children are plain files.
/// `None` means the path is not under `/share` at all.
fn share_entry(path: &str) -> Option<Option<Entry>> {
    let rel = if path == SHARE_ROOT {
        ""
    } else {
        path.strip_prefix("/share/")?
    };
    let name = paths::file_name(path).to_string();
    if rel.is_empty() {
        let present = crate::drivers::lobster_share::detect();
        return Some(present.then_some(Entry {
            name,
            is_dir: true,
            size: 0,
            mtime: 0,
        }));
    }
    let size = crate::drivers::lobster_share::stat(rel);
    Some(size.map(|size| Entry {
        name,
        is_dir: false,
        size,
        mtime: 0,
    }))
}

impl Fs for VfsFs {
    fn list(&self, dir: &str) -> Result<Vec<Entry>, VfsError> {
        let listed = vfs::list_dir(dir)?;
        Ok(listed
            .iter()
            .filter(|e| e.name_str() != "." && e.name_str() != "..")
            .map(|e| {
                entry_of(
                    e.name_str(),
                    &paths::join(dir, e.name_str()),
                    e.is_dir,
                    e.size,
                )
            })
            .collect())
    }

    fn stat(&self, path: &str) -> Option<Entry> {
        if let Some(entry) = share_entry(path) {
            return entry;
        }
        let meta = vfs::stat(path)?;
        let is_dir = meta.file_type == vfs::FileType::Directory;
        Some(Entry {
            name: paths::file_name(path).to_string(),
            is_dir,
            size: meta.size,
            mtime: meta.modified_time,
        })
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        vfs::read_file(path)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        vfs::write_file(path, data).map(|_| ())
    }

    fn mkdir(&mut self, path: &str) -> Result<(), VfsError> {
        if is_share(path) {
            return Err(VfsError::ReadOnly);
        }
        vfs::mkdir(path).map(|_| ())
    }

    fn remove(&mut self, path: &str) -> Result<(), VfsError> {
        if is_share(path) {
            return Err(VfsError::ReadOnly);
        }
        vfs::remove(path)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), VfsError> {
        if is_share(from) || is_share(to) {
            return Err(VfsError::ReadOnly);
        }
        vfs::rename(from, to)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::apps::files::testfs::MemFs;

    const CAP: u64 = 1000;

    fn tree() -> MemFs {
        MemFs::with(&[
            ("/a", None),
            ("/a/one.txt", Some("1")),
            ("/a/sub", None),
            ("/a/sub/two.txt", Some("22")),
            ("/dest", None),
            ("/note.txt", Some("hello")),
        ])
    }

    #[test]
    fn copy_file_into_a_folder_keeps_the_name() {
        let mut fs = tree();
        let out = copy_item(&mut fs, "/note.txt", "/dest", CAP);
        assert_eq!(out, Ok("/dest/note.txt".into()));
        assert_eq!(fs.read("/dest/note.txt"), Ok(b"hello".to_vec()));
        assert!(fs.stat("/note.txt").is_some());
    }

    #[test]
    fn copy_beside_the_original_numbers_the_copies() {
        let mut fs = tree();
        assert_eq!(
            copy_item(&mut fs, "/note.txt", "/", CAP),
            Ok("/note (copy).txt".into())
        );
        assert_eq!(
            copy_item(&mut fs, "/note.txt", "/", CAP),
            Ok("/note (copy 2).txt".into())
        );
    }

    #[test]
    fn copy_folder_is_recursive() {
        let mut fs = tree();
        let out = copy_item(&mut fs, "/a", "/dest", CAP);
        assert_eq!(out, Ok("/dest/a".into()));
        assert_eq!(fs.read("/dest/a/sub/two.txt"), Ok(b"22".to_vec()));
        assert_eq!(fs.read("/dest/a/one.txt"), Ok(b"1".to_vec()));
    }

    #[test]
    fn copying_a_folder_into_itself_is_refused_and_writes_nothing() {
        let mut fs = tree();
        let before = fs.paths();
        let err = copy_item(&mut fs, "/a", "/a/sub", CAP);
        assert_eq!(err, Err(OpError::IntoSelf("a".into())));
        assert_eq!(fs.paths(), before);
    }

    #[test]
    fn a_file_over_the_cap_refuses_the_whole_copy_up_front() {
        let mut fs = tree();
        fs.nodes
            .insert("/a/sub/big.bin".into(), Some(alloc::vec![0; 1500]));
        let before = fs.paths();
        let err = copy_item(&mut fs, "/a", "/dest", CAP);
        assert!(matches!(err, Err(OpError::TooLarge { size: 1500, .. })));
        assert_eq!(fs.paths(), before);
    }

    #[test]
    fn a_failing_write_rolls_back_everything_created() {
        let mut fs = tree();
        fs.fail_write_containing = Some("two.txt");
        let before = fs.paths();
        let err = copy_item(&mut fs, "/a", "/dest", CAP);
        assert!(matches!(err, Err(OpError::Fs { .. })));
        assert_eq!(fs.paths(), before);
    }

    #[test]
    fn the_plan_lists_folders_before_their_files() {
        let fs = tree();
        let plan = plan_copy(&fs, "/a", "/dest/a", CAP).unwrap_or_default();
        assert_eq!(plan.steps.first(), Some(&Step::Mkdir("/dest/a".into())));
        assert_eq!(plan.steps.len(), 4);
    }

    #[test]
    fn move_renames_and_removes_the_source() {
        let mut fs = tree();
        assert_eq!(
            move_item(&mut fs, "/note.txt", "/dest", CAP),
            Ok("/dest/note.txt".into())
        );
        assert!(fs.stat("/note.txt").is_none());
        assert!(fs.stat("/dest/note.txt").is_some());
    }

    #[test]
    fn move_into_the_same_folder_does_nothing() {
        let mut fs = tree();
        assert_eq!(
            move_item(&mut fs, "/note.txt", "/", CAP),
            Ok("/note.txt".into())
        );
        assert!(fs.stat("/note.txt").is_some());
    }

    #[test]
    fn moving_a_folder_into_itself_is_refused() {
        let mut fs = tree();
        let before = fs.paths();
        assert_eq!(
            move_item(&mut fs, "/a", "/a/sub", CAP),
            Err(OpError::IntoSelf("a".into()))
        );
        assert_eq!(fs.paths(), before);
    }

    #[test]
    fn move_falls_back_to_copy_and_delete() {
        let mut fs = tree();
        fs.no_rename = true;
        assert_eq!(move_item(&mut fs, "/a", "/dest", CAP), Ok("/dest/a".into()));
        assert!(fs.stat("/a").is_none());
        assert_eq!(fs.read("/dest/a/sub/two.txt"), Ok(b"22".to_vec()));
    }

    #[test]
    fn moving_out_of_the_share_is_refused_before_copying() {
        let mut fs = MemFs::with(&[
            ("/share", None),
            ("/share/a.txt", Some("x")),
            ("/dest", None),
        ]);
        let err = move_item(&mut fs, "/share/a.txt", "/dest", CAP);
        assert!(matches!(err, Err(OpError::Fs { .. })));
        assert!(fs.stat("/dest/a.txt").is_none());
    }

    #[test]
    fn share_errors_explain_what_is_unsupported() {
        let err = OpError::Fs {
            path: "/share/x".into(),
            err: VfsError::ReadOnly,
        };
        assert!(err.text().starts_with("The shared folder does not support"));
        assert!(is_share("/share") && is_share("/share/a") && !is_share("/shared"));
    }

    #[test]
    fn move_to_path_moves_to_the_exact_name() {
        let mut fs = tree();
        assert_eq!(
            move_to_path(&mut fs, "/note.txt", "/dest/renamed.txt", CAP),
            Ok(())
        );
        assert!(fs.stat("/dest/renamed.txt").is_some() && fs.stat("/note.txt").is_none());
    }

    #[test]
    fn remove_tree_deletes_nested_contents() {
        let mut fs = tree();
        assert_eq!(remove_tree(&mut fs, "/a"), Ok(()));
        assert!(fs.paths().iter().all(|p| !p.starts_with("/a")));
        assert!(fs.stat("/note.txt").is_some());
    }

    #[test]
    fn removing_a_missing_item_reports_it() {
        let mut fs = tree();
        assert_eq!(
            remove_tree(&mut fs, "/gone"),
            Err(OpError::Missing("gone".into()))
        );
    }
}
