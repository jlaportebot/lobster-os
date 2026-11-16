//! Tray popups — panels that open from the taskbar tray
//!
//! - Clock popup: calendar-ish grid (uptime-based date approximation) +
//!   recent notifications (notification center).
//! - Network popup: interface state + IP.
//! - Volume/battery popup: level bars.
//! - Quick Settings popup: brightness/volume sliders, network toggle, theme toggle.
//! All popups render above the panel, below toasts, and close on outside click.

use crate::display::fb::get_info;
use crate::display::graphics::{draw_rect, draw_rect_outline, draw_text, fill_rounded_rect};
use crate::wm::notifications;
use crate::wm::panel;
use crate::wm::theme;

const POPUP_W: u32 = 320;
const POPUP_H: u32 = 380;
const PAD: u32 = 10;

/// Quick settings slider state
#[derive(Clone, Copy)]
struct SliderState {
    value: u32, // 0..=100
    dragging: bool,
}

static mut BRIGHTNESS_SLIDER: SliderState = SliderState {
    value: 80,
    dragging: false,
};
static mut VOLUME_SLIDER: SliderState = SliderState {
    value: 70,
    dragging: false,
};
static mut NETWORK_ENABLED: bool = true;

#[derive(Clone, Copy, PartialEq)]
pub enum PopupKind {
    None,
    Clock,
    Network,
    Volume,
    Battery,
    QuickSettings,
}

static mut OPEN: PopupKind = PopupKind::None;

pub fn open(kind: PopupKind) {
    unsafe {
        OPEN = kind;
    }
    crate::wm::compositor::mark_full_redraw();
}

pub fn close() {
    unsafe {
        OPEN = PopupKind::None;
        // Reset slider drag state
        BRIGHTNESS_SLIDER.dragging = false;
        VOLUME_SLIDER.dragging = false;
    }
    crate::wm::compositor::mark_full_redraw();
}

pub fn is_open() -> bool {
    unsafe { OPEN != PopupKind::None }
}

pub fn open_kind() -> PopupKind {
    unsafe { OPEN }
}

/// Handle a click. Returns true if consumed.
pub fn handle_click(px: u32, py: u32) -> bool {
    let kind = open_kind();
    if kind == PopupKind::None {
        return false;
    }
    let (x0, y0) = origin();
    if px < x0 || px >= x0 + POPUP_W || py < y0 || py >= y0 + POPUP_H {
        close();
        return false;
    }
    // Clicks inside: nothing actionable yet beyond dismissal on title row
    if py < y0 + 30 {
        close();
        return true;
    }

    // Handle QuickSettings interactions
    if kind == PopupKind::QuickSettings {
        return handle_quick_settings_click(px, py, x0, y0);
    }

    true
}

/// Handle mouse drag for slider dragging
pub fn handle_drag(px: u32, py: u32) -> bool {
    let kind = open_kind();
    if kind != PopupKind::QuickSettings {
        return false;
    }
    let (x0, y0) = origin();
    if px < x0 || px >= x0 + POPUP_W || py < y0 || py >= y0 + POPUP_H {
        return false;
    }
    handle_quick_settings_drag(px, py, x0, y0)
}

/// Handle mouse release
pub fn handle_release(_px: u32, _py: u32) -> bool {
    let kind = open_kind();
    if kind != PopupKind::QuickSettings {
        return false;
    }
    unsafe {
        BRIGHTNESS_SLIDER.dragging = false;
        VOLUME_SLIDER.dragging = false;
    }
    crate::wm::compositor::mark_full_redraw();
    true
}

/// Anchor above the panel, right-aligned to the tray column.
fn origin() -> (u32, u32) {
    let fb = get_info();
    let panel_h = panel::panel_height();
    // Tray items sit ~120px from the right edge (clock ~70px). Anchor the
    // popup right side at 8px margin.
    let x0 = fb.width.saturating_sub(POPUP_W + 8);
    let y0 = fb.height.saturating_sub(panel_h + POPUP_H + 4);
    (x0, y0)
}

