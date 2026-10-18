//! LobsterOS Virtual Disk — host-backed block device via the lobster-disk MMIO window
//!
//! On QEMU raspi5b, a paravirtual block device at 0x7D200000 (magic "LDRK")
//! proxies 512-byte block reads/writes to a host-side raw image (selected via
//! the QEMU process's LBST_DISKIMG environment variable). One MMIO command
//! per block; the payload moves through the 512-byte BLKBUF window.
//!
//! Implements the `BlockDevice` trait, so the ext2 driver can mount a
//! persistent filesystem from it — the emulator equivalent of a Pi 5's
//! microSD card, minus the SDHCI emulation overhead.

use crate::fs::block_dev::{BlockDevice, BlockError};

/// MMIO base of the lobster-disk device (QEMU raspi5b)
#[cfg(feature = "qemu_raspi5b")]
const LBST_DISK_BASE: u64 = 0x7D20_0000;
#[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
const LBST_DISK_BASE: u64 = 0x7D20_0000;

/// Register offsets
const REG_MAGIC: u64 = 0x00; // read: 0x4C44524B ("LDRK")
const REG_NBLK: u64 = 0x08; // read: total blocks
const REG_BLKSZ: u64 = 0x0C; // read: 512
const REG_STATUS: u64 = 0x10; // read: 0 idle, 1 busy, 2 done, 3 error
const REG_CMD: u64 = 0x14; // write: 1 read, 2 write, 3 flush
const REG_BLOCK: u64 = 0x18; // r/w: block index
const REG_BLKBUF: u64 = 0x1000; // r/w: 512-byte payload window

/// Device magic
const MAGIC: u32 = 0x4C44_524B;

/// Command codes
const CMD_READ: u32 = 1;
const CMD_WRITE: u32 = 2;
const CMD_FLUSH: u32 = 3;

/// Status codes
const STATUS_DONE: u32 = 2;
const STATUS_ERROR: u32 = 3;

/// Max poll iterations for a block command (~a few ms of virtual time)
const MAX_POLL: u32 = 2_000_000;

#[inline]
unsafe fn mmio_read32(offset: u64) -> u32 {
    core::ptr::read_volatile((LBST_DISK_BASE + offset) as *const u32)
}

#[inline]
unsafe fn mmio_write32(offset: u64, val: u32) {
    core::ptr::write_volatile((LBST_DISK_BASE + offset) as *mut u32, val)
}

#[inline]
unsafe fn mmio_read_bytes(offset: u64, buf: &mut [u8]) {
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = core::ptr::read_volatile((LBST_DISK_BASE + offset + i as u64) as *const u8);
    }
}

#[inline]
unsafe fn mmio_write_bytes(offset: u64, buf: &[u8]) {
    for (i, &b) in buf.iter().enumerate() {
        core::ptr::write_volatile((LBST_DISK_BASE + offset + i as u64) as *mut u8, b);
    }
}

/// Probe for the device. Returns total block count, or None if absent.
pub fn detect() -> Option<u64> {
    unsafe {
        if mmio_read32(REG_MAGIC) != MAGIC {
            return None;
        }
        let nblk = mmio_read32(REG_NBLK) as u64;
        if nblk == 0 {
            return None; // device present, no media
        }
        Some(nblk)
    }
}

/// Execute one block command, waiting for completion.
unsafe fn do_cmd(cmd: u32, block: u64) -> Result<(), BlockError> {
    mmio_write32(REG_BLOCK, block as u32);
    mmio_write32(REG_CMD, cmd);
    let mut polls = 0;
    loop {
        let status = mmio_read32(REG_STATUS);
        if status == STATUS_DONE {
            return Ok(());
        }
        if status == STATUS_ERROR {
            return Err(if cmd == CMD_WRITE {
                BlockError::WriteError
            } else {
                BlockError::ReadError
            });
        }
        polls += 1;
        if polls > MAX_POLL {
            return Err(BlockError::Timeout);
        }
        core::hint::spin_loop();
    }
}

/// The guest-side block device backed by the lobster-disk window.
///
/// The block count lives in a static (patched once at boot from the device
/// probe) so the device object itself is a const-constructible unit and can
/// live in a `static`, satisfying the `&'static dyn BlockDevice` registry.
pub struct LobsterDisk;

/// Block count published by `detect()` (0 = no media).
pub static TOTAL_BLOCKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

impl LobsterDisk {
    /// Const-constructible handle.
    pub const fn new() -> Self {
        Self
    }

    fn blocks(&self) -> u64 {
        TOTAL_BLOCKS.load(core::sync::atomic::Ordering::Relaxed)
    }
}

impl BlockDevice for LobsterDisk {
    fn read_block(&self, block: u64, buf: &mut [u8; 512]) -> Result<(), BlockError> {
        let total = self.blocks();
        if block >= total {
            return Err(BlockError::InvalidBlock);
        }
        unsafe {
            do_cmd(CMD_READ, block)?;
            mmio_read_bytes(REG_BLKBUF, buf);
        }
        Ok(())
    }

    fn write_block(&self, block: u64, buf: &[u8; 512]) -> Result<(), BlockError> {
        let total = self.blocks();
        if block >= total {
            return Err(BlockError::InvalidBlock);
        }
        unsafe {
            mmio_write_bytes(REG_BLKBUF, buf);
            do_cmd(CMD_WRITE, block)?;
        }
        Ok(())
    }

    fn block_size(&self) -> usize {
        512
    }

    fn total_blocks(&self) -> u64 {
        self.blocks()
    }

    fn name(&self) -> &str {
        "lobster-vd0"
    }

    fn media_present(&self) -> bool {
        self.blocks() > 0
    }

    fn device_type(&self) -> &str {
        "paravirtual"
    }

    fn flush(&self) -> Result<(), BlockError> {
        unsafe { do_cmd(CMD_FLUSH, 0) }
    }
}
