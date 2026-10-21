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

impl Clone for ContentPtr {
    fn clone(&self) -> Self {
        ContentPtr(self.0)
    }
}
impl Copy for ContentPtr {}

/// Window structure
#[derive(Clone, Copy)]
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

    /// Check if title matches exactly
    pub fn title_matches(&self, s: &str) -> bool {
        self.title_str() == s
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

/// Double-click tracking for title-bar maximize
static mut LAST_TITLE_CLICK_MS: u64 = 0;
static mut LAST_TITLE_CLICK_WIN: u32 = 0;

/// Drag-to-edge snap: when a drag ends with the window at a screen edge,
/// snap it to that half (Windows-style).
static mut SNAP_PREVIEW: Option<(u32, u32, u32, u32)> = None; // (x,y,w,h) preview rect

/// Drag state for window moving
static DRAG_WIN_ID: Spinlock<u32> = Spinlock::new(0);
static DRAG_START_CURSOR_X: Spinlock<i32> = Spinlock::new(0);
static DRAG_START_CURSOR_Y: Spinlock<i32> = Spinlock::new(0);
static DRAG_WIN_START_X: Spinlock<u32> = Spinlock::new(0);
static DRAG_WIN_START_Y: Spinlock<u32> = Spinlock::new(0);
static DRAGGING: Spinlock<bool> = Spinlock::new(false);

/// Resize state
static RESIZE_WIN_ID: Spinlock<u32> = Spinlock::new(0);
static RESIZE_START_CURSOR_X: Spinlock<i32> = Spinlock::new(0);
static RESIZE_START_CURSOR_Y: Spinlock<i32> = Spinlock::new(0);
static RESIZE_START_W: Spinlock<u32> = Spinlock::new(0);
static RESIZE_START_H: Spinlock<u32> = Spinlock::new(0);
static RESIZING: Spinlock<bool> = Spinlock::new(false);
static RESIZE_EDGE: Spinlock<u8> = Spinlock::new(0); // 0=right, 1=bottom, 2=corner

/// Close button area: top-right, 14x14, 4px margin
const CLOSE_BTN_OFFSET_X: u32 = CLOSE_BTN_SIZE + 6;
const CLOSE_BTN_Y: u32 = 5;

/// Resize handle thickness (pixels from border edge)
const RESIZE_HANDLE: u32 = 6;

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
            // On QEMU, skip content buffer allocation to avoid O(n²) heap expansion.
            // Apps will render directly to framebuffer via their render methods in wm::tick().
            let content_pixels = (win.client_w * win.client_h) as usize;
            win.content_size = content_pixels * 4;
            let content_ptr = if content_pixels > 0 {
                #[cfg(not(feature = "qemu_raspi5b"))]
                {
                    let layout =
                        core::alloc::Layout::from_size_align(win.content_size, 16).unwrap();
                    let ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
                    if !ptr.is_null() {
                        unsafe { core::ptr::write_bytes(ptr, 0, win.content_size) };
                        ptr as *mut u32
                    } else {
                        core::ptr::null_mut()
                    }
                }
                #[cfg(feature = "qemu_raspi5b")]
                {
                    // No content buffer - apps render directly
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

            // Open animation
            crate::wm::window_frame::mark_start();
            crate::wm::window_frame::start(crate::wm::window_frame::Anim::Open, id);

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
    crate::wm::dirty::mark();
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
    crate::wm::dirty::mark();
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

/// Hit test: find the topmost window at cursor position (cx, cy)
fn window_at(cx: u32, cy: u32) -> Option<(u32, u32, u32, u32, u32)> {
    // Returns (id, x, y, w, h) of topmost non-minimized window containing point
    let windows = WINDOWS.lock();
    let mut best: Option<(u32, u32, u32, u32, u32)> = None;
    let mut best_id = 0u32;

    // Iterate in reverse to find topmost (last drawn = top)
    for i in 0..MAX_WINDOWS {
        if !windows[i].active || windows[i].state == WinState::Minimized {
            continue;
        }
        let win = &windows[i];
        if cx >= win.x && cx < win.x + win.w && cy >= win.y && cy < win.y + win.h {
            // Track the highest z-order window (later windows are on top)
            best = Some((win.id, win.x, win.y, win.w, win.h));
            best_id = win.id;
        }
    }
    let _ = best_id;
    best
}

/// Check if point is on the close button of a window
fn is_on_close_button(cx: u32, cy: u32, win_x: u32, win_y: u32, win_w: u32) -> bool {
    let btn_x = win_x + win_w - CLOSE_BTN_OFFSET_X;
    let btn_y = win_y + CLOSE_BTN_Y;
    cx >= btn_x && cx < btn_x + CLOSE_BTN_SIZE && cy >= btn_y && cy < btn_y + CLOSE_BTN_SIZE
}

/// Check if point is on the maximize button (green, left of close)
fn is_on_maximize_button(cx: u32, cy: u32, win_x: u32, win_y: u32, win_w: u32) -> bool {
    let btn_x = win_x + win_w - CLOSE_BTN_OFFSET_X - 20;
    let btn_y = win_y + CLOSE_BTN_Y;
    cx >= btn_x && cx < btn_x + CLOSE_BTN_SIZE && cy >= btn_y && cy < btn_y + CLOSE_BTN_SIZE
}

/// Check if point is on the minimize button (yellow, left of maximize)
fn is_on_minimize_button(cx: u32, cy: u32, win_x: u32, win_y: u32, win_w: u32) -> bool {
    let btn_x = win_x + win_w - CLOSE_BTN_OFFSET_X - 40;
    let btn_y = win_y + CLOSE_BTN_Y;
    cx >= btn_x && cx < btn_x + CLOSE_BTN_SIZE && cy >= btn_y && cy < btn_y + CLOSE_BTN_SIZE
}

/// Check if point is on the title bar (for dragging)
fn is_on_title_bar(cy: u32, win_y: u32) -> bool {
    cy >= win_y && cy < win_y + TITLE_BAR_H
}

/// Check if point is on a resize edge
/// Returns: Some(edge) where 0=right, 1=bottom, 2=bottom-right corner
fn resize_edge_at(cx: u32, cy: u32, x: u32, y: u32, w: u32, h: u32) -> Option<u8> {
    let on_right = cx >= x + w - RESIZE_HANDLE && cx < x + w;
    let on_bottom = cy >= y + h - RESIZE_HANDLE && cy < y + h;
    let in_window = cx >= x && cx < x + w && cy >= y && cy < y + h;

    if !in_window {
        return None;
    }

    if on_right && on_bottom {
        return Some(2); // corner
    }
    if on_right {
        return Some(0); // right edge
    }
    if on_bottom {
        return Some(1); // bottom edge
    }
    None
}

/// Handle a keyboard press
fn handle_keypress(scancode: u8) {
    if crate::usb::hid::alt_held() && scancode == 0x2B {
        cycle_focus();
        return;
    }
    // Editor: tabs / save-as / find / typing when its window is focused
    #[cfg(feature = "qemu_raspi5b")]
    if crate::apps::editor::handle_key(scancode) {
        return;
    }
    let focused_id = *FOCUSED_ID.lock();
    if focused_id != 0 {
        let windows = WINDOWS.lock();
        for i in 0..MAX_WINDOWS {
            if windows[i].active && windows[i].id == focused_id {
                if windows[i].terminal_widget_id.is_some() {
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
    let _ = scancode;
}

/// Handle mouse movement — process drag and resize operations
fn handle_mouse_move(dx: i16, dy: i16) {
    let dragging = *DRAGGING.lock();
    let resizing = *RESIZING.lock();

    if dragging {
        let drag_win_id = *DRAG_WIN_ID.lock();
        if drag_win_id != 0 {
            let (cx, cy) = crate::wm::input::cursor_pos();
            let start_cx = *DRAG_START_CURSOR_X.lock();
            let start_cy = *DRAG_START_CURSOR_Y.lock();
            let win_start_x = *DRAG_WIN_START_X.lock();
            let win_start_y = *DRAG_WIN_START_Y.lock();

            let new_x = (win_start_x as i32 + cx as i32 - start_cx).max(0) as u32;
            let new_y = (win_start_y as i32 + cy as i32 - start_cy).max(0) as u32;
            move_window(drag_win_id, new_x, new_y);
        }
        return;
    }

    if resizing {
        let resize_id = *RESIZE_WIN_ID.lock();
        if resize_id != 0 {
            let (cx, cy) = crate::wm::input::cursor_pos();
            let start_cx = *RESIZE_START_CURSOR_X.lock();
            let start_cy = *RESIZE_START_CURSOR_Y.lock();
            let start_w = *RESIZE_START_W.lock();
            let start_h = *RESIZE_START_H.lock();
            let edge = *RESIZE_EDGE.lock();

            let dx_total = cx as i32 - start_cx;
            let dy_total = cy as i32 - start_cy;

            let new_w = match edge {
                0 | 2 => (start_w as i32 + dx_total).max(80) as u32, // right or corner
                _ => start_w,
            };
            let new_h = match edge {
                1 | 2 => (start_h as i32 + dy_total).max(60) as u32, // bottom or corner
                _ => start_h,
            };
            resize(resize_id, new_w, new_h);
        }
        return;
    }

    let _ = (dx, dy);
}

/// Handle mouse button changes — proper hit testing with focus, drag, close, resize
fn handle_mouse_button(left: bool, _right: bool, _middle: bool) {
    if left {
        // Left button pressed — hit test and determine action
        let (cx, cy) = crate::wm::input::cursor_pos();

        if let Some((win_id, win_x, win_y, win_w, win_h)) = window_at(cx, cy) {
            // Check resize edge first (border area)
            if let Some(edge) = resize_edge_at(cx, cy, win_x, win_y, win_w, win_h) {
                // Start resizing
                *RESIZE_WIN_ID.lock() = win_id;
                *RESIZE_START_CURSOR_X.lock() = cx as i32;
                *RESIZE_START_CURSOR_Y.lock() = cy as i32;
                *RESIZE_START_W.lock() = win_w;
                *RESIZE_START_H.lock() = win_h;
                *RESIZE_EDGE.lock() = edge;
                *RESIZING.lock() = true;
                focus(win_id);
                return;
            }

            // Check close button
            if is_on_close_button(cx, cy, win_x, win_y, win_w) {
                // S6: closing an EL0 app window kills its process (its
                // slices stop; reap frees stacks). Kernel windows just
                // destroy as before.
                if let Some(owner_pid) = crate::wm::user_windows::owner_of_wm(win_id) {
                    // S6: deactivate the app's user windows NOW (not at
                    // reap) — the WM would keep re-drawing the dead app's
                    // pixels every dirty cycle otherwise.
                    if let Some((wx, wy, ww, wh)) = get_info(win_id) {
                        crate::wm::wallpaper::erase_region(wx, wy, ww, wh);
                    }
                    crate::wm::user_windows::close_all_for_pid(owner_pid);
                    crate::sched::kill(owner_pid);
                    crate::wm::dirty::mark();
                    return;
                }
                destroy(win_id);
                return;
            }

            // Check maximize button
            if is_on_maximize_button(cx, cy, win_x, win_y, win_w) {
                maximize(win_id);
                return;
            }

            // Check minimize button
            if is_on_minimize_button(cx, cy, win_x, win_y, win_w) {
                minimize(win_id);
                return;
            }

            // Check title bar for dragging
            if is_on_title_bar(cy, win_y) {
                focus(win_id);

                // Double-click on title bar → toggle maximize
                unsafe {
                    let now = crate::arch::ArchTimer::uptime_ms();
                    if LAST_TITLE_CLICK_WIN == win_id
                        && now.saturating_sub(LAST_TITLE_CLICK_MS) < 400
                    {
                        // Double-click: toggle maximize, no drag start
                        LAST_TITLE_CLICK_MS = 0;
                        LAST_TITLE_CLICK_WIN = 0;
                        maximize(win_id);
                        return;
                    }
                    LAST_TITLE_CLICK_MS = now;
                    LAST_TITLE_CLICK_WIN = win_id;
                }

                *DRAG_WIN_ID.lock() = win_id;
                *DRAG_START_CURSOR_X.lock() = cx as i32;
                *DRAG_START_CURSOR_Y.lock() = cy as i32;
                *DRAG_WIN_START_X.lock() = win_x;
                *DRAG_WIN_START_Y.lock() = win_y;
                *DRAGGING.lock() = true;
                return;
            }

            // Click inside client area — just focus
            focus(win_id);
        }
    } else {
        // Left button released — stop all drag/resize
        let was_dragging = *DRAGGING.lock();
        let drag_win = *DRAG_WIN_ID.lock();

        // Drag-to-edge snap: if the cursor is within 24px of a screen edge
        // at release, snap the window to that half (or full for top).
        if was_dragging && drag_win != 0 {
            if let Some(fb) = compositor::fb_info() {
                let (cx, cy) = crate::wm::input::cursor_pos();
                let edge_zone = 24u32;
                use crate::wm::window::SnapPosition;
                if cx <= edge_zone {
                    resize(drag_win, fb.width / 2, fb.height);
                    move_window(drag_win, 0, 0);
                } else if cx >= fb.width.saturating_sub(edge_zone) {
                    resize(drag_win, fb.width / 2, fb.height);
                    move_window(drag_win, fb.width / 2, 0);
                } else if cy <= edge_zone {
                    // Top edge → maximize
                    maximize(drag_win);
                }
                unsafe { SNAP_PREVIEW = None; }
            }
        }

        *DRAGGING.lock() = false;
        *DRAG_WIN_ID.lock() = 0;
        *RESIZING.lock() = false;
        *RESIZE_WIN_ID.lock() = 0;
    }
}

/// Cycle focus to the next window
pub fn cycle_focus() {
    crate::wm::dirty::mark();
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

/// Get window by ID (returns copy of Window struct)
pub fn get(id: u32) -> Option<Window> {
    let windows = WINDOWS.lock();
    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].id == id {
            return Some(windows[i]);
        }
    }
    None
}

/// Find window ID by exact title match
pub fn find_by_title(title: &str) -> Option<u32> {
    let windows = WINDOWS.lock();
    for i in 0..MAX_WINDOWS {
        if windows[i].active && windows[i].title_matches(title) {
            return Some(windows[i].id);
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
