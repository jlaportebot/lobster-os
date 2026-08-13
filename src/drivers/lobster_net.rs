//! LobsterOS Network MMIO Driver — reads HTTP responses from QEMU lobster-net device
//!
//! The lobster-net MMIO device at 0x7D00_9000 provides HTTP fetch capability
//! by proxying requests to the host's curl binary. The guest writes a URL
//! to the device, triggers a fetch, and reads the response.

use crate::drivers::uart::Uart10;

/// MMIO base address for the lobster-net device
const LOBSTER_NET_BASE: u64 = 0x7D00_9000;

/// Register offsets
const LOBSTER_NET_MAGIC: u64 = 0x00;      // 0x4C4E4554 ("LNET")
const LOBSTER_NET_VERSION: u64 = 0x04;     // 0x00010000
const LOBSTER_NET_BUF_SIZE: u64 = 0x08;
const LOBSTER_NET_URL_OFFSET: u64 = 0x10;   // 0x100
const LOBSTER_NET_RESP_OFFSET: u64 = 0x14;  // 0x1000
const LOBSTER_NET_URL_MAX: u64 = 0x18;      // 256
const LOBSTER_NET_RESP_MAX: u64 = 0x1C;     // 65536
const LOBSTER_NET_URL_LEN: u64 = 0x20;      // read/write
const LOBSTER_NET_CMD: u64 = 0x24;          // write-only: 1=fetch, 2=reset
const LOBSTER_NET_STATUS: u64 = 0x28;       // read-only: 0=idle, 1=fetching, 2=done, 3=error
const LOBSTER_NET_RESP_LEN: u64 = 0x2C;     // read-only
const LOBSTER_NET_RESP_CODE: u64 = 0x30;    // read-only
const LOBSTER_NET_URL_DATA: u64 = 0x100;    // URL bytes (write)
const LOBSTER_NET_RESP_DATA: u64 = 0x1000;  // response body (read)

/// Status codes
const NET_STATUS_IDLE: u32 = 0;
const NET_STATUS_FETCHING: u32 = 1;
const NET_STATUS_DONE: u32 = 2;
const NET_STATUS_ERROR: u32 = 3;

/// Commands
const NET_CMD_FETCH: u32 = 1;
const NET_CMD_RESET: u32 = 2;

/// Maximum URL length
const MAX_URL_LEN: usize = 256;

/// Maximum response body length
const MAX_RESP_LEN: usize = 65536;

/// MMIO read/write helpers
#[inline]
unsafe fn mmio_read32(offset: u64) -> u32 {
    core::ptr::read_volatile((LOBSTER_NET_BASE + offset) as *const u32)
}

#[inline]
unsafe fn mmio_write32(offset: u64, val: u32) {
    core::ptr::write_volatile((LOBSTER_NET_BASE + offset) as *mut u32, val)
}

#[inline]
unsafe fn mmio_read_bytes(offset: u64, buf: &mut [u8]) {
    let src = (LOBSTER_NET_BASE + offset) as *const u8;
    core::ptr::copy_nonoverlapping(src, buf.as_mut_ptr(), buf.len());
}

#[inline]
unsafe fn mmio_write_bytes(offset: u64, buf: &[u8]) {
    let dst = (LOBSTER_NET_BASE + offset) as *mut u8;
    core::ptr::copy_nonoverlapping(buf.as_ptr(), dst, buf.len());
}

/// Check if the lobster-net MMIO device is present
pub unsafe fn detect() -> bool {
    let magic = mmio_read32(LOBSTER_NET_MAGIC);
    magic == 0x4C4E4554  // "LNET"
}

/// Initialize the lobster-net device
pub unsafe fn init() {
    Uart10::puts("[lobster-net] Probing MMIO device at 0x7D00_9000...\r\n");

    if detect() {
        let version = mmio_read32(LOBSTER_NET_VERSION);
        let url_max = mmio_read32(LOBSTER_NET_URL_MAX);
        let resp_max = mmio_read32(LOBSTER_NET_RESP_MAX);
        Uart10::puts("[lobster-net] LobsterOS network device detected!\r\n");
        Uart10::puts(&alloc::format!("[lobster-net] Version: {:#010x}, URL max: {}, Response max: {}\r\n",
            version, url_max, resp_max));

        // Reset the device
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
    } else {
        let magic = mmio_read32(LOBSTER_NET_MAGIC);
        Uart10::puts(&alloc::format!("[lobster-net] No network device at 0x7D00_9000 (magic={:#010x})\r\n", magic));
    }
}

/// Fetch a URL via the lobster-net device
/// Returns Some(response_body) on success, None on failure
pub fn http_fetch(url: &str) -> Option<alloc::vec::Vec<u8>> {
    // Check device is present
    let magic = unsafe { mmio_read32(LOBSTER_NET_MAGIC) };
    if magic != 0x4C4E4554 {
        return None;
    }

    let url_bytes = url.as_bytes();
    if url_bytes.len() > MAX_URL_LEN {
        return None;
    }

    // Write URL length
    unsafe {
        mmio_write32(LOBSTER_NET_URL_LEN, url_bytes.len() as u32);
    }

    // Write URL bytes
    unsafe {
        mmio_write_bytes(LOBSTER_NET_URL_DATA, url_bytes);
    }

    // Trigger fetch
    unsafe {
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_FETCH);
    }

    // Poll for completion
    for _ in 0..50000 {
        let status = unsafe { mmio_read32(LOBSTER_NET_STATUS) };
        if status == NET_STATUS_IDLE {
            // Still idle, keep polling
        } else if status == NET_STATUS_FETCHING {
            // Still fetching
        } else if status == NET_STATUS_DONE {
            let resp_code = unsafe { mmio_read32(LOBSTER_NET_RESP_CODE) };
            let resp_len = unsafe { mmio_read32(LOBSTER_NET_RESP_LEN) };

            if resp_code == 200 && resp_len > 0 && resp_len <= MAX_RESP_LEN as u32 {
                let mut resp = alloc::vec::Vec::with_capacity(resp_len as usize);
                unsafe {
                    let src = (LOBSTER_NET_BASE + LOBSTER_NET_RESP_DATA) as *const u8;
                    core::ptr::copy_nonoverlapping(src, resp.as_mut_ptr(), resp_len as usize);
                    resp.set_len(resp_len as usize);
                }
                // Reset device for next use
                unsafe { mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET); }
                return Some(resp);
            } else {
                // Reset device
                unsafe { mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET); }
                return None;
            }
        } else if status == NET_STATUS_ERROR {
            unsafe { mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET); }
            return None;
        }
        crate::arch::cpu::wfe();
    }

    // Timeout
    unsafe { mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET); }
    None
}

/// Fetch a URL and return the response as a string
pub fn http_fetch_string(url: &str) -> Option<alloc::string::String> {
    http_fetch(url).and_then(|bytes| {
        alloc::string::String::from_utf8(bytes).ok()
    })
}

/// Check if lobster-net device is available
pub fn is_available() -> bool {
    unsafe { detect() }
}