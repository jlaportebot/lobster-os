//! Display mode changes: one place that re-fits everything that remembers
//! screen geometry (windows, dialogs, the scene back buffer, popups).
//!
//! Both the `resolution` shell command and Settings call [`apply_mode`], so
//! they cannot drift apart.

use crate::wm::panel_layout::PANEL_HEIGHT;
use crate::wm::window_frame::TITLE_H;

/// Windows are never shrunk below this size when they are fitted.
const MIN_WINDOW_W: u32 = 240;
/// See [`MIN_WINDOW_W`].
const MIN_WINDOW_H: u32 = 160;

/// Outer window rectangle in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRect {
    /// Left edge.
    pub x: u32,
    /// Top edge (top of the title bar).
    pub y: u32,
    /// Width including borders.
    pub w: u32,
    /// Height including the title bar.
    pub h: u32,
}

/// Fit `rect` into a `screen_w` x `screen_h` screen whose bottom
/// `reserved_bottom` pixels belong to the taskbar.
///
/// The window shrinks when larger than the usable area, then moves so that
/// the whole window (and so its title bar) is on screen.
#[must_use]
pub fn clamp_window(
    rect: WindowRect,
    screen_w: u32,
    screen_h: u32,
    reserved_bottom: u32,
) -> WindowRect {
    let usable_h = screen_h.saturating_sub(reserved_bottom).max(TITLE_H);
    let w = rect.w.min(screen_w).max(MIN_WINDOW_W.min(screen_w));
    let h = rect.h.min(usable_h).max(MIN_WINDOW_H.min(usable_h));
    let x = rect.x.min(screen_w - w);
    let y = rect.y.min(usable_h - h);
    WindowRect { x, y, w, h }
}

/// Fit every window to the current screen size.
#[cfg(target_arch = "aarch64")]
fn fit_windows(screen_w: u32, screen_h: u32) {
    use crate::wm::window;
    for id in 1..=window::max_id() {
        let Some((x, y, w, h)) = window::get_info(id) else {
            continue;
        };
        let before = WindowRect { x, y, w, h };
        let after = clamp_window(before, screen_w, screen_h, PANEL_HEIGHT);
        if after.w != w || after.h != h {
            window::resize(id, after.w, after.h);
        }
        if (after.x, after.y) != (x, y) {
            window::move_window(id, after.x, after.y);
        }
    }
}

/// Switch the display to `width` x `height` and re-fit the desktop to it.
///
/// # Errors
/// Returns the framebuffer error when the mode is rejected; nothing else
/// changes in that case.
#[cfg(target_arch = "aarch64")]
pub fn apply_mode(
    width: u32,
    height: u32,
) -> Result<crate::display::fb::FramebufferInfo, crate::display::fb::FbError> {
    use crate::display::fb;
    let old = fb::device_info();
    let info = fb::init(width, height, 32)?;
    crate::display::console::init(info);
    crate::wm::start_menu::close();
    crate::wm::tray_popups::close();
    crate::wm::context_menu::hide();
    crate::wm::dialog::on_screen_resize((old.width, old.height), (info.width, info.height));
    #[cfg(feature = "qemu_raspi5b")]
    crate::wm::scene::init();
    fit_windows(info.width, info.height);
    crate::wm::wallpaper::paint_to_screen();
    crate::wm::compositor::mark_full_redraw();
    crate::wm::dirty::mark();
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: WindowRect = WindowRect {
        x: 710,
        y: 365,
        w: 500,
        h: 350,
    };

    #[test]
    fn window_inside_screen_is_untouched() {
        assert_eq!(clamp_window(R, 1920, 1080, PANEL_HEIGHT), R);
    }

    #[test]
    fn window_past_right_edge_moves_left() {
        let out = clamp_window(R, 800, 600, PANEL_HEIGHT);
        assert_eq!((out.x, out.w), (300, 500));
    }

    #[test]
    fn window_below_usable_area_moves_up_so_title_bar_is_visible() {
        let out = clamp_window(R, 1280, 720, PANEL_HEIGHT);
        assert_eq!(out.y, 720 - PANEL_HEIGHT - 350);
        assert!(out.y + TITLE_H <= 720 - PANEL_HEIGHT);
    }

    #[test]
    fn window_larger_than_screen_shrinks_and_sits_at_origin() {
        let big = WindowRect {
            x: 100,
            y: 100,
            w: 1600,
            h: 900,
        };
        let out = clamp_window(big, 800, 600, PANEL_HEIGHT);
        assert_eq!(out, WindowRect { x: 0, y: 0, w: 800, h: 600 - PANEL_HEIGHT });
    }

    #[test]
    fn clamp_is_idempotent() {
        let once = clamp_window(R, 800, 600, PANEL_HEIGHT);
        assert_eq!(clamp_window(once, 800, 600, PANEL_HEIGHT), once);
    }

    #[test]
    fn tiny_screen_never_panics() {
        let out = clamp_window(R, 100, 50, PANEL_HEIGHT);
        assert!(out.x + out.w <= 100);
    }
}
