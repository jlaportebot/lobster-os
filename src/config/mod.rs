//! System configuration: TOML-like parser + /etc persistence
//!
//! Provides runtime system config via /etc/lobsteros.conf
//! Supports key-value pairs, sections, and typed values.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// System configuration
pub struct SystemConfig {
    entries: BTreeMap<String, ConfigSection>,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ConfigSection {
    pub name: String,
    pub keys: BTreeMap<String, ConfigValue>,
}

#[derive(Debug, Clone)]
pub enum ConfigValue {
    Bool(bool),
    Int(i64),
    String(String),
}

impl SystemConfig {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Load default configuration
    pub fn defaults() -> Self {
        let mut config = Self::new();

        // [boot]
        let mut boot = ConfigSection {
            name: "boot".into(),
            keys: BTreeMap::new(),
        };
        boot.keys.insert("splash".into(), ConfigValue::Bool(true));
        boot.keys
            .insert("autologin".into(), ConfigValue::Bool(false));
        boot.keys.insert("timeout".into(), ConfigValue::Int(3));
        config.entries.insert("boot".into(), boot);

        // [display]
        let mut display = ConfigSection {
            name: "display".into(),
            keys: BTreeMap::new(),
        };
        display.keys.insert("width".into(), ConfigValue::Int(1920));
        display.keys.insert("height".into(), ConfigValue::Int(1080));
        display.keys.insert("depth".into(), ConfigValue::Int(32));
        display.keys.insert("rotation".into(), ConfigValue::Int(0));
        display.keys.insert("dpi".into(), ConfigValue::Int(96));
        config.entries.insert("display".into(), display);

        // [audio]
        let mut audio = ConfigSection {
            name: "audio".into(),
            keys: BTreeMap::new(),
        };
        audio.keys.insert("volume".into(), ConfigValue::Int(75));
        audio
            .keys
            .insert("device".into(), ConfigValue::String("auto".into()));
        audio
            .keys
            .insert("sample_rate".into(), ConfigValue::Int(48000));
        config.entries.insert("audio".into(), audio);

        // [network]
        let mut network = ConfigSection {
            name: "network".into(),
            keys: BTreeMap::new(),
        };
        network
            .keys
            .insert("hostname".into(), ConfigValue::String("lobsteros".into()));
        network.keys.insert("dhcp".into(), ConfigValue::Bool(true));
        network
            .keys
            .insert("wifi_auto".into(), ConfigValue::Bool(true));
        config.entries.insert("network".into(), network);

        // [power]
        let mut power = ConfigSection {
            name: "power".into(),
            keys: BTreeMap::new(),
        };
        power
            .keys
            .insert("governor".into(), ConfigValue::String("ondemand".into()));
        power
            .keys
            .insert("suspend_timeout".into(), ConfigValue::Int(0));
        power
            .keys
            .insert("watchdog".into(), ConfigValue::Bool(true));
        config.entries.insert("power".into(), power);

        // [security]
        let mut security = ConfigSection {
            name: "security".into(),
            keys: BTreeMap::new(),
        };
        security
            .keys
            .insert("firewall".into(), ConfigValue::Bool(false));
        security
            .keys
            .insert("selinux".into(), ConfigValue::Bool(false));
        config.entries.insert("security".into(), security);

        // [desktop]
        let mut desktop = ConfigSection {
            name: "desktop".into(),
            keys: BTreeMap::new(),
        };
        desktop
            .keys
            .insert("theme".into(), ConfigValue::String("dark".into()));
        desktop
            .keys
            .insert("accent".into(), ConfigValue::String("#FF6B35".into()));
        desktop
            .keys
            .insert("font_size".into(), ConfigValue::Int(14));
        desktop
            .keys
            .insert("workspaces".into(), ConfigValue::Int(4));
        desktop
            .keys
            .insert("snapping".into(), ConfigValue::Bool(true));
        config.entries.insert("desktop".into(), desktop);

        // [locale]
        let mut locale = ConfigSection {
            name: "locale".into(),
            keys: BTreeMap::new(),
        };
        locale
            .keys
            .insert("lang".into(), ConfigValue::String("en_US".into()));
        locale
            .keys
            .insert("timezone".into(), ConfigValue::String("UTC".into()));
        locale
            .keys
            .insert("keyboard".into(), ConfigValue::String("us".into()));
        config.entries.insert("locale".into(), locale);

        config
    }

    /// Get a value from a section
    pub fn get(&self, section: &str, key: &str) -> Option<&ConfigValue> {
        self.entries.get(section).and_then(|s| s.keys.get(key))
    }

