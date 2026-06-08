//! /sys virtual filesystem — kernel and hardware information
//!
//! Provides a sysfs-like hierarchy exposing:
//! - /sys/kernel/version — kernel version string
//! - /sys/kernel/hostname — system hostname
//! - /sys/kernel/uptime — uptime in seconds
//! - /sys/kernel/tick_count — raw tick counter
//! - /sys/kernel/pid_max — maximum PID value
//! - /sys/kernel/threads_max — maximum concurrent tasks
//! - /sys/cpu/core_count — number of CPU cores
//! - /sys/cpu/core<N>/freq — per-core frequency (MHz)
//! - /sys/cpu/core<N>/online — per-core online status
//! - /sys/memory/total — total RAM in bytes
//! - /sys/memory/used — used RAM in bytes
//! - /sys/memory/free — free RAM in bytes
//! - /sys/memory/page_size — page size in bytes
//! - /sys/devices/ — registered device count + list
//!
//! All entries are generated dynamically on read.

use crate::memory::frame_alloc;
use crate::println;
use crate::sched;

/// Maximum sysfs entries
const MAX_SYS_ENTRIES: usize = 16;

/// Sysfs entry types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SysEntry {
    // /sys/kernel/
    KernelVersion,
    KernelHostname,
    KernelUptime,
    KernelTickCount,
    KernelPidMax,
    KernelThreadsMax,
    // /sys/cpu/
    CpuCoreCount,
    CpuCoreFreq,   // param = core_id
    CpuCoreOnline, // param = core_id
    // /sys/memory/
    MemoryTotal,
    MemoryUsed,
    MemoryFree,
    MemoryPageSize,
    // /sys/devices/
    DeviceCount,
}

/// Sysfs directory entry
#[derive(Clone, Copy)]
pub struct SysDirEntry {
    pub name: [u8; 48],
    pub name_len: usize,
    pub entry_type: SysEntry,
    pub param: u64,
}

impl SysDirEntry {
    pub const fn empty() -> Self {
        Self {
            name: [0u8; 48],
            name_len: 0,
            entry_type: SysEntry::KernelVersion,
            param: 0,
        }
    }

    pub fn set_name(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(48);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name_len = len;
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("?")
    }
}

static mut SYS_ENTRIES: [SysDirEntry; MAX_SYS_ENTRIES] = [SysDirEntry::empty(); MAX_SYS_ENTRIES];
static mut SYS_ENTRY_COUNT: usize = 0;

