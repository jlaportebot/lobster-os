//! Preemptive round-robin scheduler with context switching
//!
//! Supports:
//! - Kernel tasks (EL1) and user processes (EL0)
//! - Timer-driven preemptive scheduling
//! - Cooperative yielding
//! - Per-process address spaces (TTBR0)
//! - Priority-based scheduling (simple: 0=low, 1=normal, 2=high)

use crate::memory::frame_alloc;
use crate::memory::{phys_to_virt, PhysAddr, VirtAddr, PAGE_SIZE};
use crate::net::dhcp;
use crate::println;
use core::arch::asm;

pub mod dlinker;
pub mod elf;
pub mod init;
pub mod process;
pub mod signal;
pub mod syscall;

/// Maximum number of concurrent tasks
pub const MAX_TASKS: usize = 32;

/// Default kernel stack size (8 pages = 32KB)
const KERN_STACK_PAGES: usize = 8;
const KERN_STACK_SIZE: usize = KERN_STACK_PAGES * PAGE_SIZE as usize;

/// Default user stack size (16 pages = 64KB)
const USER_STACK_PAGES: usize = 16;
const USER_STACK_SIZE: usize = USER_STACK_PAGES * PAGE_SIZE as usize;

/// Scheduler tick interval in milliseconds
pub const TICK_MS: u64 = 10;

/// Task states
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskState {
    /// Created but not yet scheduled
    Created,
    /// Ready to run
    Ready,
    /// Currently executing
    Running,
    /// Waiting for I/O, sleep, or other event
    Blocked,
    /// Exited but not yet reaped
    Zombie,
}

/// Task priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

/// Saved register context for context switching
/// Must match the layout used in switch_to() assembly
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    // General-purpose registers (x0-x30)
    pub x0: u64,
    pub x1: u64,
    pub x2: u64,
    pub x3: u64,
    pub x4: u64,
    pub x5: u64,
    pub x6: u64,
    pub x7: u64,
    pub x8: u64,
    pub x9: u64,
    pub x10: u64,
    pub x11: u64,
    pub x12: u64,
    pub x13: u64,
    pub x14: u64,
    pub x15: u64,
    pub x16: u64,
    pub x17: u64,
    pub x18: u64,
    pub x19: u64,
    pub x20: u64,
    pub x21: u64,
    pub x22: u64,
    pub x23: u64,
    pub x24: u64,
    pub x25: u64,
    pub x26: u64,
    pub x27: u64,
    pub x28: u64,
    pub x29: u64,
    pub x30: u64, // x30 = LR
    // Special registers
    pub sp: u64,
    pub elr: u64,  // Exception Link Register (return address)
    pub spsr: u64, // Saved Program Status Register
    pub esr: u64,  // Exception Syndrome Register (for fault info)
}

impl TrapFrame {
    pub const fn new() -> Self {
        Self {
            x0: 0,
            x1: 0,
            x2: 0,
            x3: 0,
            x4: 0,
            x5: 0,
            x6: 0,
            x7: 0,
            x8: 0,
            x9: 0,
            x10: 0,
            x11: 0,
            x12: 0,
            x13: 0,
            x14: 0,
            x15: 0,
            x16: 0,
            x17: 0,
            x18: 0,
            x19: 0,
            x20: 0,
            x21: 0,
            x22: 0,
            x23: 0,
            x24: 0,
            x25: 0,
            x26: 0,
            x27: 0,
            x28: 0,
            x29: 0,
            x30: 0,
            sp: 0,
            elr: 0,
            spsr: 0,
            esr: 0,
        }
    }
}

impl Default for TrapFrame {
    fn default() -> Self {
        Self::new()
    }
}

/// Process ID type
pub type Pid = u64;

/// Maximum signals (POSIX: 1-31 standard + 32-63 realtime)
pub const SIG_COUNT: usize = 32;

/// Maximum open file descriptors per process
pub const MAX_FDS: usize = 32;

/// Maximum pending IPC messages
pub const MAX_IPC_PENDING: usize = 8;

