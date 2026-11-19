//! The single settings file, `/sd/etc/settings.conf`: plain `key=value`
//! lines, parsed at boot before the desktop draws.

use crate::wm::theme::ThemeId;
use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

/// Where the settings live on the persistent disk.
pub const PATH: &str = "/sd/etc/settings.conf";

/// Highest volume, in percent.
const MAX_VOLUME: u32 = 100;

/// Every persisted preference.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub theme: ThemeId,
    /// Accent colour, `0xAARRGGBB`.
    pub accent: u32,
    /// Wallpaper preset name.
    pub wallpaper: String,
    pub large_text: bool,
    pub clock_24h: bool,
    /// Output volume in percent, 0..=100.
    pub volume: u32,
    pub muted: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ThemeId::Dark,
            accent: 0xFF5E81AC,
            wallpaper: String::from("Nord Dusk"),
            large_text: false,
            clock_24h: true,
            volume: 70,
            muted: false,
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_theme(value: &str) -> Option<ThemeId> {
    match value.to_ascii_lowercase().as_str() {
        "dark" => Some(ThemeId::Dark),
        "light" => Some(ThemeId::Light),
        _ => None,
    }
}

fn parse_hex(value: &str) -> Option<u32> {
    let digits = value.strip_prefix("0x").unwrap_or(value);
    u32::from_str_radix(digits, 16).ok()
}

fn apply(config: &mut Config, key: &str, value: &str) {
    match key {
        "theme" => config.theme = parse_theme(value).unwrap_or(config.theme),
        "accent" => config.accent = parse_hex(value).unwrap_or(config.accent),
        "wallpaper" if !value.is_empty() => config.wallpaper = String::from(value),
        "large_text" => config.large_text = parse_bool(value).unwrap_or(config.large_text),
        "clock_24h" => config.clock_24h = parse_bool(value).unwrap_or(config.clock_24h),
        "volume" => match value.parse::<u32>() {
            Ok(v) if v <= MAX_VOLUME => config.volume = v,
            _ => {}
        },
        "muted" => config.muted = parse_bool(value).unwrap_or(config.muted),
        _ => {}
    }
}

/// Parse the file text. Unknown keys, comments and malformed lines are
/// skipped; a bad value leaves that setting at its default.
#[must_use]
pub fn parse(text: &str) -> Config {
    let mut config = Config::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            apply(&mut config, key.trim(), value.trim());
        }
    }
    config
}

/// Render the config as file text; `parse(serialize(c)) == c`.
#[must_use]
pub fn serialize(config: &Config) -> String {
    let theme = match config.theme {
        ThemeId::Dark => "dark",
        ThemeId::Light => "light",
    };
    alloc::format!(
        "# LobsterOS settings\ntheme={}\naccent={:#010x}\nwallpaper={}\nlarge_text={}\nclock_24h={}\nvolume={}\nmuted={}\n",
        theme,
        config.accent,
        config.wallpaper,
        config.large_text,
        config.clock_24h,
        config.volume,
        config.muted
    )
}
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::wm::theme::ThemeId;

    fn custom() -> Config {
        Config {
            theme: ThemeId::Light,
            accent: 0xFFBF616A,
            wallpaper: String::from("Deep Ocean"),
            large_text: true,
            clock_24h: false,
            volume: 35,
            muted: true,
        }
    }

    #[test]
    fn serialize_then_parse_roundtrips() {
        let config = custom();
        assert_eq!(parse(&serialize(&config)), config);
    }

    #[test]
    fn defaults_roundtrip_too() {
        let config = Config::default();
        assert_eq!(parse(&serialize(&config)), config);
    }

    #[test]
    fn unknown_keys_and_comments_are_ignored() {
        let text = "# comment\nflavour=mint\n\ntheme=light\n  future_key = 1\n";
        let config = parse(text);
        assert_eq!(config.theme, ThemeId::Light);
        assert_eq!(config.volume, Config::default().volume);
    }

    #[test]
    fn bad_values_fall_back_to_defaults() {
        let text =
            "theme=purple\naccent=banana\nvolume=loud\nlarge_text=maybe\nclock_24h=2\nmuted=\n";
        assert_eq!(parse(text), Config::default());
    }

    #[test]
    fn out_of_range_volume_falls_back_to_the_default() {
        assert_eq!(parse("volume=250\n").volume, Config::default().volume);
        assert_eq!(parse("volume=100\n").volume, 100);
    }

    #[test]
    fn accent_accepts_hex_with_or_without_prefix() {
        assert_eq!(parse("accent=0xFFA3BE8C\n").accent, 0xFFA3BE8C);
        assert_eq!(parse("accent=FFA3BE8C\n").accent, 0xFFA3BE8C);
    }

    #[test]
    fn wallpaper_name_keeps_inner_spaces() {
        assert_eq!(parse("wallpaper=Nord Aurora\n").wallpaper, "Nord Aurora");
    }

    #[test]
    fn empty_input_gives_defaults() {
        assert_eq!(parse(""), Config::default());
    }

    #[test]
    fn a_good_line_survives_a_bad_neighbour() {
        let config = parse("volume=abc\nclock_24h=false\n");
        assert!(!config.clock_24h);
    }
}

/// Set while the file is being applied, so the setters it calls do not
/// write a half-restored state back.
static LOADING: AtomicBool = AtomicBool::new(false);

/// Config describing the live desktop.
#[must_use]
pub fn snapshot() -> Config {
    use crate::wm::{theme, wallpaper};
    Config {
        theme: theme::active_id(),
        accent: theme::accent_color(),
        wallpaper: String::from(wallpaper::active().name),
        large_text: theme::font_size() == theme::FontSize::Large,
        clock_24h: crate::clock::use_24_hour(),
        volume: crate::wm::tray_popups::volume_level(),
        muted: super::sound::is_muted(),
    }
}

/// Write the live preferences to [`PATH`] (best effort: the disk may not be
/// mounted yet).
pub fn save() {
    if LOADING.load(Ordering::Relaxed) {
        return;
    }
    let text = serialize(&snapshot());
    if crate::fs::vfs::write_file(PATH, text.as_bytes()).is_err() {
        crate::println!("[settings] could not write {}", PATH);
    }
}

/// Apply a parsed config to the live desktop.
pub fn apply_config(config: &Config) {
    use crate::wm::{theme, wallpaper};
    LOADING.store(true, Ordering::Relaxed);
    theme::set(config.theme);
    theme::restore_accent(config.accent);
    wallpaper::set_by_name(&config.wallpaper);
    let size = if config.large_text {
        theme::FontSize::Large
    } else {
        theme::FontSize::Medium
    };
    theme::set_font_size(size);
    crate::clock::set_use_24_hour(config.clock_24h);
    super::sound::set_volume(config.volume);
    super::sound::set_muted(config.muted);
    LOADING.store(false, Ordering::Relaxed);
}

/// Restore the saved preferences; called once /sd is mounted, before the
/// desktop draws.
pub fn load_at_boot() {
    let Ok(data) = crate::fs::vfs::read_file(PATH) else {
        return;
    };
    let text = String::from_utf8_lossy(&data);
    apply_config(&parse(&text));
    crate::println!("[settings] restored preferences from {}", PATH);
}
