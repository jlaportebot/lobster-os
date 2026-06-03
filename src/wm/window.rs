//! Window management — create, destroy, move, resize, focus windows
//!
//! Each window has an ID, position, size, title, and content buffer.
//! The window manager tracks focus, stacking order, and routes input
//! events to the correct window.

use crate::display::fb::FramebufferInfo;
use crate::display::graphics;
use crate::display::console::WHITE;
use crate::wm::compositor;
use crate::wm::theme;
use crate::println;

/// Maximum number of windows
const MAX_WINDOWS: usize = 32;

/// Title bar height in pixels
const TITLE_BAR_H: u32 = 24;

/// Window border width in pixels
const BORDER_W: u32 = 2;

/// Close button size
const CLOSE_BTN_SIZE: u32 = 14;

// ─── Window State ──────────────────────────────────────────────────────

/// Window state
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WinState {
    /// Window is visible and interactive
    Normal,
    /// Window is minimized (not rendered)
    Minimized,
    /// Window is maximized (fills screen)
    Maximized,
}

/// Window structure
pub struct Window {
    pub id: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub state: WinState,
    pub title: [u8; 64],
    pub title_len: usize,
    pub focused: bool,
    pub active: bool,
    /// Client content area (inside borders + title bar)
    pub client_x: u32,
    pub client_y: u32,
    pub client_w: u32,
    pub client_h: u32,
    /// Content buffer (pixel data for the client area)
    pub content: *mut u32,
    pub content_size: usize,
}

impl Window {
    const ZERO: Window = Window {
        id: 0, x: 0, y: 0, w: 0, h: 0,
        state: WinState::Normal,
        title: [0u8; 64],
        title_len: 0,
        focused: false,
        active: false,
        client_x: 0, client_y: 0, client_w: 0, client_h: 0,
        content: core::ptr::null_mut(),
        content_size: 0,
    };

    /// Get the title as a string slice
    pub fn title_str(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len]).unwrap_or("")
    }

    /// Set the title from a string
    pub fn set_title_str(&mut self, s: &str) {
        let len = s.len().min(self.title.len());
        self.title[..len].copy_from_slice(s.as_bytes());
        self.title_len = len;
    }
}

// ─── Global Window State ───────────────────────────────────────────────

static mut WINDOWS: [Window; MAX_WINDOWS] = [Window::ZERO; MAX_WINDOWS];
static mut WIN_COUNT: usize = 0;
static mut NEXT_WIN_ID: u32 = 1;
static mut FOCUSED_ID: u32 = 0;

/// Drag state for window moving
static mut DRAG_WIN_ID: u32 = 0;
static mut DRAG_START_X: u32 = 0;
static mut DRAG_START_Y: u32 = 0;
static mut DRAG_WIN_START_X: u32 = 0;
static mut DRAG_WIN_START_Y: u32 = 0;
static mut DRAGGING: bool = false;

/// Resize state
static mut RESIZE_WIN_ID: u32 = 0;
static mut RESIZE_START_W: u32 = 0;
static mut RESIZE_START_H: u32 = 0;
static mut RESIZING: bool = false;

// ─── Initialization ────────────────────────────────────────────────────

/// Initialize the window manager
pub fn init() {
    unsafe {
        WIN_COUNT = 0;
        NEXT_WIN_ID = 1;
        FOCUSED_ID = 0;
        DRAGGING = false;
        RESIZING = false;
    }
}

// ─── Window Operations ─────────────────────────────────────────────────

/// Create a new window and return its ID
pub fn create(x: u32, y: u32, w: u32, h: u32, title: &str) -> Option<u32> {
    unsafe {
        if WIN_COUNT >= MAX_WINDOWS {
            println!("[wm] Window table full");
            return None;
        }

        let id = NEXT_WIN_ID;
        NEXT_WIN_ID += 1;

        // Find a slot
        for i in 0..MAX_WINDOWS {
            if !WINDOWS[i].active {
                let mut win = Window::ZERO;
                win.id = id;
                win.x = x;
                win.y = y;
                win.w = w;
                win.h = h;
                win.state = WinState::Normal;
                win.active = true;
                win.set_title_str(title);
                win.client_x = x + BORDER_W;
                win.client_y = y + TITLE_BAR_H;
                win.client_w = w.saturating_sub(2 * BORDER_W);
                win.client_h = h.saturating_sub(TITLE_BAR_H + BORDER_W);

                // Allocate content buffer for client area
                let content_pixels = (win.client_w * win.client_h) as usize;
                win.content_size = content_pixels * 4;
                if content_pixels > 0 {
                    let layout = core::alloc::Layout::from_size_align(win.content_size, 16).unwrap();
                    let ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
                        if !ptr.is_null() {
                            core::ptr::write_bytes(ptr, 0, win.content_size);
                            win.content = ptr as *mut u32;
                        }
                }

                WINDOWS[i] = win;
                WIN_COUNT += 1;

                // Register with compositor
                compositor::add_layer(id, x, y, w, h, id);

                // Focus the new window
                focus(id);

                println!("[wm] Created window id={} '{}'", id, title);
                return Some(id);
            }
        }
        None
    }
}

