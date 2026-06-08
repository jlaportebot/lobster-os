//! Window Manager for LobsterOS
//!
//! Provides a tiling+floating hybrid window manager with:
//! - Compositor: double-buffered scene composition
//! - Window management: create, destroy, move, resize, focus
//! - Theme engine: colors, fonts, spacing, rounded corners
//! - Widget toolkit: buttons, labels, text inputs, scrollbars, panels
//! - Desktop Panel: taskbar, app launcher, system tray, clock
//! - Application Launcher: .desktop file parsing and app launching
//! - Input bridge: routes USB HID keyboard/mouse events to windows
//!
//! All rendering goes through `display::graphics` primitives.

pub mod compositor;
pub mod input;
pub mod launcher;
pub mod panel;
pub mod theme;
pub mod widget;
pub mod window;

use crate::display::fb::get_info;
use crate::println;

/// Initialize the window manager
pub fn init() {
    let fb = get_info();
    if fb.width == 0 {
        println!("[wm] No framebuffer — window manager disabled");
        return;
    }
    compositor::init(fb);
    window::init();
    theme::init();
    input::init();
    panel::init(&fb);
    println!(
        "[wm] Window manager initialized ({}x{})",
        fb.width, fb.height
    );
}

/// Run one frame of the window manager: poll input, update, composite
pub fn tick() {
    // Poll HID events and dispatch through the input bridge
    input::pump_events();
    // Composite all windows
    compositor::composite();
    // Draw panel on top
    let fb = get_info();
    if fb.width > 0 {
        panel::draw(&fb);
    }
}
