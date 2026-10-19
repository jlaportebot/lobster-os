//! Window manager input bridge — routes USB HID events to the WM
//!
//! Connects the USB HID subsystem (keyboard/mouse) to the window manager,
//! dispatching keyboard input to the focused window and mouse events to
//! the window under the cursor.
//!
//! Event flow:
//!   USB xHCI → HID poll → InputEvent → WM dispatch → Window/Widget

use crate::println;
use crate::usb::hid::{self, HidEvent};
use crate::wm::compositor;
use crate::wm::panel;

// ─── Cursor State ──────────────────────────────────────────────────

/// Current mouse cursor position (pixels from top-left)
static mut CURSOR_X: u32 = 512;
static mut CURSOR_Y: u32 = 384;

/// Mouse button state
static mut MOUSE_LEFT: bool = false;
static mut MOUSE_RIGHT: bool = false;
static mut MOUSE_MIDDLE: bool = false;

// ─── Initialization ────────────────────────────────────────────────

/// Initialize the WM input subsystem
pub fn init() {
    let info = match compositor::fb_info() {
        Some(info) => info,
        None => return,
    };
    unsafe {
        CURSOR_X = info.width / 2;
        CURSOR_Y = info.height / 2;
    }
    println!("[wm::input] Cursor at {},{}", unsafe { CURSOR_X }, unsafe {
        CURSOR_Y
    });
}

// ─── Event Pump ────────────────────────────────────────────────────

/// Poll HID devices and dispatch all pending events to the WM
/// Call this once per scheduler tick or main loop iteration
/// Number of events the previous pump consumed — non-zero means the UI
/// must re-render (input → interaction).
pub fn poll_count() -> u32 {
    PUMPED.load(core::sync::atomic::Ordering::Relaxed)
}

static PUMPED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

