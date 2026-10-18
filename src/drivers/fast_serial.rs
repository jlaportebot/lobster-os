//! LobsterOS Fast Serial — bulk TX via the QEMU lobster-serial MMIO window
//!
//! On QEMU raspi5b, a companion MMIO device ("lobster-serial" at 0x7D100000)
//! accepts up to 2048 bytes of payload plus one register write to flush the
//! whole chunk to the serial chardev in a single host call. This replaces
//! the PL011 per-byte (FR poll + DR write) path for bulk console output —
//! roughly 2 MMIO ops per byte collapse to 2 per 2048-byte chunk.
//!
//! The device is optional: `flush()` falls back to the PL011 automatically
//! when the MAGIC register doesn't match, so the same kernel image boots on
//! real hardware unchanged.

use crate::drivers::uart::Uart10;

/// MMIO base of the lobster-serial device (QEMU raspi5b)
#[cfg(feature = "qemu_raspi5b")]
const FAST_SERIAL_BASE: u64 = 0x7D10_0000;
#[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
const FAST_SERIAL_BASE: u64 = 0x7D10_0000;

/// Register offsets
const REG_TXLEN: u64 = 0x00; // write: flush N bytes from the buffer
const REG_MAGIC: u64 = 0x04; // read: 0x4C425558 ("LBUF")
const REG_BUFSIZE: u64 = 0x0C; // read: 2048
const REG_TXBUF: u64 = 0x100; // write: payload bytes

/// Device magic value
const MAGIC: u32 = 0x4C42_5558;

/// Payload window size (must match the QEMU device)
const BUFSIZE: usize = 2048;

/// Whether the device has been probed
static mut AVAILABLE: bool = false;
static mut PROBED: bool = false;

#[inline]
unsafe fn mmio_read32(offset: u64) -> u32 {
    core::ptr::read_volatile((FAST_SERIAL_BASE + offset) as *const u32)
}

#[inline]
unsafe fn mmio_write32(offset: u64, val: u32) {
    core::ptr::write_volatile((FAST_SERIAL_BASE + offset) as *mut u32, val)
}

/// Probe for the bulk-serial device (idempotent).
pub fn available() -> bool {
    unsafe {
        if PROBED {
            return AVAILABLE;
        }
        PROBED = true;
        // Read MAGIC before touching anything else; an absent device reads 0
        // (the region is unmapped — reads return garbage or fault, so only
        // the exact magic counts).
        AVAILABLE = mmio_read32(REG_MAGIC) == MAGIC;
        AVAILABLE
    }
}

/// Write a byte slice through the bulk window, chunk by chunk.
///
/// Falls back to the PL011 when the device isn't present.
pub fn write(bytes: &[u8]) {
    if !available() {
        Uart10::puts_bytes(bytes);
        return;
    }
    let mut off = 0usize;
    while off < bytes.len() {
        let remaining = bytes.len() - off;
        let chunk = remaining.min(BUFSIZE);
        unsafe {
            // Copy the chunk into the payload window, then flush with ONE
            // register write. The window is write-only so use copy_nonoverlapping
            // against the MMIO region byte by byte (it is not cacheable RAM;
            // volatile byte writes are required for correctness).
            for i in 0..chunk {
                core::ptr::write_volatile(
                    (FAST_SERIAL_BASE + REG_TXBUF + (off + i) as u64) as *mut u8,
                    bytes[off + i],
                );
            }
            mmio_write32(REG_TXLEN, chunk as u32);
        }
        off += chunk;
    }
}

/// Convenience wrapper for &str.
pub fn write_str(s: &str) {
    write(s.as_bytes());
}
