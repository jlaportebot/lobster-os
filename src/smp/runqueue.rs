//! Per-core runqueues for SMP scheduling
//!
//! Each core maintains its own runqueue of tasks.
//! Load balancing migrates tasks between cores when imbalances are detected.
//!
//! Design:
//! - Each core has a runqueue (doubly-linked list of PIDs)
//! - Tasks are assigned to cores based on affinity and load
//! - The core scheduler picks the next task from its local runqueue
//! - Cross-core migration is done via IPI reschedule

use crate::arch::cpu;
use crate::println;
use crate::sched::{self, time_slice_for, Pid, Priority, TaskState, MAX_TASKS, TASK_TABLE};
use crate::smp::{is_core_online, set_core_current_pid, CORE_COUNT};

/// Maximum tasks per core's runqueue
const RUNQUEUE_CAPACITY: usize = 64;

/// Per-core runqueue
struct CoreRunqueue {
    /// PIDs in this runqueue (0 = empty slot)
    pids: [Pid; RUNQUEUE_CAPACITY],
    /// Number of PIDs in the runqueue
    count: usize,
    /// Index of the current scheduling position (round-robin cursor)
    cursor: usize,
}

impl CoreRunqueue {
    const fn new() -> Self {
        Self {
            pids: [0; RUNQUEUE_CAPACITY],
            count: 0,
            cursor: 0,
        }
    }

    /// Add a PID to this runqueue
    fn enqueue(&mut self, pid: Pid) -> bool {
        if self.count >= RUNQUEUE_CAPACITY {
            return false;
        }
        // Don't add duplicates
        for i in 0..self.count {
            if self.pids[i] == pid {
                return true; // Already enqueued
            }
        }
        self.pids[self.count] = pid;
        self.count += 1;
        true
    }

    /// Remove a PID from this runqueue
    fn dequeue(&mut self, pid: Pid) -> bool {
        for i in 0..self.count {
            if self.pids[i] == pid {
                // Shift remaining entries left
                for j in i..(self.count - 1) {
                    self.pids[j] = self.pids[j + 1];
                }
                self.count -= 1;
                self.pids[self.count] = 0;
                // Adjust cursor if needed
                if self.cursor > 0 && self.cursor >= self.count {
                    self.cursor = 0;
                }
                return true;
            }
        }
        false
    }

    /// Pick the next task to run using round-robin with priority
    /// Returns the PID of the next task, or None if empty
    fn pick_next(&mut self) -> Option<Pid> {
        if self.count == 0 {
            return None;
        }

        // First pass: high priority tasks
        let start = self.cursor;
        let mut idx = start;
        for _ in 0..self.count {
            let pid = self.pids[idx];
            if is_task_runnable(pid, Priority::High) {
                self.cursor = (idx + 1) % self.count;
                return Some(pid);
            }
            idx = (idx + 1) % self.count;
        }

        // Second pass: normal priority tasks
        idx = start;
        for _ in 0..self.count {
            let pid = self.pids[idx];
            if is_task_runnable(pid, Priority::Normal) {
                self.cursor = (idx + 1) % self.count;
                return Some(pid);
            }
            idx = (idx + 1) % self.count;
        }

        // Third pass: low priority tasks
        idx = start;
        for _ in 0..self.count {
            let pid = self.pids[idx];
            if is_task_runnable(pid, Priority::Low) {
                self.cursor = (idx + 1) % self.count;
                return Some(pid);
            }
            idx = (idx + 1) % self.count;
        }

        // No runnable tasks
        None
    }

    /// Get the number of tasks in this runqueue
    fn len(&self) -> usize {
        self.count
    }

    /// Get the number of runnable tasks (Ready or Running state)
    fn runnable_count(&self) -> usize {
        let mut count = 0usize;
        for i in 0..self.count {
            let pid = self.pids[i];
            unsafe {
                if let Some(ref task) = TASK_TABLE[sched::find_task_index(pid).unwrap_or(MAX_TASKS)]
                {
                    if task.state == TaskState::Ready || task.state == TaskState::Running {
                        count += 1;
                    }
                }
            }
        }
        count
    }
}

