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

/// Directories searched for `.desktop` files, in priority order.
pub const APPLICATION_DIRS: [&str; 3] = [
    "/usr/share/applications",
    "/usr/local/share/applications",
    "/home/.local/share/applications",
];

/// Placeholders in an `Exec=` line that stand for files or URLs the menu
/// never passes.
const EXEC_PLACEHOLDERS: [&str; 4] = ["%f", "%u", "%F", "%U"];

/// Directories packages install runnable programs into. `pkg install hello`
/// puts its EL0 GUI app in `/apps`; classic tools go to a `bin` directory.
const PROGRAM_DIRS: [&str; 4] = ["/apps/", "/bin/", "/usr/bin/", "/usr/local/bin/"];

/// Whether `path` is a file inside one of [`PROGRAM_DIRS`].
pub fn is_program_path(path: &str) -> bool {
    PROGRAM_DIRS.iter().any(|dir| path.starts_with(dir))
}

/// Program path of an `Exec=` value: first word once placeholders are gone.
pub fn exec_program(exec: &str) -> Option<String> {
    let mut cleaned = exec.to_string();
    for placeholder in EXEC_PLACEHOLDERS {
        cleaned = cleaned.replace(placeholder, "");
    }
    cleaned.split_whitespace().next().map(ToString::to_string)
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
        for path in APPLICATION_DIRS {
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

        let exec_path = exec_program(&entry.exec).ok_or("Empty exec command")?;

        // Spawn the process via scheduler
        match spawn_from_path(&exec_path, Priority::Normal) {
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

// ─── Installed applications (start menu) ─────────────────────────────────

/// An application provided by an installed package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    /// Label shown in the menu.
    pub name: String,
    /// Program to spawn (absolute path).
    pub exec: String,
    /// Extra search words (description, comment).
    pub keywords: String,
}

/// What the menu needs to know about one installed package: its name,
/// description and the executables it put into the binary directory.
pub struct PackageBins {
    pub name: String,
    pub description: String,
    pub binaries: Vec<String>,
}

/// Merge `.desktop` entries and package executables into one app list.
///
/// A package that ships a `.desktop` file is listed once, under the
/// desktop entry's name. A package without one is listed under its own name
/// with its first executable, so `pkg install` makes something visible.
pub fn collect_installed_apps(
    desktop_files: &[String],
    packages: &[PackageBins],
) -> Vec<InstalledApp> {
    let mut apps: Vec<InstalledApp> = desktop_files
        .iter()
        .filter_map(|content| parse_desktop_file(content))
        .filter(|entry| !entry.hidden && !entry.no_display)
        .filter_map(|entry| {
            let exec = exec_program(&entry.exec)?;
            Some(InstalledApp {
                name: entry.name,
                exec,
                keywords: entry.comment,
            })
        })
        .collect();
    for package in packages {
        let Some(binary) = package.binaries.first() else {
            continue;
        };
        if apps.iter().any(|app| package.binaries.contains(&app.exec)) {
            continue;
        }
        apps.push(InstalledApp {
            name: package.name.clone(),
            exec: binary.clone(),
            keywords: package.description.clone(),
        });
    }
    apps
}

/// Installed applications on this system: `.desktop` files in the standard
/// directories plus executables of packages in the package database.
pub fn scan_installed_apps() -> Vec<InstalledApp> {
    let mut desktop_files = Vec::new();
    for dir in APPLICATION_DIRS {
        let Ok(entries) = list_dir(dir) else { continue };
        for entry in entries
            .iter()
            .filter(|e| e.name_str().ends_with(".desktop"))
        {
            let path = alloc::format!("{}/{}", dir, entry.name_str());
            if let Ok(bytes) = read_file(&path) {
                desktop_files.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }
    collect_installed_apps(&desktop_files, &installed_package_bins())
}

fn installed_package_bins() -> Vec<PackageBins> {
    let Ok(db) = crate::pkg::install::PackageDatabase::load() else {
        return Vec::new();
    };
    db.list_installed()
        .into_iter()
        .map(|installed| PackageBins {
            name: installed.metadata.name.clone(),
            description: installed.metadata.description.clone(),
            binaries: installed
                .files
                .iter()
                .filter(|file| is_program_path(&file.path) && !file.is_symlink)
                .map(|file| file.path.clone())
                .collect(),
        })
        .collect()
}

/// Start an installed app the way `run <path> &` does: spawn it as a
/// background user process so the desktop stays responsive.
pub fn launch_installed(app: &InstalledApp) -> Result<(), &'static str> {
    let pid = crate::sched::spawn_from_path_args(&app.exec, Priority::Normal, &[])
        .ok_or("Failed to spawn process")?;
    crate::sched::mark_bg(pid);
    println!(
        "[launcher] Launched {} (PID {}) from {}",
        app.name, pid, app.exec
    );
    Ok(())
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

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::{
        collect_installed_apps, exec_program, is_program_path, parse_desktop_file, AppLauncher,
        DesktopEntry, PackageBins,
    };

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

    fn package(name: &str, binaries: &[&str]) -> PackageBins {
        PackageBins {
            name: name.to_string(),
            description: format!("{name} tool"),
            binaries: binaries.iter().map(|b| b.to_string()).collect(),
        }
    }

    #[test]
    fn installed_app_from_desktop_sample() {
        let sample =
            "[Desktop Entry]\nName=Hello World\nExec=/usr/bin/hello %f\nComment=Greets you\n";
        let apps = collect_installed_apps(&[sample.to_string()], &[]);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Hello World");
        assert_eq!(apps[0].exec, "/usr/bin/hello");
        assert_eq!(apps[0].keywords, "Greets you");
    }

    #[test]
    fn hidden_desktop_entries_are_not_listed() {
        let hidden = "[Desktop Entry]\nName=H\nExec=h\nNoDisplay=true\n";
        assert!(collect_installed_apps(&[hidden.to_string()], &[]).is_empty());
    }

    #[test]
    fn package_without_desktop_file_uses_its_binary() {
        let apps = collect_installed_apps(&[], &[package("hello", &["/usr/bin/hello"])]);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "hello");
        assert_eq!(apps[0].exec, "/usr/bin/hello");
    }

    #[test]
    fn package_with_desktop_file_is_listed_once() {
        let sample = "[Desktop Entry]\nName=Hello World\nExec=/usr/bin/hello\n";
        let apps = collect_installed_apps(
            &[sample.to_string()],
            &[
                package("hello", &["/usr/bin/hello"]),
                package("data-only", &[]),
            ],
        );
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Hello World");
    }

    #[test]
    fn program_paths_cover_apps_and_bin_directories() {
        assert!(is_program_path("/apps/hello"));
        assert!(is_program_path("/usr/bin/tool"));
        assert!(!is_program_path("/etc/tool.conf"));
        assert!(!is_program_path("/usr/share/doc/readme"));
    }

    #[test]
    fn exec_program_strips_placeholders() {
        assert_eq!(exec_program("viewer %U").as_deref(), Some("viewer"));
        assert_eq!(exec_program("%f"), None);
    }

    #[test]
    fn test_app_launcher_new() {
        let launcher = AppLauncher::new();
        assert!(launcher.get_all_apps().is_empty());
        assert!(launcher.get_categories().is_empty());
    }
}
