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
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

/// Maximum dirty rectangles per frame
const MAX_DIRTY: usize = 8;

/// Maximum number of layers (windows) the compositor tracks
const MAX_LAYERS: usize = 8;

/// Maximum frame history for timing statistics
const MAX_FRAME_HISTORY: usize = 120;

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

// ─── Frame Timing Metrics ──────────────────────────────────────────────

/// Frame timing snapshot
#[derive(Clone, Copy, Debug)]
pub struct FrameTiming {
    /// Frame number
    pub frame_number: u64,
    /// Composition time (microseconds)
    pub compose_us: u64,
    /// Buffer blit/present time (microseconds)
    pub present_us: u64,
    /// VSYNC wait time (microseconds)
    pub vsync_wait_us: u64,
    /// Total frame time (microseconds)
    pub total_us: u64,
    /// Timestamp when frame started (system timer µs)
    pub start_timestamp_us: u64,
    /// Timestamp when frame completed (system timer µs)
    pub end_timestamp_us: u64,
}

/// Frame statistics
#[derive(Clone, Copy, Debug)]
pub struct FrameStats {
    /// Total frames rendered
    pub total_frames: u64,
    /// Average frame time (µs)
    pub avg_frame_us: u64,
    /// Minimum frame time (µs)
    pub min_frame_us: u64,
    /// Maximum frame time (µs)
    pub max_frame_us: u64,
    /// Current FPS (frames per second)
    pub fps: f32,
    /// Average composition time (µs)
    pub avg_compose_us: u64,
    /// Average present time (µs)
    pub avg_present_us: u64,
    /// Average VSYNC wait time (µs)
    pub avg_vsync_wait_us: u64,
}

/// Frame timing history for statistics
static FRAME_HISTORY: Spinlock<[FrameTiming; MAX_FRAME_HISTORY]> =
    Spinlock::new([const { FrameTiming::zero() }; MAX_FRAME_HISTORY]);
static FRAME_HISTORY_COUNT: Spinlock<usize> = Spinlock::new(0);
static FRAME_HISTORY_HEAD: Spinlock<usize> = Spinlock::new(0);

impl FrameTiming {
    pub const fn zero() -> Self {
        FrameTiming {
            frame_number: 0,
            compose_us: 0,
            present_us: 0,
            vsync_wait_us: 0,
            total_us: 0,
            start_timestamp_us: 0,
            end_timestamp_us: 0,
        }
    }
}

/// Record a frame timing entry
fn record_frame_timing(timing: FrameTiming) {
    let mut history = FRAME_HISTORY.lock();
    let mut count = FRAME_HISTORY_COUNT.lock();
    let mut head = FRAME_HISTORY_HEAD.lock();

    history[*head] = timing;
    *head = (*head + 1) % MAX_FRAME_HISTORY;
    if *count < MAX_FRAME_HISTORY {
        *count += 1;
    }
}

/// Get current frame statistics
pub fn frame_stats() -> FrameStats {
    let history = FRAME_HISTORY.lock();
    let count = *FRAME_HISTORY_COUNT.lock();

    if count == 0 {
        return FrameStats {
            total_frames: 0,
            avg_frame_us: 0,
            min_frame_us: 0,
            max_frame_us: 0,
            fps: 0.0,
            avg_compose_us: 0,
            avg_present_us: 0,
            avg_vsync_wait_us: 0,
        };
    }

    let mut total_us = 0u64;
    let mut min_us = u64::MAX;
    let mut max_us = 0u64;
    let mut total_compose = 0u64;
    let mut total_present = 0u64;
    let mut total_vsync = 0u64;

    for i in 0..count {
        let t = history[i];
        total_us += t.total_us;
        total_compose += t.compose_us;
        total_present += t.present_us;
        total_vsync += t.vsync_wait_us;
        min_us = min_us.min(t.total_us);
        max_us = max_us.max(t.total_us);
    }

    let frames = count as u64;
    let avg_frame = total_us / frames;
    let avg_compose = total_compose / frames;
    let avg_present = total_present / frames;
    let avg_vsync = total_vsync / frames;

    // Calculate FPS from average frame time
    let fps = if avg_frame > 0 {
        1_000_000.0 / avg_frame as f32
    } else {
        0.0
    };

    FrameStats {
        total_frames: frames,
        avg_frame_us: avg_frame,
        min_frame_us: if min_us == u64::MAX { 0 } else { min_us },
        max_frame_us: max_us,
        fps,
        avg_compose_us: avg_compose,
        avg_present_us: avg_present,
        avg_vsync_wait_us: avg_vsync,
    }
}

