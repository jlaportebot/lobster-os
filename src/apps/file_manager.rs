//! File Manager for LobsterOS

use crate::fs::vfs::list_dir;
use crate::println;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// File Manager state
pub struct FileManager {
    pub current_path: String,
    pub selected_index: usize,
    pub entries: Vec<DirEntry>,
}

/// Directory entry for display
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub uid: u32,
    pub gid: u32,
}

impl FileManager {
    pub fn new() -> Self {
        Self {
            current_path: String::from("/"),
            selected_index: 0,
            entries: Vec::new(),
        }
    }

    /// Refresh the directory listing
    pub fn refresh(&mut self) {
        self.entries.clear();
        self.selected_index = 0;

        match list_dir(&self.current_path) {
            Ok(entries) => {
                for entry in entries {
                    self.entries.push(DirEntry {
                        name: entry.name_str().to_string(),
                        is_dir: entry.is_dir,
                        size: entry.size,
                        uid: entry.uid,
                        gid: entry.gid,
                    });
                }
            }
            Err(e) => {
                println!("[file_manager] Error listing directory: {:?}", e);
            }
        }
    }

    /// Change directory
    pub fn cd(&mut self, path: &str) {
        let new_path = if path.starts_with('/') {
            path.to_string()
        } else {
            if self.current_path == "/" {
                alloc::format!("/{}", path)
            } else {
                alloc::format!("{}/{}", self.current_path, path)
            }
        };

        // Normalize path
        let normalized = normalize_path(&new_path);

        // Check if it's a directory
        match list_dir(&normalized) {
            Ok(_) => {
                self.current_path = normalized;
                self.refresh();
            }
            Err(e) => {
                println!("[file_manager] cd failed: {:?}", e);
            }
        }
    }

    /// Go to parent directory
    pub fn cd_up(&mut self) {
        if self.current_path != "/" {
            let parent = self
                .current_path
                .rsplit_once('/')
                .map(|(parent, _)| {
                    if parent.is_empty() {
                        "/".to_string()
                    } else {
                        parent.to_string()
                    }
                })
                .unwrap_or("/".to_string());
            self.current_path = parent;
            self.refresh();
        }
    }

    /// Get selected entry
    pub fn get_selected(&self) -> Option<&DirEntry> {
        self.entries.get(self.selected_index)
    }

    /// Move selection up
    pub fn select_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Move selection down
    pub fn select_down(&mut self) {
        if self.selected_index + 1 < self.entries.len() {
            self.selected_index += 1;
        }
    }

    /// Open selected entry (file or directory)
    pub fn open_selected(&mut self) {
        let entry_name = if let Some(entry) = self.get_selected() {
            if entry.is_dir {
                Some(entry.name.clone())
            } else {
                println!("[file_manager] Opening file: {}", entry.name);
                None
            }
        } else {
            None
        };

        if let Some(name) = entry_name {
            self.cd(&name);
        }
    }

    /// Get current path
    pub fn current_path(&self) -> &str {
        &self.current_path
    }

