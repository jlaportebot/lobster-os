//! User-level operations: new item, rename, copy / cut / paste and delete,
//! plus the dialog plumbing that asks for input and applies the answer.

use super::fsops::{self, Fs, OpError, VfsFs};
use super::paths::{self, NameError};
use super::properties::{self, Details, Summary};
use super::state::{Ask, Clipboard, Pending, PropsPanel, Queued, State};
use super::trash;
use crate::wm::dialog::{self, DialogResult};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Default name of a new folder.
const NEW_FOLDER_NAME: &str = "New Folder";
/// Default stem and extension of a new text file.
const NEW_FILE_STEM: &str = "New File";
const NEW_FILE_EXT: &str = ".txt";
/// Per-file size limit of the VFS.
const FILE_CAP: u64 = crate::fs::vfs::MAX_FILE_DATA as u64;

/// Queue an error or notice dialog.
pub fn alert(st: &mut State, title: &str, message: &str) {
    st.queue.push(Queued {
        ask: Ask::Alert,
        title: title.to_string(),
        message: message.to_string(),
        pending: None,
    });
    crate::wm::dirty::mark();
}

fn ask(st: &mut State, ask: Ask, title: &str, message: &str, pending: Pending) {
    st.queue.push(Queued {
        ask,
        title: title.to_string(),
        message: message.to_string(),
        pending: Some(pending),
    });
    crate::wm::dirty::mark();
}

fn op_alert(st: &mut State, title: &str, err: &OpError) {
    alert(st, title, &err.text());
}

/// Ask for the name of a new folder or text file.
pub fn ask_new(st: &mut State, is_dir: bool) {
    if is_dir && fsops::is_share(st.cwd()) {
        alert(
            st,
            "Shared folder",
            &fsops::share_unsupported("creating folders"),
        );
        return;
    }
    let taken = |n: &str| st.has_name(n);
    let (title, default, pending) = if is_dir {
        let name = paths::unique_default(NEW_FOLDER_NAME, &taken);
        ("New folder", name, Pending::NewFolder)
    } else {
        let stem = paths::unique_default(NEW_FILE_STEM, &|n| {
            taken(&alloc::format!("{n}{NEW_FILE_EXT}"))
        });
        (
            "New file",
            alloc::format!("{stem}{NEW_FILE_EXT}"),
            Pending::NewFile,
        )
    };
    ask(st, Ask::Prompt(default), title, "Name:", pending);
}

/// Ask for the new name of the single selected item.
pub fn ask_rename(st: &mut State) {
    let selected = st.selected_paths();
    let (true, Some(path)) = (selected.len() == 1, selected.first().cloned()) else {
        return;
    };
    let name = paths::file_name(&path).to_string();
    ask(
        st,
        Ask::Prompt(name),
        "Rename",
        "New name:",
        Pending::Rename(path),
    );
}

/// Ask before permanently deleting the selection.
pub fn ask_delete_permanent(st: &mut State) {
    let paths = st.selected_paths();
    if paths.iter().any(|p| fsops::is_share(p)) {
        alert(
            st,
            "Shared folder",
            &fsops::share_unsupported("deleting items"),
        );
        return;
    }
    let message = match paths.as_slice() {
        [] => return,
        [one] => alloc::format!("Delete \"{}\" permanently?", paths::file_name(one)),
        many => alloc::format!("Delete {} items permanently?", many.len()),
    };
    ask(st, Ask::Confirm, "Delete", &message, Pending::Delete(paths));
}

/// Remember the selection for a later paste.
pub fn copy_selection(st: &mut State, cut: bool) {
    let paths = st.selected_paths();
    if paths.is_empty() {
        return;
    }
    st.clipboard = Some(Clipboard { paths, cut });
    crate::wm::dirty::mark();
}

/// Paste the clipboard into the current folder and select what arrived.
pub fn paste(st: &mut State) {
    let Some(clip) = st.clipboard.clone() else {
        return;
    };
    let cwd = st.cwd().to_string();
    let mut arrived: Vec<String> = Vec::new();
    for src in &clip.paths {
        let done = if clip.cut {
            fsops::move_item(&mut VfsFs, src, &cwd, FILE_CAP)
        } else {
            fsops::copy_item(&mut VfsFs, src, &cwd, FILE_CAP)
        };
        match done {
            Ok(dest) => arrived.push(paths::file_name(&dest).to_string()),
            Err(err) => {
                op_alert(st, "Paste failed", &err);
                break;
            }
        }
    }
    if clip.cut {
        st.clipboard = None;
    }
    super::nav::load(st);
    super::nav::select_names(st, &arrived);
    crate::wm::dirty::mark();
}

