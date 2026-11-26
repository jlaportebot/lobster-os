//! Kernel logging subsystem — ring buffer, dmesg syscall, /dev/kmsg
//!
//! Provides:
//! - Kernel ring buffer for log messages (like Linux's kmsg)
//! - dmesg syscall for reading the log buffer
//! - /dev/kmsg character device for userspace access
//! - Log levels (emerg, alert, crit, err, warning, notice, info, debug)

use crate::println;

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

/// Longest message kept for one log line; longer lines are truncated.
const MAX_LINE_LEN: usize = 480;
/// Room for the `<level>timestamp ` prefix and the trailing newline.
const ENTRY_OVERHEAD: usize = 32;
/// Level given to text captured from `println!` (it carries no level).
const CAPTURE_LEVEL: LogLevel = LogLevel::Info;
/// Milliseconds per second, for the `[ssss.mmm]` dmesg timestamp.
const MS_PER_SEC: u64 = 1000;

/// Kernel log ring buffer. Whole lines are evicted oldest-first, so the
/// buffer always starts at a line boundary and `dmesg` never shows a
/// half-line. Nothing here allocates.
struct KernelLog {
    buffer: [u8; KLOG_BUF_SIZE],
    /// Index of the oldest byte.
    start: usize,
    /// Bytes currently stored.
    len: usize,
    /// Bytes ever stored; `total - len` is the sequence number of the
    /// oldest byte, so readers can follow the log across evictions.
    total: u64,
    /// Line being assembled from raw `println!` output.
    partial: [u8; MAX_LINE_LEN],
    partial_len: usize,
}

impl KernelLog {
    const fn new() -> Self {
        Self {
            buffer: [0; KLOG_BUF_SIZE],
            start: 0,
            len: 0,
            total: 0,
            partial: [0; MAX_LINE_LEN],
            partial_len: 0,
        }
    }

    fn push_byte(&mut self, byte: u8) {
        self.buffer[(self.start + self.len) % KLOG_BUF_SIZE] = byte;
        self.len += 1;
        self.total += 1;
    }

    /// Drop the oldest line (through its newline) to make room.
    fn evict_line(&mut self) {
        while self.len > 0 {
            let byte = self.buffer[self.start];
            self.start = (self.start + 1) % KLOG_BUF_SIZE;
            self.len -= 1;
            if byte == b'\n' {
                break;
            }
        }
    }

    /// Append `<level>timestamp message\n`, evicting old lines as needed.
    fn append_entry(&mut self, level: LogLevel, timestamp_ms: u64, message: &[u8]) {
        let message = &message[..message.len().min(MAX_LINE_LEN)];
        while KLOG_BUF_SIZE - self.len < message.len() + ENTRY_OVERHEAD {
            self.evict_line();
        }
        self.push_byte(b'<');
        self.push_byte(b'0' + level.as_prio());
        self.push_byte(b'>');
        let mut digits = [0u8; 20];
        for &digit in decimal(timestamp_ms, &mut digits) {
            self.push_byte(digit);
        }
        self.push_byte(b' ');
        for &byte in message {
            self.push_byte(byte);
        }
        self.push_byte(b'\n');
    }

    /// Feed raw console text; each completed line becomes a log entry.
    fn capture(&mut self, timestamp_ms: u64, text: &str) {
        for &byte in text.as_bytes() {
            match byte {
                b'\n' => self.flush_partial(timestamp_ms),
                b'\r' => {}
                _ if self.partial_len < MAX_LINE_LEN => {
                    self.partial[self.partial_len] = byte;
                    self.partial_len += 1;
                }
                _ => {}
            }
        }
    }

    fn flush_partial(&mut self, timestamp_ms: u64) {
        if self.partial_len == 0 {
            return;
        }
        let line = self.partial;
        let len = self.partial_len;
        self.partial_len = 0;
        self.append_entry(CAPTURE_LEVEL, timestamp_ms, &line[..len]);
    }

