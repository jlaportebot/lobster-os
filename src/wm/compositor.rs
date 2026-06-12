//! Compositor — double/triple-buffered scene composition for LobsterOS
//!
//! The compositor maintains multiple buffers for flicker-free display:
//! - Double buffering: front + back (current implementation)
//! - Triple buffering: front + middle + back (lower latency, better throughput)
//!
//! Optimization: dirty rectangles track which regions changed so we only
//! repaint the affected areas.

use crate::display::fb::FramebufferInfo;
use crate::display::graphics;
use crate::println;
use alloc::vec::Vec;

/// Maximum dirty rectangles per frame
const MAX_DIRTY: usize = 8;

/// Maximum number of layers (windows) the compositor tracks
const MAX_LAYERS: usize = 8;

/// Buffering mode
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BufferingMode {
    /// Double buffering (front + back)
    Double,
    /// Triple buffering (front + middle + back)
    Triple,
}

/// Default buffering mode
const DEFAULT_BUFFERING: BufferingMode = BufferingMode::Triple;

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
        DirtyRect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }
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
        DirtyRect {
            x,
            y,
            w: r - x,
            h: b - y,
        }
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
            id: 0,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            z_order: 0,
            visible: false,
            has_alpha: false,
            opacity: 255,
        }
    }

    /// Get the bounding rect of this layer
    pub fn bounds(&self) -> DirtyRect {
        DirtyRect {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }
}

// ─── Compositor State ──────────────────────────────────────────────────

use crate::sync::spinlock::Spinlock;

/// Wrapper for raw pointer to make it Send + Sync
struct RawPtr(*mut u32);
unsafe impl Send for RawPtr {}
unsafe impl Sync for RawPtr {}

/// Buffer pointers for double/triple buffering
/// - front: currently displayed (read by display hardware)
/// - middle: ready to be displayed next (triple buffering only)
/// - back: being rendered to by CPU
static FRONT_BUF: Spinlock<Option<RawPtr>> = Spinlock::new(None);
static MIDDLE_BUF: Spinlock<Option<RawPtr>> = Spinlock::new(None);
static BACK_BUF: Spinlock<Option<RawPtr>> = Spinlock::new(None);
static BACK_BUF_SIZE: Spinlock<usize> = Spinlock::new(0);
static FB_INFO: Spinlock<Option<FramebufferInfo>> = Spinlock::new(None);

/// Current buffering mode
static BUFFERING_MODE: Spinlock<BufferingMode> = Spinlock::new(DEFAULT_BUFFERING);

/// Dirty rectangle list
static DIRTY_LIST: Spinlock<[DirtyRect; MAX_DIRTY]> = Spinlock::new([DirtyRect::zero(); MAX_DIRTY]);
static DIRTY_COUNT: Spinlock<usize> = Spinlock::new(0);

/// Layer list (sorted by z_order on each composite)
static LAYERS: Spinlock<[Layer; MAX_LAYERS]> = Spinlock::new([Layer::zero(); MAX_LAYERS]);
static LAYER_COUNT: Spinlock<usize> = Spinlock::new(0);

/// Full-screen dirty flag (set on init or resize)
static FULL_REDRAW: Spinlock<bool> = Spinlock::new(true);

// ─── Initialization ────────────────────────────────────────────────────

