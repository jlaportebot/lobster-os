//! Kernel logging subsystem — ring buffer, dmesg syscall, /dev/kmsg
//!
//! Provides:
//! - Kernel ring buffer for log messages (like Linux's kmsg)
//! - dmesg syscall for reading the log buffer
//! - /dev/kmsg character device for userspace access
//! - Log levels (emerg, alert, crit, err, warning, notice, info, debug)

use crate::println;
use crate::sync::Spinlock;
use alloc::string::{String, ToString};

/// Maximum size of the kernel log ring buffer (64KB)
const KLOG_BUF_SIZE: usize = 65536;

/// Log levels (matching syslog/kernel conventions)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogLevel {
    Emergency = 0, // system is unusable
    Alert = 1,     // action must be taken immediately
    Critical = 2,  // critical conditions
    Error = 3,     // error conditions
    Warning = 4,   // warning conditions
    Notice = 5,    // normal but significant condition
    Info = 6,      // informational
    Debug = 7,     // debug-level messages
}

impl LogLevel {
    pub const fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Emergency => "emerg",
            LogLevel::Alert => "alert",
            LogLevel::Critical => "crit",
            LogLevel::Error => "err",
            LogLevel::Warning => "warn",
            LogLevel::Notice => "notice",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
        }
    }

    pub const fn as_prio(&self) -> u8 {
        *self as u8
    }
}

/// A single log entry
#[derive(Clone, Debug)]
struct LogEntry {
    level: LogLevel,
    timestamp_ms: u64,
    message: String,
}

/// Kernel log ring buffer
struct KernelLog {
    buffer: [u8; KLOG_BUF_SIZE],
    head: usize, // Next write position
    tail: usize, // Next read position
    len: usize,  // Current length
    wrapped: bool,
}

impl KernelLog {
    const fn new() -> Self {
        Self {
            buffer: [0; KLOG_BUF_SIZE],
            head: 0,
            tail: 0,
            len: 0,
            wrapped: false,
        }
    }

    /// Write a log entry to the ring buffer
    fn write(&mut self, entry: &LogEntry) {
        // Format: <level>timestamp message\n
        let formatted = alloc::format!(
            "<{}>{} {}\n",
            entry.level.as_prio(),
            entry.timestamp_ms,
            entry.message
        );
        let bytes = formatted.as_bytes();

        // If message is larger than buffer, truncate
        let write_len = bytes.len().min(KLOG_BUF_SIZE - 1);

        if write_len >= KLOG_BUF_SIZE - self.len {
            // Need to make space - advance tail
            self.tail = (self.tail + write_len + 1) % KLOG_BUF_SIZE;
            self.len = self.len.saturating_sub(write_len + 1);
            self.wrapped = true;
        }

        // Write bytes
        for (i, &b) in bytes.iter().enumerate().take(write_len) {
            self.buffer[(self.head + i) % KLOG_BUF_SIZE] = b;
        }
        self.head = (self.head + write_len) % KLOG_BUF_SIZE;
        self.len += write_len;
    }

    /// Read log entries into a buffer, starting from a sequence number
    /// Returns (bytes_read, new_seq)
    fn read(&self, buf: &mut [u8], seq: usize) -> (usize, usize) {
        if seq >= self.len && self.wrapped {
            // Sequence is too old, start from tail
        }

        let read_start = if self.wrapped {
            (self.head + KLOG_BUF_SIZE - self.len) % KLOG_BUF_SIZE
        } else {
            0
        };

        let available = self.len.saturating_sub(seq);
        let read_len = available.min(buf.len());

        for i in 0..read_len {
            buf[i] = self.buffer[(read_start + seq + i) % KLOG_BUF_SIZE];
        }

        (read_len, seq + read_len)
    }

    /// Get total log length
    fn len(&self) -> usize {
        self.len
    }

    /// Clear the log
    fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.len = 0;
        self.wrapped = false;
    }
}

static KLOG: Spinlock<KernelLog> = Spinlock::new(KernelLog::new());

/// Initialize the kernel logging subsystem
pub fn init() {
    println!(
        "[klog] Kernel log ring buffer initialized ({} KB)",
        KLOG_BUF_SIZE / 1024
    );
}

/// Write a log entry to the kernel log buffer
pub fn klog_write(level: LogLevel, message: &str) {
    let timestamp = crate::arch::timer::Timer::uptime_ms();
    let entry = LogEntry {
        level,
        timestamp_ms: timestamp,
        message: message.to_string(),
    };
    KLOG.lock().write(&entry);
}

/// Print kernel log to console (dmesg equivalent)
pub fn klog_dump() {
    let log = KLOG.lock();
    let mut buf = [0u8; 4096];
    let mut seq = 0;

    while seq < log.len() {
        let (read_len, new_seq) = log.read(&mut buf, seq);
        if read_len == 0 {
            break;
        }
        // Print directly to console (bypass klog to avoid recursion)
        crate::drivers::uart::Uart10::puts(core::str::from_utf8(&buf[..read_len]).unwrap_or(""));
        seq = new_seq;
    }
}

/// Read kernel log into user buffer (for dmesg syscall / /dev/kmsg)
/// Returns number of bytes read
pub fn klog_read(buf: &mut [u8], seq: &mut usize) -> usize {
    let log = KLOG.lock();
    let (read_len, new_seq) = log.read(buf, *seq);
    *seq = new_seq;
    read_len
}

/// Clear the kernel log (for testing/debugging)
pub fn klog_clear() {
    KLOG.lock().clear();
}

// Convenience macros for different log levels
#[macro_export]
macro_rules! klog_emerg {
    ($($arg:tt)*) => { $crate::klog::klog_write($crate::klog::LogLevel::Emergency, &alloc::format!($($arg)*)) };
}

#[macro_export]
macro_rules! klog_alert {
    ($($arg:tt)*) => { $crate::klog::klog_write($crate::klog::LogLevel::Alert, &alloc::format!($($arg)*)) };
}

#[macro_export]
macro_rules! klog_crit {
    ($($arg:tt)*) => { $crate::klog::klog_write($crate::klog::LogLevel::Critical, &alloc::format!($($arg)*)) };
}

#[macro_export]
macro_rules! klog_err {
    ($($arg:tt)*) => { $crate::klog::klog_write($crate::klog::LogLevel::Error, &alloc::format!($($arg)*)) };
}

#[macro_export]
macro_rules! klog_warn {
    ($($arg:tt)*) => { $crate::klog::klog_write($crate::klog::LogLevel::Warning, &alloc::format!($($arg)*)) };
}

#[macro_export]
macro_rules! klog_notice {
    ($($arg:tt)*) => { $crate::klog::klog_write($crate::klog::LogLevel::Notice, &alloc::format!($($arg)*)) };
}

#[macro_export]
macro_rules! klog_info {
    ($($arg:tt)*) => { $crate::klog::klog_write($crate::klog::LogLevel::Info, &alloc::format!($($arg)*)) };
}

#[macro_export]
macro_rules! klog_debug {
    ($($arg:tt)*) => { $crate::klog::klog_write($crate::klog::LogLevel::Debug, &alloc::format!($($arg)*)) };
}