    /// Copy stored bytes from sequence number `seq` into `buf`. A `seq`
    /// older than the oldest stored byte jumps forward to it.
    /// Returns (bytes copied, new sequence number).
    fn read(&self, buf: &mut [u8], seq: u64) -> (usize, u64) {
        let oldest = self.total - self.len as u64;
        let seq = seq.max(oldest);
        let offset = (seq - oldest) as usize;
        let count = (self.len - offset).min(buf.len());
        for (i, slot) in buf.iter_mut().enumerate().take(count) {
            *slot = self.buffer[(self.start + offset + i) % KLOG_BUF_SIZE];
        }
        (count, seq + count as u64)
    }

    /// Copy the entry starting at `seq` (without its newline) into `line`.
    /// Returns (bytes copied, sequence number of the next entry).
    fn read_line(&self, line: &mut [u8], seq: u64) -> Option<(usize, u64)> {
        let oldest = self.total - self.len as u64;
        let seq = seq.max(oldest);
        let offset = (seq - oldest) as usize;
        if offset >= self.len {
            return None;
        }
        let mut copied = 0;
        let mut consumed = 0;
        while offset + consumed < self.len {
            let byte = self.buffer[(self.start + offset + consumed) % KLOG_BUF_SIZE];
            consumed += 1;
            if byte == b'\n' {
                break;
            }
            if copied < line.len() {
                line[copied] = byte;
                copied += 1;
            }
        }
        Some((copied, seq + consumed as u64))
    }

    fn clear(&mut self) {
        self.start = 0;
        self.total += self.len as u64;
        self.len = 0;
        self.partial_len = 0;
    }
}

/// Decimal digits of `value` written into `out`; returns the used slice.
fn decimal(mut value: u64, out: &mut [u8; 20]) -> &[u8] {
    let mut pos = out.len();
    loop {
        pos -= 1;
        out[pos] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    &out[pos..]
}

/// Format a stored entry (`<6>1234 text`) as a dmesg line (`[   1.234] text`).
fn format_dmesg_line(entry: &str) -> alloc::string::String {
    let body = entry
        .strip_prefix('<')
        .and_then(|rest| rest.split_once('>'));
    let Some((_, rest)) = body else {
        return alloc::format!("{}\n", entry);
    };
    let (stamp, text) = rest.split_once(' ').unwrap_or((rest, ""));
    let ms: u64 = stamp.parse().unwrap_or(0);
    alloc::format!("[{:>5}.{:03}] {}\n", ms / MS_PER_SEC, ms % MS_PER_SEC, text)
}

/// Cell holding the global ring. Access is serialised by `BUSY`.
struct RingCell(core::cell::UnsafeCell<KernelLog>);

// SAFETY: every access goes through `with_log`, which holds the `BUSY`
// flag for the whole access, so the cell is never aliased across threads.
unsafe impl Sync for RingCell {}

static RING: RingCell = RingCell(core::cell::UnsafeCell::new(KernelLog::new()));
static BUSY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Take the ring if nobody else holds it. The console path uses this
/// instead of a blocking lock so a panic message printed while the ring is
/// busy is dropped rather than deadlocking.
fn try_acquire() -> bool {
    use core::sync::atomic::Ordering;
    BUSY.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
}

fn release() {
    BUSY.store(false, core::sync::atomic::Ordering::Release);
}

/// Run `f` on the ring if it is free, else skip it.
fn try_with_log<R>(f: impl FnOnce(&mut KernelLog) -> R) -> Option<R> {
    if !try_acquire() {
        return None;
    }
    // SAFETY: `try_acquire` succeeded, so this is the only reference.
    let result = f(unsafe { &mut *RING.0.get() });
    release();
    Some(result)
}

/// Run `f` on the ring, spinning until it is free.
fn with_log<R>(f: impl FnOnce(&mut KernelLog) -> R) -> R {
    while !try_acquire() {
        core::hint::spin_loop();
    }
    // SAFETY: `try_acquire` succeeded, so this is the only reference.
    let result = f(unsafe { &mut *RING.0.get() });
    release();
    result
}

/// Current uptime for entry timestamps (0 on host builds).
fn now_ms() -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        crate::arch::timer::Timer::uptime_ms()
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        0
    }
}

