//! GUI file-manager keyboard router
//!
//! When the File Manager window is focused:
//!   Up/Down   move selection
//!   Enter     open directory (or preview file info toast)
//!   Backspace cd up
//!   Delete    delete the selected entry (with toast; files only)
//!   F5        refresh listing
//!   r         rename selected → "ren-<name>" quick rename (no IME yet);
//!             the shell `fm rename <newname>` does arbitrary renames.

use crate::wm::window;

/// Whether the File Manager window owns keyboard input.
pub fn fm_focused() -> bool {
    let fid = window::focused_id();
    fid != 0 && window::get_title(fid) == "File Manager"
}

/// Handle a keypress while the FM is focused. Returns true when consumed.
pub fn handle_key(scancode: u8) -> bool {
    if !fm_focused() {
        return false;
    }

    let fm = crate::apps::file_manager::get_file_manager();

    match scancode {
        0x52 => {
            // Up
            fm.select_up();
            true
        }
        0x51 => {
            // Down
            fm.select_down();
            true
        }
        0x28 => {
            // Enter: open dir / info toast for files
            if let Some(entry) = fm.get_selected() {
                if entry.is_dir {
                    fm.open_selected();
                } else {
                    let name = entry.name.clone();
                    crate::wm::notifications::info(
                        "File",
                        &alloc::format!("{} ({} bytes) — 'fm' shell for ops", name, entry.size),
                    );
                }
            }
            true
        }
        0x2A => {
            // Backspace: cd up
            fm.cd_up();
            true
        }
        0x2C => {
            // Delete: remove selected (persisted via ext2 write-through)
            if let Some(entry) = fm.get_selected() {
                if entry.is_dir {
                    crate::wm::notifications::warning(
                        "Delete",
                        "Directories must be removed from the shell",
                    );
                } else {
                    let name = entry.name.clone();
                    match fm.delete_selected() {
                        Ok(()) => crate::wm::notifications::success(
                            "Deleted",
                            &alloc::format!("{} removed from disk", name),
                        ),
                        Err(e) => crate::wm::notifications::error("Delete failed", e),
                    }
                }
            }
            true
        }
        0x3E => {
            // F5: refresh
            fm.refresh();
            crate::wm::notifications::info("Files", "Listing refreshed");
            true
        }
        0x15 => {
            // 'r': quick rename to ren-<name>
            if let Some(entry) = fm.get_selected() {
                let old = entry.name.clone();
                let new = alloc::format!("ren-{}", old);
                match fm.rename_selected(&new) {
                    Ok(()) => crate::wm::notifications::success(
                        "Renamed",
                        &alloc::format!("{} → {}", old, new),
                    ),
                    Err(e) => crate::wm::notifications::error("Rename failed", e),
                }
            }
            true
        }
        _ => false,
    }
}
