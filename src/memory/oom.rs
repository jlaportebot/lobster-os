//! Out-of-memory handler for LobsterOS
//!
//! When the frame allocator runs out of frames, the OOM handler kicks in.
//! It walks the task list, finds the task using the most memory, and either:
//! - Sends SIGKILL if severity is Hard
//! - Logs a warning if severity is Soft
//!
//! After a hard OOM, the frame allocator retries the allocation.

use crate::println;
use crate::sched;

/// OOM severity levels
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OomSeverity {
    /// Soft OOM: low on memory, log warning
    Soft,
    /// Hard OOM: completely out of memory, kill a process
    Hard,
}

/// OOM event counters
#[derive(Debug, Clone, Copy, Default)]
pub struct OomStats {
    pub soft_oom_count: u64,
    pub hard_oom_count: u64,
    pub last_killed_pid: u64,
    pub last_oom_reason: u64,
}

static mut OOM_STATS: OomStats = OomStats {
    soft_oom_count: 0,
    hard_oom_count: 0,
    last_killed_pid: 0,
    last_oom_reason: 0,
};

static mut OOM_ACTIVE: bool = false;

/// Get current OOM statistics
pub fn stats() -> OomStats {
    unsafe { OOM_STATS }
}

/// Check if OOM handler is currently active (handling an OOM event)
pub fn is_active() -> bool {
    unsafe { OOM_ACTIVE }
}

/// Handle an OOM event
///
/// Returns true if memory was freed (by killing a task), false if no action was taken.
pub fn handle(severity: OomSeverity, requested_frames: u64) -> bool {
    unsafe {
        if OOM_ACTIVE {
            // Recursive OOM — can't do much
            println!(
                "[oom] Recursive OOM during handling! requested_frames={}",
                requested_frames
            );
            return false;
        }
        OOM_ACTIVE = true;
    }

    match severity {
        OomSeverity::Soft => {
            unsafe {
                OOM_STATS.soft_oom_count += 1;
                OOM_STATS.last_oom_reason = requested_frames;
            }
            println!(
                "[oom] Soft OOM: low on frames (requested {}). {} soft OOM events total.",
                requested_frames,
                stats().soft_oom_count
            );
            // Try to free some caches (page cache, dentry cache in VFS)
            let freed = try_free_caches();
            unsafe {
                OOM_ACTIVE = false;
            }
            freed
        }
        OomSeverity::Hard => {
            unsafe {
                OOM_STATS.hard_oom_count += 1;
                OOM_STATS.last_oom_reason = requested_frames;
            }
            println!(
                "[oom] Hard OOM: no frames available (requested {}). Attempting process kill.",
                requested_frames
            );

            // Find the most memory-hungry task (excluding the kernel idle task)
            let victim_pid = find_oom_victim();
            if let Some(pid) = victim_pid {
                println!("[oom] Killing PID {} (largest memory consumer)", pid);
                unsafe {
                    OOM_STATS.last_killed_pid = pid;
                }
                // Send SIGKILL to the victim
                let _ = sched::signal::kill(pid, sched::signal::SIGKILL);
                unsafe {
                    OOM_ACTIVE = false;
                }
                true
            } else {
                println!("[oom] No userspace task to kill. System may crash.");
                unsafe {
                    OOM_ACTIVE = false;
                }
                false
            }
        }
    }
}

/// Try to free kernel caches to reclaim memory
fn try_free_caches() -> bool {
    // In a full implementation, we'd walk the page cache and free clean pages.
    // For now, report that we're trying.
    println!("[oom] Attempting to free kernel caches...");
    // The frame allocator may have some reserved low-memory frames
    false
}

/// Find the task consuming the most memory (OOM victim selection)
fn find_oom_victim() -> Option<u64> {
    // Walk the task table, find the one with the largest heap or highest runtime
    // For safety, never kill PID 0 (idle) or PID 1 (init)
    // This is a simplified heuristic — real OOM uses oom_score_adj, memory cgroup
    // accounting, etc.
    //
    // We iterate through tasks and pick the one with the largest runtime_ticks
    // as a proxy for "doing the most work" (and likely using the most memory).
    sched::get_oom_victim()
}
