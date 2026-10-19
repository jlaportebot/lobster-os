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
pub mod alt_tab;
pub mod context_menu;
pub mod desktop_icons;
pub mod gui_shell;
pub mod start_menu;
pub mod notifications;
pub mod wallpaper;
pub mod input;
pub mod launcher;
pub mod panel;
pub mod theme;
pub mod tray_popups;
pub mod widget;
pub mod window;
pub mod window_frame;
pub mod workspace;

use crate::display::fb::get_info;
use crate::println;

/// Initialize the window manager
pub fn init() {
    let fb = get_info();
    if fb.width == 0 {
        println!("[wm] No framebuffer — window manager disabled");
        return;
    }
    println!(
        "[wm] FB info: {}x{} @ {}bpp, virt={:#018x}",
        fb.width, fb.height, fb.bpp, fb.virt_addr
    );
    println!("[wm] Step 1: compositor::init...");
    compositor::init(fb);
    println!("[wm] Step 2: window::init...");
    window::init();
    println!("[wm] Step 3: theme::init...");
    theme::init();
    println!("[wm] Step 4: workspace::init...");
    workspace::init();
    println!("[wm] Step 5: input::init...");
    input::init();
    println!("[wm] Step 6: panel::init...");
    panel::init(&fb);
    println!("[wm] Step 6b: desktop_icons::init...");
    desktop_icons::init();
    println!(
        "[wm] Window manager initialized ({}x{})",
        fb.width, fb.height
    );
}

/// Run one frame of the window manager: poll input, update, composite
pub fn tick() {
    // Poll HID events and dispatch through the input bridge
    input::pump_events();

    // Composite all windows (draws frames, wallpaper, blits content buffers)
    compositor::composite();

    // Render terminal content directly to framebuffer in window client areas (QEMU only)
    #[cfg(feature = "qemu_raspi5b")]
    {
        let fb = get_info();
        if fb.width > 0 {
            crate::apps::terminal::render_to_windows(&fb);
            // Render other apps directly to their window client areas
            crate::apps::sysmon::render_to_windows(&fb);
            crate::apps::calculator::render_to_windows(&fb);
            crate::apps::file_manager::render_to_windows(&fb);
            crate::apps::process_manager::render_to_windows(&fb);
            crate::apps::browser::render_to_windows(&fb);
            crate::apps::editor::render_to_windows(&fb);
            crate::apps::notes::render_to_windows(&fb);
            crate::apps::settings::render_settings_window(&fb);
        }
    }

    // Draw desktop icons (after windows, before panel)
    let fb = get_info();
    if fb.width > 0 {
        desktop_icons::draw(&fb);
    }

    // Draw panel on top
    let fb = get_info();
    if fb.width > 0 {
        panel::draw(&fb);
    }
    // Window animations tick
    window_frame::tick_animations();

    // Notifications tick (animate/expire toasts)
    notifications::tick();

    // Start menu (above windows, below context menu)
    start_menu::draw(&fb);

    // Tray popups (above windows, below toasts)
    tray_popups::draw(&fb);

    // Alt-Tab switcher overlay (above nearly everything)
    alt_tab::draw(&fb);

    // Toasts (above windows, top-right)
    notifications::draw(&fb);

    // Draw context menu on top of everything (if visible)
    context_menu::draw();
}

/// Handle keyboard shortcuts (called from input module)
pub fn handle_shortcut(scancode: u8) {
    use crate::usb::hid;
    use crate::wm::window::SnapPosition;

    // Mod + key shortcuts
    let mod_key = hid::gui_held();

    if mod_key {
        match scancode {
            // Super+D: minimize all / show desktop
            0x07 => {
                let count = window::count();
                for id in 1..=count as u32 {
                    if window::exists(id) {
                        window::minimize(id);
                    }
                }
            }
            // Super+Left: snap focused window left
            0x50 => {
                let fid = window::focused_id();
                if fid != 0 {
                    window::snap(fid, SnapPosition::Left);
                }
            }
            // Super+Right: snap focused window right
            0x4F => {
                let fid = window::focused_id();
                if fid != 0 {
                    window::snap(fid, SnapPosition::Right);
                }
            }
            // Super+Up: snap focused window top half
            0x52 => {
                let fid = window::focused_id();
                if fid != 0 {
                    window::snap(fid, SnapPosition::Full);
                }
            }
            // Super+Down: minimize focused window or restore from snapped
            0x51 => {
                let fid = window::focused_id();
                if fid != 0 {
                    window::minimize(fid);
                }
            }
            // Ctrl+Alt+1..4: switch workspace
            _ if hid::alt_held() => match scancode {
                0x1E => workspace::switch_to(0), // 1
                0x1F => workspace::switch_to(1), // 2
                0x20 => workspace::switch_to(2), // 3
                0x21 => workspace::switch_to(3), // 4
                _ => {}
            },
            _ => {}
        }
    }

    // Alt+F4: close focused window
    if hid::alt_held() && scancode == 0x29 {
        let fid = window::focused_id();
        if fid != 0 {
            window::destroy(fid);
        }
    }
}
