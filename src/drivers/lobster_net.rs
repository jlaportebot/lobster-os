//! LobsterOS Network MMIO Driver — HTTP fetch + C compilation via QEMU lobster-net device
//!
//! The lobster-net MMIO device at 0x7D00_9000 provides two capabilities:
//! 1. HTTP fetch — proxy requests to the host's curl binary
//! 2. C compilation — compile C source via host's aarch64-linux-gnu-gcc
//!
//! The guest writes data (URL or C source) to the device, triggers a command,
//! and reads the response (HTTP body or compiled ELF binary).

use crate::drivers::uart::Uart10;
use crate::net::fetch::{classify, Completion, DeviceState, FetchError};

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

/// Status polls before a fetch is declared timed out.
const FETCH_POLL_LIMIT: u32 = 50_000;

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
    crate::drivers::mmio::read_bytes(LOBSTER_NET_BASE + offset, buf);
}

#[inline]
unsafe fn mmio_write_bytes(offset: u64, buf: &[u8]) {
    crate::drivers::mmio::write_bytes(LOBSTER_NET_BASE + offset, buf);
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

/// Fetch a URL and return the HTTP status together with the body, for any
/// status the server answers with.
pub fn http_fetch_response(url: &str) -> Result<crate::net::fetch::Response, FetchError> {
    let (status, body, truncated) = fetch_raw(url, None)?;
    Ok(crate::net::fetch::Response {
        status,
        body,
        truncated,
    })
}

fn fetch_impl(url: &str, token: Option<&str>) -> Option<alloc::vec::Vec<u8>> {
    match fetch_raw(url, token) {
        Ok((200, body, _)) if !body.is_empty() => Some(body),
        _ => None,
    }
}

/// Validate the token and compute the payload length / auth mode pair.
fn payload_for(url: &str, token: Option<&str>) -> Result<(usize, u32), FetchError> {
    let Some(tok) = token else {
        return Ok((url.len(), AUTH_NONE));
    };
    let valid = tok
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.');
    if !valid || tok.len() + 1 + url.len() > MAX_URL_LEN {
        return Err(FetchError::InvalidUrl);
    }
    Ok((tok.len() + 1 + url.len(), AUTH_BEARER))
}

/// Write the request into the device and start the fetch.
fn start_fetch(url: &str, token: Option<&str>, payload_len: usize, auth_mode: u32) {
    // SAFETY: MMIO accesses to the lobster-net window; the caller checked the
    // magic register, and the payload length was validated against the device's
    // URL region size by `payload_for`.
    unsafe {
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET);
        mmio_write32(LOBSTER_NET_AUTH_MODE, auth_mode);
        mmio_write32(LOBSTER_NET_URL_LEN, payload_len as u32);
        let dst = LOBSTER_NET_BASE + LOBSTER_NET_URL_DATA;
        let mut off = 0usize;
        if let Some(tok) = token {
            crate::drivers::mmio::write_bytes(dst, tok.as_bytes());
            off = tok.len();
            crate::drivers::mmio::write8(dst + off as u64, b'\n');
            off += 1;
        }
        crate::drivers::mmio::write_bytes(dst + off as u64, url.as_bytes());
        mmio_write32(LOBSTER_NET_CMD, NET_CMD_FETCH);
    }
}

/// Sample the device registers and classify them.
fn poll_completion() -> Completion {
    // SAFETY: read-only MMIO register reads of the lobster-net device.
    let (status, code, len) = unsafe {
        (
            mmio_read32(LOBSTER_NET_STATUS),
            mmio_read32(LOBSTER_NET_RESP_CODE),
            mmio_read32(LOBSTER_NET_RESP_LEN),
        )
    };
    let state = match status {
        NET_STATUS_DONE => DeviceState::Done,
        NET_STATUS_ERROR => DeviceState::Error,
        _ => DeviceState::Pending,
    };
    classify(state, code, len)
}

/// Run one fetch to completion: (status, body, truncated).
fn fetch_raw(
    url: &str,
    token: Option<&str>,
) -> Result<(u16, alloc::vec::Vec<u8>, bool), FetchError> {
    // SAFETY: read of the device magic register.
    if unsafe { mmio_read32(LOBSTER_NET_MAGIC) } != 0x4C4E4554 {
        return Err(FetchError::NoNetwork);
    }
    let (payload_len, auth_mode) = payload_for(url, token)?;
    start_fetch(url, token, payload_len, auth_mode);
    let result = wait_for_response();
    // SAFETY: MMIO write releasing the device after the response was copied.
    unsafe { mmio_write32(LOBSTER_NET_CMD, NET_CMD_RESET) };
    result
}

fn wait_for_response() -> Result<(u16, alloc::vec::Vec<u8>, bool), FetchError> {
    for _ in 0..FETCH_POLL_LIMIT {
        match poll_completion() {
            Completion::Waiting => crate::arch::cpu::wfe(),
            Completion::Failed(e) => return Err(e),
            Completion::Ready {
                status,
                len,
                truncated,
            } => return Ok((status, read_body(len), truncated)),
        }
    }
    Err(FetchError::TimedOut)
}

fn read_body(len: usize) -> alloc::vec::Vec<u8> {
    let mut body = alloc::vec::Vec::new();
    body.resize(len, 0);
    // SAFETY: `len` was validated against the device's response buffer size.
    unsafe {
        crate::drivers::mmio::read_bytes(LOBSTER_NET_BASE + LOBSTER_NET_RESP_DATA, &mut body);
    }
    body
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
                    elf.resize(resp_len as usize, 0);
                    crate::drivers::mmio::read_bytes(
                        LOBSTER_NET_BASE + LOBSTER_NET_RESP_DATA,
                        &mut elf,
                    );
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
                    err_msg.resize(resp_len as usize, 0);
                    crate::drivers::mmio::read_bytes(
                        LOBSTER_NET_BASE + LOBSTER_NET_RESP_DATA,
                        &mut err_msg,
                    );
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
