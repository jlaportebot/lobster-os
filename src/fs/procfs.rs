//! /proc virtual filesystem — process and kernel information
//!
//! Provides read-only virtual files exposing:
//! - /proc/self → current process info
//! - /proc/<pid>/ → per-process directories
//! - /proc/cpuinfo → CPU identification
//! - /proc/meminfo → memory statistics
//! - /proc/uptime → system uptime
//! - /proc/version → kernel version
//! - /proc/cmdline → boot command line
//!
//! All content is generated dynamically on read.

use crate::memory::frame_alloc;
use crate::println;
use crate::sched::{self, Pid, TaskState, MAX_TASKS, TASK_TABLE};

/// Maximum length of a generated /proc file content
const PROC_BUF_SIZE: usize = 2048;

/// /proc file types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcEntry {
    Cpuinfo,
    Meminfo,
    Uptime,
    Version,
    Cmdline,
    SelfLink,     // /proc/self → link to current PID dir
    SelfStat,     // /proc/self/stat
    SelfStatus,   // /proc/self/status
    SelfCmdline,  // /proc/self/cmdline
    SelfFd,       // /proc/self/fd
    PidStat,      // /proc/<pid>/stat
    PidStatus,    // /proc/<pid>/status
    PidCmdline,   // /proc/<pid>/cmdline
    PidFd,        // /proc/<pid>/fd
    Capabilities, // /proc/capabilities — global capability info
    SelfCap,      // /proc/self/capabilities
    PidCap,       // /proc/<pid>/capabilities
}

/// A /proc directory entry
#[derive(Clone, Copy)]
pub struct ProcDirEntry {
    pub name: [u8; 32],
    pub name_len: usize,
    pub entry_type: ProcEntry,
    pub pid: Pid, // 0 for global entries
}

