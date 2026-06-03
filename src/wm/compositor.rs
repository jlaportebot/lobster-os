//! Compositor — double-buffered scene composition for LobsterOS
//!
//! The compositor maintains a back buffer, renders all windows into it
//! (painter's algorithm: bottom to top), then swaps it to the front buffer
//! in a single blit operation for flicker-free display.
//!
//! Optimization: dirty rectangles track which regions changed so we only
//! repaint the affected areas.

use crate::display::fb::{FramebufferInfo, get_info};
use crate::display::graphics;
use crate::display::console::{WHITE, BLACK};
use crate::println;

/// Maximum dirty rectangles per frame
const MAX_DIRTY: usize = 32;

/// Maximum number of layers (windows) the compositor tracks
const MAX_LAYERS: usize = 32;

// ─── Dirty Rectangle ──────────────────────────────────────────────────

/// A rectangular region that needs repainting
#[derive(Clone, Copy, Debug)]
pub struct DirtyRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl DirtyRect {
    pub const fn zero() -> Self {
        DirtyRect { x: 0, y: 0, w: 0, h: 0 }
    }

    /// Check if this rect overlaps another
    pub fn overlaps(&self, other: &DirtyRect) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }

    /// Merge with another rect (bounding box)
    pub fn merge(&self, other: &DirtyRect) -> DirtyRect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = (self.x + self.w).max(other.x + other.w);
        let b = (self.y + self.h).max(other.y + other.h);
        DirtyRect { x, y, w: r - x, h: b - y }
    }
}

// ─── Layer (compositor's view of a window) ─────────────────────────────

/// A compositor layer representing a visible window
#[derive(Clone, Copy, Debug)]
pub struct Layer {
    pub id: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub z_order: u32,
    pub visible: bool,
    pub has_alpha: bool,
    pub opacity: u8,
}

impl Layer {
    pub const fn zero() -> Self {
        Layer {
            id: 0, x: 0, y: 0, w: 0, h: 0,
            z_order: 0, visible: false, has_alpha: false, opacity: 255,
        }
    }

    /// Get the bounding rect of this layer
    pub fn bounds(&self) -> DirtyRect {
        DirtyRect { x: self.x, y: self.y, w: self.w, h: self.h }
    }
}

// ─── Compositor State ──────────────────────────────────────────────────

/// Back buffer dimensions and pointer
static mut BACK_BUF: *mut u32 = core::ptr::null_mut();
static mut BACK_BUF_SIZE: usize = 0;
static mut FB_INFO: FramebufferInfo = FramebufferInfo::empty();

/// Dirty rectangle list
static mut DIRTY_LIST: [DirtyRect; MAX_DIRTY] = [DirtyRect::zero(); MAX_DIRTY];
static mut DIRTY_COUNT: usize = 0;

/// Layer list (sorted by z_order on each composite)
static mut LAYERS: [Layer; MAX_LAYERS] = [Layer::zero(); MAX_LAYERS];
static mut LAYER_COUNT: usize = 0;

/// Full-screen dirty flag (set on init or resize)
static mut FULL_REDRAW: bool = true;

// ─── Initialization ────────────────────────────────────────────────────

/// Initialize the compositor
pub fn init(fb: FramebufferInfo) {
    unsafe {
        FB_INFO = fb;
        DIRTY_COUNT = 0;
        LAYER_COUNT = 0;
        FULL_REDRAW = true;

        // Allocate back buffer (same size as framebuffer)
        let total_pixels = (fb.width * fb.height) as usize;
        BACK_BUF_SIZE = total_pixels * 4;

        // Allocate from heap
        let layout = core::alloc::Layout::from_size_align(BACK_BUF_SIZE, 16).unwrap();
        let ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
            if ptr.is_null() {
                println!("[compositor] FATAL: cannot allocate back buffer ({} bytes)", BACK_BUF_SIZE);
                return;
            }
            BACK_BUF = ptr as *mut u32;

        // Clear back buffer to black
        core::ptr::write_bytes(ptr, 0, BACK_BUF_SIZE);

        println!("[compositor] Back buffer at {:#x} ({} KB)", ptr as u64, BACK_BUF_SIZE / 1024);
    }
}

// ─── Layer Management ──────────────────────────────────────────────────

/// Register a window as a compositor layer
pub fn add_layer(id: u32, x: u32, y: u32, w: u32, h: u32, z_order: u32) {
    unsafe {
        if LAYER_COUNT >= MAX_LAYERS {
            println!("[compositor] Layer table full, cannot add id={}", id);
            return;
        }
        LAYERS[LAYER_COUNT] = Layer {
            id, x, y, w, h, z_order, visible: true,
            has_alpha: false, opacity: 255,
        };
        LAYER_COUNT += 1;
        mark_dirty(DirtyRect { x, y, w, h });
    }
}

