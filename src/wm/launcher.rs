//! Application Launcher — reads .desktop files and launches applications
//!
use crate::fs::vfs::{list_dir, read_file};
use crate::println;
use crate::sched::{spawn_from_path, Priority};
use crate::wm::widget::{WidgetId, WidgetTable};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ─── Desktop Entry Parsing ───────────────────────────────────────────────

/// Parsed .desktop file entry
#[derive(Debug, Clone)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub comment: String,
    pub categories: Vec<String>,
    pub terminal: bool,
    pub no_display: bool,
    pub hidden: bool,
}

/// Parse a .desktop file content
pub fn parse_desktop_file(content: &str) -> Option<DesktopEntry> {
    let mut entry = DesktopEntry {
        name: String::new(),
        exec: String::new(),
        icon: String::new(),
        comment: String::new(),
        categories: Vec::new(),
        terminal: false,
        no_display: false,
        hidden: false,
    };

    let mut in_desktop_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line == "[Desktop Entry]" {
            in_desktop_entry = true;
            continue;
        }
        if line.starts_with('[') && line != "[Desktop Entry]" {
            in_desktop_entry = false;
            continue;
        }
        if !in_desktop_entry {
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();

            match key {
                "Name" => entry.name = value.to_string(),
                "Exec" => entry.exec = value.to_string(),
                "Icon" => entry.icon = value.to_string(),
                "Comment" => entry.comment = value.to_string(),
                "Categories" => {
                    entry.categories = value
                        .split(';')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "Terminal" => entry.terminal = value == "true",
                "NoDisplay" => entry.no_display = value == "true",
                "Hidden" => entry.hidden = value == "true",
                _ => {}
            }
        }
    }

    if entry.name.is_empty() || entry.exec.is_empty() {
        None
    } else {
        Some(entry)
    }
}

// ─── Application Database ────────────────────────────────────────────────

/// Manages the application database from .desktop files
pub struct AppLauncher {
    apps: BTreeMap<String, DesktopEntry>,          // name -> entry
    category_index: BTreeMap<String, Vec<String>>, // category -> app names
}

impl AppLauncher {
    pub fn new() -> Self {
        Self {
            apps: BTreeMap::new(),
            category_index: BTreeMap::new(),
        }
    }