impl ProcDirEntry {
    pub const fn empty() -> Self {
        Self {
            name: [0u8; 32],
            name_len: 0,
            entry_type: ProcEntry::Cpuinfo,
            pid: 0,
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

/// Maximum directory entries in /proc root
const MAX_PROC_ROOT_ENTRIES: usize = 16;

/// Cached /proc root directory entries (rebuilt on each list)
static mut PROC_ROOT_ENTRIES: [ProcDirEntry; MAX_PROC_ROOT_ENTRIES] =
    [ProcDirEntry::empty(); MAX_PROC_ROOT_ENTRIES];
static mut PROC_ROOT_COUNT: usize = 0;

/// Build /proc root directory listing
pub fn build_root_listing() {
    unsafe {
        let mut count = 0usize;

        // Static entries
        let statics: &[(&str, ProcEntry)] = &[
            ("cpuinfo", ProcEntry::Cpuinfo),
            ("meminfo", ProcEntry::Meminfo),
            ("uptime", ProcEntry::Uptime),
            ("version", ProcEntry::Version),
            ("cmdline", ProcEntry::Cmdline),
            ("self", ProcEntry::SelfLink),
            ("capabilities", ProcEntry::Capabilities),
        ];

        for (name, etype) in statics {
            if count >= MAX_PROC_ROOT_ENTRIES {
                break;
            }
            PROC_ROOT_ENTRIES[count].set_name(name);
            PROC_ROOT_ENTRIES[count].entry_type = *etype;
            PROC_ROOT_ENTRIES[count].pid = 0;
            count += 1;
        }

        // Per-PID directories
        for i in 0..MAX_TASKS {
            if count >= MAX_PROC_ROOT_ENTRIES {
                break;
            }
            if let Some(ref task) = TASK_TABLE[i] {
                if task.state != TaskState::Zombie {
                    let mut buf = [0u8; 32];
                    let pid_str = uitoa(task.pid, &mut buf);
                    PROC_ROOT_ENTRIES[count].set_name(pid_str);
                    PROC_ROOT_ENTRIES[count].entry_type = ProcEntry::PidStat;
                    PROC_ROOT_ENTRIES[count].pid = task.pid;
                    count += 1;
                }
            }
        }

        PROC_ROOT_COUNT = count;
    }
}

/// Convert u64 to ASCII string, returns &str into the provided buffer
fn uitoa(val: u64, buf: &mut [u8; 32]) -> &str {
    if val == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap_or("0");
    }
    let mut pos = 31usize;
    let mut v = val;
    while v > 0 && pos > 0 {
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
        pos -= 1;
    }
    let start = pos + 1;
    core::str::from_utf8(&buf[start..32]).unwrap_or("0")
}

/// Generate content for a /proc file. Returns the number of bytes written.
pub fn read_proc_file(entry: ProcEntry, pid: Pid, buf: &mut [u8]) -> usize {
    match entry {
        ProcEntry::Cpuinfo => generate_cpuinfo(buf),
        ProcEntry::Meminfo => generate_meminfo(buf),
        ProcEntry::Uptime => generate_uptime(buf),
        ProcEntry::Version => generate_version(buf),
        ProcEntry::Cmdline => generate_cmdline(buf),
        ProcEntry::SelfStat | ProcEntry::PidStat => generate_pid_stat(pid, buf),
        ProcEntry::SelfStatus | ProcEntry::PidStatus => generate_pid_status(pid, buf),
        ProcEntry::SelfCmdline | ProcEntry::PidCmdline => generate_pid_cmdline(pid, buf),
        ProcEntry::SelfFd | ProcEntry::PidFd => generate_pid_fd(pid, buf),
        ProcEntry::Capabilities => generate_capabilities(buf),
        ProcEntry::SelfCap | ProcEntry::PidCap => generate_pid_capabilities(pid, buf),
        ProcEntry::SelfLink => {
            // /proc/self → return the PID as text (like a symlink target)
            let current = sched::current_pid();
            let mut tmp = [0u8; 32];
            let s = uitoa(current, &mut tmp);
            let bytes = s.as_bytes();
            let len = bytes.len().min(buf.len());
            buf[..len].copy_from_slice(&bytes[..len]);
            len
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Content generators
// ═══════════════════════════════════════════════════════════════

fn generate_cpuinfo(buf: &mut [u8]) -> usize {
    let midr = unsafe {
        let val: u64;
        core::arch::asm!("mrs {}, MIDR_EL1", out(reg) val);
        val
    };
    let implementer = (midr >> 24) & 0xFF;
    let variant = (midr >> 20) & 0xF;
    let architecture = (midr >> 16) & 0xF;
    let part_num = (midr >> 4) & 0xFFF;
    let revision = midr & 0xF;

    let num_cores = 4u64; // BCM2712 quad-core

    // Format: similar to Linux /proc/cpuinfo
    let mut off = 0usize;
    for core in 0..num_cores {
        off += write_str(buf, off, "processor\t: ");
        off += write_u64(buf, off, core);
        off += write_str(buf, off, "\n");
        off += write_str(buf, off, "BogoMIPS\t: 24.00\n");
        off += write_str(
            buf,
            off,
            "Features\t: fp asimd evtstrm aes pmull sha1 sha2 crc32 atomics\n",
        );
        off += write_str(buf, off, "CPU implementer\t: 0x");
        off += write_hex(buf, off, implementer as u32);
        off += write_str(buf, off, "\n");
        off += write_str(buf, off, "CPU architecture\t: ");
        off += write_u64(buf, off, architecture);
        off += write_str(buf, off, "\n");
        off += write_str(buf, off, "CPU variant\t: 0x");
        off += write_hex(buf, off, variant as u32);
        off += write_str(buf, off, "\n");
        off += write_str(buf, off, "CPU part\t: 0x");
        off += write_hex(buf, off, part_num as u32);
        off += write_str(buf, off, "\n");
        off += write_str(buf, off, "CPU revision\t: ");
        off += write_u64(buf, off, revision);
        off += write_str(buf, off, "\n\n");
        if off >= buf.len() - 128 {
            break;
        }
    }
    off
}

fn generate_meminfo(buf: &mut [u8]) -> usize {
    let (total, allocated, _first_free) = frame_alloc::stats();
    let total_kb = (total as u64) * 4096 / 1024;
    let used_kb = (allocated as u64) * 4096 / 1024;
    let free_kb = total_kb.saturating_sub(used_kb);

    let mut off = 0usize;
    off += write_str(buf, off, "MemTotal:\t");
    off += write_u64(buf, off, total_kb);
    off += write_str(buf, off, " kB\n");
    off += write_str(buf, off, "MemFree:\t");
    off += write_u64(buf, off, free_kb);
    off += write_str(buf, off, " kB\n");
    off += write_str(buf, off, "MemAvailable:\t");
    off += write_u64(buf, off, free_kb);
    off += write_str(buf, off, " kB\n");
    off += write_str(buf, off, "Buffers:\t0 kB\n");
    off += write_str(buf, off, "Cached:\t0 kB\n");
    off += write_str(buf, off, "SwapTotal:\t0 kB\n");
    off += write_str(buf, off, "SwapFree:\t0 kB\n");
    off
}

fn generate_uptime(buf: &mut [u8]) -> usize {
    let ticks = sched::tick_count();
    let uptime_secs = (ticks * sched::TICK_MS) / 1000;
    let uptime_frac = ((ticks * sched::TICK_MS) % 1000) / 100;

    let mut off = 0usize;
    off += write_u64(buf, off, uptime_secs);
    off += write_str(buf, off, ".");
    off += write_u64(buf, off, uptime_frac);
    off += write_str(buf, off, " ");
    off += write_u64(buf, off, uptime_secs); // idle = uptime for now
    off += write_str(buf, off, ".00\n");
    off
}

fn generate_version(buf: &mut [u8]) -> usize {
    write_str(
        buf,
        0,
        "LobsterOS v0.2.0 (aarch64) #1 SMP PREEMPT\n\
         BCM2712 Quad-core Cortex-A76 @ 2.4 GHz\n\
         Rust #![no_std] bare-metal\n",
    )
}

fn generate_cmdline(buf: &mut [u8]) -> usize {
    // No boot cmdline for bare-metal; return empty with newline
    write_str(buf, 0, "console=ttyS0,115200\n")
}

fn generate_pid_stat(pid: Pid, buf: &mut [u8]) -> usize {
    let task_idx = match sched::find_task_index(pid) {
        Some(i) => i,
        None => return write_str(buf, 0, "(no such process)\n"),
    };

    let (state_char, _state_name) = unsafe {
        match TASK_TABLE[task_idx] {
            Some(ref t) => match t.state {
                TaskState::Running => ('R', "running"),
                TaskState::Ready => ('R', "ready"),
                TaskState::Blocked => ('S', "sleeping"),
                TaskState::Zombie => ('Z', "zombie"),
                TaskState::Created => ('I', "idle"),
            },
            None => return write_str(buf, 0, "(no such process)\n"),
        }
    };

    let (uid, _euid, gid, _egid, runtime, priority, nice) = unsafe {
        match TASK_TABLE[task_idx] {
            Some(ref t) => (
                t.uid,
                t.euid,
                t.gid,
                t.egid,
                t.runtime_ticks,
                t.priority as u8,
                t.nice as u8,
            ),
            None => (0, 0, 0, 0, 0, 0, 0),
        }
    };

    let mut off = 0usize;
    off += write_u64(buf, off, pid);
    off += write_str(buf, off, " (lobsteros) ");
    buf[off] = state_char as u8;
    off += 1;
    off += write_str(buf, off, " 1 0 0 0 "); // ppid, pgid, sid, tty
    off += write_u64(buf, off, uid as u64);
    off += write_str(buf, off, " ");
    off += write_u64(buf, off, gid as u64);
    off += write_str(buf, off, " ");
    off += write_u64(buf, off, runtime);
    off += write_str(buf, off, " ");
    off += write_u64(buf, off, priority as u64);
    off += write_str(buf, off, " ");
    off += write_u64(buf, off, nice as u64);
    off += write_str(buf, off, "\n");
    off
}

fn generate_pid_status(pid: Pid, buf: &mut [u8]) -> usize {
    let task_idx = match sched::find_task_index(pid) {
        Some(i) => i,
        None => return write_str(buf, 0, "(no such process)\n"),
    };

    let (uid, euid, gid, egid, state, runtime, priority, nice, cpu_id) = unsafe {
        match TASK_TABLE[task_idx] {
            Some(ref t) => (
                t.uid,
                t.euid,
                t.gid,
                t.egid,
                t.state,
                t.runtime_ticks,
                t.priority,
                t.nice,
                t.cpu_id,
            ),
            None => return write_str(buf, 0, "(no such process)\n"),
        }
    };

    let state_str = match state {
        TaskState::Running => "R (running)",
        TaskState::Ready => "R (ready)",
        TaskState::Blocked => "S (sleeping)",
        TaskState::Zombie => "Z (zombie)",
        TaskState::Created => "I (idle)",
    };

    let prio_str = match priority {
        sched::Priority::Low => "Low",
        sched::Priority::Normal => "Normal",
        sched::Priority::High => "High",
    };

    let mut off = 0usize;
    off += write_str(buf, off, "Name:\tlobsteros\n");
    off += write_str(buf, off, "State:\t");
    off += write_str(buf, off, state_str);
    off += write_str(buf, off, "\n");
    off += write_str(buf, off, "Pid:\t");
    off += write_u64(buf, off, pid);
    off += write_str(buf, off, "\n");
    off += write_str(buf, off, "Uid:\t");
    off += write_u64(buf, off, uid as u64);
    off += write_str(buf, off, "\t");
    off += write_u64(buf, off, euid as u64);
    off += write_str(buf, off, "\nGid:\t");
    off += write_u64(buf, off, gid as u64);
    off += write_str(buf, off, "\t");
    off += write_u64(buf, off, egid as u64);
    off += write_str(buf, off, "\n");
    off += write_str(buf, off, "Threads:\t1\n");
    off += write_str(buf, off, "Processor:\t");
    off += write_u64(buf, off, cpu_id);
    off += write_str(buf, off, "\n");
    off += write_str(buf, off, "Priority:\t");
    off += write_str(buf, off, prio_str);
    off += write_str(buf, off, "\n");
    off += write_str(buf, off, "Nice:\t");
    off += write_u64(buf, off, nice as u64);
    off += write_str(buf, off, "\n");
    off += write_str(buf, off, "Runtime:\t");
    off += write_u64(buf, off, runtime);
    off += write_str(buf, off, " ticks\n");
    off
}

fn generate_pid_cmdline(_pid: Pid, buf: &mut [u8]) -> usize {
    // No argv for kernel tasks
    write_str(buf, 0, "[kernel]\n")
}

fn generate_pid_fd(_pid: Pid, buf: &mut [u8]) -> usize {
    write_str(buf, 0, "0:/dev/ttyS0\n1:/dev/ttyS0\n2:/dev/ttyS0\n")
}

/// Generate /proc/capabilities — global capability info
fn generate_capabilities(buf: &mut [u8]) -> usize {
    let mut off = 0usize;
    off += write_str(buf, off, "LobsterOS Capability List\n");
    off += write_str(buf, off, "CAP_CHOWN      0\n");
    off += write_str(buf, off, "CAP_DAC_OVERRIDE 1\n");
    off += write_str(buf, off, "CAP_DAC_READ_SEARCH 2\n");
    off += write_str(buf, off, "CAP_FOWNER     3\n");
    off += write_str(buf, off, "CAP_FSETID     4\n");
    off += write_str(buf, off, "CAP_KILL        5\n");
    off += write_str(buf, off, "CAP_SETGID      6\n");
    off += write_str(buf, off, "CAP_SETUID      7\n");
    off += write_str(buf, off, "CAP_SETPCAP     8\n");
    off += write_str(buf, off, "CAP_NET_BIND_SERVICE 9\n");
    off += write_str(buf, off, "CAP_NET_RAW     10\n");
    off += write_str(buf, off, "CAP_SYS_ADMIN   11\n");
    off += write_str(buf, off, "CAP_SYS_PTRACE  12\n");
    off += write_str(buf, off, "CAP_SYS_RESOURCE 13\n");
    off += write_str(buf, off, "CAP_SYS_TIME    14\n");
    off += write_str(buf, off, "CAP_AUDIT_WRITE 15\n");
    off
}

/// Generate /proc/<pid>/capabilities — per-process capability dump
fn generate_pid_capabilities(pid: Pid, buf: &mut [u8]) -> usize {
    let task_idx = match sched::find_task_index(pid) {
        Some(i) => i,
        None => return write_str(buf, 0, "(no such process)\n"),
    };

    let caps = crate::security::get_capabilities(task_idx);
    let mut off = 0usize;
    off += write_str(buf, off, "CapEff:\t");
    for i in (0..16).rev() {
        if caps & (1u64 << i) != 0 {
            off += write_str(buf, off, "1");
        } else {
            off += write_str(buf, off, "0");
        }
    }
    off += write_str(buf, off, "\n");
    off
}

// ═══════════════════════════════════════════════════════════════
// Write helpers
// ═══════════════════════════════════════════════════════════════

/// Write a &str into buf at offset, returns bytes written
fn write_str(buf: &mut [u8], off: usize, s: &str) -> usize {
    let bytes = s.as_bytes();
    let len = bytes.len().min(buf.len().saturating_sub(off));
    buf[off..off + len].copy_from_slice(&bytes[..len]);
    len
}

/// Write a u64 as decimal ASCII, returns bytes written
fn write_u64(buf: &mut [u8], off: usize, val: u64) -> usize {
    let mut tmp = [0u8; 20];
    if val == 0 {
        if off < buf.len() {
            buf[off] = b'0';
        }
        return 1;
    }
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
    buf[off..off + to_write].copy_from_slice(&tmp[start..start + to_write]);
    to_write
}

/// Write a u32 as hex (no prefix), returns bytes written
fn write_hex(buf: &mut [u8], off: usize, val: u32) -> usize {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut tmp = [0u8; 8];
    for i in 0..8 {
        tmp[7 - i] = HEX[((val >> (i * 4)) & 0xF) as usize];
    }
    let avail = buf.len().saturating_sub(off);
    let to_write = 8.min(avail);
    buf[off..off + to_write].copy_from_slice(&tmp[..to_write]);
    to_write
}

// ═══════════════════════════════════════════════════════════════
// Initialization
// ═══════════════════════════════════════════════════════════════

/// Initialize /proc filesystem
pub fn init() {
    build_root_listing();
    println!("[procfs] /proc virtual filesystem initialized");
}