/// Destroy a window by ID
pub fn destroy(id: u32) {
    unsafe {
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active && WINDOWS[i].id == id {
                // Free content buffer
                if !WINDOWS[i].content.is_null() && WINDOWS[i].content_size > 0 {
                        let layout = core::alloc::Layout::from_size_align(WINDOWS[i].content_size, 16).unwrap();
                        crate::memory::heap::heap_free(WINDOWS[i].content as *mut u8, layout);
                    }

                    // Remove from compositor
                compositor::remove_layer(id);

                WINDOWS[i] = Window::ZERO;
                WIN_COUNT -= 1;

                if FOCUSED_ID == id {
                    FOCUSED_ID = 0;
                    // Focus the next active window
                    for j in 0..MAX_WINDOWS {
                        if WINDOWS[j].active {
                            focus(WINDOWS[j].id);
                            break;
                        }
                    }
                }
                println!("[wm] Destroyed window id={}", id);
                return;
            }
        }
    }
}

/// Focus a window (bring to top, highlight title bar)
pub fn focus(id: u32) {
    unsafe {
        // Unfocus all windows
        for i in 0..MAX_WINDOWS {
            WINDOWS[i].focused = false;
        }
        // Focus the target
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active && WINDOWS[i].id == id {
                WINDOWS[i].focused = true;
                FOCUSED_ID = id;
                compositor::raise_layer(id);
                return;
            }
        }
    }
}

/// Get the currently focused window ID
pub fn focused_id() -> u32 {
    unsafe { FOCUSED_ID }
}

/// Move a window to a new position
pub fn move_window(id: u32, x: u32, y: u32) {
    unsafe {
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active && WINDOWS[i].id == id {
                WINDOWS[i].x = x;
                WINDOWS[i].y = y;
                WINDOWS[i].client_x = x + BORDER_W;
                WINDOWS[i].client_y = y + TITLE_BAR_H;
                compositor::update_layer(id, x, y, WINDOWS[i].w, WINDOWS[i].h);
                return;
            }
        }
    }
}

/// Resize a window
pub fn resize(id: u32, w: u32, h: u32) {
    unsafe {
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active && WINDOWS[i].id == id {
                let min_w = 80u32;
                let min_h = 60u32;
                if w < min_w || h < min_h { return; }

                // Free old content buffer
                if !WINDOWS[i].content.is_null() && WINDOWS[i].content_size > 0 {
                        let layout = core::alloc::Layout::from_size_align(WINDOWS[i].content_size, 16).unwrap();
                        crate::memory::heap::heap_free(WINDOWS[i].content as *mut u8, layout);
                        WINDOWS[i].content = core::ptr::null_mut();
                    }

                WINDOWS[i].w = w;
                WINDOWS[i].h = h;
                WINDOWS[i].client_w = w.saturating_sub(2 * BORDER_W);
                WINDOWS[i].client_h = h.saturating_sub(TITLE_BAR_H + BORDER_W);

                // Allocate new content buffer
                let content_pixels = (WINDOWS[i].client_w * WINDOWS[i].client_h) as usize;
                WINDOWS[i].content_size = content_pixels * 4;
                if content_pixels > 0 {
                    let layout = core::alloc::Layout::from_size_align(WINDOWS[i].content_size, 16).unwrap();
                    let ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
                        if !ptr.is_null() {
                            core::ptr::write_bytes(ptr, 0, WINDOWS[i].content_size);
                            WINDOWS[i].content = ptr as *mut u32;
                        }
                }

                compositor::update_layer(id, WINDOWS[i].x, WINDOWS[i].y, w, h);
                return;
            }
        }
    }
}

/// Minimize a window
pub fn minimize(id: u32) {
    unsafe {
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active && WINDOWS[i].id == id {
                WINDOWS[i].state = WinState::Minimized;
                compositor::set_layer_visible(id, false);
                return;
            }
        }
    }
}

/// Maximize a window (fill screen)
pub fn maximize(id: u32) {
    let fb = compositor::fb_info();
    unsafe {
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active && WINDOWS[i].id == id {
                if WINDOWS[i].state == WinState::Maximized {
                    // Restore
                    WINDOWS[i].state = WinState::Normal;
                    compositor::set_layer_visible(id, true);
                } else {
                    WINDOWS[i].state = WinState::Maximized;
                    resize(id, fb.width, fb.height);
                    move_window(id, 0, 0);
                }
                return;
            }
        }
    }
}