/// Initialize the compositor
pub fn init(fb: FramebufferInfo) {
    let mut fb_info = FB_INFO.lock();
    *fb_info = Some(fb);
    *DIRTY_COUNT.lock() = 0;
    *LAYER_COUNT.lock() = 0;
    *FULL_REDRAW.lock() = true;

    // Allocate buffers (same size as framebuffer)
    let fb = FB_INFO.lock();
    let fb = fb.as_ref().expect("FB_INFO not set");
    let total_pixels = (fb.width * fb.height) as usize;
    *BACK_BUF_SIZE.lock() = total_pixels * 4;
    let buf_size = *BACK_BUF_SIZE.lock();

    let layout = core::alloc::Layout::from_size_align(buf_size, 16).unwrap();

    // Allocate back buffer (CPU writes here)
    let back_ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
    if back_ptr.is_null() {
        println!(
            "[compositor] FATAL: cannot allocate back buffer ({} bytes)",
            buf_size
        );
        return;
    }
    *BACK_BUF.lock() = Some(RawPtr(back_ptr as *mut u32));
    unsafe {
        core::ptr::write_bytes(back_ptr, 0, buf_size);
    }

    // Allocate middle buffer (triple buffering)
    let middle_ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
    if middle_ptr.is_null() {
        println!(
            "[compositor] FATAL: cannot allocate middle buffer ({} bytes)",
            buf_size
        );
        return;
    }
    *MIDDLE_BUF.lock() = Some(RawPtr(middle_ptr as *mut u32));
    unsafe {
        core::ptr::write_bytes(middle_ptr, 0, buf_size);
    }

    // Allocate front buffer (initially same as back)
    let front_ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
    if front_ptr.is_null() {
        println!(
            "[compositor] FATAL: cannot allocate front buffer ({} bytes)",
            buf_size
        );
        return;
    }
    *FRONT_BUF.lock() = Some(RawPtr(front_ptr as *mut u32));
    unsafe {
        core::ptr::write_bytes(front_ptr, 0, buf_size);
    }

    println!(
        "[compositor] Triple buffering: front={:#x} middle={:#x} back={:#x} ({} KB each)",
        front_ptr as u64,
        middle_ptr as u64,
        back_ptr as u64,
        buf_size / 1024
    );
}

/// Add a new layer (window) to the compositor
pub fn add_layer(id: u32, x: u32, y: u32, w: u32, h: u32, z_order: u32) {
    let mut layers = LAYERS.lock();
    let mut layer_count = LAYER_COUNT.lock();

    if *layer_count >= MAX_LAYERS {
        println!("[compositor] Layer table full, cannot add id={}", id);
        return;
    }
    layers[*layer_count] = Layer {
        id,
        x,
        y,
        w,
        h,
        z_order,
        visible: true,
        has_alpha: false,
        opacity: 255,
    };
    *layer_count += 1;
    drop(layers);
    drop(layer_count);
    mark_dirty(DirtyRect { x, y, w, h });
}

/// Remove a layer by window ID
pub fn remove_layer(id: u32) {
    let mut layers = LAYERS.lock();
    let mut layer_count = LAYER_COUNT.lock();

    let mut i = 0;
    while i < *layer_count {
        if layers[i].id == id {
            let bounds = layers[i].bounds();
            // Shift remaining layers down
            for j in i..(*layer_count - 1) {
                layers[j] = layers[j + 1];
            }
            *layer_count -= 1;
            drop(layers);
            drop(layer_count);
            mark_dirty(bounds);
            return;
        }
        i += 1;
    }
}

/// Update a layer's position/size
pub fn update_layer(id: u32, x: u32, y: u32, w: u32, h: u32) {
    let mut layers = LAYERS.lock();
    let layer_count = *LAYER_COUNT.lock();

    for i in 0..layer_count {
        if layers[i].id == id {
            let old_bounds = layers[i].bounds();
            layers[i].x = x;
            layers[i].y = y;
            layers[i].w = w;
            layers[i].h = h;
            let new_bounds = layers[i].bounds();
            drop(layers);
            mark_dirty(old_bounds.merge(&new_bounds));
            return;
        }
    }
}

/// Update a layer's visibility
pub fn set_layer_visible(id: u32, visible: bool) {
    let mut layers = LAYERS.lock();
    let layer_count = *LAYER_COUNT.lock();

    for i in 0..layer_count {
        if layers[i].id == id {
            layers[i].visible = visible;
            let bounds = layers[i].bounds();
            drop(layers);
            mark_dirty(bounds);
            return;
        }
    }
}

/// Update a layer's z-order (for raise/lower)
pub fn set_layer_z(id: u32, z_order: u32) {
    let mut layers = LAYERS.lock();
    let layer_count = *LAYER_COUNT.lock();

    for i in 0..layer_count {
        if layers[i].id == id {
            layers[i].z_order = z_order;
            *FULL_REDRAW.lock() = true; // Z-order change requires full redraw
            return;
        }
    }
}