/// Remove a layer by window ID
pub fn remove_layer(id: u32) {
    unsafe {
        let mut i = 0;
        while i < LAYER_COUNT {
            if LAYERS[i].id == id {
                let bounds = LAYERS[i].bounds();
                // Shift remaining layers down
                for j in i..(LAYER_COUNT - 1) {
                    LAYERS[j] = LAYERS[j + 1];
                }
                LAYER_COUNT -= 1;
                mark_dirty(bounds);
                return;
            }
            i += 1;
        }
    }
}

/// Update a layer's position/size
pub fn update_layer(id: u32, x: u32, y: u32, w: u32, h: u32) {
    unsafe {
        for i in 0..LAYER_COUNT {
            if LAYERS[i].id == id {
                let old_bounds = LAYERS[i].bounds();
                LAYERS[i].x = x;
                LAYERS[i].y = y;
                LAYERS[i].w = w;
                LAYERS[i].h = h;
                let new_bounds = LAYERS[i].bounds();
                mark_dirty(old_bounds.merge(&new_bounds));
                return;
            }
        }
    }
}

/// Update a layer's visibility
pub fn set_layer_visible(id: u32, visible: bool) {
    unsafe {
        for i in 0..LAYER_COUNT {
            if LAYERS[i].id == id {
                LAYERS[i].visible = visible;
                mark_dirty(LAYERS[i].bounds());
                return;
            }
        }
    }
}

/// Update a layer's z-order (for raise/lower)
pub fn set_layer_z(id: u32, z_order: u32) {
    unsafe {
        for i in 0..LAYER_COUNT {
            if LAYERS[i].id == id {
                LAYERS[i].z_order = z_order;
                FULL_REDRAW = true; // Z-order change requires full redraw
                return;
            }
        }
    }
}

/// Bring a layer to the top (highest z-order)
pub fn raise_layer(id: u32) {
    unsafe {
        let max_z = LAYERS.iter().map(|l| l.z_order).max().unwrap_or(0);
        for i in 0..LAYER_COUNT {
            if LAYERS[i].id == id {
                LAYERS[i].z_order = max_z + 1;
                FULL_REDRAW = true;
                return;
            }
        }
    }
}

// ─── Dirty Rectangle Management ────────────────────────────────────────

/// Mark a region as needing repaint
pub fn mark_dirty(rect: DirtyRect) {
    unsafe {
        if DIRTY_COUNT >= MAX_DIRTY {
            // Too many dirty rects — just do a full redraw
            FULL_REDRAW = true;
            return;
        }
        // Try to merge with existing dirty rect
        for i in 0..DIRTY_COUNT {
            if DIRTY_LIST[i].overlaps(&rect) {
                DIRTY_LIST[i] = DIRTY_LIST[i].merge(&rect);
                return;
            }
        }
        DIRTY_LIST[DIRTY_COUNT] = rect;
        DIRTY_COUNT += 1;
    }
}

/// Mark the entire screen dirty
pub fn mark_full_redraw() {
    unsafe { FULL_REDRAW = true; }
}

// ─── Composition ───────────────────────────────────────────────────────

/// Composite all visible layers and present to the framebuffer
pub fn composite() {
    let fb = unsafe { FB_INFO };
    if fb.width == 0 || unsafe { BACK_BUF.is_null() } {
        return;
    }

    let do_full = unsafe { FULL_REDRAW };

    if do_full {
        // Full redraw: clear entire back buffer with desktop color
        let bg = crate::wm::theme::desktop_bg();
        unsafe {
            let total = (fb.width * fb.height) as usize;
            for i in 0..total {
                core::ptr::write_volatile(BACK_BUF.add(i), bg);
            }
        }

        // Sort layers by z_order (simple bubble sort — small N)
        unsafe { sort_layers(); }

        // Render each visible layer into back buffer
        unsafe {
            for i in 0..LAYER_COUNT {
                if LAYERS[i].visible {
                    render_layer(&LAYERS[i], fb);
                }
            }
        }

        // Blit entire back buffer to front
        blit_full(fb);

        unsafe {
            FULL_REDRAW = false;
            DIRTY_COUNT = 0;
        }
    } else if unsafe { DIRTY_COUNT > 0 } {
        // Partial redraw: only repaint dirty regions
        let dirty_count = unsafe { DIRTY_COUNT };
        let dirty_list = unsafe { &DIRTY_LIST[..dirty_count] };

        unsafe { sort_layers(); }

        for rect in dirty_list.iter() {
            // Clear dirty rect with desktop color
            let bg = crate::wm::theme::desktop_bg();
            fill_back_buffer_rect(fb, rect.x, rect.y, rect.w, rect.h, bg);

            // Re-render layers that overlap this dirty rect
            unsafe {
                for i in 0..LAYER_COUNT {
                    if LAYERS[i].visible {
                        let layer_bounds = LAYERS[i].bounds();
                        let layer_rect = DirtyRect {
                            x: layer_bounds.x, y: layer_bounds.y,
                            w: layer_bounds.w, h: layer_bounds.h,
                        };
                        if rect.overlaps(&layer_rect) {
                            render_layer(&LAYERS[i], fb);
                        }
                    }
                }
            }
        }

        // Blit dirty rects to front buffer
        for rect in dirty_list.iter() {
            blit_rect(fb, rect);
        }

        unsafe { DIRTY_COUNT = 0; }
    }
}