/// Task Control Block — full process context
#[repr(C)]
pub struct Task {
    // ── Identity ──
    /// Unique process ID
    pub pid: Pid,
    /// Parent PID (0 for kernel/init)
    pub parent_pid: Pid,
    /// Process group ID
    pub pgid: Pid,
    /// Session ID
    pub sid: Pid,
    /// Real user ID
    pub uid: u32,
    /// Effective user ID
    pub euid: u32,
    /// Real group ID
    pub gid: u32,
    /// Effective group ID
    pub egid: u32,
    /// Supplementary groups (up to 8)
    pub groups: [u32; 4],
    pub group_count: u32,

    // ── Scheduling ──
    /// Current state
    pub state: TaskState,
    /// Scheduling priority
    pub priority: Priority,
    /// CPU affinity mask (bit per core)
    pub affinity: u64,
    /// Which core is this task currently on
    pub cpu_id: u64,
    /// Total runtime in ticks
    pub runtime_ticks: u64,
    /// Ticks remaining in current time slice
    pub time_slice: u64,
    /// Niceness value (-20 to +19, 0 default)
    pub nice: i8,

    // ── Register context ──
    /// Saved register context
    pub regs: TrapFrame,
    /// Kernel stack pointer (top of stack, grows down)
    pub kernel_sp: u64,
    /// Kernel stack base (for deallocation)
    pub kernel_stack_base: u64,
    /// User stack pointer (EL0 SP)
    pub user_sp: u64,
    /// User stack base (for deallocation)
    pub user_stack_base: u64,
    /// TTBR0 physical address for this process (0 for kernel tasks)
    pub ttbr0_phys: u64,
    /// Process entry point
    pub entry_point: u64,
    /// Is this a user-space process (EL0)?
    pub is_user: bool,

    // ── Signals ──
    /// Bitmask of pending signals
    pub pending_signals: u64,
    /// Bitmask of blocked signals (signal mask)
    pub signal_mask: u64,
    /// Per-signal actions (handler or SIG_DFL/SIG_IGN)
    pub signal_handlers: [u64; SIG_COUNT],

    // ── IPC ──
    /// IPC namespace ID (0 = global)
    pub ipc_ns: u32,
    /// Pipe read-end file descriptor (or -1)
    pub pipe_read_fd: i64,
    /// Pipe write-end file descriptor (or -1)
    pub pipe_write_fd: i64,

    // ── Exit ──
    /// Exit code (set when task becomes Zombie)
    pub exit_code: i64,
    /// Signal that caused termination (0 if not killed by signal)
    pub exit_signal: u64,
}

/// Global scheduler state
pub static mut TASK_TABLE: [Option<Task>; MAX_TASKS] = [const { None }; MAX_TASKS];
static mut CURRENT_PID: Pid = 0;
static mut NEXT_PID: Pid = 1;
static mut SCHEDULER_ENABLED: bool = false;
static mut TICK_COUNT: u64 = 0;

/// Get current tick count
pub fn tick_count() -> u64 {
    unsafe { TICK_COUNT }
}

/// Get current PID
pub fn current_pid() -> Pid {
    unsafe { CURRENT_PID }
}

/// Get and increment next PID (for fork)
pub fn get_next_pid() -> Pid {
    unsafe {
        let pid = NEXT_PID;
        NEXT_PID += 1;
        pid
    }
}

/// Check if scheduler is active
pub fn is_enabled() -> bool {
    unsafe { SCHEDULER_ENABLED }
}

/// Allocate a kernel stack (multiple pages)
fn alloc_kernel_stack() -> Option<(u64, u64)> {
    let base_phys = frame_alloc::allocate_contiguous(KERN_STACK_PAGES)?;
    let base_virt = phys_to_virt(PhysAddr::new(base_phys.0)).0;
    // Map all pages
    for i in 0..KERN_STACK_PAGES {
        let offset = (i as u64) * PAGE_SIZE;
        unsafe {
            crate::memory::vmem::map_kernel_page(
                base_virt + offset,
                base_phys.0 + offset,
                crate::memory::vmem::PTE_KERNEL_DATA,
            );
        }
    }
    // Stack pointer is at the top (grows downward)
    let sp = base_virt + KERN_STACK_SIZE as u64;
    Some((sp, base_virt))
}