    /// Get entry count
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Create a new file
    pub fn create_file(&mut self, name: &str) -> Result<(), &'static str> {
        use crate::fs::vfs::{create, open, write};
        let path = if self.current_path == "/" {
            alloc::format!("/{}", name)
        } else {
            alloc::format!("{}/{}", self.current_path, name)
        };
        let _ = create(&path);
        let fd = match open(&path, 1) {
            // O_WRONLY | O_CREAT
            Ok(fd) => fd,
            Err(_) => return Err("Failed to create file"),
        };
        let _ = write(fd, &[]);
        self.refresh();
        Ok(())
    }

    /// Create a new directory
    pub fn create_dir(&mut self, name: &str) -> Result<(), &'static str> {
        use crate::fs::vfs::create_dir;
        let path = if self.current_path == "/" {
            alloc::format!("/{}", name)
        } else {
            alloc::format!("{}/{}", self.current_path, name)
        };
        match create_dir(&path) {
            Ok(_) => {
                self.refresh();
                Ok(())
            }
            Err(_) => Err("Failed to create directory"),
        }
    }

    /// Delete selected entry (file or directory)
    pub fn delete_selected(&mut self) -> Result<(), &'static str> {
        use crate::fs::vfs::remove;
        if let Some(entry) = self.get_selected() {
            let path = if self.current_path == "/" {
                alloc::format!("/{}", entry.name)
            } else {
                alloc::format!("{}/{}", self.current_path, entry.name)
            };
            match remove(&path) {
                Ok(_) => {
                    self.refresh();
                    Ok(())
                }
                Err(_) => Err("Failed to delete"),
            }
        } else {
            Err("No entry selected")
        }
    }

    /// Rename selected entry
    pub fn rename_selected(&mut self, new_name: &str) -> Result<(), &'static str> {
        use crate::fs::vfs::rename;
        if let Some(entry) = self.get_selected() {
            let old_path = if self.current_path == "/" {
                alloc::format!("/{}", entry.name)
            } else {
                alloc::format!("{}/{}", self.current_path, entry.name)
            };
            let new_path = if self.current_path == "/" {
                alloc::format!("/{}", new_name)
            } else {
                alloc::format!("{}/{}", self.current_path, new_name)
            };
            match rename(&old_path, &new_path) {
                Ok(_) => {
                    self.refresh();
                    Ok(())
                }
                Err(_) => Err("Failed to rename"),
            }
        } else {
            Err("No entry selected")
        }
    }

    /// Copy selected entry to destination
    pub fn copy_selected(&mut self, dest_path: &str) -> Result<(), &'static str> {
        use crate::fs::vfs::{create, open, read_file, write};
        if let Some(entry) = self.get_selected() {
            let entry_name = &entry.name; // Already a String
            let src_path = if self.current_path == "/" {
                alloc::format!("/{}", entry_name)
            } else {
                alloc::format!("{}/{}", self.current_path, entry_name)
            };
            let dest_path = if dest_path == "/" {
                alloc::format!("/{}", entry_name)
            } else {
                alloc::format!("{}/{}", dest_path, entry_name)
            };

            if entry.is_dir {
                // Create destination directory
                use crate::fs::vfs::create_dir;
                let _ = create_dir(&dest_path);
                // Recursively copy directory contents
                self.copy_dir_recursive(&src_path, &dest_path)?;
            } else {
                // Copy file
                let content = read_file(&src_path).map_err(|_| "Failed to read source file")?;
                let _ = create(&dest_path);
                let fd = match open(&dest_path, 1) {
                    Ok(fd) => fd,
                    Err(_) => return Err("Failed to create destination file"),
                };
                let _ = write(fd, &content);
            }
            Ok(())
        } else {
            Err("No entry selected")
        }
    }

    /// Recursively copy a directory
    fn copy_dir_recursive(&self, src: &str, dest: &str) -> Result<(), &'static str> {
        use crate::fs::vfs::{create, create_dir, list_dir, open, read_file, write};

        let entries = list_dir(src).map_err(|_| "Failed to list source directory")?;
        let _ = create_dir(dest);

        for entry in entries {
            let entry_name = core::str::from_utf8(&entry.name[..entry.name_len]).unwrap_or("");
            let src_path = if src == "/" {
                alloc::format!("/{}", entry_name)
            } else {
                alloc::format!("{}/{}", src, entry_name)
            };
            let dest_path = if dest == "/" {
                alloc::format!("/{}", entry_name)
            } else {
                alloc::format!("{}/{}", dest, entry_name)
            };

            if entry.is_dir {
                self.copy_dir_recursive(&src_path, &dest_path)?;
            } else {
                let content = read_file(&src_path).map_err(|_| "Failed to read source file")?;
                let _ = create(&dest_path);
                let fd = match open(&dest_path, 1) {
                    Ok(fd) => fd,
                    Err(_) => return Err("Failed to create destination file"),
                };
                let _ = write(fd, &content);
            }
        }
        Ok(())
    }

    /// Move selected entry (copy + delete)
    pub fn move_selected(&mut self, dest_path: &str) -> Result<(), &'static str> {
        self.copy_selected(dest_path)?;
        self.delete_selected()
    }

    /// Get MIME type for a file based on extension
    pub fn get_mime_type(&self, filename: &str) -> &'static str {
        let ext = filename
            .rfind('.')
            .map(|i| &filename[i + 1..])
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "txt" | "md" | "log" => "text/plain",
            "rs" => "text/rust",
            "c" | "h" => "text/x-c",
            "cpp" | "cc" | "cxx" => "text/x-c++",
            "py" => "text/x-python",
            "js" => "text/javascript",
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "json" => "application/json",
            "xml" => "application/xml",
            "pdf" => "application/pdf",
            "zip" => "application/zip",
            "tar" => "application/x-tar",
            "gz" => "application/gzip",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "ico" => "image/x-icon",
            "svg" => "image/svg+xml",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "ogg" => "audio/ogg",
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            "sh" => "application/x-shellscript",
            "elf" | "bin" => "application/x-executable",
            _ => "application/octet-stream",
        }
    }
}

