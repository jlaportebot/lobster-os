//! Window management — create, destroy, move, resize, focus windows
//!
//! Each window has an ID, position, size, title, and content buffer.
//! The window manager tracks focus, stacking order, and routes input
//! events to the correct window.

use crate::println;
use crate::wm::compositor;

/// Maximum number of windows
const MAX_WINDOWS: usize = 8;

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

/// Wrapper for raw pointer that implements Send (content buffer is only accessed with lock held)
pub struct ContentPtr(*mut u32);
unsafe impl Send for ContentPtr {}
unsafe impl Sync for ContentPtr {}

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
    pub content: ContentPtr,
    pub content_size: usize,
    /// Associated terminal widget ID (if this window hosts a terminal)
    pub terminal_widget_id: Option<crate::wm::widget::WidgetId>,
}

impl Window {
    const ZERO: Window = Window {
        id: 0,
        x: 0,
        y: 0,
        w: 0,
        h: 0,
        state: WinState::Normal,
        title: [0u8; 64],
        title_len: 0,
        focused: false,
        active: false,
        client_x: 0,
        client_y: 0,
        client_w: 0,
        client_h: 0,
        content: ContentPtr(core::ptr::null_mut()),
        content_size: 0,
        terminal_widget_id: None,
    };

    /// Get the content pointer as *mut u32
    pub fn content_ptr(&self) -> *mut u32 {
        self.content.0
    }

    /// Set the content pointer
    pub fn set_content_ptr(&mut self, ptr: *mut u32) {
        self.content = ContentPtr(ptr);
    }

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

use crate::sync::spinlock::Spinlock;

static WINDOWS: Spinlock<[Window; MAX_WINDOWS]> = Spinlock::new([Window::ZERO; MAX_WINDOWS]);
static WIN_COUNT: Spinlock<usize> = Spinlock::new(0);
static NEXT_WIN_ID: Spinlock<u32> = Spinlock::new(1);
static FOCUSED_ID: Spinlock<u32> = Spinlock::new(0);

/// Drag state for window moving
static DRAG_WIN_ID: Spinlock<u32> = Spinlock::new(0);
static DRAG_START_X: Spinlock<u32> = Spinlock::new(0);
static DRAG_START_Y: Spinlock<u32> = Spinlock::new(0);
static DRAG_WIN_START_X: Spinlock<u32> = Spinlock::new(0);
static DRAG_WIN_START_Y: Spinlock<u32> = Spinlock::new(0);
static DRAGGING: Spinlock<bool> = Spinlock::new(false);

/// Resize state
static RESIZE_WIN_ID: Spinlock<u32> = Spinlock::new(0);
static RESIZE_START_W: Spinlock<u32> = Spinlock::new(0);
static RESIZE_START_H: Spinlock<u32> = Spinlock::new(0);
static RESIZING: Spinlock<bool> = Spinlock::new(false);

// ─── Initialization ────────────────────────────────────────────────────

/// Initialize the window manager
pub fn init() {
    *WIN_COUNT.lock() = 0;
    *NEXT_WIN_ID.lock() = 1;
    *FOCUSED_ID.lock() = 0;
    *DRAGGING.lock() = false;
    *RESIZING.lock() = false;
}

// ─── Window Operations ─────────────────────────────────────────────────

/// Create a new window and return its ID
pub fn create(x: u32, y: u32, w: u32, h: u32, title: &str) -> Option<u32> {
    let mut windows = WINDOWS.lock();
    let mut win_count = WIN_COUNT.lock();
    let mut next_win_id = NEXT_WIN_ID.lock();

    if *win_count >= MAX_WINDOWS {
        println!("[wm] Window table full");
        return None;
    }

    let id = *next_win_id;
    *next_win_id += 1;

    // Find a slot
    for i in 0..MAX_WINDOWS {
        if !windows[i].active {
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
            let content_ptr = if content_pixels > 0 {
                let layout = core::alloc::Layout::from_size_align(win.content_size, 16).unwrap();
                let ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
                if !ptr.is_null() {
                    unsafe { core::ptr::write_bytes(ptr, 0, win.content_size) };
                    ptr as *mut u32
                } else {
                    core::ptr::null_mut()
                }
            } else {
                core::ptr::null_mut()
            };
            win.set_content_ptr(content_ptr);

            // Save these before move
            let layer_content_ptr = win.content_ptr();
            let layer_content_size = win.content_size;

            windows[i] = win;
            *win_count += 1;

            // Register with compositor
            drop(windows);
            drop(win_count);
            drop(next_win_id);
            compositor::add_layer(id, x, y, w, h, id, layer_content_ptr, layer_content_size);

            // Focus the new window
            focus(id);

            println!("[wm] Created window id={} '{}'", id, title);
            return Some(id);
        }
    }
    None
}

/// Destroy a window by ID
pub fn destroy(id: u32) {
    // Find the window index first
    let win_idx = {
        let windows = WINDOWS.lock();
        let mut found = None;
        for i in 0..MAX_WINDOWS {
            if windows[i].active && windows[i].id == id {
                found = Some(i);
                break;
            }
        }
        found
    };

    let Some(i) = win_idx else { return };

    // Free content buffer
    {
        let windows = WINDOWS.lock();
        let content_ptr = windows[i].content_ptr();
        if !content_ptr.is_null() && windows[i].content_size > 0 {
            let layout = core::alloc::Layout::from_size_align(windows[i].content_size, 16).unwrap();
            unsafe { crate::memory::heap::heap_free(content_ptr as *mut u8, layout) };
        }
    }

    // Remove from compositor
    compositor::remove_layer(id);

    // Clear window and update state
    let mut next_focus = None;
    {
        let mut windows = WINDOWS.lock();
        let mut win_count = WIN_COUNT.lock();
        let mut focused_id = FOCUSED_ID.lock();

        windows[i] = Window::ZERO;
        *win_count -= 1;

        if *focused_id == id {
            *focused_id = 0;
            // Find next active window to focus
            for j in 0..MAX_WINDOWS {
                if windows[j].active {
                    next_focus = Some(windows[j].id);
                    break;
                }
            }
        }
    }

    if let Some(next_id) = next_focus {
        focus(next_id);
    }

    println!("[wm] Destroyed window id={}", id);
}

/// Focus a window (bring to top, highlight title bar)
pub fn focus(id: u32) {
    let mut windows = WINDOWS.lock();
    let mut focused_id = FOCUSED_ID.lock();

    // Unfocus all windows
    for i in 0..MAX_WINDOWS {
        windows[i].focused = false;
    }
    // Focus the target
    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            windows[i].focused = true;
            *focused_id = id;
            drop(windows);
            drop(focused_id);
            compositor::raise_layer(id);
            return;
        }
    }
}

