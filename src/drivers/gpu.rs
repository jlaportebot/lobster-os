//! VideoCore VII GPU driver for BCM2712 (Raspberry Pi 5)
//!
//! The BCM2712 SoC includes a VideoCore VII GPU. This driver communicates
//! with the GPU via the VideoCore mailbox interface to:
//! - Query GPU firmware and hardware info
//! - Manage framebuffer allocation and configuration
//! - Control display power state and blanking
//! - Set display resolution, depth, and pixel order
//! - Manage cursor overlay
//! - Query and set clock rates for the V3D engine
//!
//! The mailbox property interface uses a shared-memory message buffer:
//!   [header: size, code] [tag1] [tag2] ... [end_tag: 0]
//! Each tag: [tag_id, buf_size, req_code, data...]
//!
//! No_std compatible — uses only core.

use crate::println;

// ─── Mailbox Interface ──────────────────────────────────────────
const MAILBOX_BASE: u64 = 0x10_7C00_B880;
const MAILBOX_READ: u64 = MAILBOX_BASE + 0x00;
const MAILBOX_STATUS: u64 = MAILBOX_BASE + 0x18;
const MAILBOX_WRITE: u64 = MAILBOX_BASE + 0x20;

const MAILBOX_CHANNEL_PROPERTY: u32 = 8;
const MAILBOX_STATUS_EMPTY: u32 = 0x40000000;
const MAILBOX_STATUS_FULL: u32 = 0x80000000;

// ─── Property Tags ──────────────────────────────────────────────
const TAG_GET_FIRMWARE_REVISION: u32 = 0x00000001;
const TAG_GET_BOARD_MODEL: u32 = 0x00010001;
const TAG_GET_BOARD_REVISION: u32 = 0x00010002;
const TAG_GET_ARM_MEMORY: u32 = 0x00010005;
const TAG_GET_VC_MEMORY: u32 = 0x00010006;
const TAG_GET_POWER_STATE: u32 = 0x00020001;
const TAG_SET_POWER_STATE: u32 = 0x00028001;
const TAG_GET_CLOCK_RATE: u32 = 0x00030002;
const TAG_SET_CLOCK_RATE: u32 = 0x00038002;
const TAG_GET_CLOCK_STATE: u32 = 0x00030001;
const TAG_SET_CLOCK_STATE: u32 = 0x00038001;
const TAG_ALLOCATE_BUFFER: u32 = 0x00040001;
const TAG_RELEASE_BUFFER: u32 = 0x00048001;
const TAG_BLANK_SCREEN: u32 = 0x00040002;
const TAG_GET_PHYSICAL_SIZE: u32 = 0x00040003;
const TAG_SET_PHYSICAL_SIZE: u32 = 0x00048003;
const TAG_GET_VIRTUAL_SIZE: u32 = 0x00040004;
const TAG_SET_VIRTUAL_SIZE: u32 = 0x00048004;
const TAG_GET_DEPTH: u32 = 0x00040005;
const TAG_SET_DEPTH: u32 = 0x00048005;
const TAG_GET_PIXEL_ORDER: u32 = 0x00040006;
const TAG_SET_PIXEL_ORDER: u32 = 0x00048006;
const TAG_GET_ALPHA_MODE: u32 = 0x00040007;
const TAG_SET_ALPHA_MODE: u32 = 0x00048007;
const TAG_GET_PITCH: u32 = 0x00040008;
const TAG_GET_VIRTUAL_OFFSET: u32 = 0x00040009;
const TAG_SET_VIRTUAL_OFFSET: u32 = 0x00048009;
const TAG_SET_CURSOR_INFO: u32 = 0x00008010;
const TAG_SET_CURSOR_STATE: u32 = 0x00008011;
const TAG_GET_EDID_BLOCK: u32 = 0x00030020;

// Clock IDs
const CLOCK_ID_ARM: u32 = 0;
const CLOCK_ID_CORE: u32 = 2;
const CLOCK_ID_V3D: u32 = 5;
const CLOCK_ID_PIXEL: u32 = 7;
const CLOCK_ID_PWM: u32 = 9;
const CLOCK_ID_EMMC: u32 = 10;

