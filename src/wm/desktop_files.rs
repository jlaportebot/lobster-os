//! Filesystem side of the desktop: the `~/Desktop` folder, the persisted icon
//! layout, file operations behind the icon menus, and opening files.

use crate::fs::vfs::{self, VfsError};
use crate::wm::desktop_layout::{self, Cell, OpenWith};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Folder whose entries appear as desktop icons.
pub const DESKTOP_DIR: &str = "/sd/home/user/Desktop";
/// Parents that must exist before the desktop folder can be created.
const DESKTOP_PARENTS: &[&str] = &["/sd/home", "/sd/home/user"];
/// Per-user configuration folder.
const CONFIG_DIR: &str = "/sd/home/user/.config";
/// Saved icon positions.
const LAYOUT_PATH: &str = "/sd/home/user/.config/desktop-icons.conf";
/// Longest accepted file or folder name.
const MAX_NAME_LEN: usize = 64;

/// One entry of the desktop folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopEntry {
    /// File or folder name.
    pub name: String,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

/// Full path of a desktop entry.
pub fn path_of(name: &str) -> String {
    alloc::format!("{}/{}", DESKTOP_DIR, name)
}

fn make_dir(path: &str) {
    match vfs::mkdir(path) {
        Ok(_) | Err(VfsError::AlreadyExists) => {}
        Err(err) => crate::println!("[desktop] mkdir {} failed: {:?}", path, err),
    }
}

/// List the desktop folder (sorted, dot-files hidden), creating it first
/// when missing. `None` while the filesystem is not usable yet.
pub fn scan() -> Option<Vec<DesktopEntry>> {
    if vfs::list_dir(DESKTOP_DIR).is_err() {
        DESKTOP_PARENTS.iter().for_each(|p| make_dir(p));
        make_dir(DESKTOP_DIR);
    }
    let raw = vfs::list_dir(DESKTOP_DIR).ok()?;
    let mut entries: Vec<DesktopEntry> = raw
        .iter()
        .map(|e| DesktopEntry {
            name: e.name_str().to_string(),
            is_dir: e.is_dir,
        })
        .filter(|e| !e.name.starts_with('.') && e.name != "lost+found")
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Some(entries)
}

/// Read the persisted layout (empty when none was saved).
pub fn load_layout() -> Vec<(String, Cell)> {
    match vfs::read_file(LAYOUT_PATH) {
        Ok(bytes) => desktop_layout::parse(&String::from_utf8_lossy(&bytes)),
        Err(_) => Vec::new(),
    }
}

/// Persist the layout under `~/.config`.
pub fn save_layout(entries: &[(String, Cell)]) {
    make_dir(CONFIG_DIR);
    let text = desktop_layout::serialize(entries);
    if let Err(err) = vfs::write_file(LAYOUT_PATH, text.as_bytes()) {
        crate::println!("[desktop] saving layout failed: {:?}", err);
    }
}

/// Reject names that are empty, too long or would escape the folder.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME_LEN && !name.contains('/') && !name.starts_with('.')
}

/// Create a file or folder on the desktop.
pub fn create(name: &str, is_dir: bool) -> Result<(), VfsError> {
    let path = path_of(name);
    if is_dir {
        vfs::mkdir(&path).map(|_| ())
    } else {
        vfs::create(&path).map(|_| ())
    }
}

/// Rename a desktop entry.
pub fn rename(old: &str, new: &str) -> Result<(), VfsError> {
    vfs::rename(&path_of(old), &path_of(new))
}

/// Delete a desktop entry (folders must be empty).
pub fn delete(name: &str) -> Result<(), VfsError> {
    vfs::remove(&path_of(name))
}

/// Open a desktop entry with the app its type maps to.
pub fn open(name: &str, is_dir: bool) {
    let path = path_of(name);
    match desktop_layout::open_with(name, is_dir) {
        OpenWith::Files => {
            crate::apps::file_manager::get_file_manager().cd(&path);
            crate::wm::start_menu::launch_by_name("Files");
        }
        OpenWith::Images => open_image(&path),
        OpenWith::Editor => open_text(&path),
    }
    crate::wm::compositor::mark_full_redraw();
}

#[cfg(feature = "qemu_raspi5b")]
fn open_image(path: &str) {
    if let Err(err) = crate::apps::image_viewer::open_image(path) {
        crate::wm::notifications::error("Images", err);
    }
}

#[cfg(not(feature = "qemu_raspi5b"))]
fn open_image(path: &str) {
    open_text(path);
}

#[cfg(feature = "qemu_raspi5b")]
fn open_text(path: &str) {
    crate::apps::editor::open_file(path);
}

/// The tabbed editor only exists in the QEMU/GUI build.
#[cfg(not(feature = "qemu_raspi5b"))]
fn open_text(path: &str) {
    crate::println!("[desktop] no editor in this build for {}", path);
}
