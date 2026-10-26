//! LobsterOS Network MMIO Driver — HTTP fetch + C compilation via QEMU lobster-net device
//!
//! The lobster-net MMIO device at 0x7D00_9000 provides two capabilities:
//! 1. HTTP fetch — proxy requests to the host's curl binary
//! 2. C compilation — compile C source via host's aarch64-linux-gnu-gcc
//!
//! The guest writes data (URL or C source) to the device, triggers a command,
//! and reads the response (HTTP body or compiled ELF binary).

use crate::drivers::uart::Uart10;

/// MMIO base address for the lobster-net device
const LOBSTER_NET_BASE: u64 = 0x7D00_9000;

/// Register offsets
const LOBSTER_NET_MAGIC: u64 = 0x00; // 0x4C4E4554 ("LNET")
const LOBSTER_NET_VERSION: u64 = 0x04; // 0x00020000 (v1.1)
const LOBSTER_NET_BUF_SIZE: u64 = 0x08;
const LOBSTER_NET_URL_OFFSET: u64 = 0x10; // 0x100
const LOBSTER_NET_RESP_OFFSET: u64 = 0x14; // 0x1000
const LOBSTER_NET_URL_MAX: u64 = 0x18; // 3840
const LOBSTER_NET_RESP_MAX: u64 = 0x1C; // 65536
const LOBSTER_NET_URL_LEN: u64 = 0x20; // read/write
const LOBSTER_NET_CMD: u64 = 0x24; // write-only: 1=fetch, 2=reset, 3=compile
const LOBSTER_NET_STATUS: u64 = 0x28; // read-only: 0=idle, 1=busy, 2=done, 3=error
const LOBSTER_NET_RESP_LEN: u64 = 0x2C; // read-only
const LOBSTER_NET_RESP_CODE: u64 = 0x30; // read-only
const LOBSTER_NET_AUTH_MODE: u64 = 0x34; // read/write: 0=none, 1=bearer
const LOBSTER_NET_URL_DATA: u64 = 0x100; // URL/source bytes (write)
const LOBSTER_NET_RESP_DATA: u64 = 0x1000; // response body/ELF (read)

/// Auth modes for the lobster-net device (match hw/misc/lobster_net.c)
const AUTH_NONE: u32 = 0;
const AUTH_BEARER: u32 = 1;

/// Status codes
const NET_STATUS_IDLE: u32 = 0;
const NET_STATUS_FETCHING: u32 = 1;
const NET_STATUS_DONE: u32 = 2;
const NET_STATUS_ERROR: u32 = 3;

/// Commands
const NET_CMD_FETCH: u32 = 1;
const NET_CMD_RESET: u32 = 2;
const NET_CMD_COMPILE: u32 = 3;

/// Maximum URL/source length (must match QEMU device: 3840 bytes)
const MAX_URL_LEN: usize = 3840;

/// Maximum response body / ELF binary length (256KB for libc-linked binaries)
const MAX_RESP_LEN: usize = 524288;

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
    magic == 0x4C4E4554 // "LNET"
}

/// Initialize the lobster-net device
pub unsafe fn init() {
    Uart10::puts("[lobster-net] Probing MMIO device at 0x7D00_9000...\r\n");

    if detect() {
        let version = mmio_read32(LOBSTER_NET_VERSION);
        let url_max = mmio_read32(LOBSTER_NET_URL_MAX);
        let resp_max = mmio_read32(LOBSTER_NET_RESP_MAX);
        Uart10::puts("[lobster-net] LobsterOS network device detected!\r\n");
        Uart10::puts(&alloc::format!(
            "[lobster-net] Version: {:#010x}, URL/src max: {}, Response max: {}\r\n",
            version,
            url_max,
            resp_max
        ));

        // Reset the device
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
    } else {
        let magic = mmio_read32(LOBSTER_NET_MAGIC);
        Uart10::puts(&alloc::format!(
            "[lobster-net] No network device at 0x7D00_9000 (magic={:#010x})\r\n",
            magic
        ));
    }
}

/// Fetch a URL via the lobster-net device
/// Returns Some(response_body) on success, None on failure
pub fn http_fetch(url: &str) -> Option<alloc::vec::Vec<u8>> {
    fetch_impl(url, None)
}

/// Fetch a URL with an optional bearer-token Authorization header.
/// The token is carried to the device as "<token>\n<url>" and the device
/// appends `-H "Authorization: Bearer <token>"` to its host curl command.
pub fn http_fetch_auth(url: &str, token: &str) -> Option<alloc::vec::Vec<u8>> {
    fetch_impl(url, Some(token))
}

