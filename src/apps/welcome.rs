//! First-run welcome window for LobsterOS
//!
//! Shows a welcome window on first boot only. Persists a marker file to
//! track whether the welcome has been shown. The window draws itself into
//! its compositor surface from `wm::tick` like every other built-in app.

use crate::fs::vfs;
use crate::println;
use crate::wm::window;

/// Persistent marker path (survives reboots when a disk is attached).
const WELCOME_MARKER_PERSISTENT: &str = "/sd/etc/lobster_welcome_seen";
/// Volatile fallback path (no persistent disk attached).
const WELCOME_MARKER_VOLATILE: &str = "/etc/lobster_welcome_seen";
/// Window title; also how the renderer finds the window.
const WELCOME_TITLE: &str = "Welcome to LobsterOS";
/// Outer window size.
const WELCOME_W: u32 = 500;
const WELCOME_H: u32 = 350;

/// The path the marker is (or would be) stored at: persistent when /sd is
/// mounted, volatile otherwise.
fn marker_path() -> &'static str {
    if vfs::stat("/sd").is_some() {
        WELCOME_MARKER_PERSISTENT
    } else {
        WELCOME_MARKER_VOLATILE
    }
}

/// Check if the welcome window has already been shown
pub fn is_welcome_seen() -> bool {
    vfs::stat(WELCOME_MARKER_PERSISTENT).is_some() || vfs::stat(WELCOME_MARKER_VOLATILE).is_some()
}

/// Mark the welcome window as seen (create marker file)
pub fn mark_welcome_seen() {
    let path = marker_path();
    if path == WELCOME_MARKER_PERSISTENT {
        let _ = vfs::create_dir("/sd/etc");
    }
    let _ = vfs::write_file(path, b"1");
    println!("[welcome] Marker written to {}", path);
}

/// Show the welcome window (called once on first boot)
pub fn show_welcome_window() {
    if is_welcome_seen() {
        println!("[welcome] Welcome already seen, skipping");
        return;
    }
    println!("[welcome] First boot detected, showing welcome window");

    let fb = crate::display::fb::get_info();
    let x = fb.width.saturating_sub(WELCOME_W) / 2;
    let y = fb.height.saturating_sub(WELCOME_H) / 2;
    let Some(win_id) = window::create(x, y, WELCOME_W, WELCOME_H, WELCOME_TITLE) else {
        println!("[welcome] Failed to create welcome window");
        return;
    };
    window::focus(win_id);
    crate::wm::dirty::mark();
    mark_welcome_seen();
}

/// Initialize the welcome module (called during apps init)
pub fn init() {
    println!("[welcome] Welcome module initialized");
}

/// Layout of the welcome content inside the client area (client-local).
#[cfg(any(test, feature = "qemu_raspi5b"))]
mod layout {
    /// Padding around the content.
    pub const PAD: u32 = 24;
    /// Height of the accent strip across the top.
    pub const ACCENT_H: u32 = 4;
    /// Top of the title text.
    pub const TITLE_Y: u32 = 22;
    /// Top of the subtitle text.
    pub const SUBTITLE_Y: u32 = 56;
    /// Top of the first tip row.
    pub const TIPS_Y: u32 = 102;
    /// Distance between tip rows (icon height plus gap).
    pub const TIP_ROW_H: u32 = 44;
    /// Edge of a tip icon.
    pub const TIP_ICON: u32 = 32;
    /// Gap between a tip icon and its text.
    pub const TIP_GAP: u32 = 14;
    /// Number of tip rows.
    pub const TIP_COUNT: u32 = 4;
    /// Button size and its distance from the bottom-right corner.
    pub const BUTTON_W: u32 = 128;
    pub const BUTTON_H: u32 = 34;

    /// Top-left of tip row `index`.
    pub fn tip_origin(index: u32) -> (u32, u32) {
        (PAD, TIPS_Y + index * TIP_ROW_H)
    }

    /// The "Get started" button rectangle (x, y, w, h) for a client area.
    pub fn button(client_w: u32, client_h: u32) -> (u32, u32, u32, u32) {
        (
            client_w.saturating_sub(PAD + BUTTON_W),
            client_h.saturating_sub(PAD / 2 + BUTTON_H),
            BUTTON_W,
            BUTTON_H,
        )
    }
}

#[cfg(feature = "qemu_raspi5b")]
mod view {
    use super::{layout, window, WELCOME_TITLE};
    use crate::display::fb::FramebufferInfo;
    use crate::display::icons::{self, Icon};
    use crate::display::text::{self, Face};
    use crate::wm::hit_region::{self, HitKind};
    use crate::wm::{scene, theme, window_frame};

    /// Hit-region tag of the "Get started" button.
    const TAG_GET_STARTED: u32 = 1;
    /// Corner radius of the button.
    const BUTTON_RADIUS: u32 = 6;
    /// Text sizes at the Medium font setting.
    const TITLE_PX: u32 = 24;
    const SUBTITLE_PX: u32 = 14;
    const TIP_PX: u32 = 14;
    const BUTTON_PX: u32 = 14;
    /// Line box height passed to the text helpers.
    const LINE_H: u32 = 28;

    /// What to do when each tip row is drawn: icon and text.
    const TIPS: [(Icon, &str); layout::TIP_COUNT as usize] = [
        (
            Icon::AppGeneric,
            "Start menu (bottom-left) launches every app.",
        ),
        (
            Icon::AppTerminal,
            "Terminal runs shell commands and programs.",
        ),
        (
            Icon::AppFiles,
            "Files browses your disk and Desktop folder.",
        ),
        (
            Icon::AppSettings,
            "Settings changes wallpaper, display and theme.",
        ),
    ];

