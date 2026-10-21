//! Settings Application for LobsterOS

#[cfg(not(feature = "qemu_raspi5b"))]
mod settings_impl {
    use crate::println;
    use crate::wm::theme::{self, set as set_theme, ThemeId};
    use alloc::string::{String, ToString};
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
            Vec::from([
                SettingsCategory::Appearance,
                SettingsCategory::Network,
                SettingsCategory::System,
                SettingsCategory::User,
                SettingsCategory::About,
            ])
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
}

#[cfg(not(feature = "qemu_raspi5b"))]
pub use settings_impl::*;

#[cfg(feature = "qemu_raspi5b")]
mod settings_stub {
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Settings categories (stub for qemu)
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum SettingsCategory {
        Appearance,
        Network,
        System,
        User,
        About,
    }

    /// Settings application state (stub for qemu)
    pub struct Settings {
        pub current_category: SettingsCategory,
        pub wallpaper_path: Option<String>,
        pub theme: u8,
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
                theme: 0,
                volume: 128,
                brightness: 200,
                auto_login: false,
                username: String::new(),
            }
        }

        pub fn load(&mut self) {}
        pub fn save(&self) {}
        pub fn set_category(&mut self, category: SettingsCategory) {
            self.current_category = category;
        }
        pub fn set_theme(&mut self, _theme: u8) {}
        pub fn set_wallpaper(&mut self, _path: Option<String>) {}
        pub fn set_volume(&mut self, volume: u8) {
            self.volume = volume;
        }
        pub fn set_brightness(&mut self, brightness: u8) {
            self.brightness = brightness;
        }
        pub fn set_auto_login(&mut self, enabled: bool) {
            self.auto_login = enabled;
        }
        pub fn set_username(&mut self, username: String) {
            self.username = username;
        }
        pub fn category_name(&self) -> &'static str {
            match self.current_category {
                SettingsCategory::Appearance => "Appearance",
                SettingsCategory::Network => "Network",
                SettingsCategory::System => "System",
                SettingsCategory::User => "User",
                SettingsCategory::About => "About",
            }
        }
        pub fn categories() -> Vec<SettingsCategory> {
            Vec::from([
                SettingsCategory::Appearance,
                SettingsCategory::Network,
                SettingsCategory::System,
                SettingsCategory::User,
                SettingsCategory::About,
            ])
        }
    }

    impl Default for Settings {
        fn default() -> Self {
            Self::new()
        }
    }

    pub fn init() {
        crate::println!("[settings] Settings application initialized (qemu stub)");
    }

    pub fn get_settings() -> &'static mut Settings {
        static mut SETTINGS: Settings = Settings {
            current_category: SettingsCategory::Appearance,
            wallpaper_path: None,
            theme: 0,
            volume: 128,
            brightness: 200,
            auto_login: false,
            username: String::new(),
        };
        unsafe { &mut SETTINGS }
    }
}

#[cfg(feature = "qemu_raspi5b")]
pub use settings_stub::*;

// ═══════════════════════════════════════════════════════════════════════
// Settings window (QEMU direct-FB render) — B3.1
// ═══════════════════════════════════════════════════════════════════════

#[cfg(feature = "qemu_raspi5b")]
pub mod settings_ui {
    use crate::display::graphics as g;
    use crate::wm::theme;
    use crate::wm::window;

    #[derive(Clone, Copy, PartialEq)]
    pub enum Page {
        Appearance,
        Display,
        Network,
        Storage,
        Users,
        About,
    }

    pub struct SettingsUi {
        pub page: Page,
    }

    static mut UI: Option<SettingsUi> = None;

    const PAGES: [(&str, Page); 6] = [
        ("Appearance", Page::Appearance),
        ("Display", Page::Display),
        ("Network", Page::Network),
        ("Storage", Page::Storage),
        ("Users", Page::Users),
        ("About", Page::About),
    ];

    pub fn ensure() {
        unsafe {
            if UI.is_none() {
                UI = Some(SettingsUi { page: Page::Appearance });
            }
        }
    }

    pub fn set_page(p: Page) {
        ensure();
        unsafe {
            if let Some(u) = UI.as_mut() {
                u.page = p;
            }
        }
    }

    pub fn next_page() {
        ensure();
        unsafe {
            if let Some(u) = UI.as_mut() {
                u.page = match u.page {
                    Page::Appearance => Page::Display,
                    Page::Display => Page::Network,
                    Page::Network => Page::Storage,
                    Page::Storage => Page::Users,
                    Page::Users => Page::About,
                    Page::About => Page::Appearance,
                };
            }
        }
    }