fn fetch_impl(url: &str, token: Option<&str>) -> Option<alloc::vec::Vec<u8>> {
    // Check device is present
    let magic = unsafe { mmio_read32(LOBSTER_NET_MAGIC) };
    if magic != 0x4C4E4554 {
        return None;
    }

    let url_bytes = url.as_bytes();

    // Reset device first to clear any previous state
    unsafe {
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
    }

    // When authenticating, prepend "<token>\n" to the payload written to the
    // URL region and set AUTH_MODE; otherwise write the bare URL.
    let (payload_len, auth_mode) = if let Some(tok) = token {
        let t = tok.as_bytes();
        if t.len() + 1 + url_bytes.len() > MAX_URL_LEN {
            return None;
        }
        // Validate token chars to match the device-side allowlist
        for &b in t {
            if !(b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.') {
                unsafe {
                    mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
                }
                return None;
            }
        }
        (t.len() + 1 + url_bytes.len(), AUTH_BEARER)
    } else {
        (url_bytes.len(), AUTH_NONE)
    };

    // Write auth mode
    unsafe {
        mmio_write32(LOBSTER_NET_AUTH_MODE, auth_mode);
    }

    // Write payload length
    unsafe {
        mmio_write32(LOBSTER_NET_URL_LEN, payload_len as u32);
    }

    // Write token prefix (if any) followed by URL bytes, contiguously
    unsafe {
        let dst = (LOBSTER_NET_BASE + LOBSTER_NET_URL_DATA) as *mut u8;
        let mut off = 0usize;
        if let Some(tok) = token {
            core::ptr::copy_nonoverlapping(tok.as_ptr(), dst, tok.len());
            off = tok.len();
            *dst.add(off) = b'\n';
            off += 1;
        }
        core::ptr::copy_nonoverlapping(url_bytes.as_ptr(), dst.add(off), url_bytes.len());
    }

    // Trigger fetch
    unsafe {
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_FETCH);
    }

    // Poll for completion
    for _ in 0..50000 {
        let status = unsafe { mmio_read32(LOBSTER_NET_STATUS) };
        if status == NET_STATUS_DONE {
            let resp_code = unsafe { mmio_read32(LOBSTER_NET_RESP_CODE) };
            let resp_len = unsafe { mmio_read32(LOBSTER_NET_RESP_LEN) };

            if resp_code == 200 && resp_len > 0 && resp_len <= MAX_RESP_LEN as u32 {
                let mut resp = alloc::vec::Vec::with_capacity(resp_len as usize);
                unsafe {
                    let src = (LOBSTER_NET_BASE + LOBSTER_NET_RESP_DATA) as *const u8;
                    core::ptr::copy_nonoverlapping(src, resp.as_mut_ptr(), resp_len as usize);
                    resp.set_len(resp_len as usize);
                }
                unsafe {
                    mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
                }
                return Some(resp);
            } else {
                unsafe {
                    mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
                }
                return None;
            }
        } else if status == NET_STATUS_ERROR {
            unsafe {
                mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
            }
            return None;
        }
        crate::arch::cpu::wfe();
    }

    // Timeout
    unsafe {
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
    }
    None
}

/// Compile C source code via the lobster-net device.
///
/// Writes C source to the device's URL/source buffer, triggers CMD=3 (compile),
/// and reads the resulting ELF binary from the response buffer.
///
/// Returns Some(elf_binary) on successful compilation, None on failure.
/// On compile error, the error message is printed to console.
pub fn compile(source: &str) -> Option<alloc::vec::Vec<u8>> {
    // Check device is present
    let magic = unsafe { mmio_read32(LOBSTER_NET_MAGIC) };
    if magic != 0x4C4E4554 {
        crate::println!("[lobster-net] compile: device not present");
        return None;
    }

    let source_bytes = source.as_bytes();
    if source_bytes.len() > MAX_URL_LEN {
        crate::println!(
            "[lobster-net] compile: source too large ({} > {} bytes)",
            source_bytes.len(),
            MAX_URL_LEN
        );
        return None;
    }

    // Reset device first
    unsafe {
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
    }

    // Write source length
    unsafe {
        mmio_write32(LOBSTER_NET_URL_LEN, source_bytes.len() as u32);
    }

    // Write source bytes
    unsafe {
        mmio_write_bytes(LOBSTER_NET_URL_DATA, source_bytes);
    }

    // Trigger compilation
    unsafe {
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_COMPILE);
    }

    // Poll for completion (compilation takes longer — use more iterations)
    for _ in 0..200000 {
        let status = unsafe { mmio_read32(LOBSTER_NET_STATUS) };
        if status == NET_STATUS_DONE {
            let resp_code = unsafe { mmio_read32(LOBSTER_NET_RESP_CODE) };
            let resp_len = unsafe { mmio_read32(LOBSTER_NET_RESP_LEN) };

            if resp_code == 0 && resp_len > 0 && resp_len <= MAX_RESP_LEN as u32 {
                // Success — read ELF binary
                let mut elf = alloc::vec::Vec::with_capacity(resp_len as usize);
                unsafe {
                    let src = (LOBSTER_NET_BASE + LOBSTER_NET_RESP_DATA) as *const u8;
                    core::ptr::copy_nonoverlapping(src, elf.as_mut_ptr(), resp_len as usize);
                    elf.set_len(resp_len as usize);
                }
                unsafe {
                    mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
                }
                return Some(elf);
            } else {
                unsafe {
                    mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
                }
                return None;
            }
        } else if status == NET_STATUS_ERROR {
            // Compilation failed — try to read error message
            let resp_len = unsafe { mmio_read32(LOBSTER_NET_RESP_LEN) };
            if resp_len > 0 && resp_len <= MAX_RESP_LEN as u32 {
                let mut err_msg = alloc::vec::Vec::with_capacity(resp_len as usize);
                unsafe {
                    let src = (LOBSTER_NET_BASE + LOBSTER_NET_RESP_DATA) as *const u8;
                    core::ptr::copy_nonoverlapping(src, err_msg.as_mut_ptr(), resp_len as usize);
                    err_msg.set_len(resp_len as usize);
                }
                let err_str = core::str::from_utf8(&err_msg).unwrap_or("(binary error)");
                crate::println!("[lobster-net] compile error:\n{}", err_str);
            }
            unsafe {
                mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
            }
            return None;
        }
        crate::arch::cpu::wfe();
    }

    // Timeout
    crate::println!("[lobster-net] compile: timeout waiting for completion");
    unsafe {
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
    }
    None
}

/// Fetch a URL and return the response as a string
pub fn http_fetch_string(url: &str) -> Option<alloc::string::String> {
    http_fetch(url).and_then(|bytes| alloc::string::String::from_utf8(bytes).ok())
}

/// Check if lobster-net device is available
pub fn is_available() -> bool {
    unsafe { detect() }
}