/// Allocate a user stack
fn alloc_user_stack() -> Option<(u64, u64)> {
    let base_phys = frame_alloc::allocate_contiguous(USER_STACK_PAGES)?;
    // For user stacks, we'll map them into the user's address space later
    // For now, use direct physical map
    let base_virt = phys_to_virt(PhysAddr::new(base_phys.0)).0;
    for i in 0..USER_STACK_PAGES {
        let offset = (i as u64) * PAGE_SIZE;
        unsafe {
            crate::memory::vmem::map_kernel_page(
                base_virt + offset,
                base_phys.0 + offset,
                crate::memory::vmem::PTE_KERNEL_DATA,
            );
        }
    }
    let sp = base_virt + USER_STACK_SIZE as u64;
    Some((sp, base_virt))
}

/// Initialize the scheduler
pub fn init() {
    println!("[sched] Initializing preemptive scheduler...");

    unsafe {
        for i in 0..MAX_TASKS {
            TASK_TABLE[i] = None;
        }
        CURRENT_PID = 0;
        NEXT_PID = 1;
        SCHEDULER_ENABLED = false;
        TICK_COUNT = 0;
    }

    // Create idle task (PID 0)
    let idle_pid = create_kernel_task(idle_task_entry, Priority::Low)
        .expect("[sched] Failed to create idle task");

    println!("[sched] Idle task created (PID {})", idle_pid);
    println!("[sched] Scheduler initialized");
}

/// Idle task — runs when nothing else is ready
fn idle_task_entry() -> ! {
    loop {
        crate::arch::cpu::wfe();
    }
}

/// Create a new kernel-space task (runs at EL1)
pub fn create_kernel_task(entry: fn() -> !, priority: Priority) -> Option<Pid> {
    let (kern_sp, kern_base) = alloc_kernel_stack()?;

    let pid = unsafe {
        let pid = NEXT_PID;
        NEXT_PID += 1;
        pid
    };

    // Set up initial register state
    let mut regs = TrapFrame::new();
    regs.elr = entry as u64;
    regs.sp = kern_sp;
    // SPSR: EL1h, all interrupts masked, AArch64
    // M[3:0] = 0100 (EL1h), DAIF = 1111
    regs.spsr = 0x3C4;
    // x30 = entry point (so eret goes there)
    regs.x30 = entry as u64;

    let task = Task {
        pid,
        parent_pid: 0,
        pgid: 0,
        sid: 0,
        uid: 0,
        euid: 0,
        gid: 0,
        egid: 0,
        groups: [0; 4],
        group_count: 0,
        state: TaskState::Ready,
        priority,
        affinity: 0xF, // all cores
        cpu_id: 0,
        runtime_ticks: 0,
        time_slice: time_slice_for(priority),
        nice: 0,
        regs,
        kernel_sp: kern_sp,
        kernel_stack_base: kern_base,
        user_sp: 0,
        user_stack_base: 0,
        ttbr0_phys: 0,
        entry_point: entry as u64,
        is_user: false,
        ipc_ns: 0, // global namespace
        pending_signals: 0,
        signal_mask: 0,
        signal_handlers: [0; SIG_COUNT],
        pipe_read_fd: -1,
        pipe_write_fd: -1,
        exit_code: 0,
        exit_signal: 0,
    };

    unsafe {
        for i in 0..MAX_TASKS {
            if TASK_TABLE[i].is_none() {
                TASK_TABLE[i] = Some(task);
                println!(
                    "[sched] Kernel task PID {} created (entry={:#018x})",
                    pid, entry as u64
                );
                return Some(pid);
            }
        }
    }

    println!("[sched] No free task slots!");
    None
}