pub fn draw(fb: &crate::display::fb::FramebufferInfo) {
    let kind = open_kind();
    if kind == PopupKind::None {
        return;
    }
    let theme = theme::current();
    let (x0, y0) = origin();

    // Card
    draw_rect(fb, x0 + 4, y0 + 6, POPUP_W, POPUP_H, 0x55000000);
    fill_rounded_rect(fb, x0, y0, POPUP_W, POPUP_H, 10, theme.menu_bg);
    draw_rect_outline(fb, x0, y0, POPUP_W, POPUP_H, theme.menu_border);

    match kind {
        PopupKind::Clock => draw_clock(fb, theme, x0, y0),
        PopupKind::Network => draw_network(fb, theme, x0, y0),
        PopupKind::Volume => draw_volume(fb, theme, x0, y0),
        PopupKind::Battery => draw_battery(fb, theme, x0, y0),
        PopupKind::QuickSettings => draw_quick_settings(fb, theme, x0, y0),
        PopupKind::None => {}
    }
}

fn header(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y0: u32,
    title: &str,
) {
    draw_text(
        fb,
        x0 + PAD + 2,
        y0 + 10,
        title,
        theme.label_text,
        theme.menu_bg,
    );
    draw_rect(
        fb,
        x0 + PAD,
        y0 + 30,
        POPUP_W - 2 * PAD,
        1,
        theme.menu_border,
    );
}

fn draw_quick_settings(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y0: u32,
) {
    header(fb, theme, x0, y0, "Quick Settings");

    let mut y = y0 + 44;

    // Brightness slider
    draw_slider(fb, theme, x0, y, "Brightness", unsafe {
        BRIGHTNESS_SLIDER.value
    });
    y += 50;

    // Volume slider
    draw_slider(fb, theme, x0, y, "Volume", unsafe { VOLUME_SLIDER.value });
    y += 50;

    // Network toggle
    draw_toggle(fb, theme, x0, y, "Network", unsafe { NETWORK_ENABLED });
    y += 40;

    // Theme toggle
    let theme_name = crate::wm::theme::current_name();
    draw_toggle(
        fb,
        theme,
        x0,
        y,
        &alloc::format!("Theme: {}", theme_name),
        theme_name == "Light",
    );
    y += 40;

    // Clear notifications button
    draw_button(fb, theme, x0, y, "Clear All Notifications");
}

fn draw_slider(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y: u32,
    label: &str,
    value: u32,
) {
    draw_text(fb, x0 + PAD + 2, y, label, theme.label_text, theme.menu_bg);
    let bar_w = POPUP_W - 2 * PAD - 4;
    let bar_x = x0 + PAD + 2;
    let bar_y = y + 22;
    // Track
    draw_rect(fb, bar_x, bar_y, bar_w, 10, theme.menu_border);
    // Fill
    let fill = bar_w * value / 100;
    draw_rect(fb, bar_x, bar_y, fill, 10, crate::wm::theme::accent_color());
    // Thumb
    let thumb_x = bar_x + fill.saturating_sub(5);
    fill_rounded_rect(fb, thumb_x, bar_y - 4, 10, 18, 5, theme.btn_bg_hover);
    draw_rect_outline(fb, thumb_x, bar_y - 4, 10, 18, theme.btn_border);
    // Value
    draw_text(
        fb,
        x0 + PAD + 2,
        y + 40,
        &alloc::format!("{}%", value),
        theme.label_text_dim,
        theme.menu_bg,
    );
}

fn draw_toggle(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y: u32,
    label: &str,
    enabled: bool,
) {
    draw_text(fb, x0 + PAD + 2, y, label, theme.label_text, theme.menu_bg);
    let toggle_x = x0 + POPUP_W - PAD - 50;
    let toggle_y = y;
    let track_w = 44;
    let track_h = 24;
    let track_color = if enabled {
        crate::wm::theme::accent_color()
    } else {
        theme.menu_border
    };
    fill_rounded_rect(
        fb,
        toggle_x,
        toggle_y,
        track_w,
        track_h,
        track_h / 2,
        track_color,
    );
    let thumb_x = if enabled {
        toggle_x + track_w - track_h + 2
    } else {
        toggle_x + 2
    };
    fill_rounded_rect(
        fb,
        thumb_x,
        toggle_y + 2,
        track_h - 4,
        track_h - 4,
        (track_h - 4) / 2,
        theme.btn_bg,
    );
    draw_rect_outline(
        fb,
        thumb_x,
        toggle_y + 2,
        track_h - 4,
        track_h - 4,
        theme.btn_border,
    );
}