/// Check if a task is runnable at the given priority
fn is_task_runnable(pid: Pid, min_priority: Priority) -> bool {
    if pid == 0 {
        return false;
    }
    if let Some(idx) = sched::find_task_index(pid) {
        unsafe {
            if let Some(ref task) = TASK_TABLE[idx] {
                if (task.state == TaskState::Ready || task.state == TaskState::Running)
                    && task.priority >= min_priority
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Per-core runqueues
static mut CORE_RUNQUEUES: [CoreRunqueue; CORE_COUNT] = [
    CoreRunqueue::new(),
    CoreRunqueue::new(),
    CoreRunqueue::new(),
    CoreRunqueue::new(),
];

/// Initialize all per-core runqueues
/// Distributes existing tasks across cores
pub fn init() {
    // Assign existing tasks to cores round-robin
    unsafe {
        let mut core_idx = 0usize;
        for i in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[i] {
                if task.state == TaskState::Ready || task.state == TaskState::Running {
                    CORE_RUNQUEUES[core_idx].enqueue(task.pid);
                    core_idx = (core_idx + 1) % CORE_COUNT;
                }
            }
        }
    }

    // Assign idle tasks to each core
    for i in 0..CORE_COUNT {
        if is_core_online(i) {
            unsafe {
                // Each core gets the idle task (PID 0)
                CORE_RUNQUEUES[i].enqueue(0);
            }
        }
    }

    println!("[runqueue] Initialized per-core runqueues");
}

/// Add a task to a specific core's runqueue
pub fn enqueue_task(core_id: usize, pid: Pid) -> bool {
    if core_id >= CORE_COUNT {
        return false;
    }
    unsafe { CORE_RUNQUEUES[core_id].enqueue(pid) }
}

/// Add a task to the least-loaded core's runqueue
pub fn enqueue_task_balanced(pid: Pid) -> bool {
    let core_id = find_least_loaded_core();
    enqueue_task(core_id, pid)
}

/// Remove a task from a specific core's runqueue
pub fn dequeue_task(core_id: usize, pid: Pid) -> bool {
    if core_id >= CORE_COUNT {
        return false;
    }
    unsafe { CORE_RUNQUEUES[core_id].dequeue(pid) }
}

/// Remove a task from whichever core's runqueue it's on
pub fn dequeue_task_any(pid: Pid) -> bool {
    for i in 0..CORE_COUNT {
        if dequeue_task(i, pid) {
            return true;
        }
    }
    false
}

/// Pick the next task to run on a specific core
pub fn pick_next_task(core_id: usize) -> Option<Pid> {
    if core_id >= CORE_COUNT {
        return None;
    }
    unsafe { CORE_RUNQUEUES[core_id].pick_next() }
}

/// Get the number of tasks on a core's runqueue
pub fn runqueue_len(core_id: usize) -> usize {
    if core_id >= CORE_COUNT {
        return 0;
    }
    unsafe { CORE_RUNQUEUES[core_id].len() }
}

/// Get the number of runnable tasks on a core's runqueue
pub fn runnable_count(core_id: usize) -> usize {
    if core_id >= CORE_COUNT {
        return 0;
    }
    unsafe { CORE_RUNQUEUES[core_id].runnable_count() }
}

/// Find the core with the fewest runnable tasks
pub fn find_least_loaded_core() -> usize {
    let mut best_core = 0;
    let mut best_load = usize::MAX;

    for i in 0..CORE_COUNT {
        if is_core_online(i) {
            let load = runnable_count(i);
            if load < best_load {
                best_load = load;
                best_core = i;
            }
        }
    }
    best_core
}

/// Core scheduler loop — runs on each secondary core
/// Picks the next task from the local runqueue and context-switches to it
pub fn core_scheduler_loop(core_id: u64) -> ! {
    let core = core_id as usize;

    loop {
        // Try to pick a task from our runqueue
        let next_pid = pick_next_task(core);

        if let Some(pid) = next_pid {
            if pid != 0 {
                // Found a real task — context switch to it
                set_core_current_pid(core, pid);
                unsafe {
                    if let Some(idx) = sched::find_task_index(pid) {
                        if let Some(ref mut task) = TASK_TABLE[idx] {
                            task.state = TaskState::Running;
                            task.cpu_id = core as u64;
                            task.time_slice = time_slice_for(task.priority);

                            // Do the context switch
                            sched::restore_context(&task.regs);
                        }
                    }
                }
            }
        }

        // No runnable task — enter low-power WFE
        cpu::wfe();
    }
}

/// Handle a timer tick on a specific core
/// Decrements the current task's time slice and triggers reschedule if expired
pub fn on_core_timer_tick(core_id: usize) {
    let current_pid = crate::smp::core_current_pid(core_id);
    if current_pid == 0 {
        return; // Idle task, no timeslicing needed
    }

    if let Some(idx) = sched::find_task_index(current_pid) {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.runtime_ticks += 1;
                if task.time_slice > 0 {
                    task.time_slice -= 1;
                }
                if task.time_slice == 0 {
                    // Time slice expired — request reschedule
                    task.state = TaskState::Ready;
                    sched::request_schedule();
                }
            }
        }
    }
}

/// Migrate a task from one core's runqueue to another
pub fn migrate_task(pid: Pid, from_core: usize, to_core: usize) -> bool {
    if from_core >= CORE_COUNT || to_core >= CORE_COUNT {
        return false;
    }
    if !is_core_online(to_core) {
        return false;
    }

    // Remove from source core
    if !dequeue_task(from_core, pid) {
        return false;
    }

    // Add to destination core
    if !enqueue_task(to_core, pid) {
        // Failed to enqueue on target — put it back
        enqueue_task(from_core, pid);
        return false;
    }

    // Update task's cpu_id field
    if let Some(idx) = sched::find_task_index(pid) {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.cpu_id = to_core as u64;
            }
        }
    }

    // Send reschedule IPI to the destination core
    crate::smp::ipi::send_reschedule(to_core as u64);

    true
}