/// Create a user-space process (runs at EL0)
pub fn create_user_process(entry: u64, ttbr0: u64, priority: Priority) -> Option<Pid> {
    let (kern_sp, kern_base) = alloc_kernel_stack()?;
    let (user_sp, user_base) = alloc_user_stack()?;

    let pid = unsafe {
        let pid = NEXT_PID;
        NEXT_PID += 1;
        pid
    };

    let mut regs = TrapFrame::new();
    regs.elr = entry;
    regs.sp = user_sp;
    // SPSR: EL0t, all interrupts unmasked, AArch64
    // M[3:0] = 0000 (EL0t), DAIF = 0000
    regs.spsr = 0x000;
    regs.x30 = entry; // LR points to entry

    let task = Task {
        pid,
        parent_pid: current_pid(),
        pgid: 0,
        sid: 0,
        uid: 0,
        euid: 0,
        gid: 0,
        egid: 0,
        groups: [0; 4],
        group_count: 0,
        state: TaskState::Ready,
        priority,
        affinity: 0xF,
        cpu_id: 0,
        runtime_ticks: 0,
        time_slice: time_slice_for(priority),
        nice: 0,
        regs,
        kernel_sp: kern_sp,
        kernel_stack_base: kern_base,
        user_sp,
        user_stack_base: user_base,
        ttbr0_phys: ttbr0,
        entry_point: entry,
        is_user: true,
        ipc_ns: 0, // global namespace
        pending_signals: 0,
        signal_mask: 0,
        signal_handlers: [0; SIG_COUNT],
        pipe_read_fd: -1,
        pipe_write_fd: -1,
        exit_code: 0,
        exit_signal: 0,
    };

    unsafe {
        for i in 0..MAX_TASKS {
            if TASK_TABLE[i].is_none() {
                TASK_TABLE[i] = Some(task);
                // Initialize credentials for this user process
                crate::security::init_credentials(i, false);
                println!(
                    "[sched] User process PID {} created (entry={:#018x}, ttbr0={:#010x})",
                    pid, entry, ttbr0
                );
                return Some(pid);
            }
        }
    }

    None
}

/// Time slice in ticks based on priority
pub fn time_slice_for(priority: Priority) -> u64 {
    match priority {
        Priority::Low => 5,     // 50ms
        Priority::Normal => 10, // 100ms
        Priority::High => 20,   // 200ms
    }
}

/// Spawn a user process from an ELF binary loaded via the ELF loader.
///
/// This ties `elf::load_elf()` → `create_user_process()` together,
/// providing the canonical path to launch a user-space program from
/// either the kernel boot sequence or a shell command.
///
/// Returns the new PID on success, or None on failure.
pub fn spawn_user_process(elf_data: &[u8], priority: Priority) -> Option<Pid> {
    // 1. Load the ELF into a fresh address space
    let load_result = elf::load_elf(elf_data).ok()?;

    // 2. Check if the ELF is dynamically linked and needs relocation
    let load_base = elf::load_base_for(elf_data);
    let entry_point = if elf::is_dynamic(elf_data) {
        // Attempt dynamic linking; on failure, fall back to raw entry
        match dlinker::link_elf(
            elf_data,
            load_base,
            load_result.ttbr0_phys,
            load_result.entry_point,
        ) {
            Ok(dyn_result) => {
                println!(
                    "[sched] Dynamic link OK: {} RELA, {} PLT, got={:#018x}",
                    dyn_result.rela_count, dyn_result.plt_count, dyn_result.got_addr
                );
                dyn_result.entry_point
            }
            Err(e) => {
                println!("[sched] Dynamic link failed: {}, using raw entry", e);
                load_result.entry_point
            }
        }
    } else {
        load_result.entry_point
    };

    // 3. Create a user-space task with the (possibly rebased) entry point + TTBR0
    let pid = create_user_process(entry_point, load_result.ttbr0_phys, priority)?;

    println!(
        "[sched] spawn_user_process: PID {} entry={:#018x} ttbr0={:#010x} brk={:#018x}",
        pid, entry_point, load_result.ttbr0_phys, load_result.brk_base
    );

    Some(pid)
}

/// Spawn a user process from a file path in the VFS.
///
/// Reads the ELF binary from VFS, loads it, and creates a user task.
/// Returns the new PID on success, or None on failure.
pub fn spawn_from_path(path: &str, priority: Priority) -> Option<Pid> {
    let elf_data = crate::fs::vfs::read_file(path).ok()?;

    if elf_data.is_empty() {
        println!("[sched] spawn_from_path: empty file {:?}", path);
        return None;
    }

    let pid = spawn_user_process(&elf_data, priority)?;

    println!("[sched] spawn_from_path: {:?} → PID {}", path, pid);
    Some(pid)
}

/// Spawn /sbin/init as the first user-space process (PID 1).
///
/// Called during kernel boot after all subsystems are initialized.
/// If /sbin/init does not exist in the VFS, falls back to the
/// built-in hello-EL0 demo so EL0 transitions are still exercised.
/// Returns the new PID on success, or None on failure.
pub fn spawn_init() -> Option<Pid> {
    // Try the real init first
    let elf_data = crate::fs::vfs::read_file("/sbin/init").ok();
    if let Some(data) = elf_data {
        if !data.is_empty() {
            let pid = spawn_user_process(&data, Priority::High)?;
            println!("[sched] spawn_init: /sbin/init → PID {}", pid);
            return Some(pid);
        }
    }

    // Fallback: use the built-in hello-EL0 demo as init
    println!("[sched] spawn_init: /sbin/init not found, using hello-EL0 demo");
    let demo_data = elf::build_hello_elf();
    let pid = spawn_user_process(demo_data, Priority::High)?;
    println!("[sched] spawn_init: demo init → PID {}", pid);
    Some(pid)
}