fn draw_button(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y: u32,
    label: &str,
) {
    let btn_w = POPUP_W - 2 * PAD;
    let btn_h = 32;
    let btn_x = x0 + PAD;
    let btn_y = y;
    fill_rounded_rect(fb, btn_x, btn_y, btn_w, btn_h, 6, theme.btn_bg);
    draw_rect_outline(fb, btn_x, btn_y, btn_w, btn_h, theme.btn_border);
    let text = label;
    let char_w = 8u32;
    let text_w = text.chars().count() as u32 * char_w;
    let text_x = btn_x + (btn_w.saturating_sub(text_w)) / 2;
    let text_y = btn_y + (btn_h.saturating_sub(theme.font_size)) / 2;
    draw_text(fb, text_x, text_y, text, theme.btn_text, theme.btn_bg);
}

fn handle_quick_settings_click(px: u32, py: u32, x0: u32, y0: u32) -> bool {
    let mut y = y0 + 44;

    // Brightness slider hit test
    let bar_w = POPUP_W - 2 * PAD - 4;
    let bar_x = x0 + PAD + 2;
    let bar_y = y + 22;
    if py >= bar_y - 4 && py < bar_y + 18 && px >= bar_x && px <= bar_x + bar_w {
        unsafe {
            BRIGHTNESS_SLIDER.dragging = true;
            let val = ((px.saturating_sub(bar_x)) * 100 / bar_w).min(100);
            BRIGHTNESS_SLIDER.value = val;
        }
        crate::wm::compositor::mark_full_redraw();
        return true;
    }
    y += 50;

    // Volume slider hit test
    let bar_y = y + 22;
    if py >= bar_y - 4 && py < bar_y + 18 && px >= bar_x && px <= bar_x + bar_w {
        unsafe {
            VOLUME_SLIDER.dragging = true;
            let val = ((px.saturating_sub(bar_x)) * 100 / bar_w).min(100);
            VOLUME_SLIDER.value = val;
        }
        crate::wm::compositor::mark_full_redraw();
        return true;
    }
    y += 50;

    // Network toggle hit test
    let toggle_x = x0 + POPUP_W - PAD - 50;
    let toggle_y = y;
    let track_w = 44;
    let track_h = 24;
    if px >= toggle_x && px < toggle_x + track_w && py >= toggle_y && py < toggle_y + track_h {
        unsafe {
            NETWORK_ENABLED = !NETWORK_ENABLED;
        }
        crate::wm::compositor::mark_full_redraw();
        crate::wm::notifications::info(
            "Network",
            if unsafe { NETWORK_ENABLED } {
                "Enabled"
            } else {
                "Disabled"
            },
        );
        return true;
    }
    y += 40;

    // Theme toggle hit test
    let toggle_y = y;
    if px >= toggle_x && px < toggle_x + track_w && py >= toggle_y && py < toggle_y + track_h {
        crate::wm::theme::toggle();
        crate::wm::compositor::mark_full_redraw();
        return true;
    }
    y += 40;

    // Clear notifications button
    let btn_x = x0 + PAD;
    let btn_y = y;
    let btn_w = POPUP_W - 2 * PAD;
    let btn_h = 32;
    if px >= btn_x && px < btn_x + btn_w && py >= btn_y && py < btn_y + btn_h {
        notifications::clear_history();
        crate::wm::notifications::info("Notifications", "History cleared");
        crate::wm::compositor::mark_full_redraw();
        return true;
    }

    true
}

fn handle_quick_settings_drag(px: u32, py: u32, x0: u32, y0: u32) -> bool {
    let bar_w = POPUP_W - 2 * PAD - 4;
    let bar_x = x0 + PAD + 2;

    unsafe {
        if BRIGHTNESS_SLIDER.dragging {
            let bar_y = y0 + 44 + 22;
            if py >= bar_y - 4 && py < bar_y + 18 {
                let val = ((px.saturating_sub(bar_x)) * 100 / bar_w).min(100);
                BRIGHTNESS_SLIDER.value = val;
                crate::wm::compositor::mark_full_redraw();
                return true;
            }
        }
        if VOLUME_SLIDER.dragging {
            let bar_y = y0 + 44 + 50 + 22;
            if py >= bar_y - 4 && py < bar_y + 18 {
                let val = ((px.saturating_sub(bar_x)) * 100 / bar_w).min(100);
                VOLUME_SLIDER.value = val;
                crate::wm::compositor::mark_full_redraw();
                return true;
            }
        }
    }
    false
}

