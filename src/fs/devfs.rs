//! /dev virtual filesystem — device node management
//!
//! Provides:
//! - /dev/null — discard writes, read returns EOF
//! - /dev/zero — read returns zeros, discard writes
//! - /dev/urandom — non-blocking pseudo-random bytes
//! - /dev/ttyS0 — serial console
//! - /dev/fb0 — framebuffer device
//! - /dev/input/mouse0, /dev/input/kbd0 — input devices
//! - Dynamic device registration for new drivers
//!
//! All device nodes support open/read/write/close via the VFS layer.

use crate::println;

/// Maximum device nodes
const MAX_DEVICES: usize = 16;

/// Device operations trait (function pointer table for no_std)
#[derive(Clone, Copy)]
pub struct DeviceOps {
    /// Read from device: returns bytes read. dev_idx is the device index.
    pub read: Option<fn(usize, *mut u8, usize) -> usize>,
    /// Write to device: returns bytes written. dev_idx is the device index.
    pub write: Option<fn(usize, *const u8, usize) -> usize>,
    /// IOCTL: returns 0 on success, negative on error. dev_idx is the device index.
    pub ioctl: Option<fn(usize, u64, u64) -> i64>,
}

impl DeviceOps {
    pub const fn null() -> Self {
        Self {
            read: None,
            write: None,
            ioctl: None,
        }
    }
}

/// A registered device
#[derive(Clone, Copy)]
pub struct Device {
    /// Device name (e.g., "null", "zero", "ttyS0")
    pub name: [u8; 32],
    pub name_len: usize,
    /// Major number
    pub major: u32,
    /// Minor number
    pub minor: u32,
    /// Device type: block or char
    pub is_block: bool,
    /// Operations
    pub ops: DeviceOps,
    /// Is this slot in use?
    pub in_use: bool,
    /// Permissions
    pub mode: u32,
}

impl Device {
    pub const fn empty() -> Self {
        Self {
            name: [0u8; 32],
            name_len: 0,
            major: 0,
            minor: 0,
            is_block: false,
            ops: DeviceOps::null(),
            in_use: false,
            mode: 0o666,
        }
    }

    pub fn set_name(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(32);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name_len = len;
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("?")
    }
}

static mut DEVICES: [Device; MAX_DEVICES] = [Device::empty(); MAX_DEVICES];
static mut DEVICE_COUNT: usize = 0;

// ═══════════════════════════════════════════════════════════════
// Built-in device implementations
// ═══════════════════════════════════════════════════════════════

/// /dev/null: read returns 0 bytes, write discards
fn dev_null_read(_dev_idx: usize, _buf: *mut u8, _len: usize) -> usize {
    0 // EOF
}

fn dev_null_write(_dev_idx: usize, _buf: *const u8, len: usize) -> usize {
    len // Discard, pretend we wrote it all
}

/// /dev/zero: read returns zero-filled bytes
fn dev_zero_read(_dev_idx: usize, buf: *mut u8, len: usize) -> usize {
    unsafe {
        core::ptr::write_bytes(buf, 0, len);
    }
    len
}

fn dev_zero_write(_dev_idx: usize, _buf: *const u8, len: usize) -> usize {
    len
}

/// /dev/urandom: simple xorshift64 PRNG (not cryptographically secure)
static mut PRNG_STATE: u64 = 0x12345678_9ABCDEF0;

fn xorshift64() -> u64 {
    unsafe {
        let mut x = PRNG_STATE;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        PRNG_STATE = x;
        x
    }
}

fn dev_urandom_read(_dev_idx: usize, buf: *mut u8, len: usize) -> usize {
    let mut off = 0usize;
    while off + 8 <= len {
        let val = xorshift64();
        unsafe {
            core::ptr::copy_nonoverlapping(&val as *const u64 as *const u8, buf.add(off), 8);
        }
        off += 8;
    }
    // Remaining bytes
    if off < len {
        let val = xorshift64();
        let remaining = len - off;
        unsafe {
            core::ptr::copy_nonoverlapping(
                &val as *const u64 as *const u8,
                buf.add(off),
                remaining,
            );
        }
    }
    len
}