/// Spawn the built-in hello-EL0 demo as a user process.
///
/// Uses the hand-crafted ELF binary in `elf::build_hello_elf()`.
/// Returns the new PID on success, or None on failure.
pub fn spawn_demo() -> Option<Pid> {
    let elf_data = elf::build_hello_elf();
    let pid = spawn_user_process(elf_data, Priority::Normal)?;

    println!("[sched] spawn_demo: hello-EL0 → PID {}", pid);
    Some(pid)
}

/// Find the next ready task to run (round-robin with priority boost)
/// On SMP systems, prefers tasks on the current core's runqueue.
fn pick_next_task() -> Option<usize> {
    let core_id = crate::smp::current_core_id() as usize;

    // First try: pick from the SMP per-core runqueue
    if let Some(pid) = crate::smp::runqueue::pick_next_task(core_id) {
        if pid != 0 {
            if let Some(idx) = find_task_index(pid) {
                return Some(idx);
            }
        }
    }

    // Fallback: global task table scan (for core 0 or when runqueue is empty)
    unsafe {
        let current_idx = find_task_index(CURRENT_PID).unwrap_or(0);
        let start = (current_idx + 1) % MAX_TASKS;

        // First pass: high priority (affinity check)
        let mut idx = start;
        for _ in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[idx] {
                if task.state == TaskState::Ready
                    && task.priority == Priority::High
                    && (task.affinity & (1 << core_id)) != 0
                {
                    return Some(idx);
                }
            }
            idx = (idx + 1) % MAX_TASKS;
        }

        // Second pass: normal priority (affinity check)
        idx = start;
        for _ in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[idx] {
                if task.state == TaskState::Ready
                    && task.priority == Priority::Normal
                    && (task.affinity & (1 << core_id)) != 0
                {
                    return Some(idx);
                }
            }
            idx = (idx + 1) % MAX_TASKS;
        }

        // Third pass: low priority (affinity check)
        idx = start;
        for _ in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[idx] {
                if task.state == TaskState::Ready
                    && task.priority == Priority::Low
                    && (task.affinity & (1 << core_id)) != 0
                {
                    return Some(idx);
                }
            }
            idx = (idx + 1) % MAX_TASKS;
        }

        // Final pass: any ready task regardless of affinity (migration)
        idx = start;
        for _ in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[idx] {
                if task.state == TaskState::Ready {
                    return Some(idx);
                }
            }
            idx = (idx + 1) % MAX_TASKS;
        }

        None
    }
}