/// Get the currently focused window ID
pub fn focused_id() -> u32 {
    *FOCUSED_ID.lock()
}

/// Move a window to a new position
pub fn move_window(id: u32, x: u32, y: u32) {
    let mut windows = WINDOWS.lock();

    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            windows[i].x = x;
            windows[i].y = y;
            windows[i].client_x = x + BORDER_W;
            windows[i].client_y = y + TITLE_BAR_H;
            let w = windows[i].w;
            let h = windows[i].h;
            drop(windows);
            compositor::update_layer(id, x, y, w, h);
            return;
        }
    }
}

/// Resize a window
pub fn resize(id: u32, w: u32, h: u32) {
    let mut windows = WINDOWS.lock();

    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            let min_w = 80u32;
            let min_h = 60u32;
            if w < min_w || h < min_h {
                return;
            }

            // Free old content buffer
            let old_ptr = windows[i].content_ptr();
            if !old_ptr.is_null() && windows[i].content_size > 0 {
                let layout =
                    core::alloc::Layout::from_size_align(windows[i].content_size, 16).unwrap();
                unsafe { crate::memory::heap::heap_free(old_ptr as *mut u8, layout) };
                windows[i].set_content_ptr(core::ptr::null_mut());
            }

            windows[i].w = w;
            windows[i].h = h;
            windows[i].client_w = w.saturating_sub(2 * BORDER_W);
            windows[i].client_h = h.saturating_sub(TITLE_BAR_H + BORDER_W);

            // Allocate new content buffer
            let content_pixels = (windows[i].client_w * windows[i].client_h) as usize;
            windows[i].content_size = content_pixels * 4;
            if content_pixels > 0 {
                let layout =
                    core::alloc::Layout::from_size_align(windows[i].content_size, 16).unwrap();
                let ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
                if !ptr.is_null() {
                    unsafe { core::ptr::write_bytes(ptr, 0, windows[i].content_size) };
                    windows[i].set_content_ptr(ptr as *mut u32);
                }
            }

            let x = windows[i].x;
            let y = windows[i].y;
            drop(windows);
            compositor::update_layer(id, x, y, w, h);
            return;
        }
    }
}

/// Minimize a window
pub fn minimize(id: u32) {
    let mut windows = WINDOWS.lock();

    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            windows[i].state = WinState::Minimized;
            drop(windows);
            compositor::set_layer_visible(id, false);
            return;
        }
    }
}