fn dev_urandom_write(_dev_idx: usize, buf: *const u8, len: usize) -> usize {
    // Writing to urandom seeds the PRNG
    if len >= 8 {
        unsafe {
            let mut seed = 0u64;
            core::ptr::copy_nonoverlapping(buf, &mut seed as *mut u64 as *mut u8, 8);
            PRNG_STATE = seed;
        }
    }
    len
}

/// PTY slave device read — reads from slave buffer (shell output)
fn dev_pty_slave_read(dev_idx: usize, buf: *mut u8, len: usize) -> usize {
    unsafe {
        if let Some(device) = get_device(dev_idx) {
            let pty_idx = device.minor as usize;
            let slice = core::slice::from_raw_parts_mut(buf, len);
            return crate::ipc::pty_slave_read(pty_idx, slice);
        }
        0
    }
}

/// PTY slave device write — writes to slave buffer (shell input)
fn dev_pty_slave_write(dev_idx: usize, buf: *const u8, len: usize) -> usize {
    unsafe {
        if let Some(device) = get_device(dev_idx) {
            let pty_idx = device.minor as usize;
            let slice = core::slice::from_raw_parts(buf, len);
            return crate::ipc::pty_slave_write(pty_idx, slice);
        }
        0
    }
}

/// PTY slave device ioctl — handles TIOCGWINSZ, TIOCSWINSZ, TIOCSCTTY
fn dev_pty_slave_ioctl(dev_idx: usize, cmd: u64, arg: u64) -> i64 {
    if let Some(device) = get_device(dev_idx) {
        let pty_idx = device.minor as usize;
        return crate::ipc::pty_ioctl(pty_idx, cmd, arg);
    }
    -25 // -ENOTTY
}

/// /dev/ptmx ioctl — handle TIOCSPTLCK (lock/unlock pt), TIOCGPTN (get PTY number)
/// Returns 0 on success, negative errno on error
fn dev_ptmx_ioctl(_dev_idx: usize, cmd: u64, arg: u64) -> i64 {
    // TIOCSPTLCK (0x40045431) - lock/unlock slave PTY
    // TIOCGPTN (0x80045430) - get PTY number
    // TIOCSPTLCK is used to unlock the slave PTY after opening ptmx
    // TIOCGPTN returns the PTY number for the slave device path

    match cmd {
        0x40045431 => {
            // TIOCSPTLCK
            // arg is pointer to int (lock value: 0=unlock, 1=lock)
            // We always succeed - the IPC layer handles locking
            0
        }
        0x80045430 => {
            // TIOCGPTN
            // arg is pointer to int to store PTY number
            // Allocate a new PTY pair via IPC
            if let Some((master_idx, slave_idx)) = crate::ipc::pty_open() {
                // Register the slave device /dev/pts/N
                let slave_name = alloc::format!("pts/{}", slave_idx);
                let _ = register_device(
                    &slave_name,
                    136,              // devpts major number
                    slave_idx as u32, // minor = PTY index
                    false,
                    DeviceOps {
                        read: Some(dev_pty_slave_read),
                        write: Some(dev_pty_slave_write),
                        ioctl: Some(dev_pty_slave_ioctl),
                    },
                    0o666,
                );

                // Write the PTY number to the user-space pointer
                let pty_num = master_idx as i32;
                unsafe {
                    *(arg as *mut i32) = pty_num;
                }
                println!(
                    "[devfs] PTY allocated: master={}, slave={} (/dev/pts/{})",
                    master_idx, slave_idx, slave_idx
                );
                0
            } else {
                -12 // -ENOMEM
            }
        }
        _ => -25, // -ENOTTY
    }
}