/// Get frame timing history for external inspection
pub fn frame_history() -> alloc::vec::Vec<FrameTiming> {
    let history = FRAME_HISTORY.lock();
    let count = *FRAME_HISTORY_COUNT.lock();
    let head = *FRAME_HISTORY_HEAD.lock();

    let mut result = alloc::vec::Vec::with_capacity(count);
    for i in 0..count {
        let idx = (head + MAX_FRAME_HISTORY - count + i) % MAX_FRAME_HISTORY;
        result.push(history[idx]);
    }
    result
}

/// Reset frame timing statistics
pub fn reset_frame_stats() {
    *FRAME_HISTORY_COUNT.lock() = 0;
    *FRAME_HISTORY_HEAD.lock() = 0;
}

// ─── Hardware Cursor ──────────────────────────────────────────────────

/// Hardware cursor state
#[derive(Clone, Copy, Debug)]
pub struct HardwareCursor {
    pub enabled: bool,
    pub x: u32,
    pub y: u32,
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    pub image_ptr: *mut u32,
    pub width: u32,
    pub height: u32,
}

static mut HW_CURSOR: HardwareCursor = HardwareCursor {
    enabled: false,
    x: 0,
    y: 0,
    hotspot_x: 0,
    hotspot_y: 0,
    image_ptr: core::ptr::null_mut(),
    width: 0,
    height: 0,
};

/// Default 32x32 arrow cursor (RGBA, 32-bit)
/// This is a simple arrow cursor pattern
const CURSOR_WIDTH: u32 = 32;
const CURSOR_HEIGHT: u32 = 32;

/// Create the default arrow cursor image data
fn create_default_cursor() -> alloc::vec::Vec<u32> {
    let mut cursor = alloc::vec::Vec::with_capacity((CURSOR_WIDTH * CURSOR_HEIGHT) as usize);

    // Arrow cursor pattern (white with black outline, transparent elsewhere)
    // Using a simple pattern: 0 = transparent, 0xFFFFFFFF = white, 0xFF000000 = black
    for y in 0..CURSOR_HEIGHT {
        for x in 0..CURSOR_WIDTH {
            let color = if x == 0 && y <= 31 {
                0xFF000000 // Black outline left edge
            } else if x == 1 && y <= 30 {
                0xFFFFFFFF // White fill
            } else if x == 2 && y <= 29 {
                0xFFFFFFFF
            } else if x == 3 && y <= 28 {
                0xFFFFFFFF
            } else if x == 4 && y <= 27 {
                0xFFFFFFFF
            } else if x == 5 && y <= 26 {
                0xFFFFFFFF
            } else if x == 6 && y <= 25 {
                0xFFFFFFFF
            } else if x == 7 && y <= 24 {
                0xFFFFFFFF
            } else if x == 8 && y <= 23 {
                0xFFFFFFFF
            } else if x == 9 && y <= 22 {
                0xFFFFFFFF
            } else if x == 10 && y <= 21 {
                0xFFFFFFFF
            } else if x == 11 && y <= 20 {
                0xFFFFFFFF
            } else if x == 12 && y <= 19 {
                0xFFFFFFFF
            } else if x == 13 && y <= 18 {
                0xFFFFFFFF
            } else if x == 14 && y <= 17 {
                0xFFFFFFFF
            } else if x == 15 && y <= 16 {
                0xFFFFFFFF
            } else {
                0x00000000 // Transparent
            };
            cursor.push(color);
        }
    }

    cursor
}