// Power domains
const POWER_DOMAIN_USB: u32 = 0;
const POWER_DOMAIN_UART0: u32 = 2;
const POWER_DOMAIN_UART1: u32 = 3;

// ─── Property Message Buffer ────────────────────────────────────
/// Maximum number of tags per message
const MAX_TAGS: usize = 16;

/// Maximum data words per tag
const MAX_TAG_DATA: usize = 32;

/// Property message buffer (static, aligned to 16 bytes for mailbox)
#[repr(C, align(16))]
struct PropertyMessage {
    buf_size: u32,
    code: u32,
    tags: [TagEntry; MAX_TAGS],
    end_tag: u32,
}

#[repr(C)]
struct TagEntry {
    tag_id: u32,
    buf_size: u32,
    req_code: u32,
    data: [u32; MAX_TAG_DATA],
}

/// Static property message buffer
static mut PROPERTY_BUFFER: [u32; 264] = [0; 264];

// ─── GPU State ──────────────────────────────────────────────────

/// Display info cached from GPU queries
#[derive(Clone, Copy, Debug)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub pitch: u32,
    pub fb_addr: u64,
    pub fb_size: u32,
    pub pixel_order: u32, // 0=BGR, 1=RGB
    pub alpha_mode: u32,  // 0=per-pixel, 1=global
}

/// GPU hardware info
#[derive(Clone, Copy, Debug)]
pub struct GpuInfo {
    pub firmware_rev: u32,
    pub board_model: u32,
    pub board_rev: u32,
    pub vc_mem_base: u32,
    pub vc_mem_size: u32,
    pub arm_mem_base: u32,
    pub arm_mem_size: u32,
    pub v3d_clock: u32,
    pub core_clock: u32,
}

static mut DISPLAY_INFO: DisplayInfo = DisplayInfo {
    width: 0,
    height: 0,
    bpp: 0,
    pitch: 0,
    fb_addr: 0,
    fb_size: 0,
    pixel_order: 0,
    alpha_mode: 0,
};

static mut GPU_INFO: GpuInfo = GpuInfo {
    firmware_rev: 0,
    board_model: 0,
    board_rev: 0,
    vc_mem_base: 0,
    vc_mem_size: 0,
    arm_mem_base: 0,
    arm_mem_size: 0,
    v3d_clock: 0,
    core_clock: 0,
};

static mut GPU_INITIALIZED: bool = false;

// ─── Low-level Mailbox Access ───────────────────────────────────

unsafe fn read_reg(addr: u64) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