fn draw_clock(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y0: u32,
) {
    header(fb, theme, x0, y0, "Clock & Notifications");

    // Big time (uptime-based)
    let now_ms = crate::arch::ArchTimer::uptime_ms();
    let secs = now_ms / 1000;
    let time_str = alloc::format!(
        "{:02}:{:02}:{:02}",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    );
    draw_text(
        fb,
        x0 + PAD + 2,
        y0 + 44,
        &time_str,
        theme.label_text,
        theme.menu_bg,
    );
    let up_str = alloc::format!("Uptime: {}h {}m", secs / 3600, (secs / 60) % 60);
    draw_text(
        fb,
        x0 + PAD + 2,
        y0 + 66,
        &up_str,
        theme.label_text_dim,
        theme.menu_bg,
    );

    // Notification history (most recent first)
    let count = notifications::history_count();
    let label = if count == 0 {
        alloc::string::String::from("No notifications")
    } else {
        alloc::format!("Recent ({}):", count)
    };
    draw_text(
        fb,
        x0 + PAD + 2,
        y0 + 92,
        &label,
        theme.label_text_dim,
        theme.menu_bg,
    );

    let mut y = y0 + 114;
    for i in 0..count.min(6) {
        if let Some(n) = notifications::history(i) {
            // Accent dot
            draw_rect(fb, x0 + PAD + 2, y + 3, 4, 4, theme.btn_bg_hover);
            draw_text(
                fb,
                x0 + PAD + 12,
                y,
                n.title_str(),
                theme.label_text,
                theme.menu_bg,
            );
            draw_text(
                fb,
                x0 + PAD + 12,
                y + 16,
                n.body_str(),
                theme.label_text_dim,
                theme.menu_bg,
            );
            y += 42;
            if y + 40 > y0 + POPUP_H {
                break;
            }
        }
    }
}

fn draw_network(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y0: u32,
) {
    header(fb, theme, x0, y0, "Network");

    #[cfg(target_arch = "aarch64")]
    {
        let cfg = crate::net::config();
        let state = if cfg.ip_addr != [0, 0, 0, 0] {
            "Connected"
        } else {
            "No link"
        };
        draw_text(
            fb,
            x0 + PAD + 2,
            y0 + 44,
            state,
            theme.label_text,
            theme.menu_bg,
        );

        if cfg.ip_addr != [0, 0, 0, 0] {
            let ip = alloc::format!(
                "IP: {}.{}.{}.{}",
                cfg.ip_addr[0],
                cfg.ip_addr[1],
                cfg.ip_addr[2],
                cfg.ip_addr[3]
            );
            draw_text(
                fb,
                x0 + PAD + 2,
                y0 + 68,
                &ip,
                theme.label_text_dim,
                theme.menu_bg,
            );
            let gw = alloc::format!(
                "GW: {}.{}.{}.{}",
                cfg.gateway[0],
                cfg.gateway[1],
                cfg.gateway[2],
                cfg.gateway[3]
            );
            draw_text(
                fb,
                x0 + PAD + 2,
                y0 + 88,
                &gw,
                theme.label_text_dim,
                theme.menu_bg,
            );
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    draw_text(
        fb,
        x0 + PAD + 2,
        y0 + 44,
        "aarch64 only",
        theme.label_text,
        theme.menu_bg,
    );
}

fn bar(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y: u32,
    pct: u32,
    label: &str,
) {
    draw_text(fb, x0 + PAD + 2, y, label, theme.label_text, theme.menu_bg);
    let bar_w = POPUP_W - 2 * PAD - 4;
    draw_rect(fb, x0 + PAD + 2, y + 22, bar_w, 10, theme.menu_border);
    let fill = bar_w * pct / 100;
    draw_rect(
        fb,
        x0 + PAD + 2,
        y + 22,
        fill,
        10,
        crate::wm::theme::accent_color(),
    );
    draw_text(
        fb,
        x0 + PAD + 2,
        y + 40,
        &alloc::format!("{}%", pct),
        theme.label_text_dim,
        theme.menu_bg,
    );
}

fn draw_volume(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y0: u32,
) {
    header(fb, theme, x0, y0, "Volume");
    bar(fb, theme, x0, y0 + 44, 80, "Output");
}

fn draw_battery(
    fb: &crate::display::fb::FramebufferInfo,
    theme: &crate::wm::theme::Theme,
    x0: u32,
    y0: u32,
) {
    header(fb, theme, x0, y0, "Power");
    bar(fb, theme, x0, y0 + 44, 100, "Battery (AC power)");
}