/// Initialize the hardware cursor
fn init_hardware_cursor(fb: FramebufferInfo) {
    unsafe {
        // Create cursor image in heap memory
        let cursor_data = create_default_cursor();
        let layout =
            core::alloc::Layout::from_size_align((cursor_data.len() * 4) as usize, 16).unwrap();

        let ptr = crate::memory::heap::heap_alloc(layout);
        if ptr.is_null() {
            println!("[compositor] Failed to allocate cursor memory");
            return;
        }

        // Copy cursor data to allocated memory
        core::ptr::copy_nonoverlapping(cursor_data.as_ptr(), ptr as *mut u32, cursor_data.len());

        // Configure hardware cursor via GPU driver
        let cursor_info = crate::drivers::gpu::CursorInfo {
            width: CURSOR_WIDTH,
            height: CURSOR_HEIGHT,
            format: 0,    // 32-bit RGBA
            hotspot_x: 1, // Arrow tip at (1, 1)
            hotspot_y: 1,
            data_ptr: ptr as u64,
        };

        if crate::drivers::gpu::set_cursor_info(&cursor_info) {
            HW_CURSOR.enabled = true;
            HW_CURSOR.image_ptr = ptr as *mut u32;
            HW_CURSOR.width = CURSOR_WIDTH;
            HW_CURSOR.height = CURSOR_HEIGHT;
            HW_CURSOR.hotspot_x = 1;
            HW_CURSOR.hotspot_y = 1;
            HW_CURSOR.x = fb.width / 2;
            HW_CURSOR.y = fb.height / 2;

            // Show cursor at center
            let _ = crate::drivers::gpu::set_cursor_state(
                crate::drivers::gpu::CursorState::Visible,
                HW_CURSOR.x,
                HW_CURSOR.y,
            );

            println!("[compositor] Hardware cursor enabled (32x32)");
        } else {
            println!("[compositor] Failed to enable hardware cursor");
            // Note: heap_dealloc may not exist, so we don't free on failure
        }
    }
}

/// Update hardware cursor position
pub fn set_hardware_cursor_pos(x: u32, y: u32) {
    unsafe {
        if !HW_CURSOR.enabled {
            return;
        }
        HW_CURSOR.x = x;
        HW_CURSOR.y = y;
        let _ =
            crate::drivers::gpu::set_cursor_state(crate::drivers::gpu::CursorState::Visible, x, y);
    }
}

/// Hide hardware cursor
pub fn hide_hardware_cursor() {
    unsafe {
        if HW_CURSOR.enabled {
            let _ = crate::drivers::gpu::set_cursor_state(
                crate::drivers::gpu::CursorState::Hidden,
                0,
                0,
            );
        }
    }
}

/// Show hardware cursor at current position
pub fn show_hardware_cursor() {
    unsafe {
        if HW_CURSOR.enabled {
            let _ = crate::drivers::gpu::set_cursor_state(
                crate::drivers::gpu::CursorState::Visible,
                HW_CURSOR.x,
                HW_CURSOR.y,
            );
        }
    }
}