/// Rename `path` to `new_name` after validating the name.
fn rename(st: &mut State, path: &str, new_name: &str) {
    let new_name = new_name.trim();
    if new_name == paths::file_name(path) {
        return;
    }
    match check(st, new_name) {
        Err(why) => alert(st, "Rename", why.text()),
        Ok(()) => {
            let target = paths::join(&paths::parent(path), new_name);
            match VfsFs.rename(path, &target) {
                Ok(()) => finish(st, new_name),
                Err(err) => alert(st, "Rename failed", &alloc::format!("{path}: {err}")),
            }
        }
    }
}

fn check(st: &State, name: &str) -> Result<(), NameError> {
    paths::check_name(name, &|n| st.has_name(n))
}

fn create(st: &mut State, name: &str, is_dir: bool) {
    let name = name.trim();
    if let Err(why) = check(st, name) {
        alert(st, "Cannot create", why.text());
        return;
    }
    let path = paths::join(st.cwd(), name);
    let made = if is_dir {
        VfsFs.mkdir(&path)
    } else {
        VfsFs.write(&path, &[])
    };
    match made {
        Ok(()) => finish(st, name),
        Err(err) => alert(st, "Cannot create", &alloc::format!("{path}: {err}")),
    }
}

fn finish(st: &mut State, select: &str) {
    super::nav::load(st);
    super::nav::select_names(st, &[select.to_string()]);
    crate::wm::dirty::mark();
}

fn delete_all(st: &mut State, targets: &[String]) {
    let dirs = super::nav::trash_dirs();
    let in_trash = super::nav::in_trash(st);
    for path in targets {
        let done = if in_trash {
            trash::purge(&mut VfsFs, &dirs, paths::file_name(path))
        } else {
            fsops::remove_tree(&mut VfsFs, path)
        };
        if let Err(err) = done {
            op_alert(st, "Delete failed", &err);
            break;
        }
    }
    st.selection.clear();
    super::nav::load(st);
    crate::wm::dirty::mark();
}

/// Apply the answer to a finished dialog.
pub fn resolve(st: &mut State, pending: Pending, result: Option<DialogResult>) {
    match (pending, result) {
        (Pending::NewFolder, Some(DialogResult::Prompt(Some(n)))) => create(st, &n, true),
        (Pending::NewFile, Some(DialogResult::Prompt(Some(n)))) => create(st, &n, false),
        (Pending::Rename(path), Some(DialogResult::Prompt(Some(n)))) => rename(st, &path, &n),
        (Pending::Delete(targets), Some(DialogResult::Confirm(true))) => delete_all(st, &targets),
        (Pending::EmptyTrash, Some(DialogResult::Confirm(true))) => empty_trash(st),
        _ => {}
    }
}

/// Per-frame dialog housekeeping: collect the answer of a closed dialog,
/// then open the next queued one once the mouse button is up (the release
/// that follows the opening press would otherwise click into it).
pub fn tick(st: &mut State) {
    if dialog::is_open() {
        return;
    }
    if let Some(pending) = st.awaiting.take() {
        let result = dialog::take_result();
        if let Some(pending) = pending {
            resolve(st, pending, result);
        }
    }
    if crate::wm::input::mouse_buttons().0 || st.queue.is_empty() {
        return;
    }
    let next = st.queue.remove(0);
    match &next.ask {
        Ask::Prompt(default) => dialog::prompt(&next.title, &next.message, default),
        Ask::Confirm => dialog::confirm(&next.title, &next.message),
        Ask::Alert => dialog::alert(&next.title, &next.message),
    }
    st.awaiting = Some(next.pending);
}

/// Local time now as the `DeletionDate` text.
fn trash_time() -> String {
    let up = crate::arch::ArchTimer::uptime_ms();
    let local = crate::clock::epoch_secs_at(up)
        .map_or(0, |s| s as i64 + i64::from(crate::clock::utc_offset_secs()));
    trash::iso_time(local.max(0) as u64)
}