impl Default for FileManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize a path (resolve . and ..)
fn normalize_path(path: &str) -> String {
    let mut parts: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => {
                parts.push(component);
            }
        }
    }
    let mut result = alloc::string::String::from("/");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result.push('/');
        }
        result.push_str(part);
    }
    result
}

// ─── Bookmarks ─────────────────────────────────────────────────────────
//
// Quick-access bookmarks for frequently used directories. Stored as a
// static array of (label, path) pairs. Users cycle through them with
// Ctrl+1..9 in the WM input handler.

/// A bookmark entry
#[derive(Clone, Copy)]
pub struct Bookmark {
    pub label: &'static str,
    pub path: &'static str,
}

/// Default bookmarks
pub static DEFAULT_BOOKMARKS: [Bookmark; 6] = [
    Bookmark {
        label: "Home",
        path: "/home",
    },
    Bookmark {
        label: "Root",
        path: "/",
    },
    Bookmark {
        label: "Etc",
        path: "/etc",
    },
    Bookmark {
        label: "Var",
        path: "/var",
    },
    Bookmark {
        label: "Tmp",
        path: "/tmp",
    },
    Bookmark {
        label: "Usr",
        path: "/usr",
    },
];

/// Get the list of bookmarks
pub fn bookmarks() -> &'static [Bookmark] {
    &DEFAULT_BOOKMARKS
}

/// Navigate to a bookmark by index
pub fn goto_bookmark(idx: usize) -> bool {
    if let Some(bm) = DEFAULT_BOOKMARKS.get(idx) {
        get_file_manager().cd(bm.path);
        true
    } else {
        false
    }
}

// ─── File Preview ──────────────────────────────────────────────────────
//
// Generate a text preview of the selected file. For text files, show
// the first 512 bytes. For directories, show entry count. For binaries,
// show file size and type.

/// Preview result
pub struct FilePreview {
    pub lines: Vec<String>,
    pub is_text: bool,
    pub file_size: u64,
    pub mime_type: &'static str,
}

impl FilePreview {
    /// Generate a preview for a file at the given path
    pub fn generate(_path: &str, fm: &FileManager) -> Option<Self> {
        use crate::fs::vfs;

        let entry = fm.get_selected()?;
        let full_path = if fm.current_path == "/" {
            alloc::format!("/{}", entry.name)
        } else {
            alloc::format!("{}/{}", fm.current_path, entry.name)
        };

        if entry.is_dir {
            let count = vfs::list_dir(&full_path).map(|e| e.len()).unwrap_or(0);
            return Some(Self {
                lines: alloc::vec![
                    alloc::format!("Directory: {}", entry.name),
                    alloc::format!("Entries: {}", count),
                    String::from("(press Enter to open)"),
                ],
                is_text: true,
                file_size: 0,
                mime_type: "inode/directory",
            });
        }

        let mime = fm.get_mime_type(&entry.name);
        let is_text = mime.starts_with("text/") || mime == "application/json";

        if is_text {
            match vfs::read_file(&full_path) {
                Ok(data) => {
                    let mut lines = Vec::new();
                    let mut current = String::new();
                    for (idx, &b) in data.iter().enumerate() {
                        if idx >= 512 {
                            lines.push(current);
                            current = String::from("(... truncated)");
                            break;
                        }
                        if b == b'\n' {
                            lines.push(current.clone());
                            current.clear();
                        } else if b >= 0x20 && b <= 0x7E || b == b'\t' {
                            current.push(b as char);
                        }
                    }
                    if !current.is_empty() {
                        lines.push(current);
                    }
                    Some(Self {
                        lines,
                        is_text: true,
                        file_size: entry.size,
                        mime_type: mime,
                    })
                }
                Err(_) => Some(Self {
                    lines: alloc::vec![alloc::format!("Error reading {}", entry.name)],
                    is_text: false,
                    file_size: entry.size,
                    mime_type: mime,
                }),
            }
        } else {
            Some(Self {
                lines: alloc::vec![
                    alloc::format!("File: {}", entry.name),
                    alloc::format!("Size: {} bytes", entry.size),
                    alloc::format!("Type: {}", mime),
                    String::from("(binary file — no preview)"),
                ],
                is_text: false,
                file_size: entry.size,
                mime_type: mime,
            })
        }
    }
}