/// Check if hardware cursor is enabled
pub fn is_hardware_cursor_enabled() -> bool {
    unsafe { HW_CURSOR.enabled }
}

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

    // Initialize hardware cursor
    init_hardware_cursor(*fb);
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
    let frame_start = crate::arch::timer::Timer::system_time_us();
    let frame_number = unsafe { FRAME_COUNTER };
    unsafe {
        FRAME_COUNTER += 1;
    }

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

    let compose_start = crate::arch::timer::Timer::system_time_us();

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
        let present_start = crate::arch::timer::Timer::system_time_us();
        rotate_and_present(fb, back_buffer);
        let present_end = crate::arch::timer::Timer::system_time_us();

        *FULL_REDRAW.lock() = false;
        *DIRTY_COUNT.lock() = 0;

        let frame_end = crate::arch::timer::Timer::system_time_us();
        record_frame_timing(FrameTiming {
            frame_number,
            compose_us: present_start - compose_start,
            present_us: present_end - present_start,
            vsync_wait_us: present_end
                - frame_start
                - (present_start - compose_start)
                - (present_end - present_start),
            total_us: frame_end - frame_start,
            start_timestamp_us: frame_start,
            end_timestamp_us: frame_end,
        });
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
        let present_start = crate::arch::timer::Timer::system_time_us();
        rotate_and_present_dirty(fb, back_buffer, &dirty_list);
        let present_end = crate::arch::timer::Timer::system_time_us();

        *DIRTY_COUNT.lock() = 0;

        let frame_end = crate::arch::timer::Timer::system_time_us();
        record_frame_timing(FrameTiming {
            frame_number,
            compose_us: present_start - compose_start,
            present_us: present_end - present_start,
            vsync_wait_us: present_end
                - frame_start
                - (present_start - compose_start)
                - (present_end - present_start),
            total_us: frame_end - frame_start,
            start_timestamp_us: frame_start,
            end_timestamp_us: frame_end,
        });
    }
}

/// Frame counter for timing
static mut FRAME_COUNTER: u64 = 0;

/// Rotate triple buffers and present the front buffer to the display
/// Rotation: back -> middle -> front -> back (for next frame)
fn rotate_and_present(fb: FramebufferInfo, back_buffer: *mut u32) {
    // Wait for VSYNC before presenting to avoid tearing
    let _ = crate::drivers::gpu::wait_for_vsync();

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
    // Wait for VSYNC before presenting to avoid tearing
    let _ = crate::drivers::gpu::wait_for_vsync();

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

    // Check if layer has alpha (for transparent windows)
    if layer.has_alpha {
        // Render with alpha blending using layer opacity
        render_layer_alpha(layer, fb, back_buf, theme);
    } else {
        // Opaque window rendering (original behavior)
        render_layer_opaque(layer, fb, back_buf, theme);
    }
}

/// Render an opaque layer (no alpha blending)
fn render_layer_opaque(
    layer: &Layer,
    fb: FramebufferInfo,
    back_buf: *mut u32,
    theme: &crate::wm::theme::Theme,
) {
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

/// Render a layer with alpha blending (Porter-Duff over operator)
fn render_layer_alpha(
    layer: &Layer,
    fb: FramebufferInfo,
    back_buf: *mut u32,
    theme: &crate::wm::theme::Theme,
) {
    let opacity = layer.opacity as u32; // 0-255
    let inv_opacity = 255 - opacity;

    // Helper: blend a source color over destination with opacity
    let blend_pixel = |src: u32, dst: u32| -> u32 {
        let src_a = opacity;
        let _dst_a = 255; // Destination is always opaque in back buffer

        // Porter-Duff "over" operator with premultiplied alpha
        // result = src * src_alpha + dst * (1 - src_alpha)
        let src_r = (src >> 16) & 0xFF;
        let src_g = (src >> 8) & 0xFF;
        let src_b = src & 0xFF;

        let dst_r = (dst >> 16) & 0xFF;
        let dst_g = (dst >> 8) & 0xFF;
        let dst_b = dst & 0xFF;

        let r = (src_r * src_a + dst_r * inv_opacity) / 255;
        let g = (src_g * src_a + dst_g * inv_opacity) / 255;
        let b = (src_b * src_a + dst_b * inv_opacity) / 255;

        0xFF000000 | (r << 16) | (g << 8) | b
    };

    // Window background with alpha
    fill_back_buffer_rect_alpha(
        fb,
        back_buf,
        layer.x,
        layer.y,
        layer.w,
        layer.h,
        theme.win_bg,
        blend_pixel,
    );

    // Window title bar with alpha
    if layer.h > 24 {
        fill_back_buffer_rect_alpha(
            fb,
            back_buf,
            layer.x,
            layer.y,
            layer.w,
            24,
            theme.title_bar,
            blend_pixel,
        );
    }

    // Window border with alpha
    draw_back_buffer_rect_outline_alpha(
        fb,
        back_buf,
        layer.x,
        layer.y,
        layer.w,
        layer.h,
        theme.win_border,
        blend_pixel,
    );

    // Close button with alpha
    if layer.w > 30 && layer.h > 20 {
        let btn_x = layer.x + layer.w - 22;
        let btn_y = layer.y + 4;
        fill_back_buffer_rect_alpha(
            fb,
            back_buf,
            btn_x,
            btn_y,
            16,
            16,
            theme.close_btn,
            blend_pixel,
        );
    }

    // Title text — alpha blended
    let title = crate::wm::window::get_title(layer.id);
    if !title.is_empty() && layer.w > 40 {
        draw_text_alpha(
            &fb,
            back_buf,
            layer.x + 8,
            layer.y + 4,
            title,
            theme.title_text,
            theme.title_bar,
            blend_pixel,
        );
    }
}

/// Fill a rectangle in the back buffer with alpha blending
fn fill_back_buffer_rect_alpha<F>(
    fb: FramebufferInfo,
    back_buf: *mut u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: u32,
    blend: F,
) where
    F: Fn(u32, u32) -> u32,
{
    unsafe {
        if back_buf.is_null() {
            return;
        }
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
                    let dst = core::ptr::read_volatile(back_buf.add(idx));
                    let blended = blend(color, dst);
                    core::ptr::write_volatile(back_buf.add(idx), blended);
                }
            }
        }
    }
}

