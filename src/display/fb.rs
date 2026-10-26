//! Framebuffer driver — BCM2835 mailbox property channel for display setup
//!
//! QEMU raspi5b: bcm2835_fb (created by base peripherals) places framebuffer at vcram_base + 0x100000 = 0x8100000
//! Default resolution: 640x480 @ 16bpp. Property channel works on QEMU raspi5b when mailbox base is correct.

use crate::memory::PHYS_MAP_OFFSET;
use crate::println;

/// Mailbox base address (mapped via BCM2712 peripheral base)
#[cfg(not(any(feature = "qemu", feature = "qemu_raspi5b")))]
const MAILBOX_BASE: u64 = 0x10_7C00_B880; // BCM2712 (real Pi 5) - SOC peripheral base + offset

#[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
const MAILBOX_BASE: u64 = 0xFE_00_B880; // BCM2711 (QEMU raspi4b) - peri_base (0xFE000000) + 0xB880

#[cfg(feature = "qemu_raspi5b")]
const MAILBOX_BASE: u64 = 0x7C00_B880; // BCM2711-style at peri_base (0x7C000000) + 0xB880 (QEMU raspi5b)

/// Mailbox register offsets (relative to MAILBOX_BASE)
const MAILBOX_READ: u64 = MAILBOX_BASE + 0x00;
const MAILBOX_STATUS: u64 = MAILBOX_BASE + 0x18;
const MAILBOX_WRITE: u64 = MAILBOX_BASE + 0x20;

/// Mailbox property channel (channel 8)
const CHANNEL_PROPERTY: u32 = 8;

/// Mailbox status flags
const MAILBOX_FULL: u32 = 0x80000000;
const MAILBOX_EMPTY: u32 = 0x40000000;

/// Property tag IDs (from raspberrypi-fw-defs.h)
const TAG_ALLOCATE_BUFFER: u32 = 0x00040001;
const TAG_SET_PHYSICAL_SIZE: u32 = 0x00048003;
const TAG_SET_VIRTUAL_SIZE: u32 = 0x00048004;
const TAG_SET_DEPTH: u32 = 0x00048005;
const TAG_SET_PIXEL_ORDER: u32 = 0x00048006;
const TAG_GET_PHYSICAL_SIZE: u32 = 0x00040003;
const TAG_GET_VIRTUAL_SIZE: u32 = 0x00040004;
const TAG_GET_DEPTH: u32 = 0x00040005;
const TAG_GET_PITCH: u32 = 0x00040008;
const TAG_RELEASE_BUFFER: u32 = 0x00048001;
const TAG_END: u32 = 0x00000000;

/// Property tag request/response codes
const CODE_REQUEST: u32 = 0x00000000;
const CODE_RESPONSE_SUCCESS: u32 = 0x80000000;

/// Framebuffer error types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FbError {
    MailboxTimeout,
    InvalidResponse,
    BufferAllocationFailed,
    UnsupportedDepth,
    ConfigurationFailed,
}

/// Framebuffer information
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub base_addr: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub virt_addr: u64,
}

impl FramebufferInfo {
    pub const fn empty() -> Self {
        Self {
            base_addr: 0,
            size: 0,
            width: 0,
            height: 0,
            pitch: 0,
            bpp: 0,
            virt_addr: 0,
        }
    }

    /// Total pixels
    pub fn total_pixels(&self) -> u32 {
        self.width * self.height
    }

    /// Size in bytes
    pub fn buffer_size(&self) -> u64 {
        self.size
    }
}

/// Property message buffer — 256 u32 words (1024 bytes, 16-byte aligned)
const PROP_BUF_SIZE: usize = 256;

/// Global property message buffer (16-byte aligned)
#[repr(C, align(16))]
struct PropertyBuffer {
    data: [u32; PROP_BUF_SIZE],
}

static mut PROP_BUFFER: PropertyBuffer = PropertyBuffer {
    data: [0u32; PROP_BUF_SIZE],
};

/// Read from mailbox
unsafe fn mailbox_read() -> u32 {
    let mut count = 0u32;
    loop {
        // Wait until there's data
        while core::ptr::read_volatile(MAILBOX_STATUS as *mut u32) & MAILBOX_EMPTY != 0 {
            count += 1;
            if count > 1_000_000 {
                return 0;
            }
            core::hint::spin_loop();
        }

        let value = core::ptr::read_volatile(MAILBOX_READ as *mut u32);

        // Check it's from the property channel
        if (value & 0xF) == CHANNEL_PROPERTY {
            return value & !0xF;
        }
    }
}