/// Bring a layer to the top (highest z-order)
pub fn raise_layer(id: u32) {
    let mut layers = LAYERS.lock();
    let layer_count = *LAYER_COUNT.lock();

    let max_z = (0..layer_count)
        .map(|i| layers[i].z_order)
        .max()
        .unwrap_or(0);
    for i in 0..layer_count {
        if layers[i].id == id {
            layers[i].z_order = max_z + 1;
            *FULL_REDRAW.lock() = true;
            return;
        }
    }
}

// ─── Dirty Rectangle Management ────────────────────────────────────────

/// Mark a region as needing repaint
pub fn mark_dirty(rect: DirtyRect) {
    let mut dirty_list = DIRTY_LIST.lock();
    let mut dirty_count = DIRTY_COUNT.lock();
    let mut full_redraw = FULL_REDRAW.lock();

    if *dirty_count >= MAX_DIRTY {
        // Too many dirty rects — just do a full redraw
        *full_redraw = true;
        return;
    }
    // Try to merge with existing dirty rect
    for i in 0..*dirty_count {
        if dirty_list[i].overlaps(&rect) {
            dirty_list[i] = dirty_list[i].merge(&rect);
            return;
        }
    }
    dirty_list[*dirty_count] = rect;
    *dirty_count += 1;
}

/// Mark the entire screen dirty
pub fn mark_full_redraw() {
    *FULL_REDRAW.lock() = true;
}

// ─── Composition ───────────────────────────────────────────────────────

/// Composite all visible layers and present to the framebuffer
pub fn composite() {
    let fb = {
        let fb_info = FB_INFO.lock();
        match *fb_info {
            Some(fb) => fb,
            None => return,
        }
    };

    let back_buffer = {
        let back_buf_opt = BACK_BUF.lock();
        match back_buf_opt.as_ref() {
            Some(buf) => buf.0,
            None => return,
        }
    };

    let do_full = *FULL_REDRAW.lock();

    if do_full {
        // Full redraw: draw wallpaper first
        draw_wallpaper(fb, back_buffer);

        // Sort layers by z_order (simple bubble sort — small N)
        sort_layers();

        // Render each visible layer into back buffer
        let layers = LAYERS.lock();
        let layer_count = *LAYER_COUNT.lock();

        for i in 0..layer_count {
            if layers[i].visible {
                render_layer(&layers[i], fb);
            }
        }

        // Triple buffering: rotate buffers and present
        rotate_and_present(fb, back_buffer);

        *FULL_REDRAW.lock() = false;
        *DIRTY_COUNT.lock() = 0;
    } else if *DIRTY_COUNT.lock() > 0 {
        // Partial redraw: only repaint dirty regions
        let dirty_count = *DIRTY_COUNT.lock();
        let dirty_list = {
            let d = DIRTY_LIST.lock();
            d[..dirty_count].to_vec()
        };

        sort_layers();

        for rect in dirty_list.iter() {
            // Clear dirty rect with wallpaper
            fill_back_buffer_rect_with_wallpaper(fb, back_buffer, rect.x, rect.y, rect.w, rect.h);

            // Re-render layers that overlap this dirty rect
            let layers = LAYERS.lock();
            let layer_count = *LAYER_COUNT.lock();

            for i in 0..layer_count {
                if layers[i].visible {
                    let layer_bounds = layers[i].bounds();
                    let layer_rect = DirtyRect {
                        x: layer_bounds.x,
                        y: layer_bounds.y,
                        w: layer_bounds.w,
                        h: layer_bounds.h,
                    };
                    if rect.overlaps(&layer_rect) {
                        render_layer(&layers[i], fb);
                    }
                }
            }
        }

        // Triple buffering: rotate buffers and present dirty rects
        rotate_and_present_dirty(fb, back_buffer, &dirty_list);

        *DIRTY_COUNT.lock() = 0;
    }
}

