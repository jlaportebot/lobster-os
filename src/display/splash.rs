//! Boot splash screen for LobsterOS
//!
//! Shows a graphical boot logo during kernel initialization,
//! then transitions smoothly to the desktop when the WM is ready.

use crate::display::fb;
use crate::display::graphics;
use crate::wm::theme;

/// Splash screen state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplashState {
    /// Not started
    Inactive,
    /// Showing splash screen
    Active,
    /// Transitioning to desktop (fade out)
    FadingOut,
    /// Done — desktop visible
    Done,
}

static mut SPLASH_STATE: SplashState = SplashState::Inactive;
static mut SPLASH_TICKS: u64 = 0;
static mut BOOT_PROGRESS: u32 = 0;
static mut BOOT_MESSAGES: u64 = 0;

/// Total boot steps for progress bar
pub const BOOT_STEPS: u32 = 10;

/// Initialize and show the boot splash
pub fn init() {
    unsafe {
        SPLASH_STATE = SplashState::Active;
        SPLASH_TICKS = 0;
        BOOT_PROGRESS = 0;
        BOOT_MESSAGES = 0;
    }

    let info = fb::get_info();
    let (w, h) = (info.width, info.height);
    let bg = theme::current().desktop_bg;

    // Fill screen with desktop background
    graphics::fill_rect(&info, 0, 0, w, h, bg);

    // Draw the LobsterOS logo (centered)
    draw_logo(&info, w, h);

    // Draw boot progress bar background
    let bar_w = w * 2 / 3;
    let bar_h = 4;
    let bar_x = (w - bar_w) / 2;
    let bar_y = h * 3 / 4;
    graphics::fill_rect(&info, bar_x, bar_y, bar_w, bar_h, 0xFF2A3A4E);
    graphics::draw_text(
        &info,
        10,
        bar_y - 3,
        "Starting LobsterOS...",
        0xFF808080,
        bg,
    );

    crate::println!("[splash] Boot splash initialized {}x{}", w, h);
}

/// Draw the LobsterOS logo — a stylized lobster silhouette
fn draw_logo(info: &fb::FramebufferInfo, w: u32, h: u32) {
    let cx = w / 2;
    let cy = h / 2 - 20;
    let accent = theme::accent().primary();

    // Simple geometric logo: a rounded rectangle body + claws
    // Body
    graphics::fill_rect(info, cx - 40, cy - 20, 80, 40, accent);
    // Left claw
    graphics::fill_rect(info, cx - 60, cy - 10, 20, 20, accent);
    // Right claw
    graphics::fill_rect(info, cx + 40, cy - 10, 20, 20, accent);

    // "LobsterOS" text below
    graphics::draw_text_centered(
        info,
        cx,
        cy + 30,
        "LobsterOS",
        0xFFE0E0E0,
        theme::current().desktop_bg,
    );
    graphics::draw_text_centered(
        info,
        cx,
        cy + 50,
        "v0.2.0",
        0xFF808080,
        theme::current().desktop_bg,
    );
}

/// Update boot progress (called from boot sequence)
pub fn update_progress(step: u32, _msg: &str) {
    unsafe {
        BOOT_PROGRESS = step;
        BOOT_MESSAGES += 1;
    }

    if unsafe { SPLASH_STATE } != SplashState::Active {
        return;
    }

    let info = fb::get_info();
    let (w, h) = (info.width, info.height);
    let bar_w = w * 2 / 3;
    let bar_h = 4;
    let bar_x = (w - bar_w) / 2;
    let bar_y = h * 3 / 4;
    let accent = theme::accent().primary();

    // Clear progress bar background
    graphics::fill_rect(&info, bar_x, bar_y, bar_w, bar_h, 0xFF2A3A4E);
    // Draw progress
    let progress_w = (bar_w * step) / BOOT_STEPS;
    graphics::fill_rect(&info, bar_x, bar_y, progress_w, bar_h, accent);
}

/// Tick the splash screen (called from main loop)
pub fn tick() {
    if unsafe { SPLASH_STATE } != SplashState::Active {
        return;
    }

    unsafe {
        SPLASH_TICKS += 1;
    }

    // After 100 ticks with full progress, start fade out
    if unsafe { SPLASH_TICKS } > 100 && unsafe { BOOT_PROGRESS } >= BOOT_STEPS {
        transition_to_desktop();
    }
}

/// Transition from splash to desktop
pub fn transition_to_desktop() {
    unsafe {
        SPLASH_STATE = SplashState::FadingOut;
    }

    // Mark the compositor for full redraw
    crate::wm::compositor::mark_full_redraw();

    // In a full implementation, we'd do a smooth fade:
    // 1. Capture desktop frame
    // 2. Blend splash → desktop over N frames
    // 3. Switch to desktop
    // For now, instant transition
    crate::println!("[splash] Transitioning to desktop");

    unsafe {
        SPLASH_STATE = SplashState::Done;
    }

    // Paint the wallpaper directly to the framebuffer (QEMU path skips the
    // compositor's back-buffer blit, so the engine writes the FB itself).
    crate::wm::wallpaper::paint_to_screen();
}

/// Check if splash is still active
pub fn is_active() -> bool {
    unsafe { SPLASH_STATE == SplashState::Active || SPLASH_STATE == SplashState::FadingOut }
}

/// Check if splash is done
pub fn is_done() -> bool {
    unsafe { SPLASH_STATE == SplashState::Done }
}

/// Get current boot progress (0-100%)
pub fn progress_percent() -> u32 {
    unsafe { (BOOT_PROGRESS * 100) / BOOT_STEPS }
}