/// Write to mailbox
unsafe fn mailbox_write(value: u32) {
    // Wait until we can write
    let mut count = 0u32;
    while core::ptr::read_volatile(MAILBOX_STATUS as *mut u32) & MAILBOX_FULL != 0 {
        count += 1;
        if count > 1_000_000 {
            return;
        }
        core::hint::spin_loop();
    }

    core::ptr::write_volatile(MAILBOX_WRITE as *mut u32, value);
}

/// Convert a virtual kernel address to physical (bus) address for DMA/mailbox
/// On this kernel, virtual = physical + PHYS_MAP_OFFSET
fn virt_to_phys(virt: u64) -> u64 {
    virt - PHYS_MAP_OFFSET
}

/// Send a property message and wait for response
unsafe fn send_property_message(_len: usize) -> Result<(), FbError> {
    // Get the virtual address of our buffer
    let buf_virt = &PROP_BUFFER as *const PropertyBuffer as u64;

    // Convert to physical (bus) address for the GPU/mailbox
    let bus_addr = virt_to_phys(buf_virt);

    mailbox_write((bus_addr as u32) | CHANNEL_PROPERTY);

    // Wait for response
    let response_addr = mailbox_read();
    if response_addr == 0 {
        return Err(FbError::MailboxTimeout);
    }

    // Check response code
    if PROP_BUFFER.data[1] != CODE_RESPONSE_SUCCESS {
        return Err(FbError::InvalidResponse);
    }

    Ok(())
}

