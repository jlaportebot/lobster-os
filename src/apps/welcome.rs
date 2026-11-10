//! First-run welcome window for LobsterOS
//!
//! Shows a welcome window on first boot only. Persists a marker file
//! to track whether the welcome has been shown.

use crate::fs::vfs;
use crate::println;
use crate::wm::window;

const WELCOME_MARKER_PATH: &str = "/etc/lobster_welcome_seen";

/// Check if the welcome window has already been shown
pub fn is_welcome_seen() -> bool {
    vfs::stat(WELCOME_MARKER_PATH).is_some()
}

/// Mark the welcome window as seen (create marker file)
pub fn mark_welcome_seen() {
    let _ = vfs::write_file(WELCOME_MARKER_PATH, b"1");
    println!("[welcome] Marker written to {}", WELCOME_MARKER_PATH);
}

/// Show the welcome window (called once on first boot)
pub fn show_welcome_window() {
    if is_welcome_seen() {
        println!("[welcome] Welcome already seen, skipping");
        return;
    }

    println!("[welcome] First boot detected, showing welcome window");

    // Create a welcome window
    let fb = crate::display::fb::get_info();
    let (win_w, win_h) = (500, 350);
    let x = (fb.width.saturating_sub(win_w)) / 2;
    let y = (fb.height.saturating_sub(win_h)) / 2;

    let win_id = window::create(x, y, win_w, win_h, "Welcome to LobsterOS");
    if win_id.is_none() {
        println!("[welcome] Failed to create welcome window");
        return;
    }
    let win_id = win_id.unwrap();

    // Render welcome content using with_window_mut
    window::with_window_mut(win_id, |win| {
        let buf_ptr = win.content.as_ptr();
        if !buf_ptr.is_null() && win.content_size > 0 {
            render_welcome_content(buf_ptr, win.client_w, win.client_h);
        }
    });

    // Focus the welcome window
    window::focus(win_id);
    crate::wm::dirty::mark();

    // Mark as seen so it doesn't show again
    mark_welcome_seen();
}

/// Render welcome content to the window's content buffer
fn render_welcome_content(buf_ptr: *mut u32, width: u32, height: u32) {
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr, (width * height) as usize) };

    // Clear with theme background
    let bg_color = 0xFF_1E_1E_2E; // Dark background
    for p in buf.iter_mut() {
        *p = bg_color;
    }

    // Draw a simple welcome message using pixel manipulation
    let accent = crate::wm::theme::accent_color();

    // Draw a decorative accent bar at the top
    for y in 0..4 {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            if idx < buf.len() {
                buf[idx] = accent;
            }
        }
    }

    // Draw text placeholders (colored rectangles for each line)
    draw_text_placeholder(buf, width, height, "Welcome to LobsterOS!", 60, accent);
    draw_text_placeholder(
        buf,
        width,
        height,
        "AArch64 Bare-Metal OS for Raspberry Pi 5",
        100,
        0xFF_A6_AD_C8,
    );
    draw_text_placeholder(buf, width, height, "", 140, 0xFF_6C_70_86);
    draw_text_placeholder(buf, width, height, "Desktop Features:", 160, 0xFF_F9_E2_AF);
    draw_text_placeholder(
        buf,
        width,
        height,
        "  • Window Manager with compositor",
        190,
        0xFF_CD_D6_F4,
    );
    draw_text_placeholder(
        buf,
        width,
        height,
        "  • Panel with start menu & system tray",
        210,
        0xFF_CD_D6_F4,
    );
    draw_text_placeholder(
        buf,
        width,
        height,
        "  • Desktop icons (left column grid)",
        230,
        0xFF_CD_D6_F4,
    );
    draw_text_placeholder(
        buf,
        width,
        height,
        "  • 12 built-in apps (Terminal, Browser, etc.)",
        250,
        0xFF_CD_D6_F4,
    );
    draw_text_placeholder(buf, width, height, "", 280, 0xFF_6C_70_86);
    draw_text_placeholder(
        buf,
        width,
        height,
        "Press any key or click to continue...",
        300,
        0xFF_94_E2_D5,
    );
}

/// Simple text placeholder drawing - draws a colored rectangle for each line
fn draw_text_placeholder(buf: &mut [u32], width: u32, height: u32, text: &str, y: u32, color: u32) {
    if text.is_empty() {
        return;
    }

    let char_w = 8;
    let char_h = 16;
    let text_w = (text.len() as u32 * char_w).min(width.saturating_sub(16));
    let start_x = (width.saturating_sub(text_w)) / 2;

    let rect_h = char_h;
    let rect_y = y.min(height.saturating_sub(rect_h));
    for dy in 0..rect_h {
        for dx in 0..text_w {
            let px = start_x + dx;
            let py = rect_y + dy;
            if px < width && py < height {
                let idx = (py * width + px) as usize;
                if idx < buf.len() {
                    // Draw a simple pattern
                    if (dx + dy) % 4 < 2 {
                        buf[idx] = color;
                    }
                }
            }
        }
    }
}

/// Handle welcome window input (key/mouse) - close on any interaction
pub fn handle_welcome_input(win_id: u32) -> bool {
    if window::get_title(win_id) == "Welcome to LobsterOS" {
        // Close the welcome window on any input
        window::destroy(win_id);
        true
    } else {
        false
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_welcome_marker_path() {
        assert_eq!(WELCOME_MARKER_PATH, "/etc/lobster_welcome_seen");
    }
}

/// Initialize the welcome module (called during apps init)
pub fn init() {
    println!("[welcome] Welcome module initialized");
}