/// Delete key: move the selection to the Trash (inside the Trash itself it
/// asks to delete permanently instead).
pub fn delete_selection(st: &mut State) {
    let targets = st.selected_paths();
    if targets.is_empty() {
        return;
    }
    if super::nav::in_trash(st) {
        ask_delete_permanent(st);
        return;
    }
    if targets.iter().any(|p| fsops::is_share(p)) {
        alert(
            st,
            "Shared folder",
            &fsops::share_unsupported("deleting items"),
        );
        return;
    }
    let dirs = super::nav::trash_dirs();
    let when = trash_time();
    for path in &targets {
        if let Err(err) = trash::trash_item(&mut VfsFs, &dirs, path, &when, FILE_CAP) {
            op_alert(st, "Move to Trash failed", &err);
            break;
        }
    }
    st.selection.clear();
    super::nav::load(st);
    crate::wm::notifications::info(
        "Files",
        &alloc::format!("Moved {} item(s) to Trash", targets.len()),
    );
    crate::wm::dirty::mark();
}

/// Restore the selected trash items to where they came from.
pub fn restore_selection(st: &mut State) {
    if !super::nav::in_trash(st) {
        return;
    }
    let dirs = super::nav::trash_dirs();
    for path in st.selected_paths() {
        let name = paths::file_name(&path).to_string();
        if let Err(err) = trash::restore(&mut VfsFs, &dirs, &name, FILE_CAP) {
            op_alert(st, "Restore failed", &err);
            break;
        }
    }
    st.selection.clear();
    super::nav::load(st);
    crate::wm::dirty::mark();
}

/// Ask before emptying the trash.
pub fn ask_empty_trash(st: &mut State) {
    if st.entries.is_empty() {
        return;
    }
    ask(
        st,
        Ask::Confirm,
        "Empty Trash",
        "Permanently delete everything in the Trash?",
        Pending::EmptyTrash,
    );
}

fn empty_trash(st: &mut State) {
    if let Err(err) = trash::empty(&mut VfsFs, &super::nav::trash_dirs()) {
        op_alert(st, "Empty Trash failed", &err);
    }
    st.selection.clear();
    super::nav::load(st);
    crate::wm::dirty::mark();
}

fn owner_names(uid: u32, gid: u32) -> (String, String) {
    let user =
        crate::users::passwd::find_user_by_uid(uid).map_or_else(|| uid.to_string(), |u| u.username);
    let group =
        crate::users::group::find_group_by_gid(gid).map_or_else(|| gid.to_string(), |g| g.name);
    (user, group)
}

fn details_of(path: &str) -> Option<Details> {
    let entry = VfsFs.stat(path)?;
    let meta = crate::fs::vfs::stat(path);
    let (owner, group) = meta.map_or_else(
        || (String::from("-"), String::from("-")),
        |m| owner_names(m.uid, m.gid),
    );
    let local = i64::from(crate::clock::utc_offset_secs());
    Some(Details {
        name: entry.name.clone(),
        kind: super::model::type_label(&entry),
        path: path.to_string(),
        is_dir: entry.is_dir,
        summary: properties::summarize(&VfsFs, path, properties::WALK_LIMIT),
        modified: (entry.mtime as i64 + local).max(0) as u64 * u64::from(entry.mtime != 0),
        owner,
        group,
        mode: meta.map_or(0, |m| m.permissions),
    })
}

/// Open the Properties panel for the selection.
pub fn show_properties(st: &mut State) {
    let targets = st.selected_paths();
    let rows = match targets.as_slice() {
        [] => return,
        [one] => details_of(one).map(|d| properties::rows(&d)),
        many => {
            let mut total = Summary::default();
            for p in many {
                total.merge(properties::summarize(&VfsFs, p, properties::WALK_LIMIT));
            }
            Some(properties::multi_rows(many.len(), total, st.cwd()))
        }
    };
    if let Some(rows) = rows {
        st.props = Some(PropsPanel {
            title: String::from("Properties"),
            rows,
        });
    }
    crate::wm::dirty::mark();
}

/// Move the selection into folder `dir` (drag and drop).
pub fn move_selection(st: &mut State, dir: &str) {
    let mut moved = 0usize;
    for src in st.selected_paths() {
        match fsops::move_item(&mut VfsFs, &src, dir, FILE_CAP) {
            Ok(_) => moved += 1,
            Err(err) => {
                op_alert(st, "Move failed", &err);
                break;
            }
        }
    }
    st.selection.clear();
    super::nav::load(st);
    if moved > 0 {
        let note = alloc::format!("Moved {moved} item(s) to {}", paths::file_name(dir));
        crate::wm::notifications::info("Files", &note);
    }
    crate::wm::dirty::mark();
}
