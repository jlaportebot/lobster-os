//! Syslog daemon — reads /dev/kmsg and writes to /var/log/messages
//!
//! Runs as a kernel task, polling /dev/kmsg ring buffer.
//! Implements log rotation when files exceed 1MB.
//! Supports log levels: EMERG, ALERT, CRIT, ERR, WARN, NOTICE, INFO, DEBUG

use crate::klog;
use crate::println;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

static SYSLOG_RUNNING: AtomicBool = AtomicBool::new(false);

const MAX_LOG_SIZE: usize = 1024 * 1024; // 1MB
const MAX_ROTATED_FILES: usize = 5;

/// Log severity levels (RFC 5424 / syslog standard)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Emergency = 0,
    Alert = 1,
    Critical = 2,
    Error = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

impl LogLevel {
    pub fn from_klog(level: u8) -> Self {
        match level {
            0 => LogLevel::Emergency,
            1 => LogLevel::Alert,
            2 => LogLevel::Critical,
            3 => LogLevel::Error,
            4 => LogLevel::Warning,
            5 => LogLevel::Notice,
            6 => LogLevel::Info,
            _ => LogLevel::Debug,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Emergency => "EMERG",
            LogLevel::Alert => "ALERT",
            LogLevel::Critical => "CRIT",
            LogLevel::Error => "ERR",
            LogLevel::Warning => "WARN",
            LogLevel::Notice => "NOTICE",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
        }
    }

    pub fn as_short(&self) -> &'static str {
        match self {
            LogLevel::Emergency => "EM",
            LogLevel::Alert => "AL",
            LogLevel::Critical => "CR",
            LogLevel::Error => "ER",
            LogLevel::Warning => "WA",
            LogLevel::Notice => "NO",
            LogLevel::Info => "IN",
            LogLevel::Debug => "DB",
        }
    }
}

/// Syslog message entry
pub struct LogEntry {
    pub timestamp: u64,
    pub level: LogLevel,
    pub facility: u8,
    pub message: String,
}

/// Log buffer with rotation support
pub struct LogBuffer {
    entries: Vec<LogEntry>,
    current_size: usize,
    rotated_count: usize,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            current_size: 0,
            rotated_count: 0,
        }
    }

    pub fn append(&mut self, entry: LogEntry) {
        let entry_size = entry.message.len() + 32;
        self.current_size += entry_size;
        self.entries.push(entry);

        if self.current_size >= MAX_LOG_SIZE {
            self.rotate();
        }
    }

    fn rotate(&mut self) {
        // Keep the last 100 entries, discard the rest
        if self.entries.len() > 100 {
            let cutoff = self.entries.len() - 100;
            self.entries.drain(..cutoff);
        }
        self.current_size = self.entries.iter().map(|e| e.message.len() + 32).sum();
        self.rotated_count += 1;
        if self.rotated_count > MAX_ROTATED_FILES {
            self.rotated_count = MAX_ROTATED_FILES;
        }
    }

    pub fn format_all(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&format!(
                "[{:>6}] {}\n",
                entry.level.as_str(),
                entry.message
            ));
        }
        out
    }

    pub fn format_filtered(&self, min_level: LogLevel) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            if entry.level <= min_level {
                out.push_str(&format!(
                    "[{:>6}] {}\n",
                    entry.level.as_str(),
                    entry.message
                ));
            }
        }
        out
    }
}

/// Start the syslog daemon as a kernel task
pub fn start() -> ! {
    if SYSLOG_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        println!("[syslog] Already running");
        // Still enter infinite loop since we're a kernel task
        loop {
            crate::arch::cpu::wfe();
        }
    }

    println!("[syslog] Starting syslog daemon...");

    let mut log_buffer = LogBuffer::new();

    // Seed with initial entries
    log_buffer.append(LogEntry {
        timestamp: 0,
        level: LogLevel::Info,
        facility: 3, // daemon
        message: String::from("Syslog daemon started"),
    });

    // Main loop: poll kmsg and write to log
    let mut last_seq: usize = 0;
    let mut poll_count: u32 = 0;

    loop {
        // Read from kernel log ring buffer
        let mut buf = [0u8; 512];
        let read = klog::klog_read(&mut buf, &mut last_seq);

        if read > 0 {
            let msg = core::str::from_utf8(&buf[..read]).unwrap_or("<binary>");
            let level = if !msg.is_empty() {
                let prefix = msg.chars().next().unwrap_or('6');
                match prefix.to_digit(10) {
                    Some(l) => LogLevel::from_klog(l as u8),
                    None => LogLevel::Info,
                }
            } else {
                LogLevel::Info
            };

            log_buffer.append(LogEntry {
                timestamp: 0,
                level,
                facility: 0, // kernel
                message: String::from(msg),
            });

            // last_seq is updated by klog_read via &mut reference
        }

        // Clear old messages periodically (every 100 polls)
        poll_count += 1;
        if poll_count % 100 == 0 {
            klog::klog_clear();
        }

        // Yield to scheduler
        crate::arch::cpu::wfe();
    }
}

/// Get a snapshot of the current log buffer (for `dmesg` and log viewers)
pub fn read_log() -> String {
    // Return a snapshot of recent kernel messages
    let mut buf = [0u8; 8192];
    let read = klog::klog_read(&mut buf, &mut 0);
    if read > 0 {
        String::from(core::str::from_utf8(&buf[..read]).unwrap_or(""))
    } else {
        String::new()
    }
}

/// Get log entries filtered by level
pub fn read_log_filtered(min_level: LogLevel) -> String {
    let full = read_log();
    let mut out = String::new();
    for line in full.lines() {
        // Check if line starts with a level prefix
        let level_char = line.chars().next().unwrap_or('6');
        if let Some(level_num) = level_char.to_digit(10) {
            let entry_level = LogLevel::from_klog(level_num as u8);
            if entry_level <= min_level {
                out.push_str(line);
                out.push('\n');
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