    /// Scan for .desktop files in standard locations
    pub fn scan_applications(&mut self) {
        let paths = [
            "/usr/share/applications",
            "/usr/local/share/applications",
            "/home/.local/share/applications",
        ];

        for path in paths {
            if let Ok(entries) = list_dir(path) {
                for entry in entries {
                    if entry.name_str().ends_with(".desktop") {
                        let full_path = alloc::format!("{}/{}", path, entry.name_str());
                        if let Ok(content) = read_file(&full_path) {
                            let content_str = core::str::from_utf8(&content).unwrap_or("");
                            if let Some(entry) = parse_desktop_file(content_str) {
                                if !entry.hidden && !entry.no_display {
                                    let name = entry.name.clone();
                                    self.category_index
                                        .entry("All".to_string())
                                        .or_default()
                                        .push(name.clone());
                                    for cat in &entry.categories {
                                        self.category_index
                                            .entry(cat.clone())
                                            .or_default()
                                            .push(name.clone());
                                    }
                                    self.apps.insert(name, entry);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Get all applications
    pub fn get_all_apps(&self) -> Vec<&DesktopEntry> {
        self.apps.values().collect()
    }

    /// Get applications in a category
    pub fn get_apps_in_category(&self, category: &str) -> Vec<&DesktopEntry> {
        self.category_index
            .get(category)
            .map(|names| names.iter().filter_map(|n| self.apps.get(n)).collect())
            .unwrap_or_default()
    }

    /// Get all categories
    pub fn get_categories(&self) -> Vec<String> {
        self.category_index.keys().cloned().collect()
    }

    /// Launch an application by name
    pub fn launch(&self, name: &str) -> Result<(), &'static str> {
        let entry = self.apps.get(name).ok_or("Application not found")?;

        // Parse exec command (handle %f, %u, etc. placeholders)
        let cmd = entry
            .exec
            .replace("%f", "")
            .replace("%u", "")
            .replace("%F", "")
            .replace("%U", "");
        let cmd = cmd.trim();

        if cmd.is_empty() {
            return Err("Empty exec command");
        }

        // Extract executable path (first word before space)
        let exec_path = cmd
            .split_whitespace()
            .next()
            .ok_or("No executable in command")?;

        // Spawn the process via scheduler
        match spawn_from_path(exec_path, Priority::Normal) {
            Some(pid) => {
                println!(
                    "[launcher] Launched {} (PID {}) from {}",
                    name, pid, exec_path
                );
                Ok(())
            }
            None => {
                println!("[launcher] Failed to launch {} from {}", name, exec_path);
                Err("Failed to spawn process")
            }
        }
    }
}

impl Default for AppLauncher {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Launcher UI Integration ─────────────────────────────────────────────

/// Build a launcher panel widget for the application menu
pub fn build_launcher_panel(
    registry: &mut WidgetTable,
    _launcher: &AppLauncher,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Option<WidgetId> {
    let panel_id = registry.add_panel(x, y, w, h, "Applications")?;

    let spacing = 4;
    let btn_height = 28;
    let mut y_offset = 30; // Below title

    // Add "All" category button
    let _ = registry.add_button(
        spacing,
        y_offset,
        w - 2 * spacing,
        btn_height,
        "All Applications",
    );
    y_offset += btn_height + 4;

    // Add category buttons (placeholder - would need integration with actual launcher)
    // y_offset += btn_height + 4; // Would be used for next button position
    let _ = registry.add_button(
        spacing,
        y_offset,
        w - 2 * spacing,
        btn_height,
        "Applications",
    );

    Some(panel_id)
}

/// Launch an application from the launcher
pub fn launch_app(launcher: &AppLauncher, name: &str) -> Result<(), &'static str> {
    launcher.launch(name)
}

#[cfg(test)]
mod tests {
    use super::{parse_desktop_file, AppLauncher, DesktopEntry};

    #[test]
    fn test_parse_desktop_file_basic() {
        let content = r#"
[Desktop Entry]
Name=Test App
Exec=/usr/bin/test-app
Comment=A test application
Categories=Utility;
Terminal=false
"#;
        let entry = parse_desktop_file(content).unwrap();
        assert_eq!(entry.name, "Test App");
        assert_eq!(entry.exec, "/usr/bin/test-app");
        assert_eq!(entry.comment, "A test application");
        assert_eq!(entry.categories, vec!["Utility"]);
        assert!(!entry.terminal);
        assert!(!entry.hidden);
        assert!(!entry.no_display);
    }

    #[test]
    fn test_parse_desktop_file_with_placeholders() {
        let content = r#"
[Desktop Entry]
Name=File Manager
Exec=file-manager %f %u
Icon=folder
Categories=System;FileManager;
Terminal=true
"#;
        let entry = parse_desktop_file(content).unwrap();
        assert_eq!(entry.name, "File Manager");
        assert_eq!(entry.exec, "file-manager %f %u");
        assert_eq!(entry.icon, "folder");
        assert!(entry.categories.contains(&"System".to_string()));
        assert!(entry.categories.contains(&"FileManager".to_string()));
        assert!(entry.terminal);
    }

    #[test]
    fn test_parse_desktop_file_hidden() {
        let content = r#"
[Desktop Entry]
Name=Hidden App
Exec=hidden-app
Hidden=true
"#;
        let entry = parse_desktop_file(content).unwrap();
        assert!(entry.hidden);
    }

    #[test]
    fn test_parse_desktop_file_no_display() {
        let content = r#"
[Desktop Entry]
Name=No Display App
Exec=no-display-app
NoDisplay=true
"#;
        let entry = parse_desktop_file(content).unwrap();
        assert!(entry.no_display);
    }

    #[test]
    fn test_parse_desktop_file_invalid_missing_name() {
        let content = r#"
[Desktop Entry]
Exec=app
"#;
        assert!(parse_desktop_file(content).is_none());
    }

    #[test]
    fn test_parse_desktop_file_invalid_missing_exec() {
        let content = r#"
[Desktop Entry]
Name=App
"#;
        assert!(parse_desktop_file(content).is_none());
    }

    #[test]
    fn test_app_launcher_new() {
        let launcher = AppLauncher::new();
        assert!(launcher.get_all_apps().is_empty());
        assert!(launcher.get_categories().is_empty());
    }
}
