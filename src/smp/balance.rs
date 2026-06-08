//! Load balancing across cores
//!
//! Periodically checks for imbalances and migrates tasks
//! from overloaded cores to underloaded ones.

use crate::println;
use crate::sched::{TaskState, MAX_TASKS, TASK_TABLE};
use crate::smp::runqueue;
use crate::smp::{is_core_online, CORE_COUNT};

/// Load imbalance threshold: if a core has more than LOAD_IMBALANCE_THRESHOLD
/// tasks than the average, it's considered overloaded
const LOAD_IMBALANCE_THRESHOLD: usize = 2;

/// Balance interval in timer ticks
const BALANCE_INTERVAL_TICKS: u64 = 100; // Every 100 ticks = ~1 second

/// Tick counter for periodic balancing
static mut BALANCE_TICK: u64 = 0;

/// Check if load balancing should run this tick
pub fn should_balance() -> bool {
    unsafe {
        BALANCE_TICK += 1;
        BALANCE_TICK % BALANCE_INTERVAL_TICKS == 0
    }
}

/// Run the load balancer
/// Called periodically from the timer tick handler
pub fn balance() {
    let online_cores: usize = (0..CORE_COUNT).filter(|&i| is_core_online(i)).count();
    if online_cores <= 1 {
        return; // No balancing needed for single core
    }

    // Calculate per-core loads
    let mut loads = [0usize; CORE_COUNT];
    let mut total_load = 0usize;
    for i in 0..CORE_COUNT {
        if is_core_online(i) {
            loads[i] = runqueue::runnable_count(i);
            total_load += loads[i];
        }
    }

    if total_load == 0 {
        return;
    }

    let avg_load = total_load / online_cores;

    // Find the most overloaded and most underloaded cores
    let mut max_core = 0usize;
    let mut max_load = 0usize;
    let mut min_core = 0usize;
    let mut min_load = usize::MAX;

    for i in 0..CORE_COUNT {
        if is_core_online(i) {
            if loads[i] > max_load {
                max_load = loads[i];
                max_core = i;
            }
            if loads[i] < min_load {
                min_load = loads[i];
                min_core = i;
            }
        }
    }

    // Check if there's a significant imbalance
    if max_load <= avg_load + LOAD_IMBALANCE_THRESHOLD {
        return; // Balanced enough
    }

    if min_load >= avg_load {
        return; // Already balanced
    }

    // Migrate one task from the overloaded core to the underloaded core
    // Pick a task that's allowed to run on the target core
    let migrated = migrate_from_overloaded(max_core, min_core);
    if migrated {
        // Update load tracking
        unsafe {
            crate::smp::core_local(max_core)
                .as_mut()
                .map(|c| c.load_avg = loads[max_load].saturating_sub(1) as u64);
            crate::smp::core_local(min_core)
                .as_mut()
                .map(|c| c.load_avg = loads[min_load].saturating_add(1) as u64);
        }
    }
}

/// Try to migrate a task from an overloaded core to an underloaded one
fn migrate_from_overloaded(from_core: usize, to_core: usize) -> bool {
    // Find a suitable task on the source core's runqueue
    // Prefer tasks that are not currently running and have no core affinity restriction
    unsafe {
        for i in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[i] {
                if task.cpu_id == from_core as u64
                    && task.state == TaskState::Ready
                    && (task.affinity & (1 << to_core)) != 0
                {
                    let pid = task.pid;
                    return runqueue::migrate_task(pid, from_core, to_core);
                }
            }
        }
    }
    false
}

/// Called from the timer tick handler to drive load balancing
pub fn on_timer_tick() {
    if should_balance() {
        balance();
    }
}

/// Print load balancing statistics
pub fn print_stats() {
    println!("[balance] === Load Balancer Stats ===");
    for i in 0..CORE_COUNT {
        if is_core_online(i) {
            let load = runqueue::runnable_count(i);
            println!("[balance] Core {} load: {} tasks", i, load);
        }
    }
}