/// Rotate triple buffers and present the front buffer to the display
/// Rotation: back -> middle -> front -> back (for next frame)
fn rotate_and_present(fb: FramebufferInfo, back_buffer: *mut u32) {
    let mode = *BUFFERING_MODE.lock();
    match mode {
        BufferingMode::Double => {
            // Double buffering: back -> front
            let front_buf = {
                let front_buf_opt = FRONT_BUF.lock();
                match front_buf_opt.as_ref() {
                    Some(buf) => buf.0,
                    None => return,
                }
            };
            blit_full(fb, front_buf, back_buffer);
            // Swap back and front pointers for next frame
            let mut back_buf_lock = BACK_BUF.lock();
            let mut front_buf_lock = FRONT_BUF.lock();
            core::mem::swap(&mut *back_buf_lock, &mut *front_buf_lock);
        }
        BufferingMode::Triple => {
            // Triple buffering: back -> middle -> front -> display
            let front_buf = {
                let front_buf_opt = FRONT_BUF.lock();
                match front_buf_opt.as_ref() {
                    Some(buf) => buf.0,
                    None => return,
                }
            };
            let middle_buf = {
                let middle_buf_opt = MIDDLE_BUF.lock();
                match middle_buf_opt.as_ref() {
                    Some(buf) => buf.0,
                    None => return,
                }
            };

            // Blit middle to front (middle has the completed frame from last rotation)
            blit_full(fb, front_buf, middle_buf);

            // Rotate pointers: back becomes middle, middle becomes front, front becomes back
            let mut back_buf_lock = BACK_BUF.lock();
            let mut middle_buf_lock = MIDDLE_BUF.lock();
            let mut front_buf_lock = FRONT_BUF.lock();

            let back_ptr = back_buf_lock.take();
            let middle_ptr = middle_buf_lock.take();
            let front_ptr = front_buf_lock.take();

            *middle_buf_lock = back_ptr; // back -> middle
            *front_buf_lock = middle_ptr; // middle -> front
            *back_buf_lock = front_ptr; // front -> back (will be rendered to next)
        }
    }
}

/// Rotate triple buffers and present only dirty rects
fn rotate_and_present_dirty(fb: FramebufferInfo, back_buffer: *mut u32, dirty_list: &[DirtyRect]) {
    let mode = *BUFFERING_MODE.lock();
    match mode {
        BufferingMode::Double => {
            let front_buf = {
                let front_buf_opt = FRONT_BUF.lock();
                match front_buf_opt.as_ref() {
                    Some(buf) => buf.0,
                    None => return,
                }
            };
            for rect in dirty_list {
                blit_rect(fb, front_buf, back_buffer, rect);
            }
            let mut back_buf_lock = BACK_BUF.lock();
            let mut front_buf_lock = FRONT_BUF.lock();
            core::mem::swap(&mut *back_buf_lock, &mut *front_buf_lock);
        }
        BufferingMode::Triple => {
            let front_buf = {
                let front_buf_opt = FRONT_BUF.lock();
                match front_buf_opt.as_ref() {
                    Some(buf) => buf.0,
                    None => return,
                }
            };
            let middle_buf = {
                let middle_buf_opt = MIDDLE_BUF.lock();
                match middle_buf_opt.as_ref() {
                    Some(buf) => buf.0,
                    None => return,
                }
            };

            for rect in dirty_list {
                blit_rect(fb, front_buf, middle_buf, rect);
            }

            let mut back_buf_lock = BACK_BUF.lock();
            let mut middle_buf_lock = MIDDLE_BUF.lock();
            let mut front_buf_lock = FRONT_BUF.lock();

            let back_ptr = back_buf_lock.take();
            let middle_ptr = middle_buf_lock.take();
            let front_ptr = front_buf_lock.take();

            *middle_buf_lock = back_ptr;
            *front_buf_lock = middle_ptr;
            *back_buf_lock = front_ptr;
        }
    }
}

/// Fill a dirty rectangle with wallpaper content
fn fill_back_buffer_rect_with_wallpaper(
    fb: FramebufferInfo,
    back_buf: *mut u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) {
    unsafe {
        let color = WALLPAPER.solid_color;
        let pitch = fb.width;
        for dy in 0..h {
            let py = y + dy;
            if py >= fb.height {
                break;
            }
            for dx in 0..w {
                let px = x + dx;
                if px >= fb.width {
                    break;
                }
                let idx = (py * pitch + px) as usize;
                let back_buf_size = *BACK_BUF_SIZE.lock();
                if idx < back_buf_size / 4 {
                    core::ptr::write_volatile(back_buf.add(idx), color);
                }
            }
        }
    }
}