/// Global file manager instance
static mut FILE_MANAGER: FileManager = FileManager {
    current_path: String::new(),
    selected_index: 0,
    entries: Vec::new(),
};

/// Initialize the file manager
pub fn init() {
    unsafe {
        FILE_MANAGER = FileManager::new();
        FILE_MANAGER.current_path = String::from("/");
        FILE_MANAGER.refresh();
    }
    println!("[file_manager] File manager initialized");
}

/// Get the global file manager instance
pub fn get_file_manager() -> &'static mut FileManager {
    unsafe { &mut FILE_MANAGER }
}

/// Render the file manager UI
#[cfg(feature = "qemu_raspi5b")]
impl FileManager {
    pub fn render(&self, x: u32, y: u32, width: u32, height: u32) {
        let info = crate::display::fb::get_info();

        // Title bar area (already drawn by compositor)
        // Path display
        let path_y = y + 4;
        let path_text = alloc::format!("{}", self.current_path);
        crate::display::graphics::draw_text(&info, x + 8, path_y, &path_text, 0xCCCCCC, 0x1E1E1E);

        // File list
        let list_y = y + 24;
        let row_h = 20;
        let max_rows = height.saturating_sub(30) / row_h;

        for row_idx in 0..max_rows as usize {
            let entry_idx = row_idx;
            if entry_idx >= self.entries.len() {
                break;
            }
            let row_y = list_y + row_idx as u32 * row_h;
            let entry = &self.entries[entry_idx];

            let is_selected = entry_idx == self.selected_index;
            let bg = if is_selected { 0x3A4A6A } else if row_idx % 2 == 0 { 0x1E1E2E } else { 0x1A1A2A };

            crate::display::graphics::draw_rect(&info, x, row_y, width, row_h, bg);

            let icon = if entry.is_dir { "[DIR] " } else { "[FILE] " };
            let name_text = alloc::format!("{}{}", icon, entry.name);
            crate::display::graphics::draw_text(&info, x + 8, row_y + 4, &name_text, 0xCCCCCC, bg);

            if !entry.is_dir {
                let size_text = alloc::format!("{} B", entry.size);
                crate::display::graphics::draw_text(
                    &info,
                    x + width - 80,
                    row_y + 4,
                    &size_text,
                    0x8888AA,
                    bg,
                );
            }
        }

        // Status bar
        let status_y = y + height.saturating_sub(20);
        let status = alloc::format!(
            "{} items | {} selected",
            self.entries.len(),
            self.selected_index
        );
        crate::display::graphics::draw_rect(&info, x, status_y, width, 20, 0x1A1A2A);
        crate::display::graphics::draw_text(&info, x + 8, status_y + 4, &status, 0x666688, 0x1A1A2A);
    }
}

/// Render File Manager to its window (QEMU direct-to-framebuffer mode)
#[cfg(feature = "qemu_raspi5b")]
pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
    if let Some(win_id) = crate::wm::window::find_by_title("File Manager") {
        if let Some(win) = crate::wm::window::get(win_id) {
            if win.active {
                let focused = crate::wm::window::focused_id() == win_id;
                let (cx, cy, cw, ch) = crate::wm::window_frame::draw_frame(
                    fb, win_id, win.x, win.y, win.w, win.h, "File Manager", focused,
                );
                let fm = get_file_manager();
                fm.render(cx, cy, cw, ch);
            }
        }
    }
}