/// Initialize the framebuffer via mailbox property channel
pub fn init(width: u32, height: u32, bpp: u32) -> Result<FramebufferInfo, FbError> {
    unsafe {
        let buf = &mut PROP_BUFFER.data;

        // Build property message
        let mut idx = 0usize;

        // Header: size, code, tags...
        // We'll fill the size at the end
        let size_pos = idx;
        idx += 1; // size placeholder
        buf[idx] = CODE_REQUEST;
        idx += 1;

        // Tag: Set physical size
        buf[idx] = TAG_SET_PHYSICAL_SIZE;
        idx += 1;
        buf[idx] = 8; // value size
        idx += 1;
        buf[idx] = 0; // request
        idx += 1;
        buf[idx] = width;
        idx += 1;
        buf[idx] = height;
        idx += 1;

        // Tag: Set virtual size
        buf[idx] = TAG_SET_VIRTUAL_SIZE;
        idx += 1;
        buf[idx] = 8;
        idx += 1;
        buf[idx] = 0;
        idx += 1;
        buf[idx] = width;
        idx += 1;
        buf[idx] = height;
        idx += 1;

        // Tag: Set depth
        buf[idx] = TAG_SET_DEPTH;
        idx += 1;
        buf[idx] = 4;
        idx += 1;
        buf[idx] = 0;
        idx += 1;
        buf[idx] = bpp;
        idx += 1;

        // Tag: Set pixel order = RGB (1). Without this the firmware defaults
        // to BGR (pixo=0) and every 0xFFRRGGBB color renders channel-swapped.
        buf[idx] = TAG_SET_PIXEL_ORDER;
        idx += 1;
        buf[idx] = 4;
        idx += 1;
        buf[idx] = 0;
        idx += 1;
        buf[idx] = 1; // RGB
        idx += 1;

        // Tag: Allocate buffer
        buf[idx] = TAG_ALLOCATE_BUFFER;
        idx += 1;
        buf[idx] = 4; // value size
        idx += 1;
        buf[idx] = 0; // request
        idx += 1;
        buf[idx] = 16; // alignment = 16
        idx += 1;

        // Tag: Get pitch
        buf[idx] = TAG_GET_PITCH;
        idx += 1;
        buf[idx] = 4;
        idx += 1;
        buf[idx] = 0;
        idx += 1;
        buf[idx] = 0; // placeholder
        idx += 1;

        // End tag
        buf[idx] = TAG_END;
        idx += 1;

        // Fill in total size
        buf[size_pos] = (idx * 4) as u32;

        // Send message
        send_property_message(idx)?;

        // Parse response — find the tags we sent
        let mut fb_addr = 0u64;
        let mut fb_size = 0u64;
        let mut pitch = 0u32;
        let mut resp_width = width;
        let mut resp_height = height;
        let mut resp_bpp = bpp;

        // Walk through tags in response
        let mut tag_idx = 2; // Skip header
        while buf[tag_idx] != TAG_END && tag_idx < PROP_BUF_SIZE - 4 {
            let tag = buf[tag_idx];
            let _val_size = buf[tag_idx + 1];
            let _code = buf[tag_idx + 2];

            match tag {
                TAG_SET_PHYSICAL_SIZE | TAG_SET_VIRTUAL_SIZE => {
                    resp_width = buf[tag_idx + 3];
                    resp_height = buf[tag_idx + 4];
                }
                TAG_SET_DEPTH => {
                    resp_bpp = buf[tag_idx + 3];
                }
                TAG_ALLOCATE_BUFFER => {
                    // Response: tag_id, value_size(8), response_code, address, size
                    // Data starts at tag_idx + 3 (after tag_id, value_size, response_code)
                    fb_addr = buf[tag_idx + 3] as u64;
                    fb_size = buf[tag_idx + 4] as u64;
                }
                TAG_GET_PITCH => {
                    pitch = buf[tag_idx + 3];
                }
                _ => {}
            }

            // Advance to next tag: header(3 words) + data (rounded up to 4 bytes)
            let data_len = buf[tag_idx + 1] as usize;
            let data_words = (data_len + 3) / 4;
            tag_idx += 3 + data_words;
        }

        if fb_addr == 0 {
            println!("[fb] Buffer allocation failed, trying fallback resolution");
            return Err(FbError::BufferAllocationFailed);
        }

        #[cfg_attr(not(feature = "qemu_raspi5b"), allow(unused_mut))]
        let mut info = FramebufferInfo {
            base_addr: fb_addr,
            size: fb_size,
            width: resp_width,
            height: resp_height,
            pitch: if pitch != 0 {
                pitch
            } else {
                resp_width * (resp_bpp / 8)
            },
            bpp: resp_bpp,
            virt_addr: fb_addr + PHYS_MAP_OFFSET,
        };

        // On QEMU raspi5b, the framebuffer is in VC RAM (physical 0x3C100000)
        // but QEMU's display reads from GPU bus address 0xFC100000 (4th RAM alias).
        // Map GPU bus 4th alias at DEVICE_MAP_START + 0x300000 (kernel VA 0xFFFF_C000_0300_0000).
        // GPU bus 0xFC100000 is offset 0x3C100000 into the 4th alias (0xC0000000..).
        // So kernel VA = 0xFFFF_C000_0300_0000 + 0x3C100000 = 0xFFFF_C000_3C10_0000
        #[cfg(feature = "qemu_raspi5b")]
        {
            const GPU_BUS_4TH_ALIAS_BASE: u64 = 0xC000_0000;
            const GPU_BUS_FB_ADDR: u64 = 0xFC10_0000; // vcram_base + FB_OFFSET
            const KERNEL_VA_BASE: u64 = 0xFFFF_C000_0300_0000; // DEVICE_MAP_START + 0x300000 (3MB)

            if fb_addr >= 0x3C00_0000 && fb_addr < 0x4000_0000 {
                // fb_addr is physical VC RAM address. Framebuffer is at GPU bus 0xFC100000.
                // Offset in 4th alias = 0xFC100000 - 0xC0000000 = 0x3C100000
                let offset_in_4th_alias = GPU_BUS_FB_ADDR - GPU_BUS_4TH_ALIAS_BASE; // = 0x3C100000
                info.virt_addr = KERNEL_VA_BASE + offset_in_4th_alias;
                println!(
                    "[fb] Using GPU bus 4th alias mapping: virt_addr={:#x} (GPU bus {:#x})",
                    info.virt_addr, GPU_BUS_FB_ADDR
                );
            }
        }

        println!(
            "[fb] Framebuffer: {}x{} @ {}bpp, addr={:#010x}, size={}KB, pitch={}",
            info.width,
            info.height,
            info.bpp,
            info.base_addr,
            info.size / 1024,
            info.pitch
        );

        // Store globally for later access
        set_info(info);

        Ok(info)
    }
}

/// Initialize with default settings (1920x1080 @ 32bpp, fallback to 640x480)
pub fn init_default() -> Result<FramebufferInfo, FbError> {
    // Try 1920x1080 first
    match init(1920, 1080, 32) {
        Ok(info) => Ok(info),
        Err(_) => {
            println!("[fb] 1920x1080 failed, trying 640x480");
            init(640, 480, 32)
        }
    }
}

/// Global framebuffer info
static mut FB_INFO: FramebufferInfo = FramebufferInfo::empty();

/// Get the global framebuffer info
pub fn get_info() -> FramebufferInfo {
    unsafe { FB_INFO }
}

/// Store framebuffer info globally
pub fn set_info(info: FramebufferInfo) {
    unsafe {
        FB_INFO = info;
    }
}

/// Check if the framebuffer has been initialized (non-zero base address)
pub fn is_initialized() -> bool {
    unsafe { FB_INFO.base_addr != 0 }
}
