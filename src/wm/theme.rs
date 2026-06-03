//! Theme engine — colors, spacing, fonts, visual identity for LobsterOS
//!
//! The theme defines the visual appearance of all UI elements:
//! - Desktop background color
//! - Window chrome (title bar, borders, buttons)
//! - Widget colors (buttons, labels, inputs, scrollbars)
//! - Spacing and sizing constants
//!
//! Themes are static configurations; at runtime `current()` returns the
//! active theme and all rendering code reads from it.

use crate::display::console::*;
use crate::println;

// ─── Theme Structure ───────────────────────────────────────────────────

/// Complete visual theme for the window manager
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    // ── Desktop ──
    pub desktop_bg: u32,

    // ── Window chrome ──
    pub title_bar: u32,
    pub title_bar_unfocused: u32,
    pub title_text: u32,
    pub title_text_unfocused: u32,
    pub win_bg: u32,
    pub win_border: u32,
    pub win_border_unfocused: u32,

    // ── Title bar buttons ──
    pub close_btn: u32,
    pub close_btn_hover: u32,
    pub maximize_btn: u32,
    pub minimize_btn: u32,

    // ── Widget: Button ──
    pub btn_bg: u32,
    pub btn_bg_hover: u32,
    pub btn_bg_pressed: u32,
    pub btn_border: u32,
    pub btn_text: u32,
    pub btn_border_radius: u32,

    // ── Widget: Label ──
    pub label_text: u32,
    pub label_text_dim: u32,

    // ── Widget: Text Input ──
    pub input_bg: u32,
    pub input_border: u32,
    pub input_border_focused: u32,
    pub input_text: u32,
    pub input_cursor: u32,
    pub input_selection: u32,

    // ── Widget: Scrollbar ──
    pub scroll_track: u32,
    pub scroll_thumb: u32,
    pub scroll_thumb_hover: u32,

    // ── Widget: Panel ──
    pub panel_bg: u32,
    pub panel_border: u32,

    // ── Widget: Checkbox ──
    pub checkbox_bg: u32,
    pub checkbox_border: u32,
    pub checkbox_check: u32,

    // ── Widget: Progress bar ──
    pub progress_track: u32,
    pub progress_fill: u32,

    // ── Widget: Menu ──
    pub menu_bg: u32,
    pub menu_border: u32,
    pub menu_item_hover: u32,
    pub menu_item_text: u32,
    pub menu_separator: u32,

    // ── Spacing ──
    pub spacing_xs: u32,  // 2px
    pub spacing_sm: u32,  // 4px
    pub spacing_md: u32,  // 8px
    pub spacing_lg: u32,  // 16px
    pub spacing_xl: u32,  // 24px

    // ── Sizing ──
    pub title_bar_height: u32,
    pub border_width: u32,
    pub widget_height: u32,     // standard widget height
    pub font_size: u32,         // character cell height

    // ── Taskbar ──
    pub taskbar_bg: u32,
    pub taskbar_border: u32,
    pub taskbar_text: u32,
    pub taskbar_height: u32,
}

// ─── Built-in Themes ───────────────────────────────────────────────────

