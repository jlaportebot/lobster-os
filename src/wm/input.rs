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
use crate::wm::event_router;

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

/// Events the restricted pump could not handle (keys, non-user-window
/// clicks). Replayed ahead of fresh HID input by the next full pump.
const DEFER_CAP: usize = 32;
static mut DEFERRED: [Option<HidEvent>; DEFER_CAP] = [None; DEFER_CAP];
static mut DEFER_HEAD: usize = 0;
static mut DEFER_LEN: usize = 0;

fn defer(ev: HidEvent) {
    unsafe {
        if DEFER_LEN == DEFER_CAP {
            // Full — drop the oldest
            DEFER_HEAD = (DEFER_HEAD + 1) % DEFER_CAP;
            DEFER_LEN -= 1;
        }
        DEFERRED[(DEFER_HEAD + DEFER_LEN) % DEFER_CAP] = Some(ev);
        DEFER_LEN += 1;
    }
}

fn next_deferred() -> Option<HidEvent> {
    unsafe {
        if DEFER_LEN == 0 {
            return None;
        }
        let ev = DEFERRED[DEFER_HEAD].take();
        DEFER_HEAD = (DEFER_HEAD + 1) % DEFER_CAP;
        DEFER_LEN -= 1;
        ev
    }
}

pub fn pump_events() {
    // Drain up to 8 events per pump to stay responsive
    PUMPED.store(0, core::sync::atomic::Ordering::Relaxed);
    for _ in 0..8 {
        let event = next_deferred().or_else(hid::poll);
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

/// Set the cursor position (used by the boot-time login greeter).
pub fn set_cursor_pos(x: u32, y: u32) {
    unsafe {
        CURSOR_X = x;
        CURSOR_Y = y;
    }
}

/// Get current cursor position
pub fn cursor_pos() -> (u32, u32) {
    unsafe { (CURSOR_X, CURSOR_Y) }
}

/// Pick the software-cursor shape from what's under the pointer:
/// I-beam over Terminal/Editor client areas, arrow elsewhere (A4.3).
pub fn update_cursor_shape() {
    let (cx, cy) = unsafe { (CURSOR_X, CURSOR_Y) };
    let title = crate::wm::window::get_title(crate::wm::window::focused_id());
    let shape = if (title == "Terminal" || title == "Editor" || title == "Notes")
        && crate::wm::window::get_info(crate::wm::window::focused_id())
            .map(|(x, y, w, h)| cx >= x + 2 && cy >= y + 26 && cx < x + w && cy < y + h)
            .unwrap_or(false)
    {
        crate::wm::compositor::CursorShape::IBeam
    } else {
        crate::wm::compositor::CursorShape::Arrow
    };
    crate::wm::compositor::set_cursor_shape(shape);
}

// ─── Restricted pump (nested ticks during EL0 app run) ──────────────

/// Move the cursor without touching menu/icon/window hover state —
/// safe to call from inside a syscall-driven nested WM tick.
fn move_cursor_only(dx: i16, dy: i16) {
    let info = match compositor::fb_info() {
        Some(info) => info,
        None => return,
    };
    unsafe {
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
    compositor::set_hardware_cursor_pos(unsafe { CURSOR_X }, unsafe { CURSOR_Y });
}

/// Deliver press/release edges to user-window *content* only — no panel/
/// desktop/shell/context-menu routing from inside a syscall (those paths
/// assume they run from the main loop and can re-enter the shell).
/// Returns true if the event was consumed; otherwise the caller defers it
/// (MOUSE_* is left untouched so the main loop still sees the edge).
fn user_window_click_only(left: bool) -> bool {
    let prev_left = unsafe { MOUSE_LEFT };
    let (cx, cy) = unsafe { (CURSOR_X, CURSOR_Y) };

    if left && !prev_left {
        if let Some((win_id, ux, uy, true)) = crate::wm::user_windows::hit_test(cx, cy) {
            unsafe { MOUSE_LEFT = true };
            begin_user_click(win_id, ux, uy);
            return true;
        }
        return false;
    }
    if !left && prev_left && unsafe { USER_CLICK_PENDING } {
        unsafe { MOUSE_LEFT = false };
        end_user_click(cx, cy);
        return true;
    }
    false
}

/// Queue a press into a user window and remember it for the release.
pub(super) fn begin_user_click(win_id: u64, ux: u32, uy: u32) {
    crate::wm::user_windows::focus_window(win_id);
    crate::wm::user_windows::click_event(win_id, 1, true, ux, uy);
    unsafe {
        USER_CLICK_PENDING_WIN = win_id as u32;
        USER_CLICK_PENDING_EPOCH = crate::wm::user_windows::epoch(win_id);
        USER_CLICK_PENDING = true;
    }
}

/// Deliver the release for a pending press (if any) and clear the pending
/// state. Safe to call on every left-release edge.
fn end_user_click(cx: u32, cy: u32) {
    if !unsafe { USER_CLICK_PENDING } {
        return;
    }
    unsafe { USER_CLICK_PENDING = false };
    let win_id = unsafe { USER_CLICK_PENDING_WIN } as u64;
    // The slot was closed/reused since the press — don't leak the release.
    if crate::wm::user_windows::epoch(win_id) != unsafe { USER_CLICK_PENDING_EPOCH } {
        return;
    }
    let (ux, uy) = match crate::wm::user_windows::hit_test(cx, cy) {
        Some((id, x, y, true)) if id == win_id => (x, y),
        _ => (0, 0),
    };
    crate::wm::user_windows::click_event(win_id, 1, false, ux, uy);
}

/// Restricted event pump for nested WM ticks (called from sys_sleep while
/// a cooperative EL0 GUI app runs). Keeps the cursor responsive and routes
/// clicks into user-window queues; deliberately skips panel/desktop/shell
/// routing which must only run from the main loop.
pub fn pump_events_user_only() {
    // NOTE: do not store(0) here — a nested pump must not clobber the
    // outer pump's event count (poll_count drives dirty-marking).
    for _ in 0..8 {
        let event = hid::poll();
        let Some(ev) = event else { break };
        PUMPED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        match ev {
            HidEvent::MouseMove { dx, dy } => move_cursor_only(dx, dy),
            HidEvent::MouseButton { left, .. } => {
                if !user_window_click_only(left) {
                    defer(ev); // panel/desktop/chrome clicks wait for the main loop
                }
            }
            // Keys wait for the main loop; replayed by the next pump_events()
            HidEvent::KeyPress { .. } | HidEvent::KeyRelease { .. } => defer(ev),
        }
    }
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

    // Task Manager: Tab/arrows/K when its window is focused
    if crate::apps::task_manager::handle_key(scancode) {
        return;
    }

    // Calculator: digits/ops/Enter/Backspace/Esc when focused (B6.2)
    if crate::wm::window::get_title(crate::wm::window::focused_id()) == "Calculator" {
        if let Some(calc) = crate::apps::calculator::instance() {
            if calc.handle_key(scancode, ch) {
                crate::wm::dirty::mark();
                return;
            }
        }
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
    update_cursor_shape();

    // Hover observers, drag/resize bookkeeping, optional focus-follows-z
    let (cx, cy) = unsafe { (CURSOR_X, CURSOR_Y) };
    event_router::route_move(cx, cy, dx, dy);
}

/// Pending EL0 user-window click awaiting its release event.
static mut USER_CLICK_PENDING: bool = false;
static mut USER_CLICK_PENDING_WIN: u32 = 0;
/// user_windows::epoch() of the window when the press landed.
static mut USER_CLICK_PENDING_EPOCH: u32 = 0;

/// Handle mouse button changes
fn handle_mouse_button(left: bool, right: bool, middle: bool) {
    let prev_left = unsafe { MOUSE_LEFT };
    let prev_right = unsafe { MOUSE_RIGHT };

    unsafe {
        MOUSE_LEFT = left;
        MOUSE_RIGHT = right;
        MOUSE_MIDDLE = middle;
    }

    // Deliver the release of a captured press *before* any early return
    // below (right-click, context menu) can swallow it and leave the
    // pending state stuck.
    if !left && prev_left {
        let (cx, cy) = unsafe { (CURSOR_X, CURSOR_Y) };
        end_user_click(cx, cy);
    }

    let (cx, cy) = unsafe { (CURSOR_X, CURSOR_Y) };

    // Right-click gesture: handled ahead of the routing table.
    if right && !prev_right {
        event_router::route_right_press(cx, cy);
        return;
    }

    // Everything else goes through the priority-ordered routing table.
    event_router::route_button(&event_router::ButtonEv {
        x: cx,
        y: cy,
        left,
        right,
        middle,
        prev_left,
    });
}

/// Handle panel click actions
pub(super) fn handle_panel_action(action: u32) {
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
                    crate::wm::notifications::error(
                        "Desktop",
                        &alloc::format!("create failed: {:?}", e),
                    );
                    return;
                }
            }
        }
    }
}

/// Handle context menu actions (IDs from context_menu::show_desktop_menu)
pub(super) fn handle_context_action(action: u32) {
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