/// Get the kernel log length
pub fn klog_len() -> usize {
    with_log(|log| log.len)
}

/// Initialize the kernel logging subsystem
pub fn init() {
    println!(
        "[klog] Kernel log ring buffer initialized ({} KB)",
        KLOG_BUF_SIZE / 1024
    );
}

/// Set once the MMU and caches are on. Exclusive loads/stores (`BUSY`'s
/// compare_exchange) only succeed on cacheable memory, so before this point
/// capture must not touch `BUSY`; a plain atomic load is legal on any memory.
static CAPTURE_READY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Start recording console output. Call right after the MMU and caches are
/// enabled; earlier console output is printed but not captured.
pub fn enable_capture() {
    CAPTURE_READY.store(true, core::sync::atomic::Ordering::Relaxed);
}

/// Record console (`println!`) output in the ring. Allocation-free and
/// non-blocking: text is dropped if the ring is busy, so this is safe to
/// call from the console path with interrupts disabled.
pub fn capture(text: &str) {
    if !CAPTURE_READY.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let stamp = now_ms();
    let _ = try_with_log(|log| log.capture(stamp, text));
}

/// Write a log entry to the kernel log buffer
pub fn klog_write(level: LogLevel, message: &str) {
    let stamp = now_ms();
    with_log(|log| log.append_entry(level, stamp, message.as_bytes()));
}

/// Print kernel log to console (dmesg equivalent)
pub fn klog_dump() {
    klog_dump_with(|line| {
        #[cfg(target_arch = "aarch64")]
        crate::drivers::uart::Uart10::puts(line);
        #[cfg(not(target_arch = "aarch64"))]
        let _ = line;
    });
}

/// Print kernel log using a custom output function, one `[secs.ms] text`
/// line at a time. The ring lock is released before each call, so the
/// output function may itself print.
pub fn klog_dump_with<F>(mut write_line: F)
where
    F: FnMut(&str),
{
    let mut raw = [0u8; MAX_LINE_LEN + ENTRY_OVERHEAD];
    let mut seq = 0u64;
    while let Some((len, next)) = with_log(|log| log.read_line(&mut raw, seq)) {
        let entry = core::str::from_utf8(&raw[..len]).unwrap_or("");
        write_line(&format_dmesg_line(entry));
        seq = next;
    }
}

/// Read kernel log into user buffer (for dmesg syscall / /dev/kmsg).
/// `seq` is a byte sequence number that stays valid as old lines are
/// evicted. Returns the number of bytes read.
pub fn klog_read(buf: &mut [u8], seq: &mut usize) -> usize {
    let (count, next) = with_log(|log| log.read(buf, *seq as u64));
    *seq = next as usize;
    count
}