/// LobsterOS Dark — the default theme (deep ocean colors)
const THEME_DARK: Theme = Theme {
    // Desktop
    desktop_bg: 0xFF0D1B2A,  // deep navy

    // Window chrome
    title_bar: 0xFF1B2838,
    title_bar_unfocused: 0xFF2A3A4E,
    title_text: 0xFFE0E0E0,
    title_text_unfocused: 0xFF808080,
    win_bg: 0xFF1E1E2E,
    win_border: 0xFF4A9EFF,
    win_border_unfocused: 0xFF3A4A5E,

    // Title bar buttons
    close_btn: 0xFFFF5F56,
    close_btn_hover: 0xFFFF3B30,
    maximize_btn: 0xFF27C93F,
    minimize_btn: 0xFFFFBD2E,

    // Button
    btn_bg: 0xFF2D3A4A,
    btn_bg_hover: 0xFF3A4D62,
    btn_bg_pressed: 0xFF1A2736,
    btn_border: 0xFF4A9EFF,
    btn_text: 0xFFE0E0E0,
    btn_border_radius: 4,

    // Label
    label_text: 0xFFE0E0E0,
    label_text_dim: 0xFF808080,

    // Text input
    input_bg: 0xFF0F1923,
    input_border: 0xFF3A4A5E,
    input_border_focused: 0xFF4A9EFF,
    input_text: 0xFFE0E0E0,
    input_cursor: 0xFF4A9EFF,
    input_selection: 0x4A9EFF40,

    // Scrollbar
    scroll_track: 0xFF0F1923,
    scroll_thumb: 0xFF3A4A5E,
    scroll_thumb_hover: 0xFF4A6A8E,

    // Panel
    panel_bg: 0xFF1A2634,
    panel_border: 0xFF2A3A4E,

    // Checkbox
    checkbox_bg: 0xFF0F1923,
    checkbox_border: 0xFF3A4A5E,
    checkbox_check: 0xFF4A9EFF,

    // Progress bar
    progress_track: 0xFF0F1923,
    progress_fill: 0xFF4A9EFF,

    // Menu
    menu_bg: 0xFF1A2634,
    menu_border: 0xFF2A3A4E,
    menu_item_hover: 0xFF2D3A4A,
    menu_item_text: 0xFFE0E0E0,
    menu_separator: 0xFF2A3A4E,

    // Spacing
    spacing_xs: 2,
    spacing_sm: 4,
    spacing_md: 8,
    spacing_lg: 16,
    spacing_xl: 24,

    // Sizing
    title_bar_height: 24,
    border_width: 2,
    widget_height: 28,
    font_size: 16,

    // Taskbar
    taskbar_bg: 0xFF0D1B2A,
    taskbar_border: 0xFF1B2838,
    taskbar_text: 0xFFE0E0E0,
    taskbar_height: 36,
};

/// LobsterOS Light — a light theme variant
const THEME_LIGHT: Theme = Theme {
    // Desktop
    desktop_bg: 0xFFF0F0F0,

    // Window chrome
    title_bar: 0xFFE8E8E8,
    title_bar_unfocused: 0xFFF0F0F0,
    title_text: 0xFF333333,
    title_text_unfocused: 0xFF999999,
    win_bg: 0xFFFFFFFF,
    win_border: 0xFF4A9EFF,
    win_border_unfocused: 0xFFCCCCCC,

    // Title bar buttons
    close_btn: 0xFFFF5F56,
    close_btn_hover: 0xFFFF3B30,
    maximize_btn: 0xFF27C93F,
    minimize_btn: 0xFFFFBD2E,

    // Button
    btn_bg: 0xFFE0E0E0,
    btn_bg_hover: 0xFFD0D0D0,
    btn_bg_pressed: 0xFFC0C0C0,
    btn_border: 0xFF4A9EFF,
    btn_text: 0xFF333333,
    btn_border_radius: 4,

    // Label
    label_text: 0xFF333333,
    label_text_dim: 0xFF999999,

    // Text input
    input_bg: 0xFFFFFFFF,
    input_border: 0xFFCCCCCC,
    input_border_focused: 0xFF4A9EFF,
    input_text: 0xFF333333,
    input_cursor: 0xFF4A9EFF,
    input_selection: 0x4A9EFF40,

    // Scrollbar
    scroll_track: 0xFFF0F0F0,
    scroll_thumb: 0xFFC0C0C0,
    scroll_thumb_hover: 0xFFA0A0A0,

    // Panel
    panel_bg: 0xFFF5F5F5,
    panel_border: 0xFFE0E0E0,

    // Checkbox
    checkbox_bg: 0xFFFFFFFF,
    checkbox_border: 0xFFCCCCCC,
    checkbox_check: 0xFF4A9EFF,

    // Progress bar
    progress_track: 0xFFF0F0F0,
    progress_fill: 0xFF4A9EFF,

    // Menu
    menu_bg: 0xFFFFFFFF,
    menu_border: 0xFFE0E0E0,
    menu_item_hover: 0xFFE0E0E0,
    menu_item_text: 0xFF333333,
    menu_separator: 0xFFE0E0E0,

    // Spacing
    spacing_xs: 2,
    spacing_sm: 4,
    spacing_md: 8,
    spacing_lg: 16,
    spacing_xl: 24,

    // Sizing
    title_bar_height: 24,
    border_width: 2,
    widget_height: 28,
    font_size: 16,

    // Taskbar
    taskbar_bg: 0xFFE8E8E8,
    taskbar_border: 0xFFD0D0D0,
    taskbar_text: 0xFF333333,
    taskbar_height: 36,
};