    /// Set a value in a section
    pub fn set(&mut self, section: &str, key: &str, value: ConfigValue) {
        self.entries
            .entry(section.into())
            .or_insert_with(|| ConfigSection {
                name: section.into(),
                keys: BTreeMap::new(),
            })
            .keys
            .insert(key.into(), value);
    }

    /// Get a string value
    pub fn get_str(&self, section: &str, key: &str) -> Option<&str> {
        match self.get(section, key) {
            Some(ConfigValue::String(s)) => Some(s),
            _ => None,
        }
    }

    /// Get an int value
    pub fn get_int(&self, section: &str, key: &str) -> Option<i64> {
        match self.get(section, key) {
            Some(ConfigValue::Int(v)) => Some(*v),
            _ => None,
        }
    }

    /// Get a bool value
    pub fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        match self.get(section, key) {
            Some(ConfigValue::Bool(v)) => Some(*v),
            _ => None,
        }
    }

    /// Serialize to TOML-like text
    pub fn to_config_string(&self) -> String {
        let mut out = String::new();
        out.push_str("# LobsterOS System Configuration\n");
        out.push_str("# /etc/lobsteros.conf\n\n");

        for section in self.entries.values() {
            out.push_str(&format!("[{}]\n", section.name));
            for (key, value) in &section.keys {
                match value {
                    ConfigValue::Bool(b) => out.push_str(&format!("{} = {}\n", key, b)),
                    ConfigValue::Int(i) => out.push_str(&format!("{} = {}\n", key, i)),
                    ConfigValue::String(s) => out.push_str(&format!("{} = \"{}\"\n", key, s)),
                }
            }
            out.push('\n');
        }

        out
    }

    /// Parse from TOML-like text
    pub fn parse(text: &str) -> Self {
        let mut config = Self::new();
        let mut current_section = String::new();

        for line in text.lines() {
            let line = line.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                current_section = line[1..line.len() - 1].to_string();
                config.entries.insert(
                    current_section.clone(),
                    ConfigSection {
                        name: current_section.clone(),
                        keys: BTreeMap::new(),
                    },
                );
                continue;
            }

            if let Some(eq_pos) = line.find('=') {
                let key = line[..eq_pos].trim().to_string();
                let value_str = line[eq_pos + 1..].trim();

                let value = if value_str.starts_with('"') && value_str.ends_with('"') {
                    ConfigValue::String(value_str[1..value_str.len() - 1].to_string())
                } else if value_str == "true" {
                    ConfigValue::Bool(true)
                } else if value_str == "false" {
                    ConfigValue::Bool(false)
                } else if let Ok(i) = value_str.parse::<i64>() {
                    ConfigValue::Int(i)
                } else {
                    ConfigValue::String(value_str.to_string())
                };

                config.set(&current_section, &key, value);
            }
        }

        config
    }

    /// Load from /etc/lobsteros.conf (if it exists)
    pub fn load_from_file() -> Self {
        // Try to read /etc/lobsteros.conf via VFS
        // If not available, return defaults
        // For now, return defaults — VFS integration will be added when
        // the filesystem is mounted at boot
        Self::defaults()
    }

    /// Save to /etc/lobsteros.conf
    pub fn save_to_file(&self) -> Result<(), &'static str> {
        let _content = self.to_config_string();
        // Write via VFS when filesystem is available
        Ok(())
    }
}

/// OS release info (/etc/os-release)
pub fn write_os_release() {
    let content = r#"NAME="LobsterOS"
VERSION="0.5.0"
ID=lobsteros
ID_LIKE=none
PRETTY_NAME="LobsterOS 0.5.0 (Daily Driver)"
ANSI_COLOR="0;31"
HOME_URL="https://github.com/jlaportebot/lobster-os"
SUPPORT_URL="https://github.com/jlaportebot/lobster-os/issues"
BUILD_ID="daily-driver"
"#;
    // Write to /etc/os-release via VFS
    let _ = content;
}

/// sysctl-like interface
pub struct Sysctl {
    config: SystemConfig,
}

impl Default for Sysctl {
    fn default() -> Self {
        Self::new()
    }
}

impl Sysctl {
    pub fn new() -> Self {
        Self {
            config: SystemConfig::load_from_file(),
        }
    }

    /// Get a sysctl value: `sysctl.get("desktop.theme")`
    pub fn get(&self, path: &str) -> Option<&ConfigValue> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() != 2 {
            return None;
        }
        self.config.get(parts[0], parts[1])
    }

    /// Set a sysctl value: `sysctl.set("desktop.theme", "light")`
    pub fn set(&mut self, path: &str, value: ConfigValue) {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 2 {
            self.config.set(parts[0], parts[1], value);
            let _ = self.config.save_to_file();
        }
    }

    /// Reload config from file
    pub fn reload(&mut self) {
        self.config = SystemConfig::load_from_file();
    }
}