pub fn pump_events() {
    // Drain up to 8 events per pump to stay responsive
    PUMPED.store(0, core::sync::atomic::Ordering::Relaxed);
    for _ in 0..8 {
        let event = hid::poll();
        if event.is_some() {
            PUMPED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        match event {
            Some(HidEvent::KeyPress { scancode, extended }) => {
                handle_key_press(scancode, extended);
            }
            Some(HidEvent::KeyRelease {
                scancode,
                extended: _,
            }) => {
                handle_key_release(scancode);
            }
            Some(HidEvent::MouseMove { dx, dy }) => {
                handle_mouse_move(dx, dy);
            }
            Some(HidEvent::MouseButton {
                left,
                right,
                middle,
            }) => {
                handle_mouse_button(left, right, middle);
            }
            None => break, // No more events
        }
    }
}

/// Get current cursor position
pub fn cursor_pos() -> (u32, u32) {
    unsafe { (CURSOR_X, CURSOR_Y) }
}

/// Get current mouse button state
pub fn mouse_buttons() -> (bool, bool, bool) {
    unsafe { (MOUSE_LEFT, MOUSE_RIGHT, MOUSE_MIDDLE) }
}

// ─── Keyboard Dispatch ─────────────────────────────────────────────

/// Handle a key press event
fn handle_key_press(scancode: u8, _extended: bool) {
    // Get ASCII character from scancode (respecting shift state)
    let shift = hid::shift_held();
    let ch = hid::scancode_to_ascii(scancode, shift);

    // Alt+Tab: window switcher overlay
    if hid::alt_held() && scancode == 0x2B {
        if crate::wm::alt_tab::is_visible() {
            crate::wm::alt_tab::advance(hid::shift_held());
        } else {
            crate::wm::alt_tab::open();
        }
        return;
    }

    // Any keypress with the switcher open while Alt released: close + activate
    if crate::wm::alt_tab::is_visible() && !hid::alt_held() {
        crate::wm::alt_tab::close(true);
    }

    // Start menu keyboard handling (Esc closes, arrows navigate, Enter opens)
    if crate::wm::start_menu::handle_key(scancode) {
        return;
    }

    // WM keyboard shortcuts (Super+arrow, Super+D, Alt+F4, Alt+1-4)
    crate::wm::handle_shortcut(scancode);

    // GUI shell: capture typing when the Terminal window is focused.
    // Echoed chars + command output mirror into the window's VT.
    if crate::wm::gui_shell::handle_key(scancode, ch) {
        return;
    }

    // GUI file manager: keyboard shortcuts when the Files window is focused.
    if crate::wm::gui_files::handle_key(scancode) {
        return;
    }

    // Route to focused window via existing handle_event
    crate::wm::window::handle_event(HidEvent::KeyPress {
        scancode,
        extended: false,
    });

    // Log printable characters (debug)
    if let Some(c) = ch {
        if c >= ' ' && c <= '~' {
            // Printable — could feed to text input widget
        }
    }
}
/// Handle a key release event
fn handle_key_release(_scancode: u8) {
    // Alt released: if the Alt-Tab switcher is open, activate the selection.
    if crate::wm::alt_tab::is_visible() && !hid::alt_held() {
        crate::wm::alt_tab::close(true);
    }
}

// ─── Mouse Dispatch ────────────────────────────────────────────────

/// Handle mouse movement
fn handle_mouse_move(dx: i16, dy: i16) {
    let info = match compositor::fb_info() {
        Some(info) => info,
        None => return,
    };

    unsafe {
        // Update cursor position (clamp to screen bounds)
        let new_x = (CURSOR_X as i32 + dx as i32).max(0) as u32;
        let new_y = (CURSOR_Y as i32 + dy as i32).max(0) as u32;
        CURSOR_X = if new_x >= info.width {
            info.width - 1
        } else {
            new_x
        };
        CURSOR_Y = if new_y >= info.height {
            info.height - 1
        } else {
            new_y
        };
    }

    // Update hardware cursor position
    compositor::set_hardware_cursor_pos(unsafe { CURSOR_X }, unsafe { CURSOR_Y });

    // Start-menu hover tracking (consumes nothing)
    {
        let (cx, cy) = unsafe { (CURSOR_X, CURSOR_Y) };
        crate::wm::start_menu::handle_move(cx, cy);
        crate::wm::desktop_icons::update_hover(cx, cy);
    }

    // Delegate to window module's existing handle_event
    crate::wm::window::handle_event(HidEvent::MouseMove { dx, dy });
}

/// Handle mouse button changes
fn handle_mouse_button(left: bool, right: bool, middle: bool) {
    let prev_left = unsafe { MOUSE_LEFT };
    let prev_right = unsafe { MOUSE_RIGHT };

    unsafe {
        MOUSE_LEFT = left;
        MOUSE_RIGHT = right;
        MOUSE_MIDDLE = middle;
    }

    // Right-click: show context menu
    if right && !prev_right {
        let (cx, cy) = unsafe { (CURSOR_X, CURSOR_Y) };

        // If context menu is already visible, hide it
        if crate::wm::context_menu::is_visible() {
            crate::wm::context_menu::hide();
            return;
        }

        // Check if click is on desktop (not on any window or panel)
        let focused = crate::wm::window::focused_id();
        let on_window = crate::wm::window::get_info(focused).is_some();

        if !on_window {
            crate::wm::context_menu::show_desktop_menu(cx, cy);
        }
        return;
    }

    // If context menu is visible, route clicks to it
    if crate::wm::context_menu::is_visible() {
        let (cx, cy) = unsafe { (CURSOR_X, CURSOR_Y) };
        // Update hover state
        crate::wm::context_menu::handle_hover(cx, cy);

        if left && !prev_left {
            if let Some(action) = crate::wm::context_menu::handle_click(cx, cy) {
                handle_context_action(action);
                return;
            }
        }
        return;
    }

    // On left press: check panel clicks first, then desktop icons, then window focus
    if left && !prev_left {
        let (cx, cy) = unsafe { (CURSOR_X, CURSOR_Y) };

        // Check if click is on panel (launcher, task buttons, tray)
        if let Some(action) = panel::handle_click(cx, cy) {
            handle_panel_action(action);
            return;
        }

        // Tray popups: consume inside-clicks; outside click closes.
        if crate::wm::tray_popups::handle_click(cx, cy) {
            return;
        }

        // Start menu: consume clicks inside its surface (launch/close),
        // and any outside click closes it.
        if crate::wm::start_menu::handle_click(cx, cy) {
            return;
        }

        // Check if click is on a desktop icon
        if let Some(icon_idx) = crate::wm::desktop_icons::hit_test(cx, cy) {
            crate::wm::desktop_icons::handle_click(icon_idx);
            return;
        }

        // Not on panel or icon - clear icon selection, then focus the window
        // under the cursor.
        crate::wm::desktop_icons::select(None);
        crate::wm::window::handle_event(HidEvent::MouseButton {
            left,
            right,
            middle,
        });
    }

    // On left release: stop any drag
    if !left && prev_left {
        crate::wm::window::handle_event(HidEvent::MouseButton {
            left,
            right,
            middle,
        });
    }
}

/// Handle panel click actions
fn handle_panel_action(action: u32) {
    match action {
        0xFFFFFFFF => {
            // Launcher button clicked - toggle the start menu
            crate::wm::start_menu::toggle();
        }
        0xFFFFFFFE => {
            // Network tray → popup
            crate::wm::tray_popups::open(crate::wm::tray_popups::PopupKind::Network);
        }
        0xFFFFFFFD => {
            // Volume tray → popup
            crate::wm::tray_popups::open(crate::wm::tray_popups::PopupKind::Volume);
        }
        0xFFFFFFFC => {
            // Battery tray → popup
            crate::wm::tray_popups::open(crate::wm::tray_popups::PopupKind::Battery);
        }
        0xFFFFFFFB => {
            // Clock → clock/notification-center popup
            crate::wm::tray_popups::open(crate::wm::tray_popups::PopupKind::Clock);
        }
        window_id => {
            // Task button clicked - focus that window
            crate::wm::window::focus(window_id);
        }
    }
}

/// Create a new file or folder in the user desktop dir (falls back to /tmp)
fn new_desktop_item(is_dir: bool) {
    use crate::fs::vfs::{self, VfsError};
    let base = "/sd/home/user";
    // Ensure the user dir is resolvable (expansion is lazy)
    let name = if is_dir { "New Folder" } else { "New File" };
    // Find a non-clashing name
    for n in 0..100 {
        let candidate = if n == 0 {
            alloc::format!("{}/{}", base, name)
        } else {
            alloc::format!("{}/{} ({})", base, name, n)
        };
        let exists = vfs::resolve_path(&candidate).is_ok();
        if !exists {
            let result = if is_dir {
                vfs::mkdir(&candidate)
            } else {
                vfs::create(&candidate)
            };
            match result {
                Ok(_) => {
                    crate::wm::notifications::success(
                        if is_dir { "Folder" } else { "File" },
                        &alloc::format!("Created {}", candidate),
                    );
                    crate::wm::compositor::mark_full_redraw();
                    return;
                }
                Err(VfsError::AlreadyExists) => continue,
                Err(e) => {
                    crate::wm::notifications::error("Desktop", &alloc::format!("create failed: {:?}", e));
                    return;
                }
            }
        }
    }
}

/// Handle context menu actions (IDs from context_menu::show_desktop_menu)
fn handle_context_action(action: u32) {
    match action {
        1 => {
            // Terminal
            crate::apps::terminal::init();
        }
        2 => {
            // File Manager
            crate::apps::file_manager::init();
        }
        3 => {
            // Settings
            crate::apps::settings::init();
        }
        4 => {
            // Calculator
            crate::apps::calculator::init();
        }
        5 => {
            // System Monitor
            crate::apps::sysmon::init();
        }
        10 => {
            // Change wallpaper: cycle through the built-in gradients
            crate::wm::wallpaper::next();
            crate::wm::compositor::mark_full_redraw();
        }
        30 => {
            // New File on the desktop (/sd/home/user/Desktop or /tmp)
            new_desktop_item(false);
        }
        31 => {
            // New Folder on the desktop
            new_desktop_item(true);
        }
        32 => {
            // Display Settings: open the Settings app
            crate::wm::start_menu::launch_by_name("Settings");
        }
        33 => {
            // Toggle Dark/Light theme
            crate::wm::theme::toggle();
        }
        34 => {
            // Refresh: full recomposition
            crate::wm::compositor::mark_full_redraw();
            crate::wm::notifications::info("Desktop", "Refreshed");
        }
        20 => {
            // About LobsterOS
            println!("[wm] LobsterOS v0.2 — Bare-metal Rust OS for Pi 5");
        }
        _ => {}
    }
}