unsafe fn write_reg(addr: u64, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

/// Send a property message via the mailbox and wait for response
/// Returns true on success
unsafe fn send_property_message(_size: usize) -> bool {
    let buf_ptr = &PROPERTY_BUFFER as *const _ as u32;

    // Wait for mailbox to not be full
    while read_reg(MAILBOX_STATUS) & MAILBOX_STATUS_FULL != 0 {
        crate::arch::cpu::nop();
    }

    // Send message on channel 8 (property)
    let msg = (buf_ptr & !0xF) | MAILBOX_CHANNEL_PROPERTY;
    write_reg(MAILBOX_WRITE, msg);

    // Wait for response
    loop {
        while read_reg(MAILBOX_STATUS) & MAILBOX_STATUS_EMPTY != 0 {
            crate::arch::cpu::nop();
        }

        let response = read_reg(MAILBOX_READ);
        if (response & 0xF) == MAILBOX_CHANNEL_PROPERTY {
            let data = response & !0xF;
            // Check if it's our response (same address)
            if data == buf_ptr {
                // Check response code: bit 31 = success
                return PROPERTY_BUFFER[1] & 0x80000000 != 0;
            }
        }
    }
}

// ─── Property Tag Helpers ───────────────────────────────────────

/// Build a single-tag property message in the static buffer
/// Returns the buffer size in u32 words
unsafe fn build_single_tag(tag_id: u32, data_words: &[u32]) -> usize {
    let data_len = data_words.len();
    let buf_size = 2 + 3 + data_len + 1; // header + tag + end_tag

    PROPERTY_BUFFER[0] = (buf_size * 4) as u32; // Total buffer size in bytes
    PROPERTY_BUFFER[1] = 0; // Request code = 0

    PROPERTY_BUFFER[2] = tag_id;
    PROPERTY_BUFFER[3] = (data_len * 4) as u32; // Buffer size in bytes
    PROPERTY_BUFFER[4] = 0; // Request code = 0

    for i in 0..data_len {
        PROPERTY_BUFFER[5 + i] = data_words[i];
    }

    // End tag
    PROPERTY_BUFFER[5 + data_len] = 0;

    buf_size
}

/// Build a two-tag property message
unsafe fn build_two_tags(
    tag1_id: u32,
    tag1_data: &[u32],
    tag2_id: u32,
    tag2_data: &[u32],
) -> usize {
    let d1 = tag1_data.len();
    let d2 = tag2_data.len();
    let buf_size = 2 + (3 + d1) + (3 + d2) + 1;

    PROPERTY_BUFFER[0] = (buf_size * 4) as u32;
    PROPERTY_BUFFER[1] = 0;

    let mut off = 2;

    // Tag 1
    PROPERTY_BUFFER[off] = tag1_id;
    PROPERTY_BUFFER[off + 1] = (d1 * 4) as u32;
    PROPERTY_BUFFER[off + 2] = 0;
    for i in 0..d1 {
        PROPERTY_BUFFER[off + 3 + i] = tag1_data[i];
    }
    off += 3 + d1;

    // Tag 2
    PROPERTY_BUFFER[off] = tag2_id;
    PROPERTY_BUFFER[off + 1] = (d2 * 4) as u32;
    PROPERTY_BUFFER[off + 2] = 0;
    for i in 0..d2 {
        PROPERTY_BUFFER[off + 3 + i] = tag2_data[i];
    }
    off += 3 + d2;

    PROPERTY_BUFFER[off] = 0; // End tag

    buf_size
}

// ─── GPU Query Functions ────────────────────────────────────────

/// Query firmware revision from GPU
unsafe fn query_firmware_revision() -> u32 {
    let size = build_single_tag(TAG_GET_FIRMWARE_REVISION, &[0]);
    if send_property_message(size) {
        PROPERTY_BUFFER[5]
    } else {
        0
    }
}

/// Query board model
unsafe fn query_board_model() -> u32 {
    let size = build_single_tag(TAG_GET_BOARD_MODEL, &[0]);
    if send_property_message(size) {
        PROPERTY_BUFFER[5]
    } else {
        0
    }
}

/// Query board revision
unsafe fn query_board_revision() -> u32 {
    let size = build_single_tag(TAG_GET_BOARD_REVISION, &[0]);
    if send_property_message(size) {
        PROPERTY_BUFFER[5]
    } else {
        0
    }
}

/// Query VideoCore memory (base and size)
unsafe fn query_vc_memory() -> (u32, u32) {
    let size = build_single_tag(TAG_GET_VC_MEMORY, &[0, 0]);
    if send_property_message(size) {
        (PROPERTY_BUFFER[5], PROPERTY_BUFFER[6])
    } else {
        (0, 0)
    }
}

/// Query ARM memory (base and size)
unsafe fn query_arm_memory() -> (u32, u32) {
    let size = build_single_tag(TAG_GET_ARM_MEMORY, &[0, 0]);
    if send_property_message(size) {
        (PROPERTY_BUFFER[5], PROPERTY_BUFFER[6])
    } else {
        (0, 0)
    }
}

/// Query a clock rate
unsafe fn query_clock_rate(clock_id: u32) -> u32 {
    let size = build_single_tag(TAG_GET_CLOCK_RATE, &[clock_id, 0]);
    if send_property_message(size) {
        PROPERTY_BUFFER[6]
    } else {
        0
    }
}

/// Set a clock rate (skip setting if rate=0)
unsafe fn set_clock_rate(clock_id: u32, rate: u32, skip_setting: u32) -> bool {
    let size = build_single_tag(TAG_SET_CLOCK_RATE, &[clock_id, rate, skip_setting]);
    send_property_message(size)
}

/// Get display physical size
unsafe fn query_physical_size() -> (u32, u32) {
    let size = build_single_tag(TAG_GET_PHYSICAL_SIZE, &[0, 0]);
    if send_property_message(size) {
        (PROPERTY_BUFFER[5], PROPERTY_BUFFER[6])
    } else {
        (0, 0)
    }
}

/// Set display physical size
pub fn set_physical_size(width: u32, height: u32) -> bool {
    unsafe {
        let size = build_single_tag(TAG_SET_PHYSICAL_SIZE, &[width, height]);
        send_property_message(size)
    }
}

/// Get display virtual size
unsafe fn query_virtual_size() -> (u32, u32) {
    let size = build_single_tag(TAG_GET_VIRTUAL_SIZE, &[0, 0]);
    if send_property_message(size) {
        (PROPERTY_BUFFER[5], PROPERTY_BUFFER[6])
    } else {
        (0, 0)
    }
}

/// Set display virtual size (width, height)
pub fn set_virtual_size(width: u32, height: u32) -> bool {
    unsafe {
        let size = build_single_tag(TAG_SET_VIRTUAL_SIZE, &[width, height]);
        send_property_message(size)
    }
}

/// Get color depth (bits per pixel)
unsafe fn query_depth() -> u32 {
    let size = build_single_tag(TAG_GET_DEPTH, &[0]);
    if send_property_message(size) {
        PROPERTY_BUFFER[5]
    } else {
        0
    }
}

/// Set color depth (bits per pixel)
pub fn set_depth(bpp: u32) -> bool {
    unsafe {
        let size = build_single_tag(TAG_SET_DEPTH, &[bpp]);
        send_property_message(size)
    }
}

/// Get pixel order (0=BGR, 1=RGB)
unsafe fn query_pixel_order() -> u32 {
    let size = build_single_tag(TAG_GET_PIXEL_ORDER, &[0]);
    if send_property_message(size) {
        PROPERTY_BUFFER[5]
    } else {
        0
    }
}

/// Set pixel order
pub fn set_pixel_order(rgb: bool) -> bool {
    unsafe {
        let size = build_single_tag(TAG_SET_PIXEL_ORDER, &[if rgb { 1 } else { 0 }]);
        send_property_message(size)
    }
}

/// Get alpha mode
unsafe fn query_alpha_mode() -> u32 {
    let size = build_single_tag(TAG_GET_ALPHA_MODE, &[0]);
    if send_property_message(size) {
        PROPERTY_BUFFER[5]
    } else {
        0
    }
}

/// Set alpha mode (0=per-pixel alpha, 1=global alpha)
pub fn set_alpha_mode(mode: u32) -> bool {
    unsafe {
        let size = build_single_tag(TAG_SET_ALPHA_MODE, &[mode]);
        send_property_message(size)
    }
}

/// Get framebuffer pitch (bytes per row)
unsafe fn query_pitch() -> u32 {
    let size = build_single_tag(TAG_GET_PITCH, &[0]);
    if send_property_message(size) {
        PROPERTY_BUFFER[5]
    } else {
        0
    }
}

/// Allocate framebuffer and get its address and size
unsafe fn allocate_buffer(alignment: u32) -> (u64, u32) {
    let size = build_single_tag(TAG_ALLOCATE_BUFFER, &[alignment, 0]);
    if send_property_message(size) {
        let addr = PROPERTY_BUFFER[5] as u64;
        let sz = PROPERTY_BUFFER[6];
        (addr, sz)
    } else {
        (0, 0)
    }
}

/// Release the framebuffer
pub fn release_buffer() -> bool {
    unsafe {
        let size = build_single_tag(TAG_RELEASE_BUFFER, &[0]);
        send_property_message(size)
    }
}

/// Blank/unblank the screen
pub fn blank_screen(blank: bool) -> bool {
    unsafe {
        let size = build_single_tag(TAG_BLANK_SCREEN, &[if blank { 1 } else { 0 }]);
        send_property_message(size)
    }
}

/// Get virtual offset (double-buffering)
unsafe fn query_virtual_offset() -> (u32, u32) {
    let size = build_single_tag(TAG_GET_VIRTUAL_OFFSET, &[0, 0]);
    if send_property_message(size) {
        (PROPERTY_BUFFER[5], PROPERTY_BUFFER[6])
    } else {
        (0, 0)
    }
}

/// Set virtual offset (for double-buffering / page flip)
pub fn set_virtual_offset(x: u32, y: u32) -> bool {
    unsafe {
        let size = build_single_tag(TAG_SET_VIRTUAL_OFFSET, &[x, y]);
        send_property_message(size)
    }
}

/// Get power state for a domain
pub fn get_power_state(domain_id: u32) -> (bool, bool) {
    unsafe {
        let size = build_single_tag(TAG_GET_POWER_STATE, &[domain_id, 0]);
        if send_property_message(size) {
            let state = PROPERTY_BUFFER[6];
            let powered = (state & 1) != 0;
            let exists = (state & 2) != 0;
            (powered, exists)
        } else {
            (false, false)
        }
    }
}

/// Set power state for a domain
pub fn set_power_state(domain_id: u32, on: bool, wait: bool) -> bool {
    unsafe {
        let state = if on { 1 } else { 0 } | if wait { 2 } else { 0 };
        let size = build_single_tag(TAG_SET_POWER_STATE, &[domain_id, state]);
        send_property_message(size)
    }
}

// ─── Cursor Support ─────────────────────────────────────────────

/// Cursor info for the set_cursor_info tag
#[derive(Clone, Copy, Debug)]
pub struct CursorInfo {
    pub width: u32,
    pub height: u32,
    pub format: u32, // 0=32bit RGBA
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    pub data_ptr: u64, // Pointer to cursor image data
}

/// Cursor state
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CursorState {
    Hidden,
    Visible,
}

/// Set cursor info (image, hotspot)
pub fn set_cursor_info(info: &CursorInfo) -> bool {
    unsafe {
        let size = build_single_tag(
            TAG_SET_CURSOR_INFO,
            &[
                info.width,
                info.height,
                info.format,
                info.hotspot_x,
                info.hotspot_y,
                (info.data_ptr & 0xFFFFFFFF) as u32,
            ],
        );
        send_property_message(size)
    }
}

/// Set cursor state (show/hide + position)
pub fn set_cursor_state(state: CursorState, x: u32, y: u32) -> bool {
    unsafe {
        let flags = match state {
            CursorState::Hidden => 0,
            CursorState::Visible => 1,
        };
        let size = build_single_tag(TAG_SET_CURSOR_STATE, &[flags, 0, x, y]);
        send_property_message(size)
    }
}

// ─── EDID Support ───────────────────────────────────────────────

/// Read an EDID block from the display
pub fn get_edid_block(block_num: u32) -> [u8; 128] {
    let mut result = [0u8; 128];
    unsafe {
        let mut data = [0u32; 33];
        data[0] = block_num;
        let size = build_single_tag(TAG_GET_EDID_BLOCK, &data[..1]);

        if send_property_message(size) {
            // Check response length
            let resp_len = PROPERTY_BUFFER[4] as usize;
            if resp_len >= 128 {
                for i in 0..32 {
                    let word = PROPERTY_BUFFER[5 + i];
                    result[i * 4] = (word & 0xFF) as u8;
                    result[i * 4 + 1] = ((word >> 8) & 0xFF) as u8;
                    result[i * 4 + 2] = ((word >> 16) & 0xFF) as u8;
                    result[i * 4 + 3] = ((word >> 24) & 0xFF) as u8;
                }
            }
        }
    }
    result
}

// ─── V3D Clock Management ───────────────────────────────────────

/// Get the current V3D clock rate
pub fn get_v3d_clock() -> u32 {
    unsafe { query_clock_rate(CLOCK_ID_V3D) }
}

/// Set the V3D clock rate (for GPU compute)
pub fn set_v3d_clock(rate_hz: u32) -> bool {
    unsafe { set_clock_rate(CLOCK_ID_V3D, rate_hz, 1) }
}

/// Get the core clock rate
pub fn get_core_clock() -> u32 {
    unsafe { query_clock_rate(CLOCK_ID_CORE) }
}

/// Set the core clock rate
pub fn set_core_clock(rate_hz: u32) -> bool {
    unsafe { set_clock_rate(CLOCK_ID_CORE, rate_hz, 1) }
}

/// Get the pixel clock rate
pub fn get_pixel_clock() -> u32 {
    unsafe { query_clock_rate(CLOCK_ID_PIXEL) }
}

/// Get the EMMC clock rate
pub fn get_emmc_clock() -> u32 {
    unsafe { query_clock_rate(CLOCK_ID_EMMC) }
}

// ─── Initialization ─────────────────────────────────────────────

/// Initialize the GPU driver: query all hardware info and configure display
pub fn init() {
    unsafe {
        if GPU_INITIALIZED {
            return;
        }

        println!("[gpu] Querying VideoCore VII...");

        // Query hardware info
        GPU_INFO.firmware_rev = query_firmware_revision();
        GPU_INFO.board_model = query_board_model();
        GPU_INFO.board_rev = query_board_revision();

        let (vc_base, vc_size) = query_vc_memory();
        GPU_INFO.vc_mem_base = vc_base;
        GPU_INFO.vc_mem_size = vc_size;

        let (arm_base, arm_size) = query_arm_memory();
        GPU_INFO.arm_mem_base = arm_base;
        GPU_INFO.arm_mem_size = arm_size;

        GPU_INFO.v3d_clock = query_clock_rate(CLOCK_ID_V3D);
        GPU_INFO.core_clock = query_clock_rate(CLOCK_ID_CORE);

        println!("[gpu] Firmware: {:#010x}", GPU_INFO.firmware_rev);
        println!(
            "[gpu] Board: model={:#x} rev={:#x}",
            GPU_INFO.board_model, GPU_INFO.board_rev
        );
        println!(
            "[gpu] VC memory: base={:#x} size={}MB",
            GPU_INFO.vc_mem_base,
            GPU_INFO.vc_mem_size / (1024 * 1024)
        );
        println!(
            "[gpu] ARM memory: base={:#x} size={}MB",
            GPU_INFO.arm_mem_base,
            GPU_INFO.arm_mem_size / (1024 * 1024)
        );
        println!("[gpu] V3D clock: {} MHz", GPU_INFO.v3d_clock / 1_000_000);
        println!("[gpu] Core clock: {} MHz", GPU_INFO.core_clock / 1_000_000);

        // Query current display settings
        let (w, h) = query_physical_size();
        DISPLAY_INFO.width = w;
        DISPLAY_INFO.height = h;

        DISPLAY_INFO.bpp = query_depth();
        DISPLAY_INFO.pitch = query_pitch();
        DISPLAY_INFO.pixel_order = query_pixel_order();
        DISPLAY_INFO.alpha_mode = query_alpha_mode();

        // Allocate framebuffer
        let (fb_addr, fb_size) = allocate_buffer(16);
        DISPLAY_INFO.fb_addr = fb_addr;
        DISPLAY_INFO.fb_size = fb_size;

        println!(
            "[gpu] Display: {}x{} @ {}bpp",
            DISPLAY_INFO.width, DISPLAY_INFO.height, DISPLAY_INFO.bpp
        );
        println!("[gpu] Pitch: {} bytes/row", DISPLAY_INFO.pitch);
        println!(
            "[gpu] Framebuffer: {:#x} ({} KB)",
            DISPLAY_INFO.fb_addr,
            DISPLAY_INFO.fb_size / 1024
        );
        println!(
            "[gpu] Pixel order: {}",
            if DISPLAY_INFO.pixel_order == 1 {
                "RGB"
            } else {
                "BGR"
            }
        );

        GPU_INITIALIZED = true;
    }
}

/// Configure the display with the given parameters
/// This sets physical + virtual size, depth, pixel order, and alpha mode in one go
pub fn configure_display(width: u32, height: u32, bpp: u32, rgb: bool) -> bool {
    unsafe {
        // Set physical size
        let size = build_single_tag(TAG_SET_PHYSICAL_SIZE, &[width, height]);
        if !send_property_message(size) {
            return false;
        }

        // Set virtual size
        let size = build_single_tag(TAG_SET_VIRTUAL_SIZE, &[width, height]);
        if !send_property_message(size) {
            return false;
        }

        // Set depth
        let size = build_single_tag(TAG_SET_DEPTH, &[bpp]);
        if !send_property_message(size) {
            return false;
        }

        // Set pixel order
        let size = build_single_tag(TAG_SET_PIXEL_ORDER, &[if rgb { 1 } else { 0 }]);
        if !send_property_message(size) {
            return false;
        }

        // Allocate framebuffer
        let (fb_addr, fb_size) = allocate_buffer(16);

        // Query pitch
        let pitch = query_pitch();

        // Update cached info
        DISPLAY_INFO.width = width;
        DISPLAY_INFO.height = height;
        DISPLAY_INFO.bpp = bpp;
        DISPLAY_INFO.pitch = pitch;
        DISPLAY_INFO.fb_addr = fb_addr;
        DISPLAY_INFO.fb_size = fb_size;
        DISPLAY_INFO.pixel_order = if rgb { 1 } else { 0 };

        true
    }
}

// ─── Public Accessors ───────────────────────────────────────────

/// Get the cached display info
pub fn display_info() -> DisplayInfo {
    unsafe { DISPLAY_INFO }
}

/// Get the cached GPU info
pub fn gpu_info() -> GpuInfo {
    unsafe { GPU_INFO }
}

/// Check if GPU driver is initialized
pub fn is_initialized() -> bool {
    unsafe { GPU_INITIALIZED }
}

/// Get framebuffer address
pub fn fb_address() -> u64 {
    unsafe { DISPLAY_INFO.fb_addr }
}

/// Get framebuffer size in bytes
pub fn fb_size() -> u32 {
    unsafe { DISPLAY_INFO.fb_size }
}

/// Get display dimensions
pub fn dimensions() -> (u32, u32) {
    unsafe { (DISPLAY_INFO.width, DISPLAY_INFO.height) }
}

/// Get bits per pixel
pub fn bpp() -> u32 {
    unsafe { DISPLAY_INFO.bpp }
}

/// Get framebuffer pitch
pub fn pitch() -> u32 {
    unsafe { DISPLAY_INFO.pitch }
}

/// Print detailed GPU status
pub fn print_status() {
    unsafe {
        let info = GPU_INFO;
        let disp = DISPLAY_INFO;
        println!("[gpu] ═══ VideoCore VII Status ═══");
        println!("[gpu] Firmware:   {:#010x}", info.firmware_rev);
        println!(
            "[gpu] Board:      model={:#x} rev={:#x}",
            info.board_model, info.board_rev
        );
        println!(
            "[gpu] VC memory:  {:#x} ({} MB)",
            info.vc_mem_base,
            info.vc_mem_size / (1024 * 1024)
        );
        println!(
            "[gpu] ARM memory: {:#x} ({} MB)",
            info.arm_mem_base,
            info.arm_mem_size / (1024 * 1024)
        );
        println!("[gpu] V3D clock:  {} MHz", info.v3d_clock / 1_000_000);
        println!("[gpu] Core clock: {} MHz", info.core_clock / 1_000_000);
        println!("[gpu] ─── Display ───");
        println!("[gpu] Resolution: {}x{}", disp.width, disp.height);
        println!("[gpu] Depth:      {} bpp", disp.bpp);
        println!("[gpu] Pitch:      {} bytes/row", disp.pitch);
        println!("[gpu] FB addr:    {:#x}", disp.fb_addr);
        println!("[gpu] FB size:    {} KB", disp.fb_size / 1024);
        println!(
            "[gpu] Pixel ord:  {}",
            if disp.pixel_order == 1 { "RGB" } else { "BGR" }
        );
        println!(
            "[gpu] Alpha mode: {}",
            if disp.alpha_mode == 1 {
                "global"
            } else {
                "per-pixel"
            }
        );
    }
}