    /// Open (or focus) the Settings window.
    pub fn open() -> u32 {
        ensure();
        match window::find_by_title("Settings") {
            Some(id) => {
                window::focus(id);
                id
            }
            None => {
                // Pick a position below the boot-time default windows
                let id = window::create(60, 600, 520, 420, "Settings").unwrap_or(0);
                if id != 0 {
                    window::focus(id);
                }
                id
            }
        }
    }

    pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
        let Some(win_id) = window::find_by_title("Settings") else { return };
        let Some((x, y, w, h)) = window::get_info(win_id) else { return };
        let focused = window::focused_id() == win_id;
        let (cx, cy, cw, ch) =
            crate::wm::window_frame::draw_frame(fb, win_id, x, y, w, h, "Settings", focused);
        let theme = theme::current();

        ensure();
        let page = unsafe { UI.as_ref().unwrap().page };

        // Sidebar: page list
        let side_w = 120u32;
        g::draw_rect(fb, cx, cy, side_w, ch, theme.menu_bg);
        let mut py = cy + 8;
        for (name, p) in PAGES {
            let active = p == page;
            let bg = if active { theme.menu_item_hover } else { theme.menu_bg };
            g::fill_rounded_rect(fb, cx + 4, py, side_w - 8, 22, 4, bg);
            g::draw_text(fb, cx + 10, py + 4, name,
                if active { theme.label_text } else { theme.label_text_dim }, bg);
            py += 26;
        }

        // Content area
        let mx = cx + side_w;
        let mw = cw.saturating_sub(side_w);
        g::draw_rect(fb, mx, cy, mw, ch, theme.win_bg);