/// /dev/kmsg read — returns kernel log entries
fn dev_kmsg_read(_dev_idx: usize, buf: *mut u8, len: usize) -> usize {
    let mut user_buf = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    let mut seq = 0;
    crate::klog::klog_read(&mut user_buf, &mut seq)
}

/// /dev/kmsg write — allows userspace to write to kernel log (for syslogd)
fn dev_kmsg_write(_dev_idx: usize, buf: *const u8, len: usize) -> usize {
    let user_buf = unsafe { core::slice::from_raw_parts(buf, len) };
    if let Ok(msg) = core::str::from_utf8(user_buf) {
        crate::klog::klog_write(crate::klog::LogLevel::Info, msg);
    }
    len
}

/// /dev/kmsg ioctl — supports klogctl operations
fn dev_kmsg_ioctl(_dev_idx: usize, cmd: u64, _arg: u64) -> i64 {
    match cmd {
        2 => {
            // Read log (SYSLOG_ACTION_READ)
            // arg = buffer, len in arg
            0 // Simplified - actual read via syscall
        }
        3 => {
            // Read and clear (SYSLOG_ACTION_READ_CLEAR)
            crate::klog::klog_clear();
            0
        }
        6 => {
            // Clear ring buffer (SYSLOG_ACTION_CLEAR)
            crate::klog::klog_clear();
            0
        }
        _ => -25, // -ENOTTY
    }
}

// ════════════════════════════════════════════════════════════════
// Device registration
// ═══════════════════════════════════════════════════════════════

/// Register a new device. Returns the device index or None on failure.
pub fn register_device(
    name: &str,
    major: u32,
    minor: u32,
    is_block: bool,
    ops: DeviceOps,
    mode: u32,
) -> Option<usize> {
    unsafe {
        if DEVICE_COUNT >= MAX_DEVICES {
            return None;
        }

        let idx = DEVICE_COUNT;
        let dev = &mut DEVICES[idx];
        dev.set_name(name);
        dev.major = major;
        dev.minor = minor;
        dev.is_block = is_block;
        dev.ops = ops;
        dev.in_use = true;
        dev.mode = mode;
        DEVICE_COUNT += 1;

        println!(
            "[devfs] Registered device /dev/{} ({}:{}, {})",
            name,
            major,
            minor,
            if is_block { "block" } else { "char" }
        );

        Some(idx)
    }
}

/// Find a device by name
pub fn find_device(name: &str) -> Option<usize> {
    unsafe {
        for i in 0..DEVICE_COUNT {
            if DEVICES[i].in_use && DEVICES[i].name_str() == name {
                return Some(i);
            }
        }
    }
    None
}

/// Get a device by index
pub fn get_device(idx: usize) -> Option<&'static Device> {
    if idx < MAX_DEVICES {
        unsafe {
            if DEVICES[idx].in_use {
                return Some(&DEVICES[idx]);
            }
        }
    }
    None
}

/// Read from a device by index. Returns bytes read.
pub fn device_read(dev_idx: usize, buf: &mut [u8]) -> usize {
    let dev = match get_device(dev_idx) {
        Some(d) => d,
        None => return 0,
    };
    match dev.ops.read {
        Some(read_fn) => read_fn(dev_idx, buf.as_mut_ptr(), buf.len()),
        None => 0,
    }
}

/// Write to a device by index. Returns bytes written.
pub fn device_write(dev_idx: usize, buf: &[u8]) -> usize {
    let dev = match get_device(dev_idx) {
        Some(d) => d,
        None => return 0,
    };
    match dev.ops.write {
        Some(write_fn) => write_fn(dev_idx, buf.as_ptr(), buf.len()),
        None => 0,
    }
}

/// IOCTL on a device. Returns 0 on success.
pub fn device_ioctl(dev_idx: usize, cmd: u64, arg: u64) -> i64 {
    let dev = match get_device(dev_idx) {
        Some(d) => d,
        None => return -1,
    };
    match dev.ops.ioctl {
        Some(ioctl_fn) => ioctl_fn(dev_idx, cmd, arg),
        None => -25, // -ENOTTY
    }
}