// ─── Active Theme ──────────────────────────────────────────────────────

/// Available theme identifiers
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThemeId {
    Dark,
    Light,
}

static mut ACTIVE_THEME: ThemeId = ThemeId::Dark;

/// Initialize the theme engine
pub fn init() {
    unsafe { ACTIVE_THEME = ThemeId::Dark; }
    println!("[theme] Loaded: Dark");
}

/// Get the current active theme
pub fn current() -> &'static Theme {
    unsafe {
        match ACTIVE_THEME {
            ThemeId::Dark => &THEME_DARK,
            ThemeId::Light => &THEME_LIGHT,
        }
    }
}

/// Switch to a different theme
pub fn set(id: ThemeId) {
    unsafe {
        ACTIVE_THEME = id;
    }
    crate::wm::compositor::mark_full_redraw();
    let name = match id {
        ThemeId::Dark => "Dark",
        ThemeId::Light => "Light",
    };
    println!("[theme] Switched to: {}", name);
}

/// Get the current theme ID
pub fn active_id() -> ThemeId {
    unsafe { ACTIVE_THEME }
}

/// Get the current theme name as a string
pub fn current_name() -> &'static str {
    match active_id() {
        ThemeId::Dark => "Dark",
        ThemeId::Light => "Light",
    }
}

/// Get the desktop background color (convenience accessor)
pub fn desktop_bg() -> u32 {
    current().desktop_bg
}

/// Blend two ARGB colors with a given alpha (0–255)
pub fn blend_colors(fg: u32, bg: u32, alpha: u32) -> u32 {
    let a = alpha.min(255) as u32;
    let inv_a = 255 - a;

    let fr = (fg >> 16) & 0xFF;
    let fg_ = (fg >> 8) & 0xFF;
    let fb = fg & 0xFF;

    let br = (bg >> 16) & 0xFF;
    let bg_ = (bg >> 8) & 0xFF;
    let bb = bg & 0xFF;

    let r = (fr * a + br * inv_a) / 255;
    let g = (fg_ * a + bg_ * inv_a) / 255;
    let b = (fb * a + bb * inv_a) / 255;

    0xFF000000 | (r << 16) | (g << 8) | b
}

/// Darken a color by a factor (0.0–1.0, encoded as 0–255)
pub fn darken(color: u32, factor: u32) -> u32 {
    let f = factor.min(255) as u32;
    let r = ((color >> 16) & 0xFF) * f / 255;
    let g = ((color >> 8) & 0xFF) * f / 255;
    let b = (color & 0xFF) * f / 255;
    0xFF000000 | (r << 16) | (g << 8) | b
}

/// Lighten a color by a factor (0.0–1.0, encoded as 0–255)
pub fn lighten(color: u32, factor: u32) -> u32 {
    let f = factor.min(255) as u32;
    let r = ((color >> 16) & 0xFF);
    let g = ((color >> 8) & 0xFF);
    let b = (color & 0xFF);
    let r = r + (255 - r) * f / 255;
    let g = g + (255 - g) * f / 255;
    let b = b + (255 - b) * f / 255;
    0xFF000000 | (r << 16) | (g << 8) | b
}