/// Find task index by PID
pub fn find_task_index(pid: Pid) -> Option<usize> {
    unsafe {
        for i in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[i] {
                if task.pid == pid {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Perform a context switch from old task to new task
///
/// This is the core of the scheduler. It saves the current context,
/// picks the next task, and restores its context.
///
/// # Safety
/// Must be called from interrupt context with IRQs disabled.
pub unsafe fn do_context_switch() {
    // Save current task context
    if CURRENT_PID != 0 {
        if let Some(idx) = find_task_index(CURRENT_PID) {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                // Save current register state
                // (In a full implementation, registers are saved on the kernel stack
                // by the exception handler. For simplicity, we save key registers.)
                save_current_context(&mut task.regs);

                if task.state == TaskState::Running {
                    task.state = TaskState::Ready;
                }
            }
        }
    }

    // Pick next task
    if let Some(next_idx) = pick_next_task() {
        if let Some(ref mut task) = TASK_TABLE[next_idx] {
            let _old_pid = CURRENT_PID;
            CURRENT_PID = task.pid;
            task.state = TaskState::Running;
            task.time_slice = time_slice_for(task.priority);

            // Switch address space if needed (user process)
            if task.is_user && task.ttbr0_phys != 0 {
                switch_address_space(task.ttbr0_phys);
            }

            // Restore context of new task
            restore_context(&task.regs);
        }
    }
    // If no ready task, current task continues
}

/// Save current execution context to a TrapFrame
/// Reads the actual CPU registers
unsafe fn save_current_context(frame: &mut TrapFrame) {
    asm!(
        "mov {0}, x19", "mov {1}, x20", "mov {2}, x21", "mov {3}, x22",
        "mov {4}, x23", "mov {5}, x24", "mov {6}, x25", "mov {7}, x26",
        "mov {8}, x27", "mov {9}, x28", "mov {10}, x29", "mov {11}, x30",
        out(reg) frame.x19, out(reg) frame.x20,
        out(reg) frame.x21, out(reg) frame.x22,
        out(reg) frame.x23, out(reg) frame.x24,
        out(reg) frame.x25, out(reg) frame.x26,
        out(reg) frame.x27, out(reg) frame.x28,
        out(reg) frame.x29, out(reg) frame.x30,
    );
    asm!("mov {}, sp", out(reg) frame.sp);
    asm!("mrs {}, ELR_EL1", out(reg) frame.elr);
    asm!("mrs {}, SPSR_EL1", out(reg) frame.spsr);
}

/// Restore execution context from a TrapFrame
/// Actually loads registers and does eret
pub unsafe fn restore_context(frame: &TrapFrame) {
    asm!(
        "mov x19, {0}", "mov x20, {1}", "mov x21, {2}", "mov x22, {3}",
        "mov x23, {4}", "mov x24, {5}", "mov x25, {6}", "mov x26, {7}",
        "mov x27, {8}", "mov x28, {9}", "mov x29, {10}", "mov x30, {11}",
        in(reg) frame.x19, in(reg) frame.x20,
        in(reg) frame.x21, in(reg) frame.x22,
        in(reg) frame.x23, in(reg) frame.x24,
        in(reg) frame.x25, in(reg) frame.x26,
        in(reg) frame.x27, in(reg) frame.x28,
        in(reg) frame.x29, in(reg) frame.x30,
    );
    asm!("msr ELR_EL1, {}", in(reg) frame.elr);
    asm!("msr SPSR_EL1, {}", in(reg) frame.spsr);
    asm!("mov sp, {}", in(reg) frame.sp);
    asm!("eret");
}

/// Switch to a user process address space
unsafe fn switch_address_space(ttbr0_phys: u64) {
    // Set TTBR0_EL1 to the process's page table root
    asm!("msr TTBR0_EL1, {}", in(reg) ttbr0_phys);
    asm!("tlbi vmalle1is");
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Timer tick handler — called from timer IRQ
/// Decrements time slice and triggers context switch when expired
pub fn on_timer_tick() {
    unsafe {
        TICK_COUNT += 1;
    }

    if !is_enabled() {
        return;
    }

    // Decrement time slice of current task
    if let Some(idx) = find_task_index(current_pid()) {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.runtime_ticks += 1;
                if task.time_slice > 0 {
                    task.time_slice -= 1;
                }
            }
        }
    }

    // Check if time slice expired
    let should_switch = unsafe {
        if let Some(idx) = find_task_index(current_pid()) {
            if let Some(ref task) = TASK_TABLE[idx] {
                task.time_slice == 0
            } else {
                false
            }
        } else {
            false
        }
    };

    // Check DHCP lease timers for renewal/rebinding
    dhcp::lease_tick();

    if should_switch {
        // Request a context switch (will happen on exception return)
        request_schedule();
    }
}

/// Flag to request a schedule on next exception return
static mut NEED_SCHEDULE: bool = false;

/// Request a schedule (sets flag)
pub fn request_schedule() {
    unsafe {
        NEED_SCHEDULE = true;
    }
}

/// Check if a schedule is pending
pub fn schedule_pending() -> bool {
    unsafe { NEED_SCHEDULE }
}

/// Clear schedule flag
pub fn clear_schedule_flag() {
    unsafe {
        NEED_SCHEDULE = false;
    }
}

/// Enable the scheduler and start running tasks
pub fn enable() {
    unsafe {
        SCHEDULER_ENABLED = true;
    }
    println!("[sched] Preemptive scheduling enabled (tick={}ms)", TICK_MS);
}

/// Schedule — the main entry point for context switching
/// Called from the timer IRQ handler when the time slice expires
pub fn schedule() {
    if !is_enabled() {
        return;
    }
    unsafe {
        do_context_switch();
    }
}

/// Yield the current task (cooperative)
pub fn yield_task() {
    if let Some(idx) = find_task_index(current_pid()) {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.time_slice = 0; // Force reschedule
                task.state = TaskState::Ready;
            }
        }
    }
    request_schedule();
}

