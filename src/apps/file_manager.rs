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
