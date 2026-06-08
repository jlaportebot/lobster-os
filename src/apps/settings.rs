//! Settings Application for LobsterOS

use crate::println;
use crate::wm::theme::{self, set as set_theme, ThemeId};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// Settings categories
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsCategory {
    Appearance,
    Network,
    System,
    User,
    About,
}

/// Settings application state
pub struct Settings {
    pub current_category: SettingsCategory,
    pub wallpaper_path: Option<String>,
    pub theme: ThemeId,
    pub volume: u8,
    pub brightness: u8,
    pub auto_login: bool,
    pub username: String,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            current_category: SettingsCategory::Appearance,
            wallpaper_path: None,
            theme: ThemeId::Dark,
            volume: 128,
            brightness: 200,
            auto_login: false,
            username: String::new(),
        }
    }

    /// Load settings from persistent storage
    pub fn load(&mut self) {
        self.theme = crate::wm::theme::active_id();
        println!("[settings] Settings loaded");
    }

    /// Save settings to persistent storage
    pub fn save(&self) {
        use crate::fs::vfs::{create, open, write};
        let _ = create("/etc/settings");
        let fd = match open("/etc/settings", 1) {
            // O_WRONLY
            Ok(fd) => fd,
            Err(_) => return,
        };
        let content = self.to_config_string();
        let _ = write(fd, content.as_bytes());
        println!("[settings] Settings saved");
    }

    /// Parse settings from string
    fn parse_settings(&mut self, content: &str) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(eq_pos) = line.find('=') {
                let key = &line[..eq_pos].trim();
                let value = &line[eq_pos + 1..].trim();
                match *key {
                    "theme" => {
                        if let Ok(id) = value.parse::<u8>() {
                            self.theme = match id {
                                0 => crate::wm::theme::ThemeId::Dark,
                                1 => crate::wm::theme::ThemeId::Light,
                                _ => crate::wm::theme::ThemeId::Dark,
                            };
                        }
                    }
                    "wallpaper" => {
                        self.wallpaper_path = if value.is_empty() {
                            None
                        } else {
                            Some(value.to_string())
                        }
                    }
                    "volume" => {
                        if let Ok(v) = value.parse::<u8>() {
                            self.volume = v
                        }
                    }
                    "brightness" => {
                        if let Ok(b) = value.parse::<u8>() {
                            self.brightness = b
                        }
                    }
                    "auto_login" => {
                        if let Ok(a) = value.parse::<bool>() {
                            self.auto_login = a
                        }
                    }
                    "username" => self.username = value.to_string(),
                    _ => {}
                }
            }
        }
    }

    /// Convert settings to string for saving
    pub fn to_config_string(&self) -> String {
        let mut s = String::new();
        s.push_str("# LobsterOS Settings\n");
        s.push_str(&alloc::format!("theme={}\n", self.theme as u8));
        if let Some(wp) = &self.wallpaper_path {
            s.push_str(&alloc::format!("wallpaper={}\n", wp));
        }
        s.push_str(&alloc::format!("volume={}\n", self.volume));
        s.push_str(&alloc::format!("brightness={}\n", self.brightness));
        s.push_str(&alloc::format!("auto_login={}\n", self.auto_login));
        s.push_str(&alloc::format!("username={}\n", self.username));
        s
    }

    /// Set the current category
    pub fn set_category(&mut self, category: SettingsCategory) {
        self.current_category = category;
    }

    /// Change theme
    pub fn set_theme(&mut self, theme: ThemeId) {
        self.theme = theme;
        set_theme(theme);
        self.save();
    }

    /// Set wallpaper
    pub fn set_wallpaper(&mut self, path: Option<String>) {
        self.wallpaper_path = path.clone();
        if let Some(path) = path {
            crate::wm::compositor::set_wallpaper_image(&path).ok();
        } else {
            crate::wm::compositor::set_wallpaper_color(theme::current().desktop_bg);
        }
        self.save();
    }

    /// Set volume (0-255)
    pub fn set_volume(&mut self, volume: u8) {
        self.volume = volume;
        crate::drivers::audio::set_volume(volume);
        self.save();
    }

    /// Set brightness (0-255)
    pub fn set_brightness(&mut self, brightness: u8) {
        self.brightness = brightness;
        // TODO: Implement brightness control via GPU driver
        self.save();
    }

    /// Toggle auto-login
    pub fn set_auto_login(&mut self, enabled: bool) {
        self.auto_login = enabled;
        self.save();
    }

    /// Set username
    pub fn set_username(&mut self, username: String) {
        self.username = username;
        self.save();
    }

    /// Get current category name
    pub fn category_name(&self) -> &'static str {
        match self.current_category {
            SettingsCategory::Appearance => "Appearance",
            SettingsCategory::Network => "Network",
            SettingsCategory::System => "System",
            SettingsCategory::User => "User",
            SettingsCategory::About => "About",
        }
    }

    /// Get all categories
    pub fn categories() -> Vec<SettingsCategory> {
        vec![
            SettingsCategory::Appearance,
            SettingsCategory::Network,
            SettingsCategory::System,
            SettingsCategory::User,
            SettingsCategory::About,
        ]
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

/// Global settings instance
static mut SETTINGS: Settings = Settings {
    current_category: SettingsCategory::Appearance,
    wallpaper_path: None,
    theme: crate::wm::theme::ThemeId::Dark,
    volume: 128,
    brightness: 200,
    auto_login: false,
    username: String::new(),
};

/// Initialize the settings application
pub fn init() {
    unsafe {
        SETTINGS = Settings::new();
        SETTINGS.load();
    }
    println!("[settings] Settings application initialized");
}

/// Get the global settings instance
pub fn get_settings() -> &'static mut Settings {
    unsafe { &mut SETTINGS }
}