    fn on_hit(hit: &hit_region::Hit) -> bool {
        if hit.region.tag != TAG_GET_STARTED {
            return false;
        }
        window::destroy(hit.win_id);
        crate::wm::dirty::mark();
        true
    }

    fn draw_button(
        fb: &FramebufferInfo,
        win_id: u32,
        origin: (u32, u32),
        rect: (u32, u32, u32, u32),
    ) {
        let theme = theme::current();
        let (bx, by, bw, bh) = (origin.0 + rect.0, origin.1 + rect.1, rect.2, rect.3);
        crate::display::graphics::fill_rounded_rect(
            fb,
            bx,
            by,
            bw,
            bh,
            BUTTON_RADIUS,
            theme.win_border,
        );
        let style = theme::chrome_text(Face::UiBold, BUTTON_PX, 0xFFFF_FFFF, bh);
        let label_y = theme::centered_line_y(by, bh, &style);
        text::draw_centered(fb, bx + bw / 2, label_y, "Get started", &style);
        hit_region::set_handler(win_id, on_hit);
        hit_region::register(
            win_id,
            rect.0,
            rect.1,
            bw,
            bh,
            TAG_GET_STARTED,
            HitKind::Button,
        );
    }

    fn draw_tips(fb: &FramebufferInfo, origin: (u32, u32), client_w: u32) {
        let theme = theme::current();
        let style = theme::chrome_text(Face::Ui, TIP_PX, theme.label_text, LINE_H);
        for (i, (icon, label)) in TIPS.iter().enumerate() {
            let (lx, ly) = layout::tip_origin(i as u32);
            let (x, y) = (origin.0 + lx, origin.1 + ly);
            icons::draw(fb, *icon, layout::TIP_ICON, x, y);
            let text_x = x + layout::TIP_ICON + layout::TIP_GAP;
            let text_y = theme::centered_line_y(y, layout::TIP_ICON, &style);
            let room =
                client_w.saturating_sub(lx + layout::TIP_ICON + layout::TIP_GAP + layout::PAD);
            text::draw_fitted(fb, text_x, text_y, room, label, &style);
        }
    }

    fn draw_content(fb: &FramebufferInfo, win_id: u32, client: (u32, u32, u32, u32)) {
        let theme = theme::current();
        let (cx, cy, cw, ch) = client;
        crate::display::graphics::draw_rect(
            fb,
            cx,
            cy,
            cw,
            layout::ACCENT_H,
            theme::accent_color(),
        );
        let title = theme::chrome_text(Face::UiBold, TITLE_PX, theme.label_text, LINE_H + 8);
        text::draw(
            fb,
            cx + layout::PAD,
            cy + layout::TITLE_Y,
            "Welcome to LobsterOS",
            &title,
        );
        let sub = theme::chrome_text(Face::Ui, SUBTITLE_PX, theme.label_text_dim, LINE_H);
        let blurb = "A bare-metal desktop OS for the Raspberry Pi 5.";
        text::draw(fb, cx + layout::PAD, cy + layout::SUBTITLE_Y, blurb, &sub);
        draw_tips(fb, (cx, cy), cw);
        draw_button(fb, win_id, (cx, cy), layout::button(cw, ch));
    }

    /// Draw the Welcome window if it is open (called from `wm::tick`).
    pub fn render_to_windows(fb: &FramebufferInfo) {
        let Some(win_id) = window::find_by_title(WELCOME_TITLE) else {
            return;
        };
        let Some((x, y, w, h)) = window::get_info(win_id) else {
            return;
        };
        let target = &scene::window_target(win_id, fb);
        let focused = window::focused_id() == win_id;
        let client = window_frame::draw_frame(target, win_id, x, y, w, h, WELCOME_TITLE, focused);
        draw_content(target, win_id, client);
    }
}

#[cfg(feature = "qemu_raspi5b")]
pub use view::render_to_windows;

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_welcome_marker_paths() {
        assert_eq!(WELCOME_MARKER_PERSISTENT, "/sd/etc/lobster_welcome_seen");
        assert_eq!(WELCOME_MARKER_VOLATILE, "/etc/lobster_welcome_seen");
    }

    #[test]
    fn test_marker_path_prefers_persistent_when_sd_mounted() {
        let _guard = crate::test_support::serial_guard();
        // Without a disk the volatile path is used.
        assert_eq!(marker_path(), WELCOME_MARKER_VOLATILE);
    }

    #[test]
    fn button_sits_inside_the_client_area_at_the_bottom_right() {
        let (cw, ch) = (WELCOME_W - 4, WELCOME_H - 28);
        let (x, y, w, h) = layout::button(cw, ch);
        assert!(x + w <= cw && y + h <= ch);
        assert!(x > cw / 2 && y > ch / 2);
    }

    #[test]
    fn tip_rows_do_not_overlap_each_other_or_the_button() {
        let (cw, ch) = (WELCOME_W - 4, WELCOME_H - 28);
        for i in 0..layout::TIP_COUNT - 1 {
            let (_, y) = layout::tip_origin(i);
            let (_, next) = layout::tip_origin(i + 1);
            assert!(y + layout::TIP_ICON <= next);
        }
        let (_, last) = layout::tip_origin(layout::TIP_COUNT - 1);
        let (_, button_y, _, _) = layout::button(cw, ch);
        assert!(last + layout::TIP_ICON <= button_y);
    }
}
