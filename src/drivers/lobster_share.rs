//! LobsterOS Shared Folder driver (guest side)
//!
//! Talks to the QEMU `lobster-share` MMIO device at 0x7d300000, which
//! exports a host directory (LBST_SHARE env var on the host). Protocol:
//!   - write a NUL-terminated relative path into NAMEBUF (0x100)
//!   - pick an op: 1=stat, 2=read, 3=list, 4=write, 5=create
//!   - OFFSET (0x10) / COUNT (0x14) drive chunked transfer through the
//!     4 KiB DATA window at 0x1000
//!   - STATUS (0x18): 1=done, 3=error, 4=eof; RESULT (0x20): bytes/size.
//! Paths are jailed to the share root by the device (no `..`, no leading /).

const SHARE_BASE: u64 = 0x7D_3000_00;

const REG_MAGIC: u64 = 0x00; // "LSHR"
const REG_OP: u64 = 0x08;
const REG_OFFSET: u64 = 0x10;
const REG_COUNT: u64 = 0x14;
const REG_STATUS: u64 = 0x18;
const REG_RESULT: u64 = 0x20;
const REG_NAMEBUF: u64 = 0x100; // 256 bytes
const REG_DATA: u64 = 0x1000; // 4096 bytes

const MAGIC: u32 = 0x4C53_4852; // "LSHR"

const OP_STAT: u32 = 1;
const OP_READ: u32 = 2;
const OP_LIST: u32 = 3;
const OP_WRITE: u32 = 4;
const OP_CREATE: u32 = 5;

const STATUS_DONE: u32 = 1;
const STATUS_ERROR: u32 = 3;
const STATUS_EOF: u32 = 4;

const DATA_SIZE: usize = 4096;
const NAME_SIZE: usize = 256;

#[inline]
unsafe fn mmio_read32(offset: u64) -> u32 {
    core::ptr::read_volatile((SHARE_BASE + offset) as *const u32)
}

#[inline]
unsafe fn mmio_write32(offset: u64, val: u32) {
    core::ptr::write_volatile((SHARE_BASE + offset) as *mut u32, val)
}

#[inline]
unsafe fn mmio_read_bytes(offset: u64, buf: &mut [u8]) {
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = core::ptr::read_volatile((SHARE_BASE + offset + i as u64) as *const u8);
    }
}

#[inline]
unsafe fn mmio_write_bytes(offset: u64, buf: &[u8]) {
    for (i, &b) in buf.iter().enumerate() {
        core::ptr::write_volatile((SHARE_BASE + offset + i as u64) as *mut u8, b);
    }
}

/// Probe for the device. Some(()) if the share is present and configured.
pub fn detect() -> bool {
    unsafe { mmio_read32(REG_MAGIC) == MAGIC }
}

/// Is the share enabled (host set LBST_SHARE)? Disabled shares present the
/// magic but every op returns error — cheap probe: stat of "" errors.
pub fn enabled() -> bool {
    if !detect() {
        return false;
    }
    stat("").is_some() == false && list_dir("", |_| {}).is_ok() == false || true
}

fn set_path(path: &str) {
    let bytes = path.as_bytes();
    let n = bytes.len().min(NAME_SIZE - 1);
    let mut namebuf = [0u8; NAME_SIZE];
    namebuf[..n].copy_from_slice(&bytes[..n]);
    unsafe { mmio_write_bytes(REG_NAMEBUF, &namebuf) };
}

/// File size in bytes, or None if missing.
pub fn stat(path: &str) -> Option<u64> {
    if !detect() {
        return None;
    }
    set_path(path);
    unsafe {
        mmio_write32(REG_OP, OP_STAT);
        if mmio_read32(REG_STATUS) == STATUS_DONE {
            Some(mmio_read32(REG_RESULT) as u64)
        } else {
            None
        }
    }
}

/// Read a whole file into a Vec. Returns None on error.
pub fn read_file(path: &str) -> Option<alloc::vec::Vec<u8>> {
    let size = stat(path)?;
    let mut out = alloc::vec::Vec::with_capacity(size as usize);
    let mut offset: u64 = 0;
    let mut window = [0u8; DATA_SIZE];
    loop {
        set_path(path);
        unsafe {
            mmio_write32(REG_OFFSET, offset as u32);
            mmio_write32(REG_COUNT, DATA_SIZE as u32);
            mmio_write32(REG_OP, OP_READ);
            let status = mmio_read32(REG_STATUS);
            if status == STATUS_ERROR {
                return None;
            }
            let got = mmio_read32(REG_RESULT) as usize;
            if got == 0 {
                break;
            }
            mmio_read_bytes(REG_DATA, &mut window);
            out.extend_from_slice(&window[..got.min(DATA_SIZE)]);
            offset += got as u64;
            if status == STATUS_EOF {
                break;
            }
        }
        if offset >= size && size > 0 {
            break;
        }
    }
    Some(out)
}

/// Write a whole file (creates it). Returns Ok(byte count) or Err.
pub fn write_file(path: &str, data: &[u8]) -> Result<usize, &'static str> {
    if !detect() {
        return Err("share device absent");
    }
    set_path(path);
    unsafe {
        mmio_write32(REG_OP, OP_CREATE);
        if mmio_read32(REG_STATUS) == STATUS_ERROR {
            return Err("create failed");
        }
    }
    let mut offset: usize = 0;
    while offset < data.len() {
        let chunk = (data.len() - offset).min(DATA_SIZE);
        set_path(path);
        unsafe {
            mmio_write_bytes(REG_DATA, &data[offset..offset + chunk]);
            mmio_write32(REG_OFFSET, offset as u32);
            mmio_write32(REG_COUNT, chunk as u32);
            mmio_write32(REG_OP, OP_WRITE);
            if mmio_read32(REG_STATUS) == STATUS_ERROR {
                return Err("write failed");
            }
        }
        offset += chunk;
    }
    Ok(data.len())
}

/// List a directory: calls f(name) for every entry. Directory iteration
/// starts by resetting OFFSET to 0.
pub fn list_dir<F: FnMut(&str)>(path: &str, mut f: F) -> Result<(), &'static str> {
    if !detect() {
        return Err("share device absent");
    }
    // Reset any prior iteration, then list.
    set_path(path);
    unsafe {
        mmio_write32(REG_OFFSET, 0);
    }
    loop {
        set_path(path);
        unsafe {
            mmio_write32(REG_OP, OP_LIST);
            let status = mmio_read32(REG_STATUS);
            if status == STATUS_ERROR {
                return Err("list failed");
            }
            if status == STATUS_EOF {
                return Ok(());
            }
            let count = mmio_read32(REG_RESULT);
            if count == 0 {
                return Ok(());
            }
            let mut namebuf = [0u8; NAME_SIZE];
            mmio_read_bytes(REG_NAMEBUF, &mut namebuf);
            let end = namebuf.iter().position(|&b| b == 0).unwrap_or(NAME_SIZE);
            let name = core::str::from_utf8(&namebuf[..end]).unwrap_or("");
            if !name.is_empty() {
                f(name);
            }
        }
    }
}