        match page {
            Page::Appearance => {
                g::draw_text(fb, mx + 12, cy + 10, "Appearance", theme.label_text, theme.win_bg);
                g::draw_rect(fb, mx + 8, cy + 28, mw - 16, 1, theme.menu_border);

                // Theme row
                g::draw_text(fb, mx + 12, cy + 40, "Theme:", theme.label_text, theme.win_bg);
                let tname = theme::current_name();
                g::fill_rounded_rect(fb, mx + 80, cy + 36, 90, 20, 6, theme.btn_bg);
                g::draw_text(fb, mx + 90, cy + 40, tname, theme.btn_text, theme.btn_bg);

                // Wallpaper row
                g::draw_text(fb, mx + 12, cy + 70, "Wallpaper:", theme.label_text, theme.win_bg);
                let wname = crate::wm::wallpaper::active().name;
                g::fill_rounded_rect(fb, mx + 90, cy + 66, 120, 20, 6, theme.btn_bg);
                g::draw_text(fb, mx + 98, cy + 70, wname, theme.btn_text, theme.btn_bg);

                // Wallpaper swatches
                g::draw_text(fb, mx + 12, cy + 100, "Presets:", theme.label_text_dim, theme.win_bg);
                let mut sx = mx + 12;
                let mut sy = cy + 120;
                let presets = crate::wm::wallpaper::PRESETS;
                let active_idx = crate::wm::wallpaper::active_index();
                for (i, preset) in presets.iter().enumerate() {
                    let sel = i == active_idx;
                    g::draw_rect(fb, sx, sy, 36, 24, preset.top);
                    g::draw_rect(fb, sx, sy + 12, 36, 12, preset.bottom);
                    if sel {
                        g::draw_rect_outline(fb, sx - 1, sy - 1, 38, 26, theme.input_border_focused);
                    }
                    sx += 44;
                    if sx + 44 > mx + mw {
                        sx = mx + 12;
                        sy += 32;
                    }
                }
                g::draw_text(fb, mx + 12, cy + 200,
                    "Shell: theme toggle | wallpaper <name/next>", theme.label_text_dim, theme.win_bg);
            }
            Page::Display => {
                g::draw_text(fb, mx + 12, cy + 10, "Display", theme.label_text, theme.win_bg);
                g::draw_rect(fb, mx + 8, cy + 28, mw - 16, 1, theme.menu_border);
                let fbinfo = crate::display::fb::get_info();
                let res = alloc::format!("Resolution: {}x{} @ {}bpp", fbinfo.width, fbinfo.height, fbinfo.bpp);
                g::draw_text(fb, mx + 12, cy + 44, &res, theme.label_text, theme.win_bg);
                let scale = alloc::format!("Framebuffer: {:#x} ({} KB)",
                    fbinfo.virt_addr, fbinfo.size / 1024);
                g::draw_text(fb, mx + 12, cy + 66, &scale, theme.label_text_dim, theme.win_bg);
                g::draw_text(fb, mx + 12, cy + 90, "Display scaling: 100%", theme.label_text_dim, theme.win_bg);
            }
            Page::Network => {
                g::draw_text(fb, mx + 12, cy + 10, "Network", theme.label_text, theme.win_bg);
                g::draw_rect(fb, mx + 8, cy + 28, mw - 16, 1, theme.menu_border);
                #[cfg(target_arch = "aarch64")]
                {
                    let cfg = crate::net::config();
                    let state = if cfg.ip_addr != [0, 0, 0, 0] { "Connected (lobster-net)" } else { "No link" };
                    g::draw_text(fb, mx + 12, cy + 44, state, theme.label_text, theme.win_bg);
                    if cfg.ip_addr != [0, 0, 0, 0] {
                        let ip = alloc::format!("IP: {}.{}.{}.{}", cfg.ip_addr[0], cfg.ip_addr[1], cfg.ip_addr[2], cfg.ip_addr[3]);
                        g::draw_text(fb, mx + 12, cy + 66, &ip, theme.label_text_dim, theme.win_bg);
                    }
                }
                g::draw_text(fb, mx + 12, cy + 100, "HTTP via lobster-net host proxy.", theme.label_text_dim, theme.win_bg);
            }
            Page::Storage => {
                g::draw_text(fb, mx + 12, cy + 10, "Storage", theme.label_text, theme.win_bg);
                g::draw_rect(fb, mx + 8, cy + 28, mw - 16, 1, theme.menu_border);
                g::draw_text(fb, mx + 12, cy + 44, "Persistent disk: lobster-vd0 (64 MB ext2)",
                    theme.label_text, theme.win_bg);
                g::draw_text(fb, mx + 12, cy + 66, "Mounted at /sd — settings, notes, files",
                    theme.label_text_dim, theme.win_bg);
                g::draw_text(fb, mx + 12, cy + 90, "Files created in the guest survive reboots.",
                    theme.label_text_dim, theme.win_bg);
            }
            Page::Users => {
                g::draw_text(fb, mx + 12, cy + 10, "Users", theme.label_text, theme.win_bg);
                g::draw_rect(fb, mx + 8, cy + 28, mw - 16, 1, theme.menu_border);
                let cur = crate::users::current_user();
                let mut uy = cy + 44;
                for name in crate::users::login_users() {
                    let line = if name == cur {
                        alloc::format!("{}  (current)  /home/{}", name, name)
                    } else {
                        alloc::format!("{}  /home/{}", name, name)
                    };
                    g::draw_text(fb, mx + 12, uy, &line, theme.label_text, theme.win_bg);
                    uy += 22;
                }
                g::draw_text(fb, mx + 12, uy + 10, "Shell: useradd <name> | userdel <name> | userlist",
                    theme.label_text_dim, theme.win_bg);
            }
            Page::About => {
                g::draw_text(fb, mx + 12, cy + 10, "About", theme.label_text, theme.win_bg);
                g::draw_rect(fb, mx + 8, cy + 28, mw - 16, 1, theme.menu_border);
                g::draw_text(fb, mx + 12, cy + 48, "\u{1F99E} LobsterOS v0.2.0", theme.label_text, theme.win_bg);
                g::draw_text(fb, mx + 12, cy + 70, "Bare-metal Rust OS for BCM2712 (Pi 5)",
                    theme.label_text_dim, theme.win_bg);
                g::draw_text(fb, mx + 12, cy + 90, "Kernel: jlaportebot/lobster-os",
                    theme.label_text_dim, theme.win_bg);
                g::draw_text(fb, mx + 12, cy + 110, "Emulator: jlaportebot/lobster-qemu",
                    theme.label_text_dim, theme.win_bg);
            }
        }
    }
}

/// Render hook used by wm::tick (QEMU).
#[cfg(feature = "qemu_raspi5b")]
pub fn render_settings_window(fb: &crate::display::fb::FramebufferInfo) {
    settings_ui::render_to_windows(fb);
}

/// Shell: open the Settings window.
#[cfg(feature = "qemu_raspi5b")]
pub fn open_window() -> u32 {
    settings_ui::open()
}

/// Shell: cycle the Settings page.
#[cfg(feature = "qemu_raspi5b")]
pub fn cycle_page() {
    settings_ui::next_page();
}