/// Block the current task
pub fn block_current(reason: &'static str) {
    if let Some(idx) = find_task_index(current_pid()) {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.state = TaskState::Blocked;
                println!("[sched] PID {} blocked: {}", task.pid, reason);
            }
        }
    }
    request_schedule();
}

/// Unblock a task by PID
pub fn unblock(pid: Pid) {
    if let Some(idx) = find_task_index(pid) {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                if task.state == TaskState::Blocked {
                    task.state = TaskState::Ready;
                }
            }
        }
    }
}

/// Exit the current task
pub fn exit_current(code: i64) -> ! {
    let pid = current_pid();
    if let Some(idx) = find_task_index(pid) {
        // Release IPC namespace refcount
        let ns_id = unsafe { TASK_TABLE[idx].as_ref().map(|t| t.ipc_ns).unwrap_or(0) };
        crate::ipc::ipc_ns_release(ns_id);

        // Close all fds
        crate::ipc::close_all_fds(idx);

        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.state = TaskState::Zombie;
                task.exit_code = code;
                println!("[sched] PID {} exited with code {}", pid, code);
            }
        }
    }
    // Force immediate schedule — we never return
    loop {
        crate::arch::cpu::wfe();
    }
}

/// Kill a task by PID
pub fn kill(pid: Pid) -> bool {
    if let Some(idx) = find_task_index(pid) {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.state = TaskState::Zombie;
                task.exit_code = -9; // SIGKILL equivalent
                println!("[sched] PID {} killed", pid);
                return true;
            }
        }
    }
    false
}

/// Reap zombie tasks (free their resources)
pub fn reap_zombies() -> usize {
    let mut reaped = 0;
    unsafe {
        for i in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[i] {
                if task.state == TaskState::Zombie {
                    // Free kernel stack
                    for page in 0..KERN_STACK_PAGES {
                        let offset = (page as u64) * PAGE_SIZE;
                        let vaddr = task.kernel_stack_base + offset;
                        if let Some(paddr) = crate::memory::vmem::translate_va(VirtAddr::new(vaddr))
                        {
                            frame_alloc::deallocate_frame(paddr);
                        }
                    }
                    // Free user stack
                    if task.user_stack_base != 0 {
                        for page in 0..USER_STACK_PAGES {
                            let offset = (page as u64) * PAGE_SIZE;
                            let vaddr = task.user_stack_base + offset;
                            if let Some(paddr) =
                                crate::memory::vmem::translate_va(VirtAddr::new(vaddr))
                            {
                                frame_alloc::deallocate_frame(paddr);
                            }
                        }
                    }
                    TASK_TABLE[i] = None;
                    reaped += 1;
                }
            }
        }
    }
    reaped
}

/// Print scheduler status
pub fn print_status() {
    println!("[sched] === Scheduler Status ===");
    println!("[sched] Current PID: {}", current_pid());
    println!("[sched] Tick count: {}", tick_count());
    println!("[sched] Enabled: {}", is_enabled());

    unsafe {
        let mut ready = 0u64;
        let mut running = 0u64;
        let mut blocked = 0u64;
        let mut zombie = 0u64;

        for i in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[i] {
                match task.state {
                    TaskState::Ready => ready += 1,
                    TaskState::Running => running += 1,
                    TaskState::Blocked => blocked += 1,
                    TaskState::Zombie => zombie += 1,
                    _ => {}
                }
                let kind = if task.is_user { "EL0" } else { "EL1" };
                println!(
                    "[sched]   PID {} | {} | {:?} | {:?} | ticks={} slice={}",
                    task.pid, kind, task.state, task.priority, task.runtime_ticks, task.time_slice
                );
            }
        }
        println!(
            "[sched] Tasks: {} ready, {} running, {} blocked, {} zombie",
            ready, running, blocked, zombie
        );
    }
}

/// Get task count
pub fn task_count() -> usize {
    unsafe { TASK_TABLE.iter().filter(|t| t.is_some()).count() }
}
