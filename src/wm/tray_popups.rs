//! Tray popups — panels that open from the taskbar tray
//!
//! - Clock popup: calendar-ish grid (uptime-based date approximation) +
//!   recent notifications (notification center).
//! - Network popup: interface state + IP.
//! - Volume/battery popup: level bars.
//! All popups render above the panel, below toasts, and close on outside click.

use crate::display::fb::get_info;
use crate::display::graphics::{draw_rect, draw_rect_outline, draw_text, fill_rounded_rect};
use crate::wm::notifications;
use crate::wm::panel;
use crate::wm::theme;

const POPUP_W: u32 = 320;
const POPUP_H: u32 = 380;
const PAD: u32 = 10;

#[derive(Clone, Copy, PartialEq)]
pub enum PopupKind {
    None,
    Clock,
    Network,
    Volume,
    Battery,
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
    }
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
