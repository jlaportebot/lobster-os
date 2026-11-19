//! Switching the display mode, shared by the Display page and the revert
//! timer.

use super::layout::initial_window;
use crate::wm::window;

/// Current mode `(width, height)`.
#[must_use]
pub fn current() -> (u32, u32) {
    let info = crate::display::fb::get_info();
    (info.width, info.height)
}

/// Keep at least this much of a window on screen after a mode change.
const MIN_VISIBLE: u32 = 64;

/// Switch to `width` x `height` with the same calls as the `resolution`
/// shell command, then pull windows back on screen.
pub fn apply(width: u32, height: u32) -> Result<(), &'static str> {
    #[cfg(target_arch = "aarch64")]
    {
        let info = crate::display::fb::init(width, height, 32).map_err(|_| "mode rejected")?;
        crate::display::console::init(info);
        crate::wm::wallpaper::paint_to_screen();
        fit_windows(info.width, info.height);
        crate::wm::dirty::mark();
        Ok(())
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (width, height);
        Err("display mode switching needs the kernel")
    }
}

#[cfg(target_arch = "aarch64")]
fn fit_windows(screen_w: u32, screen_h: u32) {
    for id in 1..=window::max_id() {
        if !window::exists(id) {
            continue;
        }
        if window::get_title(id) == super::TITLE {
            refit_settings(id, screen_w, screen_h);
        } else if let Some((x, y, _, _)) = window::get_info(id) {
            let nx = x.min(screen_w.saturating_sub(MIN_VISIBLE));
            let ny = y.min(screen_h.saturating_sub(MIN_VISIBLE));
            if (nx, ny) != (x, y) {
                window::move_window(id, nx, ny);
            }
        }
    }
}

/// Keep the Settings window fully visible: shrink it if the screen got
/// smaller and re-centre it when it would fall off.
#[cfg(target_arch = "aarch64")]
fn refit_settings(id: u32, screen_w: u32, screen_h: u32) {
    let Some((x, y, w, h)) = window::get_info(id) else {
        return;
    };
    let panel_h = crate::wm::panel::panel_height();
    let off_screen = x + w > screen_w || y + h > screen_h.saturating_sub(panel_h);
    if !off_screen {
        return;
    }
    let (nx, ny, nw, nh) = initial_window(screen_w, screen_h, panel_h);
    window::resize(id, nw.min(w), nh.min(h));
    window::move_window(id, nx, ny);
}