/// Generate content for a sysfs entry. Returns bytes written.
pub fn read_sysfs_entry(entry: SysEntry, param: u64, buf: &mut [u8]) -> usize {
    match entry {
        SysEntry::KernelVersion => write_str(buf, 0, "0.1.0\n"),
        SysEntry::KernelHostname => write_str(buf, 0, "lobsteros\n"),
        SysEntry::KernelUptime => {
            let ticks = sched::tick_count();
            let secs = ticks * sched::TICK_MS / 1000;
            let mut off = write_u64(buf, 0, secs);
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::KernelTickCount => {
            let mut off = write_u64(buf, 0, sched::tick_count());
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::KernelPidMax => {
            let mut off = write_u64(buf, 0, sched::MAX_TASKS as u64);
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::KernelThreadsMax => {
            let mut off = write_u64(buf, 0, sched::MAX_TASKS as u64);
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::CpuCoreCount => {
            let mut off = write_u64(buf, 0, 4); // BCM2712 quad-core
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::CpuCoreFreq => {
            // BCM2712 runs at 2.4 GHz
            let mut off = write_u64(buf, 0, 2400);
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::CpuCoreOnline => {
            // Check SMP online mask
            let online = crate::smp::is_core_online(param as usize);
            let mut off = write_u64(buf, 0, if online { 1 } else { 0 });
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::MemoryTotal => {
            let (total, _, _) = frame_alloc::stats();
            let total_bytes = total as u64 * 4096;
            let mut off = write_u64(buf, 0, total_bytes);
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::MemoryUsed => {
            let (_, allocated, _) = frame_alloc::stats();
            let used_bytes = allocated as u64 * 4096;
            let mut off = write_u64(buf, 0, used_bytes);
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::MemoryFree => {
            let (total, allocated, _) = frame_alloc::stats();
            let free_bytes = (total.saturating_sub(allocated)) as u64 * 4096;
            let mut off = write_u64(buf, 0, free_bytes);
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::MemoryPageSize => {
            let mut off = write_u64(buf, 0, 4096);
            off += write_str(buf, off, "\n");
            off
        }
        SysEntry::DeviceCount => {
            let count = crate::fs::devfs::device_count();
            let mut off = write_u64(buf, 0, count as u64);
            off += write_str(buf, off, "\n");
            off
        }
    }
}

/// Build the sysfs directory listing
pub fn build_listing() {
    unsafe {
        let mut count = 0usize;

        let entries: &[(&str, SysEntry, u64)] = &[
            ("kernel/version", SysEntry::KernelVersion, 0),
            ("kernel/hostname", SysEntry::KernelHostname, 0),
            ("kernel/uptime", SysEntry::KernelUptime, 0),
            ("kernel/tick_count", SysEntry::KernelTickCount, 0),
            ("kernel/pid_max", SysEntry::KernelPidMax, 0),
            ("kernel/threads_max", SysEntry::KernelThreadsMax, 0),
            ("cpu/core_count", SysEntry::CpuCoreCount, 0),
            ("cpu/core0/freq", SysEntry::CpuCoreFreq, 0),
            ("cpu/core0/online", SysEntry::CpuCoreOnline, 0),
            ("cpu/core1/freq", SysEntry::CpuCoreFreq, 1),
            ("cpu/core1/online", SysEntry::CpuCoreOnline, 1),
            ("cpu/core2/freq", SysEntry::CpuCoreFreq, 2),
            ("cpu/core2/online", SysEntry::CpuCoreOnline, 2),
            ("cpu/core3/freq", SysEntry::CpuCoreFreq, 3),
            ("cpu/core3/online", SysEntry::CpuCoreOnline, 3),
            ("memory/total", SysEntry::MemoryTotal, 0),
            ("memory/used", SysEntry::MemoryUsed, 0),
            ("memory/free", SysEntry::MemoryFree, 0),
            ("memory/page_size", SysEntry::MemoryPageSize, 0),
            ("devices/count", SysEntry::DeviceCount, 0),
        ];

        for (name, etype, param) in entries {
            if count >= MAX_SYS_ENTRIES {
                break;
            }
            SYS_ENTRIES[count].set_name(name);
            SYS_ENTRIES[count].entry_type = *etype;
            SYS_ENTRIES[count].param = *param;
            count += 1;
        }

        SYS_ENTRY_COUNT = count;
    }
}

/// Get the sysfs entry list (read-only reference)
pub fn get_entries() -> (usize, &'static [SysDirEntry]) {
    unsafe { (SYS_ENTRY_COUNT, &SYS_ENTRIES) }
}

// ═══════════════════════════════════════════════════════════════
// Write helpers (same as procfs — duplicated for no_std module isolation)
// ═══════════════════════════════════════════════════════════════

fn write_str(buf: &mut [u8], off: usize, s: &str) -> usize {
    let bytes = s.as_bytes();
    let len = bytes.len().min(buf.len().saturating_sub(off));
    if len > 0 {
        buf[off..off + len].copy_from_slice(&bytes[..len]);
    }
    len
}

fn write_u64(buf: &mut [u8], off: usize, val: u64) -> usize {
    if val == 0 {
        if off < buf.len() {
            buf[off] = b'0';
        }
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut pos = 19usize;
    let mut v = val;
    while v > 0 {
        tmp[pos] = b'0' + (v % 10) as u8;
        v /= 10;
        if pos == 0 {
            break;
        }
        pos -= 1;
    }
    let start = if v > 0 { 0 } else { pos + 1 };
    let len = 20 - start;
    let avail = buf.len().saturating_sub(off);
    let to_write = len.min(avail);
    if to_write > 0 {
        buf[off..off + to_write].copy_from_slice(&tmp[start..start + to_write]);
    }
    to_write
}

// ═══════════════════════════════════════════════════════════════
// Initialization
// ═══════════════════════════════════════════════════════════════

pub fn init() {
    build_listing();
    println!(
        "[sysfs] /sys virtual filesystem initialized ({} entries)",
        unsafe { SYS_ENTRY_COUNT }
    );
}