// ─── Internal Rendering ────────────────────────────────────────────────

/// Sort layers by z_order (ascending)
unsafe fn sort_layers() {
    for i in 1..LAYER_COUNT {
        let key = LAYERS[i];
        let mut j = i;
        while j > 0 && LAYERS[j - 1].z_order > key.z_order {
            LAYERS[j] = LAYERS[j - 1];
            j -= 1;
        }
        LAYERS[j] = key;
    }
}

/// Render a single layer into the back buffer
fn render_layer(layer: &Layer, fb: FramebufferInfo) {
    // The window manager provides the window's pixel content
    // For now, render a themed window frame
    let theme = crate::wm::theme::current();

    // Window background
    fill_back_buffer_rect(fb, layer.x, layer.y, layer.w, layer.h, theme.win_bg);

    // Window title bar (24px high)
    if layer.h > 24 {
        fill_back_buffer_rect(fb, layer.x, layer.y, layer.w, 24, theme.title_bar);
    }

    // Window border
    draw_back_buffer_rect_outline(fb, layer.x, layer.y, layer.w, layer.h, theme.win_border);

    // Close button (red circle, 16x16 at top-right)
    if layer.w > 30 && layer.h > 20 {
        let btn_x = layer.x + layer.w - 22;
        let btn_y = layer.y + 4;
        fill_back_buffer_rect(fb, btn_x, btn_y, 16, 16, theme.close_btn);
    }

    // Title text — ask the window manager for the title
    let title = crate::wm::window::get_title(layer.id);
    if !title.is_empty() && layer.w > 40 {
        graphics::draw_text(
            &fb,
            layer.x + 8,
            layer.y + 4,
            title,
            theme.title_text,
            theme.title_bar,
        );
    }
}

/// Fill a rectangle in the back buffer
fn fill_back_buffer_rect(fb: FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    unsafe {
        if BACK_BUF.is_null() { return; }
        let pitch = fb.width; // pitch in pixels
        for dy in 0..h {
            let py = y + dy;
            if py >= fb.height { break; }
            for dx in 0..w {
                let px = x + dx;
                if px >= fb.width { break; }
                let idx = (py * pitch + px) as usize;
                if idx < BACK_BUF_SIZE / 4 {
                    core::ptr::write_volatile(BACK_BUF.add(idx), color);
                }
            }
        }
    }
}

/// Draw a rectangle outline in the back buffer
fn draw_back_buffer_rect_outline(fb: FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    // Top and bottom
    for dx in 0..w {
        let px = x + dx;
        if px >= fb.width { break; }
        set_back_pixel(fb, px, y, color);
        if y + h > 0 {
            set_back_pixel(fb, px, y + h - 1, color);
        }
    }
    // Left and right
    for dy in 0..h {
        let py = y + dy;
        if py >= fb.height { break; }
        set_back_pixel(fb, x, py, color);
        if x + w > 0 {
            set_back_pixel(fb, x + w - 1, py, color);
        }
    }
}

/// Set a single pixel in the back buffer
fn set_back_pixel(fb: FramebufferInfo, x: u32, y: u32, color: u32) {
    unsafe {
        if x >= fb.width || y >= fb.height { return; }
        let idx = (y * fb.width + x) as usize;
        if idx < BACK_BUF_SIZE / 4 {
            core::ptr::write_volatile(BACK_BUF.add(idx), color);
        }
    }
}

/// Blit the entire back buffer to the front framebuffer
fn blit_full(fb: FramebufferInfo) {
    unsafe {
        if BACK_BUF.is_null() { return; }
        let front = fb.base_addr as *mut u32;
        let total = (fb.width * fb.height) as usize;
        for i in 0..total {
            core::ptr::write_volatile(front.add(i), core::ptr::read_volatile(BACK_BUF.add(i)));
        }
    }
}

/// Blit a dirty rectangle from back to front buffer
fn blit_rect(fb: FramebufferInfo, rect: &DirtyRect) {
    unsafe {
        if BACK_BUF.is_null() { return; }
        let front = fb.base_addr as *mut u32;
        let pitch = fb.width;
        for dy in 0..rect.h {
            let py = rect.y + dy;
            if py >= fb.height { break; }
            for dx in 0..rect.w {
                let px = rect.x + dx;
                if px >= fb.width { break; }
                let idx = (py * pitch + px) as usize;
                let pixel = core::ptr::read_volatile(BACK_BUF.add(idx));
                core::ptr::write_volatile(front.add(idx), pixel);
            }
        }
    }
}

// ─── Query ─────────────────────────────────────────────────────────────

/// Get the framebuffer info the compositor is using
pub fn fb_info() -> FramebufferInfo {
    unsafe { FB_INFO }
}

/// Get the number of registered layers
pub fn layer_count() -> usize {
    unsafe { LAYER_COUNT }
}