/// Clear the kernel log (for testing/debugging)
pub fn klog_clear() {
    with_log(KernelLog::clear);
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn capture_records_nothing_until_enabled() {
        CAPTURE_READY.store(false, core::sync::atomic::Ordering::Relaxed);
        let before = klog_len();
        capture("early line\n");
        assert_eq!(klog_len(), before);
        enable_capture();
        capture("late line\n");
        assert!(klog_len() > before);
    }

    fn log() -> alloc::boxed::Box<KernelLog> {
        alloc::boxed::Box::new(KernelLog::new())
    }

    fn lines(log: &KernelLog) -> alloc::vec::Vec<alloc::string::String> {
        let mut out = alloc::vec::Vec::new();
        let mut raw = [0u8; 600];
        let mut seq = 0;
        while let Some((len, next)) = log.read_line(&mut raw, seq) {
            out.push(alloc::string::String::from_utf8_lossy(&raw[..len]).into_owned());
            seq = next;
        }
        out
    }

    #[test]
    fn capture_splits_text_into_lines_in_order() {
        let mut log = log();
        log.capture(5, "[boot] one\n[boot] two\r\npart");
        log.capture(9, "ial\n");
        assert_eq!(
            lines(&log),
            ["<6>5 [boot] one", "<6>5 [boot] two", "<6>9 partial"]
        );
    }

    #[test]
    fn capture_keeps_an_unfinished_line_out_of_the_log() {
        let mut log = log();
        log.capture(1, "no newline yet");
        assert!(lines(&log).is_empty());
    }

    #[test]
    fn capture_skips_blank_lines() {
        let mut log = log();
        log.capture(1, "\n\na\n\n");
        assert_eq!(lines(&log), ["<6>1 a"]);
    }

    #[test]
    fn full_ring_drops_oldest_whole_lines() {
        let mut log = log();
        let line = "x".repeat(100);
        let total = KLOG_BUF_SIZE / 100 * 2;
        for i in 0..total {
            log.append_entry(LogLevel::Info, i as u64, line.as_bytes());
        }
        let kept = lines(&log);
        assert!(kept.len() < total);
        assert!(kept.iter().all(|l| l.ends_with(&line)));
        let first: u64 = kept[0][3..kept[0].find(' ').unwrap_or(0)]
            .parse()
            .unwrap_or(0);
        let last: u64 = kept[kept.len() - 1][3..kept[kept.len() - 1].find(' ').unwrap_or(0)]
            .parse()
            .unwrap_or(0);
        assert_eq!(last, total as u64 - 1);
        assert_eq!(last - first + 1, kept.len() as u64);
    }

    #[test]
    fn oversized_line_is_truncated() {
        let mut log = log();
        let long = "y".repeat(MAX_LINE_LEN * 2);
        log.append_entry(LogLevel::Error, 7, long.as_bytes());
        let kept = lines(&log);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].len(), "<3>7 ".len() + MAX_LINE_LEN);
    }

    #[test]
    fn read_follows_sequence_across_eviction() {
        let mut log = log();
        log.append_entry(LogLevel::Info, 1, b"first");
        let mut buf = [0u8; 64];
        let (n, seq) = log.read(&mut buf, 0);
        assert_eq!(&buf[..n], b"<6>1 first\n");
        let filler = "z".repeat(400);
        for i in 0..(KLOG_BUF_SIZE / 400 + 10) {
            log.append_entry(LogLevel::Info, i as u64, filler.as_bytes());
        }
        let (n, _) = log.read(&mut buf, seq);
        assert!(n > 0, "a stale sequence number jumps to the oldest entry");
        assert!(buf.starts_with(b"<6>"));
    }

    #[test]
    fn dmesg_line_shows_seconds_and_text() {
        assert_eq!(
            format_dmesg_line("<6>12345 [net] up"),
            "[   12.345] [net] up\n"
        );
        assert_eq!(format_dmesg_line("plain"), "plain\n");
    }

    #[test]
    fn clear_empties_log_but_keeps_sequence_monotonic() {
        let mut log = log();
        log.append_entry(LogLevel::Info, 1, b"a");
        let before = log.total;
        log.clear();
        assert!(lines(&log).is_empty());
        log.append_entry(LogLevel::Info, 2, b"b");
        assert!(log.total > before);
        assert_eq!(lines(&log), ["<6>2 b"]);
    }

    #[test]
    fn global_capture_is_visible_through_dump() {
        let _guard = crate::test_support::serial_guard();
        klog_clear();
        capture("[test] hello world\n");
        let mut dumped = alloc::string::String::new();
        klog_dump_with(|line| dumped.push_str(line));
        assert!(dumped.contains("[test] hello world\n"), "{dumped:?}");
        klog_clear();
    }
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