/// Maximize a window (fill screen)
pub fn maximize(id: u32) {
    let fb = match compositor::fb_info() {
        Some(fb) => fb,
        None => return,
    };

    let mut windows = WINDOWS.lock();

    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            if windows[i].state == WinState::Maximized {
                // Restore
                windows[i].state = WinState::Normal;
                drop(windows);
                compositor::set_layer_visible(id, true);
            } else {
                windows[i].state = WinState::Maximized;
                drop(windows);
                resize(id, fb.width, fb.height);
                move_window(id, 0, 0);
            }
            return;
        }
    }
}

/// Set a window's title
pub fn set_title(id: u32, title: &str) {
    let mut windows = WINDOWS.lock();

    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            windows[i].set_title_str(title);
            drop(windows);
            compositor::mark_full_redraw();
            return;
        }
    }
}

/// Get a window's title (used by compositor for rendering)
/// Returns a copy of the title in a static buffer (safe for compositor rendering)
pub fn get_title(id: u32) -> &'static str {
    static mut TITLE_BUFFER: [u8; 64] = [0; 64];

    let windows = WINDOWS.lock();
    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            let title = windows[i].title_str();
            let len = title.len().min(63);
            unsafe {
                TITLE_BUFFER[..len].copy_from_slice(&title.as_bytes()[..len]);
                TITLE_BUFFER[len] = 0;
                return core::str::from_utf8_unchecked(&TITLE_BUFFER[..len]);
            }
        }
    }
    ""
}

// ─── Input Event Handling ──────────────────────────────────────────────

/// Handle an HID event from the USB subsystem
pub fn handle_event(evt: crate::usb::hid::HidEvent) {
    match evt {
        crate::usb::hid::HidEvent::KeyPress {
            scancode,
            extended: _,
        } => {
            handle_keypress(scancode);
        }
        crate::usb::hid::HidEvent::KeyRelease {
            scancode: _,
            extended: _,
        } => {}
        crate::usb::hid::HidEvent::MouseMove { dx, dy } => {
            handle_mouse_move(dx, dy);
        }
        crate::usb::hid::HidEvent::MouseButton {
            left,
            right,
            middle,
        } => {
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

    // Check if focused window has a terminal widget
    let focused_id = *FOCUSED_ID.lock();
    if focused_id != 0 {
        let windows = WINDOWS.lock();
        for i in 0..MAX_WINDOWS {
            if windows[i].active && windows[i].id == focused_id {
                if windows[i].terminal_widget_id.is_some() {
                    // Route to terminal app
                    let shift = crate::usb::hid::shift_held();
                    let ctrl = crate::usb::hid::ctrl_held();
                    let alt = crate::usb::hid::alt_held();
                    crate::apps::terminal::get_terminal()
                        .handle_special_key(scancode, shift, ctrl, alt);
                    return;
                }
                break;
            }
        }
    }

    // Forward to focused window (future: keyboard input routing)
    let _ = scancode;
}

/// Handle mouse movement
fn handle_mouse_move(dx: i16, dy: i16) {
    let dragging = *DRAGGING.lock();
    let drag_win_id = *DRAG_WIN_ID.lock();

    if dragging && drag_win_id != 0 {
        let drag_start_x = *DRAG_START_X.lock();
        let drag_start_y = *DRAG_START_Y.lock();
        let drag_win_start_x = *DRAG_WIN_START_X.lock();
        let drag_win_start_y = *DRAG_WIN_START_Y.lock();

        let new_x = (drag_win_start_x as i32 + dx as i32 - drag_start_x as i32).max(0) as u32;
        let new_y = (drag_win_start_y as i32 + dy as i32 - drag_start_y as i32).max(0) as u32;
        move_window(drag_win_id, new_x, new_y);
    }
}

/// Handle mouse button changes
fn handle_mouse_button(left: bool, _right: bool, _middle: bool) {
    // This is called on button state *change*
    // For a full implementation we'd track previous button state
    if left {
        // Button pressed — start drag or focus window under cursor
        // For now, just focus the topmost window
        let focused_id = *FOCUSED_ID.lock();
        if focused_id != 0 {
            // Begin drag of focused window
            *DRAG_WIN_ID.lock() = focused_id;
            // DRAG_START_X/Y would be set from cursor position
            *DRAGGING.lock() = true;
            let windows = WINDOWS.lock();
            for i in 0..MAX_WINDOWS {
                if windows[i].active && windows[i].id == focused_id {
                    *DRAG_WIN_START_X.lock() = windows[i].x;
                    *DRAG_WIN_START_Y.lock() = windows[i].y;
                    break;
                }
            }
        }
    } else {
        // Button released — stop drag
        *DRAGGING.lock() = false;
        *DRAG_WIN_ID.lock() = 0;
    }
}

/// Cycle focus to the next window
pub fn cycle_focus() {
    let windows = WINDOWS.lock();
    let current = *FOCUSED_ID.lock();
    let mut found_current = false;
    let mut next_id: Option<u32> = None;

    // Find the window after the current one
    for i in 0..MAX_WINDOWS {
        if windows[i].active {
            if found_current && next_id.is_none() {
                next_id = Some(windows[i].id);
                break;
            }
            if windows[i].id == current {
                found_current = true;
            }
        }
    }

    // Wrap around to first window
    if next_id.is_none() {
        for i in 0..MAX_WINDOWS {
            if windows[i].active {
                next_id = Some(windows[i].id);
                break;
            }
        }
    }

    drop(windows);
    if let Some(id) = next_id {
        focus(id);
    }
}

// ─── Query ─────────────────────────────────────────────────────────────

/// Get the number of active windows
pub fn count() -> usize {
    *WIN_COUNT.lock()
}

/// Check if a window exists
pub fn exists(id: u32) -> bool {
    let windows = WINDOWS.lock();
    windows.iter().any(|w| w.active && w.id == id)
}

/// Get window info by ID
pub fn get_info(id: u32) -> Option<(u32, u32, u32, u32)> {
    let windows = WINDOWS.lock();
    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            return Some((windows[i].x, windows[i].y, windows[i].w, windows[i].h));
        }
    }
    None
}