// ─── Internal Rendering ────────────────────────────────────────────────

/// Sort layers by z_order (ascending)
fn sort_layers() {
    let mut layers = LAYERS.lock();
    let layer_count = *LAYER_COUNT.lock();

    for i in 1..layer_count {
        let key = layers[i];
        let mut j = i;
        while j > 0 && layers[j - 1].z_order > key.z_order {
            layers[j] = layers[j - 1];
            j -= 1;
        }
        layers[j] = key;
    }
}

/// Render a single layer into the back buffer
fn render_layer(layer: &Layer, fb: FramebufferInfo) {
    // The window manager provides the window's pixel content
    // For now, render a themed window frame
    let theme = crate::wm::theme::current();
    let back_buf = {
        let buf = BACK_BUF.lock();
        buf.as_ref().expect("back buffer not initialized").0
    };

    // Window background
    fill_back_buffer_rect(
        fb,
        back_buf,
        layer.x,
        layer.y,
        layer.w,
        layer.h,
        theme.win_bg,
    );

    // Window title bar (24px high)
    if layer.h > 24 {
        fill_back_buffer_rect(fb, back_buf, layer.x, layer.y, layer.w, 24, theme.title_bar);
    }

    // Window border
    draw_back_buffer_rect_outline(
        fb,
        back_buf,
        layer.x,
        layer.y,
        layer.w,
        layer.h,
        theme.win_border,
    );

    // Close button (red circle, 16x16 at top-right)
    if layer.w > 30 && layer.h > 20 {
        let btn_x = layer.x + layer.w - 22;
        let btn_y = layer.y + 4;
        fill_back_buffer_rect(fb, back_buf, btn_x, btn_y, 16, 16, theme.close_btn);
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
fn fill_back_buffer_rect(
    fb: FramebufferInfo,
    back_buf: *mut u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: u32,
) {
    unsafe {
        if back_buf.is_null() {
            return;
        }
        let pitch = fb.width; // pitch in pixels
        for dy in 0..h {
            let py = y + dy;
            if py >= fb.height {
                break;
            }
            for dx in 0..w {
                let px = x + dx;
                if px >= fb.width {
                    break;
                }
                let idx = (py * pitch + px) as usize;
                let back_buf_size = *BACK_BUF_SIZE.lock();
                if idx < back_buf_size / 4 {
                    core::ptr::write_volatile(back_buf.add(idx), color);
                }
            }
        }
    }
}

/// Draw a rectangle outline in the back buffer
fn draw_back_buffer_rect_outline(
    fb: FramebufferInfo,
    back_buf: *mut u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: u32,
) {
    // Top and bottom
    for dx in 0..w {
        let px = x + dx;
        if px >= fb.width {
            break;
        }
        set_back_pixel(fb, back_buf, px, y, color);
        if y + h > 0 {
            set_back_pixel(fb, back_buf, px, y + h - 1, color);
        }
    }
    // Left and right
    for dy in 0..h {
        let py = y + dy;
        if py >= fb.height {
            break;
        }
        set_back_pixel(fb, back_buf, x, py, color);
        if x + w > 0 {
            set_back_pixel(fb, back_buf, x + w - 1, py, color);
        }
    }
}

/// Set a single pixel in the back buffer
fn set_back_pixel(fb: FramebufferInfo, back_buf: *mut u32, x: u32, y: u32, color: u32) {
    unsafe {
        if x >= fb.width || y >= fb.height {
            return;
        }
        let idx = (y * fb.width + x) as usize;
        let back_buf_size = *BACK_BUF_SIZE.lock();
        if idx < back_buf_size / 4 {
            core::ptr::write_volatile(back_buf.add(idx), color);
        }
    }
}

/// Blit the entire source buffer to the destination buffer
fn blit_full(fb: FramebufferInfo, dst: *mut u32, src: *mut u32) {
    unsafe {
        if src.is_null() || dst.is_null() {
            return;
        }
        let total = (fb.width * fb.height) as usize;
        for i in 0..total {
            core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
        }
    }
}

/// Blit a dirty rectangle from source to destination buffer
fn blit_rect(fb: FramebufferInfo, dst: *mut u32, src: *mut u32, rect: &DirtyRect) {
    unsafe {
        if src.is_null() || dst.is_null() {
            return;
        }
        let pitch = fb.width;
        for dy in 0..rect.h {
            let py = rect.y + dy;
            if py >= fb.height {
                break;
            }
            for dx in 0..rect.w {
                let px = rect.x + dx;
                if px >= fb.width {
                    break;
                }
                let idx = (py * pitch + px) as usize;
                let pixel = core::ptr::read_volatile(src.add(idx));
                core::ptr::write_volatile(dst.add(idx), pixel);
            }
        }
    }
}

// ─── Query ─────────────────────────────────────────────────────────────

/// Get the framebuffer info the compositor is using
pub fn fb_info() -> Option<FramebufferInfo> {
    let guard = FB_INFO.lock();
    guard.map(|x| x)
}

/// Get the number of registered layers
pub fn layer_count() -> usize {
    *LAYER_COUNT.lock()
}

/// Get the current buffering mode
pub fn buffering_mode() -> BufferingMode {
    *BUFFERING_MODE.lock()
}

/// Set the buffering mode (double or triple buffering)
/// Note: Changing mode requires reinitialization for full effect
pub fn set_buffering_mode(mode: BufferingMode) {
    *BUFFERING_MODE.lock() = mode;
    mark_full_redraw();
}

/// Get the total buffer memory usage in bytes
pub fn buffer_memory_usage() -> usize {
    let buf_size = *BACK_BUF_SIZE.lock();
    match *BUFFERING_MODE.lock() {
        BufferingMode::Double => buf_size * 2,
        BufferingMode::Triple => buf_size * 3,
    }
}

// ─── Wallpaper Support ─────────────────────────────────────────────────

/// Wallpaper scaling mode
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WallpaperMode {
    Fit,    // Scale to fit entirely (letterbox)
    Fill,   // Scale to cover entirely (crop)
    Center, // Center at 1:1
    Tile,   // Tile the image
}

/// Background layer for wallpaper
struct Wallpaper {
    mode: WallpaperMode,
    image_data: Option<Vec<u32>>, // ARGB pixels
    image_width: u32,
    image_height: u32,
    solid_color: u32,
}

static mut WALLPAPER: Wallpaper = Wallpaper {
    mode: WallpaperMode::Fill,
    image_data: None,
    image_width: 0,
    image_height: 0,
    solid_color: 0xFF0D1B2A, // Default dark theme color
};

/// Set wallpaper to a solid color
pub fn set_wallpaper_color(color: u32) {
    unsafe {
        WALLPAPER.solid_color = color;
        WALLPAPER.image_data = None;
        WALLPAPER.image_width = 0;
        WALLPAPER.image_height = 0;
    }
    mark_full_redraw();
}

/// Set wallpaper to an image file
pub fn set_wallpaper_image(path: &str) -> Result<(), &'static str> {
    // Read the image file from VFS
    let data = crate::fs::vfs::read_file(path).map_err(|_| "Failed to read file")?;

    // For now, only support simple formats or use the framebuffer directly
    // In a real implementation, we'd decode PNG/JPEG/BMP here
    // For now, just use the desktop background color from theme
    let _ = data;

    unsafe {
        WALLPAPER.image_data = None;
        WALLPAPER.image_width = 0;
        WALLPAPER.image_height = 0;
    }

    // For now, just return success (placeholder)
    // Real implementation would decode the image
    Ok(())
}

/// Set wallpaper scaling mode
pub fn set_wallpaper_mode(mode: WallpaperMode) {
    unsafe {
        WALLPAPER.mode = mode;
    }
    mark_full_redraw();
}

/// Get current wallpaper mode
pub fn get_wallpaper_mode() -> WallpaperMode {
    unsafe { WALLPAPER.mode }
}

/// Draw the wallpaper to the back buffer
fn draw_wallpaper(fb: FramebufferInfo, back_buf: *mut u32) {
    unsafe {
        let color = WALLPAPER.solid_color;
        let total = (fb.width * fb.height) as usize;
        for i in 0..total {
            core::ptr::write_volatile(back_buf.add(i), color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffering_mode_enum() {
        assert_eq!(BufferingMode::Double as u8, 0);
        assert_eq!(BufferingMode::Triple as u8, 1);
    }

    #[test]
    fn test_buffering_mode_partial_eq() {
        assert_eq!(BufferingMode::Double, BufferingMode::Double);
        assert_eq!(BufferingMode::Triple, BufferingMode::Triple);
        assert_ne!(BufferingMode::Double, BufferingMode::Triple);
    }

    #[test]
    fn test_buffering_mode_debug() {
        let double_str = format!("{:?}", BufferingMode::Double);
        let triple_str = format!("{:?}", BufferingMode::Triple);
        assert_eq!(double_str, "Double");
        assert_eq!(triple_str, "Triple");
    }

    #[test]
    fn test_dirty_rect_zero() {
        let rect = DirtyRect::zero();
        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
        assert_eq!(rect.w, 0);
        assert_eq!(rect.h, 0);
    }

    #[test]
    fn test_dirty_rect_overlaps() {
        let rect1 = DirtyRect {
            x: 10,
            y: 10,
            w: 100,
            h: 100,
        };
        let rect2 = DirtyRect {
            x: 50,
            y: 50,
            w: 100,
            h: 100,
        };
        let rect3 = DirtyRect {
            x: 200,
            y: 200,
            w: 100,
            h: 100,
        };

        assert!(rect1.overlaps(&rect2));
        assert!(rect2.overlaps(&rect1));
        assert!(!rect1.overlaps(&rect3));
        assert!(!rect3.overlaps(&rect1));
    }

    #[test]
    fn test_dirty_rect_merge() {
        let rect1 = DirtyRect {
            x: 10,
            y: 10,
            w: 100,
            h: 100,
        };
        let rect2 = DirtyRect {
            x: 50,
            y: 50,
            w: 100,
            h: 100,
        };
        let merged = rect1.merge(&rect2);

        assert_eq!(merged.x, 10);
        assert_eq!(merged.y, 10);
        assert_eq!(merged.w, 140); // max(110, 150) - 10 = 140
        assert_eq!(merged.h, 140); // max(110, 150) - 10 = 140
    }

    #[test]
    fn test_layer_zero() {
        let layer = Layer::zero();
        assert_eq!(layer.id, 0);
        assert_eq!(layer.x, 0);
        assert_eq!(layer.y, 0);
        assert_eq!(layer.w, 0);
        assert_eq!(layer.h, 0);
        assert_eq!(layer.z_order, 0);
        assert!(!layer.visible);
        assert!(!layer.has_alpha);
        assert_eq!(layer.opacity, 255);
    }

    #[test]
    fn test_layer_bounds() {
        let layer = Layer {
            id: 1,
            x: 100,
            y: 200,
            w: 800,
            h: 600,
            z_order: 0,
            visible: true,
            has_alpha: false,
            opacity: 255,
        };
        let bounds = layer.bounds();
        assert_eq!(bounds.x, 100);
        assert_eq!(bounds.y, 200);
        assert_eq!(bounds.w, 800);
        assert_eq!(bounds.h, 600);
    }

    #[test]
    fn test_hotplug_state_enum() {
        assert_eq!(HotplugState::Disconnected as u8, 0);
        assert_eq!(HotplugState::Connected as u8, 1);
    }

    #[test]
    fn test_hdmi_audio_infoframe_default() {
        let info = HdmiAudioInfoFrame {
            coding_type: 1, // PCM
            sample_rate: 3, // 48kHz
            sample_size: 2, // 20-bit
            channels: 2,    // stereo
            speaker_placement: 0,
            channel_allocation: 0,
            level_shift: 0,
            downmix_inhibit: false,
        };
        assert_eq!(info.coding_type, 1);
        assert_eq!(info.sample_rate, 3);
        assert_eq!(info.channels, 2);
    }
}
