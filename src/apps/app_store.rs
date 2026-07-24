//! App Store — graphical package manager for LobsterOS
//!
//! Browse, search, install, and remove .lobpkg packages.

use crate::display::fb;
use crate::display::graphics;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const APP_NAME: &str = "App Store";

/// Package entry
#[derive(Clone)]
struct PackageEntry {
    name: String,
    version: String,
    description: String,
    size: u64,
    installed: bool,
    category: PackageCategory,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum PackageCategory {
    System,
    Utils,
    Network,
    Media,
    Development,
    Games,
}

impl PackageCategory {
    fn as_str(&self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Utils => "Utils",
            Self::Network => "Network",
            Self::Media => "Media",
            Self::Development => "Dev",
            Self::Games => "Games",
        }
    }
}

/// App store state
pub struct AppStore {
    packages: Vec<PackageEntry>,
    selected_idx: usize,
    scroll_offset: usize,
    search_query: String,
    active_category: Option<PackageCategory>,
    running: bool,
    installing: bool,
    install_progress: u32,
}

impl AppStore {
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            selected_idx: 0,
            scroll_offset: 0,
            search_query: String::new(),
            active_category: None,
            running: true,
            installing: false,
            install_progress: 0,
        }
    }

    /// Populate with default package list
    pub fn load_packages(&mut self) {
        self.packages.clear();
        self.packages.push(PackageEntry {
            name: String::from("coreutils"),
            version: String::from("1.0.0"),
            description: String::from("Basic system utilities"),
            size: 256_000,
            installed: true,
            category: PackageCategory::System,
        });
        self.packages.push(PackageEntry {
            name: String::from("vim"),
            version: String::from("9.1.0"),
            description: String::from("Vi IMproved text editor"),
            size: 1_800_000,
            installed: false,
            category: PackageCategory::Utils,
        });
        self.packages.push(PackageEntry {
            name: String::from("htop"),
            version: String::from("3.3.0"),
            description: String::from("Interactive process viewer"),
            size: 120_000,
            installed: false,
            category: PackageCategory::System,
        });
        self.packages.push(PackageEntry {
            name: String::from("curl"),
            version: String::from("8.8.0"),
            description: String::from("Command-line HTTP client"),
            size: 340_000,
            installed: true,
            category: PackageCategory::Network,
        });
        self.packages.push(PackageEntry {
            name: String::from("git"),
            version: String::from("2.45.0"),
            description: String::from("Distributed version control"),
            size: 5_200_000,
            installed: false,
            category: PackageCategory::Development,
        });
        self.packages.push(PackageEntry {
            name: String::from("ffmpeg"),
            version: String::from("7.0.0"),
            description: String::from("Multimedia framework"),
            size: 12_000_000,
            installed: false,
            category: PackageCategory::Media,
        });
        self.packages.push(PackageEntry {
            name: String::from("lobster-shell"),
            version: String::from("2.0.0"),
            description: String::from("Enhanced shell with themes"),
            size: 450_000,
            installed: true,
            category: PackageCategory::Utils,
        });
        self.packages.push(PackageEntry {
            name: String::from("pizza"),
            version: String::from("1.0.0"),
            description: String::from("Pi 5 temperature monitor"),
            size: 80_000,
            installed: false,
            category: PackageCategory::System,
        });
    }

    fn filtered_packages(&self) -> Vec<usize> {
        let mut indices = Vec::new();
        for (i, pkg) in self.packages.iter().enumerate() {
            let matches_category = self.active_category.map_or(true, |cat| pkg.category == cat);
            let matches_search = self.search_query.is_empty()
                || pkg.name.contains(&self.search_query)
                || pkg.description.contains(&self.search_query);
            if matches_category && matches_search {
                indices.push(i);
            }
        }
        indices
    }

    pub fn select_up(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn select_down(&mut self) {
        self.selected_idx += 1;
    }

    pub fn install_selected(&mut self) {
        let filtered = self.filtered_packages();
        if let Some(&idx) = filtered.get(self.selected_idx) {
            self.installing = true;
            self.install_progress = 0;
            self.packages[idx].installed = true;
            crate::println!("[app_store] Installed package: {}", self.packages[idx].name);
            self.installing = false;
        }
    }

    pub fn remove_selected(&mut self) {
        let filtered = self.filtered_packages();
        if let Some(&idx) = filtered.get(self.selected_idx) {
            self.packages[idx].installed = false;
            crate::println!("[app_store] Removed package: {}", self.packages[idx].name);
        }
    }

    pub fn set_category(&mut self, cat: Option<PackageCategory>) {
        self.active_category = cat;
        self.selected_idx = 0;
    }

    fn format_size(size: u64) -> String {
        if size >= 1_000_000 {
            format!("{:.1} MB", size as f64 / 1_000_000.0)
        } else {
            format!("{} KB", size / 1024)
        }
    }

    /// Render the app store
    pub fn render(&self, x: u32, y: u32, width: u32, height: u32) {
        let info = fb::get_info();

        // Title bar
        graphics::draw_rect(&info, x, y, width, 28, 0x2D2D30);
        graphics::draw_text(&info, x + 8, y + 6, APP_NAME, 0xFFFFFF, 0x2D2D30);

        // Category bar
        let cat_y = y + 30;
        graphics::draw_rect(&info, x, cat_y, width, 24, 0x1A1A2A);
        let mut cat_x = x + 8;
        graphics::draw_text(&info, cat_x, cat_y + 4, "All", 0x4A9EFF, 0x1A1A2A);
        cat_x += 40;
        for cat in [
            PackageCategory::System,
            PackageCategory::Utils,
            PackageCategory::Network,
            PackageCategory::Media,
            PackageCategory::Development,
            PackageCategory::Games,
        ] {
            let color = if self.active_category == Some(cat) {
                0x4A9EFF
            } else {
                0x666688
            };
            graphics::draw_text(&info, cat_x, cat_y + 4, cat.as_str(), color, 0x1A1A2A);
            cat_x += match cat {
                PackageCategory::System => 60,
                PackageCategory::Utils => 50,
                PackageCategory::Network => 60,
                PackageCategory::Media => 50,
                PackageCategory::Development => 50,
                PackageCategory::Games => 50,
            };
        }

        // Package list
        let filtered = self.filtered_packages();
        let list_y = cat_y + 26;
        let row_h = 28u32;
        let desc_y = y + height - 80;

        for (display_idx, &pkg_idx) in filtered.iter().enumerate() {
            let ry = list_y + display_idx as u32 * row_h;
            if ry + row_h > desc_y {
                break;
            }

            let pkg = &self.packages[pkg_idx];
            let is_selected = display_idx == self.selected_idx;
            let bg = if is_selected {
                0x3A4A6A
            } else if display_idx % 2 == 0 {
                0x1E1E2E
            } else {
                0x1A1A2A
            };
            graphics::draw_rect(&info, x, ry, width, row_h, bg);

            // Package name
            graphics::draw_text(&info, x + 8, ry + 6, &pkg.name, 0xCCCCCC, bg);

            // Version
            graphics::draw_text(&info, x + 160, ry + 6, &pkg.version, 0x666688, bg);

            // Description (truncated)
            let max_desc = width.saturating_sub(280) as usize / 6;
            let desc = if pkg.description.len() > max_desc {
                &pkg.description[..max_desc]
            } else {
                &pkg.description
            };
            graphics::draw_text(&info, x + 220, ry + 6, desc, 0x8888AA, bg);

            // Size
            let size_str = Self::format_size(pkg.size);
            graphics::draw_text(&info, x + width - 120, ry + 6, &size_str, 0x666688, bg);

            // Status
            let status = if pkg.installed {
                "Installed"
            } else {
                "Available"
            };
            let status_color = if pkg.installed { 0x4EC9B0 } else { 0xFFBD2E };
            graphics::draw_text(&info, x + width - 50, ry + 6, status, status_color, bg);
        }

        // Detail section at bottom
        graphics::draw_rect(&info, x, desc_y, width, 80, 0x1A1A2A);

        if let Some(&idx) = filtered.get(self.selected_idx) {
            let pkg = &self.packages[idx];
            graphics::draw_text(&info, x + 8, desc_y + 6, &pkg.name, 0xCCCCCC, 0x1A1A2A);
            graphics::draw_text(
                &info,
                x + 8,
                desc_y + 26,
                &pkg.description,
                0x8888AA,
                0x1A1A2A,
            );
            let info_str = format!(
                "Version: {} | Size: {}",
                pkg.version,
                Self::format_size(pkg.size)
            );
            graphics::draw_text(&info, x + 8, desc_y + 46, &info_str, 0x666688, 0x1A1A2A);
            let action = if pkg.installed {
                "[Remove]"
            } else {
                "[Install]"
            };
            graphics::draw_text(
                &info,
                x + width - 80,
                desc_y + 46,
                action,
                0x4A9EFF,
                0x1A1A2A,
            );
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn close(&mut self) {
        self.running = false;
    }
}

impl Default for AppStore {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init() {
    crate::println!("[apps] App Store initialized");
}