/// Draw a rectangle outline in the back buffer with alpha blending
fn draw_back_buffer_rect_outline_alpha<F>(
    fb: FramebufferInfo,
    back_buf: *mut u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: u32,
    blend: F,
) where
    F: Fn(u32, u32) -> u32,
{
    // Top and bottom
    for dx in 0..w {
        let px = x + dx;
        if px >= fb.width {
            break;
        }
        set_back_pixel_alpha(fb, back_buf, px, y, color, &blend);
        if y + h > 0 {
            set_back_pixel_alpha(fb, back_buf, px, y + h - 1, color, &blend);
        }
    }
    // Left and right
    for dy in 0..h {
        let py = y + dy;
        if py >= fb.height {
            break;
        }
        set_back_pixel_alpha(fb, back_buf, x, py, color, &blend);
        if x + w > 0 {
            set_back_pixel_alpha(fb, back_buf, x + w - 1, py, color, &blend);
        }
    }
}

/// Set a single pixel in the back buffer with alpha blending
fn set_back_pixel_alpha<F>(
    fb: FramebufferInfo,
    back_buf: *mut u32,
    x: u32,
    y: u32,
    color: u32,
    blend: &F,
) where
    F: Fn(u32, u32) -> u32,
{
    unsafe {
        if x >= fb.width || y >= fb.height {
            return;
        }
        let idx = (y * fb.width + x) as usize;
        let back_buf_size = *BACK_BUF_SIZE.lock();
        if idx < back_buf_size / 4 {
            let dst = core::ptr::read_volatile(back_buf.add(idx));
            let blended = blend(color, dst);
            core::ptr::write_volatile(back_buf.add(idx), blended);
        }
    }
}