/// Set a window's title
pub fn set_title(id: u32, title: &str) {
    unsafe {
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active && WINDOWS[i].id == id {
                WINDOWS[i].set_title_str(title);
                compositor::mark_full_redraw();
                return;
            }
        }
    }
}

/// Get a window's title (used by compositor for rendering)
pub fn get_title(id: u32) -> &'static str {
    unsafe {
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active && WINDOWS[i].id == id {
                return WINDOWS[i].title_str();
            }
        }
        ""
    }
}

// ─── Input Event Handling ──────────────────────────────────────────────

/// Handle an HID event from the USB subsystem
pub fn handle_event(evt: crate::usb::hid::HidEvent) {
    match evt {
        crate::usb::hid::HidEvent::KeyPress { scancode, extended: _ } => {
            handle_keypress(scancode);
        }
        crate::usb::hid::HidEvent::KeyRelease { scancode: _, extended: _ } => {}
        crate::usb::hid::HidEvent::MouseMove { dx, dy } => {
            handle_mouse_move(dx, dy);
        }
        crate::usb::hid::HidEvent::MouseButton { left, right, middle } => {
            handle_mouse_button(left, right, middle);
        }
    }
}

/// Handle a keyboard press
fn handle_keypress(scancode: u8) {
    // Alt+Tab: cycle focus
    if crate::usb::hid::alt_held() && scancode == 0x2B {
        // Tab scancode in USB HID = 0x2B
        cycle_focus();
        return;
    }

    // Forward to focused window (future: keyboard input routing)
    let _ = scancode;
}

/// Handle mouse movement
fn handle_mouse_move(dx: i16, dy: i16) {
    unsafe {
        if DRAGGING && DRAG_WIN_ID != 0 {
            let new_x = (DRAG_WIN_START_X as i32 + dx as i32 - DRAG_START_X as i32).max(0) as u32;
            let new_y = (DRAG_WIN_START_Y as i32 + dy as i32 - DRAG_START_Y as i32).max(0) as u32;
            move_window(DRAG_WIN_ID, new_x, new_y);
        }
    }
}

/// Handle mouse button changes
fn handle_mouse_button(left: bool, _right: bool, _middle: bool) {
    // This is called on button state *change*
    // For a full implementation we'd track previous button state
    if left {
        // Button pressed — start drag or focus window under cursor
        // For now, just focus the topmost window
        unsafe {
            if FOCUSED_ID != 0 {
                // Begin drag of focused window
                DRAG_WIN_ID = FOCUSED_ID;
                // DRAG_START_X/Y would be set from cursor position
                DRAGGING = true;
                for i in 0..MAX_WINDOWS {
                    if WINDOWS[i].active && WINDOWS[i].id == FOCUSED_ID {
                        DRAG_WIN_START_X = WINDOWS[i].x;
                        DRAG_WIN_START_Y = WINDOWS[i].y;
                        break;
                    }
                }
            }
        }
    } else {
        // Button released — stop drag
        unsafe {
            DRAGGING = false;
            DRAG_WIN_ID = 0;
        }
    }
}

/// Cycle focus to the next window
fn cycle_focus() {
    unsafe {
        let current = FOCUSED_ID;
        let mut found_current = false;
        let mut next_id: Option<u32> = None;

        // Find the window after the current one
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active {
                if found_current && next_id.is_none() {
                    next_id = Some(WINDOWS[i].id);
                    break;
                }
                if WINDOWS[i].id == current {
                    found_current = true;
                }
            }
        }

        // Wrap around to first window
        if next_id.is_none() {
            for i in 0..MAX_WINDOWS {
                if WINDOWS[i].active {
                    next_id = Some(WINDOWS[i].id);
                    break;
                }
            }
        }

        if let Some(id) = next_id {
            focus(id);
        }
    }
}

// ─── Query ─────────────────────────────────────────────────────────────

/// Get the number of active windows
pub fn count() -> usize {
    unsafe { WIN_COUNT }
}

/// Check if a window exists
pub fn exists(id: u32) -> bool {
    unsafe {
        WINDOWS.iter().any(|w| w.active && w.id == id)
    }
}

/// Get window info by ID
pub fn get_info(id: u32) -> Option<(u32, u32, u32, u32)> {
    unsafe {
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active && WINDOWS[i].id == id {
                return Some((WINDOWS[i].x, WINDOWS[i].y, WINDOWS[i].w, WINDOWS[i].h));
            }
        }
        None
    }
}