/// Get terminal widget ID on a window
pub fn get_terminal_widget_id(id: u32) -> Option<crate::wm::widget::WidgetId> {
    let windows = WINDOWS.lock();
    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            return windows[i].terminal_widget_id;
        }
    }
    None
}

/// Set terminal widget ID on a window
pub fn set_terminal_widget_id(id: u32, widget_id: crate::wm::widget::WidgetId) {
    let mut windows = WINDOWS.lock();
    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            windows[i].terminal_widget_id = Some(widget_id);
            return;
        }
    }
}

// ─── Window Snapping ────────────────────────────────────────────────────

/// Snap position for window snapping
#[derive(Clone, Copy, Debug)]
pub enum SnapPosition {
    Left,
    Right,
    Top,
    Bottom,
    Full,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

/// Get the panel height to avoid covering windows under the panel
fn panel_height() -> u32 {
    36
}

/// Snap a window to a position on screen
pub fn snap(id: u32, position: SnapPosition) {
    let fb = match compositor::fb_info() {
        Some(fb) => fb,
        None => return,
    };

    let screen_w = fb.width;
    let screen_h = fb.height.saturating_sub(panel_height());

    let (x, y, w, h) = match position {
        SnapPosition::Left => (0, 0, screen_w / 2, screen_h),
        SnapPosition::Right => (screen_w / 2, 0, screen_w / 2, screen_h),
        SnapPosition::Top => (0, 0, screen_w, screen_h / 2),
        SnapPosition::Bottom => (0, screen_h / 2, screen_w, screen_h / 2),
        SnapPosition::Full => (0, 0, screen_w, screen_h),
        SnapPosition::TopLeft => (0, 0, screen_w / 2, screen_h / 2),
        SnapPosition::TopRight => (screen_w / 2, 0, screen_w / 2, screen_h / 2),
        SnapPosition::BottomLeft => (0, screen_h / 2, screen_w / 2, screen_h / 2),
        SnapPosition::BottomRight => (screen_w / 2, screen_h / 2, screen_w / 2, screen_h / 2),
        SnapPosition::Center => {
            let win_w = 640u32.min(screen_w.saturating_sub(20));
            let win_h = 480u32.min(screen_h.saturating_sub(20));
            (
                (screen_w.saturating_sub(win_w)) / 2,
                (screen_h.saturating_sub(win_h)) / 2,
                win_w,
                win_h,
            )
        }
    };

    move_window(id, x, y);
    resize(id, w, h);
    println!("[wm] Snapped window {} to {:?}", id, position);
}

/// Restore a window from snapped/maximized state to a default size
pub fn restore(id: u32) {
    let win_w = 640u32;
    let win_h = 480u32;
    let fb = match compositor::fb_info() {
        Some(fb) => fb,
        None => return,
    };
    let x = (fb.width.saturating_sub(win_w)) / 2;
    let y = (fb
        .height
        .saturating_sub(panel_height())
        .saturating_sub(win_h))
        / 2;
    move_window(id, x, y);
    resize(id, win_w, win_h);
}