/// Get number of registered devices
pub fn device_count() -> usize {
    unsafe { DEVICE_COUNT }
}

// ═══════════════════════════════════════════════════════════════
// Initialization
// ═══════════════════════════════════════════════════════════════

/// Initialize /dev filesystem and register built-in devices
pub fn init() {
    // Register standard devices

    // /dev/null (1:3)
    register_device(
        "null",
        1,
        3,
        false,
        DeviceOps {
            read: Some(dev_null_read),
            write: Some(dev_null_write),
            ioctl: None,
        },
        0o666,
    );

    // /dev/zero (1:5)
    register_device(
        "zero",
        1,
        5,
        false,
        DeviceOps {
            read: Some(dev_zero_read),
            write: Some(dev_zero_write),
            ioctl: None,
        },
        0o666,
    );

    // /dev/urandom (1:9)
    register_device(
        "urandom",
        1,
        9,
        false,
        DeviceOps {
            read: Some(dev_urandom_read),
            write: Some(dev_urandom_write),
            ioctl: None,
        },
        0o666,
    );

    // /dev/kmsg (1:11) — kernel message log
    register_device(
        "kmsg",
        1,
        11,
        false,
        DeviceOps {
            read: Some(dev_kmsg_read),
            write: Some(dev_kmsg_write),
            ioctl: Some(dev_kmsg_ioctl),
        },
        0o644,
    );

    // /dev/ttyS0 (4:64) — serial console (ops are stubs for now)
    register_device(
        "ttyS0",
        4,
        64,
        false,
        DeviceOps {
            read: None, // Handled by VFS fd layer
            write: None,
            ioctl: None,
        },
        0o666,
    );

    // /dev/fb0 (29:0) — framebuffer
    register_device("fb0", 29, 0, false, DeviceOps::null(), 0o660);

    // /dev/input/mouse0 (13:32)
    register_device("mouse0", 13, 32, false, DeviceOps::null(), 0o660);

    // /dev/input/kbd0 (13:64)
    register_device("kbd0", 13, 64, false, DeviceOps::null(), 0o660);

    // /dev/nvme0 (8:0) — NVMe block device
    register_device("nvme0", 8, 0, true, DeviceOps::null(), 0o660);

    // /dev/rtc0 (253:0) — real-time clock
    register_device("rtc0", 253, 0, false, DeviceOps::null(), 0o660);

    // /dev/ptmx (5:2) — pseudo-terminal master multiplexer
    register_device(
        "ptmx",
        5,
        2,
        false,
        DeviceOps {
            read: None,
            write: None,
            ioctl: Some(dev_ptmx_ioctl),
        },
        0o666,
    );

    // /dev/pts directory (created by devpts filesystem)
    // Individual PTY slave devices (pts/0, pts/1, ...) are created dynamically
    // when PTY pairs are allocated via /dev/ptmx
    crate::fs::vfs::create_dir("/dev/pts").ok();
    println!("[devfs] Created /dev/pts directory");

    // Seed PRNG with timer value
    unsafe {
        PRNG_STATE ^= crate::arch::ArchTimer::system_time_us();
    }

    println!(
        "[devfs] /dev virtual filesystem initialized ({} devices)",
        unsafe { DEVICE_COUNT }
    );
}

/// Print /dev debug info
pub fn print_debug() {
    println!("[devfs] === Device List ===");
    unsafe {
        for i in 0..DEVICE_COUNT {
            let dev = &DEVICES[i];
            println!(
                "[devfs] /dev/{} ({}) {}:{}, mode={:#06o}",
                dev.name_str(),
                if dev.is_block { "block" } else { "char" },
                dev.major,
                dev.minor,
                dev.mode
            );
        }
    }
}