/// Print runqueue status for all cores
pub fn print_status() {
    println!("[runqueue] === Per-Core Runqueues ===");
    for i in 0..CORE_COUNT {
        let len = runqueue_len(i);
        let runnable = runnable_count(i);
        unsafe {
            let rq = &CORE_RUNQUEUES[i];
            let mut pids_str = [0u8; 256]; // Buffer for PID list
            let mut pos = 0;
            for j in 0..rq.count.min(8) {
                // Print first 8 PIDs
                let pid = rq.pids[j];
                if pos < 240 {
                    let digits = format_pid(pid);
                    for &d in &digits {
                        if pos < 255 {
                            pids_str[pos] = d;
                            pos += 1;
                        }
                    }
                    if j < rq.count.min(8) - 1 && pos < 254 {
                        pids_str[pos] = b',';
                        pos += 1;
                        pids_str[pos] = b' ';
                        pos += 1;
                    }
                }
            }
            let pid_str = core::str::from_utf8(&pids_str[..pos]).unwrap_or("?");
            let online = if is_core_online(i) { "UP" } else { "DN" };
            let ellipsis = if rq.count > 8 { "..." } else { "" };
            println!(
                "[runqueue] Core {} [{}] {}/{} runnable, tasks: [{}{}]",
                i, online, runnable, len, pid_str, ellipsis
            );
        }
    }
}

/// Format a PID as a byte string
fn format_pid(pid: Pid) -> [u8; 20] {
    let mut buf = [0u8; 20];
    if pid == 0 {
        buf[0] = b'i';
        buf[1] = b'd';
        buf[2] = b'l';
        buf[3] = b'e';
        return buf;
    }
    let mut n = pid;
    let mut pos = 19;
    if n == 0 {
        buf[pos] = b'0';
        return buf;
    }
    while n > 0 {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // Shift to start
    let len = 20 - pos;
    for i in 0..len {
        buf[i] = buf[pos + i];
    }
    // Zero out the rest (we don't need them, just return)
    buf
}