/// Draw text with alpha blending
fn draw_text_alpha<F>(
    fb: &FramebufferInfo,
    back_buf: *mut u32,
    x: u32,
    y: u32,
    text: &str,
    fg: u32,
    bg: u32,
    blend: F,
) where
    F: Fn(u32, u32) -> u32,
{
    use crate::display::console::{CHAR_HEIGHT, CHAR_WIDTH, FONT};

    let mut cx = x;
    for c in text.chars() {
        let code = c as usize;
        if code < 128 {
            let glyph = &FONT[code];
            for row in 0..CHAR_HEIGHT {
                let row_bits = glyph[row as usize];
                for col in 0..CHAR_WIDTH {
                    let px = cx + col;
                    let py = y + row;
                    if px >= fb.width || py >= fb.height {
                        continue;
                    }
                    let color = if (row_bits >> (7 - col)) & 1 != 0 {
                        fg
                    } else {
                        bg
                    };
                    set_back_pixel_alpha(*fb, back_buf, px, py, color, &blend);
                }
            }
        }
        cx += CHAR_WIDTH;
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

    #[test]
    fn test_frame_timing_zero() {
        let timing = FrameTiming::zero();
        assert_eq!(timing.frame_number, 0);
        assert_eq!(timing.compose_us, 0);
        assert_eq!(timing.present_us, 0);
        assert_eq!(timing.vsync_wait_us, 0);
        assert_eq!(timing.total_us, 0);
        assert_eq!(timing.start_timestamp_us, 0);
        assert_eq!(timing.end_timestamp_us, 0);
    }

    #[test]
    fn test_frame_timing_struct() {
        let timing = FrameTiming {
            frame_number: 42,
            compose_us: 1000,
            present_us: 500,
            vsync_wait_us: 2000,
            total_us: 3500,
            start_timestamp_us: 1000000,
            end_timestamp_us: 1003500,
        };
        assert_eq!(timing.frame_number, 42);
        assert_eq!(timing.compose_us, 1000);
        assert_eq!(timing.present_us, 500);
        assert_eq!(timing.vsync_wait_us, 2000);
        assert_eq!(timing.total_us, 3500);
    }

    #[test]
    fn test_frame_stats_zero() {
        reset_frame_stats();
        let stats = frame_stats();
        assert_eq!(stats.total_frames, 0);
        assert_eq!(stats.avg_frame_us, 0);
        assert_eq!(stats.min_frame_us, 0);
        assert_eq!(stats.max_frame_us, 0);
        assert_eq!(stats.fps, 0.0);
        assert_eq!(stats.avg_compose_us, 0);
        assert_eq!(stats.avg_present_us, 0);
        assert_eq!(stats.avg_vsync_wait_us, 0);
    }

    #[test]
    fn test_frame_stats_recording() {
        reset_frame_stats();

        // Record a few frames
        record_frame_timing(FrameTiming {
            frame_number: 1,
            compose_us: 1000,
            present_us: 500,
            vsync_wait_us: 2000,
            total_us: 3500,
            start_timestamp_us: 1000000,
            end_timestamp_us: 1003500,
        });
        record_frame_timing(FrameTiming {
            frame_number: 2,
            compose_us: 1100,
            present_us: 600,
            vsync_wait_us: 2100,
            total_us: 3800,
            start_timestamp_us: 1003500,
            end_timestamp_us: 1007300,
        });
        record_frame_timing(FrameTiming {
            frame_number: 3,
            compose_us: 900,
            present_us: 400,
            vsync_wait_us: 1900,
            total_us: 3200,
            start_timestamp_us: 1007300,
            end_timestamp_us: 1010500,
        });

        let stats = frame_stats();
        assert_eq!(stats.total_frames, 3);
        assert_eq!(stats.avg_compose_us, 1000); // (1000+1100+900)/3
        assert_eq!(stats.avg_present_us, 500); // (500+600+400)/3
        assert_eq!(stats.avg_vsync_wait_us, 2000); // (2000+2100+1900)/3
        assert_eq!(stats.min_frame_us, 3200);
        assert_eq!(stats.max_frame_us, 3800);
        // FPS = 1_000_000 / avg_frame_us ≈ 1_000_000 / 3500 ≈ 285
        assert!((stats.fps - 285.7).abs() < 1.0);
    }

    #[test]
    fn test_frame_history() {
        reset_frame_stats();

        record_frame_timing(FrameTiming {
            frame_number: 1,
            compose_us: 1000,
            present_us: 500,
            vsync_wait_us: 2000,
            total_us: 3500,
            start_timestamp_us: 1000000,
            end_timestamp_us: 1003500,
        });
        record_frame_timing(FrameTiming {
            frame_number: 2,
            compose_us: 1100,
            present_us: 600,
            vsync_wait_us: 2100,
            total_us: 3800,
            start_timestamp_us: 1003500,
            end_timestamp_us: 1007300,
        });

        let history = frame_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].frame_number, 1);
        assert_eq!(history[1].frame_number, 2);
    }

    #[test]
    fn test_hardware_cursor_struct() {
        let cursor = HardwareCursor {
            enabled: true,
            x: 100,
            y: 200,
            hotspot_x: 1,
            hotspot_y: 1,
            image_ptr: 0x1000 as *mut u32,
            width: 32,
            height: 32,
        };
        assert!(cursor.enabled);
        assert_eq!(cursor.x, 100);
        assert_eq!(cursor.y, 200);
        assert_eq!(cursor.width, 32);
        assert_eq!(cursor.height, 32);
    }

    #[test]
    fn test_alpha_blending_pixel() {
        // Test alpha blending: 50% white over black = gray
        let src = 0xFFFFFFFF; // White
        let dst = 0xFF000000; // Black
        let opacity: u32 = 128; // 50%
        let inv_opacity = 255 - opacity;

        let src_r = (src >> 16) & 0xFF;
        let src_g = (src >> 8) & 0xFF;
        let src_b = src & 0xFF;

        let dst_r = (dst >> 16) & 0xFF;
        let dst_g = (dst >> 8) & 0xFF;
        let dst_b = dst & 0xFF;

        let r = (src_r * opacity + dst_r * inv_opacity) / 255;
        let g = (src_g * opacity + dst_g * inv_opacity) / 255;
        let b = (src_b * opacity + dst_b * inv_opacity) / 255;

        let blended = 0xFF000000 | (r << 16) | (g << 8) | b;

        // 50% of 255 = ~127, so we expect ~127 for each channel
        assert!((r as i32 - 127).abs() <= 2);
        assert!((g as i32 - 127).abs() <= 2);
        assert!((b as i32 - 127).abs() <= 2);
    }

    #[test]
    fn test_alpha_blending_opaque() {
        // Test alpha blending: 100% opacity = source color
        let src = 0xFF123456;
        let dst = 0xFFABCDEF;
        let opacity: u32 = 255;
        let inv_opacity = 255 - opacity;

        let src_r = (src >> 16) & 0xFF;
        let src_g = (src >> 8) & 0xFF;
        let src_b = src & 0xFF;

        let dst_r = (dst >> 16) & 0xFF;
        let dst_g = (dst >> 8) & 0xFF;
        let dst_b = dst & 0xFF;

        let r = (src_r * opacity + dst_r * inv_opacity) / 255;
        let g = (src_g * opacity + dst_g * inv_opacity) / 255;
        let b = (src_b * opacity + dst_b * inv_opacity) / 255;

        let blended = 0xFF000000 | (r << 16) | (g << 8) | b;

        assert_eq!(blended, src);
    }

    #[test]
    fn test_alpha_blending_transparent() {
        // Test alpha blending: 0% opacity = destination color
        let src = 0xFF123456;
        let dst = 0xFFABCDEF;
        let opacity: u32 = 0;
        let inv_opacity = 255 - opacity;

        let src_r = (src >> 16) & 0xFF;
        let src_g = (src >> 8) & 0xFF;
        let src_b = src & 0xFF;

        let dst_r = (dst >> 16) & 0xFF;
        let dst_g = (dst >> 8) & 0xFF;
        let dst_b = dst & 0xFF;

        let r = (src_r * opacity + dst_r * inv_opacity) / 255;
        let g = (src_g * opacity + dst_g * inv_opacity) / 255;
        let b = (src_b * opacity + dst_b * inv_opacity) / 255;

        let blended = 0xFF000000 | (r << 16) | (g << 8) | b;

        assert_eq!(blended, dst);
    }
}
