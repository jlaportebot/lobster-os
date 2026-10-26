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
pub mod shared_lib;
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
    // Special registers — exactly matching the 272-byte boot.S frame:
    // x0..x30 (31 regs) + sp + elr + spsr = 34 * 8 = 272 bytes.
    // ESR is deliberately NOT here: including it grew the struct to 280 bytes and
    // sigreturn/restore wrote 8 bytes past the asm frame (audit Q1c).
    pub sp: u64,
    pub elr: u64,  // Exception Link Register (return address)
    pub spsr: u64, // Saved Program Status Register
}

const _: () = {
    assert!(
        core::mem::size_of::<TrapFrame>() == 272,
        "TrapFrame must match the 272-byte asm exception frame"
    );
    assert!(
        core::mem::offset_of!(TrapFrame, elr) == 256,
        "ELR must live at frame offset 256 (boot.S SAVE/RESTORE)"
    );
    assert!(
        core::mem::offset_of!(TrapFrame, spsr) == 264,
        "SPSR must live at frame offset 264 (boot.S SAVE/RESTORE)"
    );
    assert!(
        core::mem::offset_of!(TrapFrame, sp) == 248,
        "SP must live at frame offset 248"
    );
};

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
    /// PID namespace ID (0 = global)
    pub pid_ns: u32,
    /// Mount namespace ID (0 = global)
    pub mnt_ns: u32,
    /// Network namespace ID (0 = global)
    pub net_ns: u32,
    /// UTS namespace ID (0 = global)
    pub uts_ns: u32,
    /// User namespace ID (0 = global)
    pub user_ns: u32,
    /// Pipe read-end file descriptor (or -1)
    pub pipe_read_fd: i64,
    /// Pipe write-end file descriptor (or -1)
    pub pipe_write_fd: i64,

    // ── Exit ──
    /// Exit code (set when task becomes Zombie)
    pub exit_code: i64,
    /// Signal that caused termination (0 if not killed by signal)
    pub exit_signal: u64,

    // ── Sliced cooperative multitasking (S2) ──
    /// Background EL0 task run via the sliced scheduler (spawned with `&`).
    pub bg: bool,
    /// Absolute uptime_ms when a sliced task's SYS_SLEEP ends (0 = none).
    pub wake_ms: u64,
    /// Round-robin ticket: order this task was last scheduled in.
    pub rr_ticket: u64,
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

/// Rough CPU load estimate (0..100): fraction of scheduler tasks that are
/// actively Running (vs Ready/Blocked) across the table, scaled to feel
/// right for the panel graph.
pub fn current_cpu_load() -> u32 {
    unsafe {
        let mut running = 0u32;
        let mut total = 0u32;
        for i in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[i] {
                total += 1;
                if task.state == TaskState::Running {
                    running += 1;
                }
            }
        }
        if total == 0 {
            return 0;
        }
        (running * 100 / total).min(100)
    }
}

/// Get current PID
pub fn current_pid() -> Pid {
    unsafe { CURRENT_PID }
}

/// Set the current PID (used by cooperative exit to reset to 0)
pub fn set_current_pid(pid: Pid) {
    unsafe {
        CURRENT_PID = pid;
    }
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

/// Allocate a user stack.
///
/// The ELF loader already maps a 1 MB user stack at 0x7ef00000..0x7f000000
/// in the user's page table. We just need to return the top of that range
/// as the initial stack pointer.
fn alloc_user_stack() -> Option<(u64, u64)> {
    const USER_STACK_TOP: u64 = 0x7f000000; // exclusive top
    const USER_STACK_BASE: u64 = 0x7ef00000; // inclusive base
    Some((USER_STACK_TOP, USER_STACK_BASE))
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
        // Poll WiFi data frames and process network events
        if crate::drivers::wifi::is_up() {
            crate::drivers::wifi::poll();
            // Process any received WiFi data frames through the network stack
            let mut buf = [0u8; 1536];
            if crate::net::iface::receive(
                crate::net::iface::find_by_type(crate::net::iface::IfaceType::Wifi).unwrap_or(0),
                &mut buf,
            ) > 0
            {
                // Frame received - process it
                if let Some(frame) = crate::net::eth::EthernetFrame::parse(&buf) {
                    crate::net::process_frame(frame, &buf);
                }
            }
        }
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
        ipc_ns: 0,  // global namespace
        pid_ns: 0,  // global namespace
        mnt_ns: 0,  // global namespace
        net_ns: 0,  // global namespace
        uts_ns: 0,  // global namespace
        user_ns: 0, // global namespace
        pending_signals: 0,
        signal_mask: 0,
        signal_handlers: [0; SIG_COUNT],
        pipe_read_fd: -1,
        pipe_write_fd: -1,
        exit_code: 0,
        exit_signal: 0,
        bg: false,
        wake_ms: 0,
        rr_ticket: 0,
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
        ipc_ns: 0,  // global namespace
        pid_ns: 0,  // global namespace
        mnt_ns: 0,  // global namespace
        net_ns: 0,  // global namespace
        uts_ns: 0,  // global namespace
        user_ns: 0, // global namespace
        pending_signals: 0,
        signal_mask: 0,
        signal_handlers: [0; SIG_COUNT],
        pipe_read_fd: -1,
        pipe_write_fd: -1,
        exit_code: 0,
        exit_signal: 0,
        bg: false,
        wake_ms: 0,
        rr_ticket: 0,
    };

    unsafe {
        for i in 0..MAX_TASKS {
            if TASK_TABLE[i].is_none() {
                TASK_TABLE[i] = Some(task);
                // Initialize credentials for this user process
                crate::security::init_credentials(i, false);
                // Add to SMP runqueue so pick_next_task can find it
                let core_id = crate::smp::current_core_id() as usize;
                crate::smp::runqueue::enqueue_task(core_id, pid);
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

/// Write an argv block onto the top of a freshly-created user stack and
/// point x0/x1 at it (AArch64 process-entry convention used by our
/// MicroPython: w0 = argc, x1 = char** argv).
/// Layout pushed at the stack top (grows down):
///   [char* argv[argc]][NULL][string bytes...\0 per arg]
/// Returns (argc, argv_ptr) on success.
unsafe fn write_argv_block(user_sp: u64, args: &[&str]) -> Option<(u64, u64, u64)> {
    const MAX_ARGV_BYTES: usize = 4096;
    let argc = args.len() as u64;
    let strings_len: usize = args.iter().map(|a| a.len() + 1).sum();
    let array_len = (args.len() + 1) * 8; // argv[] + NULL terminator
    let total = array_len + strings_len;
    if total > MAX_ARGV_BYTES {
        return None;
    }
    // Layout (addresses grow up): [argv array + NULL][string bytes...]
    // SP lands ABOVE the strings so the program's own stack pushes (which
    // grow DOWN from SP) go below the array, never over the strings.
    let block_top = user_sp & !0xF;
    let block_base = block_top - total as u64;
    let array_ptr = block_base; // array at the bottom, already 8-aligned
    let strings_base = block_base + array_len as u64;
    let mut p = strings_base;
    let mut ptrs: alloc::vec::Vec<u64> = alloc::vec::Vec::new();
    for a in args {
        let sptr = p;
        for &b in a.as_bytes() {
            core::ptr::write_volatile(p as *mut u8, b);
            p += 1;
        }
        core::ptr::write_volatile(p as *mut u8, 0);
        p += 1;
        ptrs.push(sptr);
    }
    let mut ap = array_ptr;
    for ptr in &ptrs {
        core::ptr::write_volatile(ap as *mut u64, *ptr);
        ap += 8;
    }
    core::ptr::write_volatile(ap as *mut u64, 0); // NULL terminator
    Some((argc, array_ptr, block_base))
}

/// Set the entry registers for argv (call before the task first runs).
/// Mirrors the MicroPython entry convention: w0 = argc, x1 = argv.
pub fn set_task_argv(pid: Pid, args: &[&str]) -> bool {
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return false,
    };
    unsafe {
        let (user_sp, ttbr0) = match &TASK_TABLE[idx] {
            Some(t) if t.is_user => (t.user_sp, t.ttbr0_phys),
            _ => return false,
        };
        if ttbr0 == 0 {
            return false;
        }
        // User pages are dual-mapped into the kernel address space at
        // ELF load time, so the kernel can write the block directly.
        let Some((argc, argv_ptr, block_base)) = write_argv_block(user_sp, args) else {
            return false;
        };
        if let Some(ref mut t) = TASK_TABLE[idx] {
            t.regs.x0 = argc;
            t.regs.x1 = argv_ptr;
            // SP above the whole block (strings end); the program's stack
            // pushes grow down into fresh stack, not over argv.
            // SP sits BELOW the whole argv block: the program's stack
            // pushes grow down from SP and can never overwrite argv
            // (the classic process-entry layout: argv just above SP).
            t.regs.sp = block_base & !0xF;
        }
        true
    }
}

/// Spawn a user process from a file path in the VFS with argv.
/// `args` are the program's arguments AFTER the program name; argv[0]
/// is set to `path` per convention (argc = args.len() + 1).
pub fn spawn_from_path_args(path: &str, priority: Priority, args: &[&str]) -> Option<Pid> {
    let pid = spawn_from_path(path, priority)?;
    if !args.is_empty() {
        // Build [path, args...] — bounded to keep the block small.
        let mut full: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
        full.push(path);
        for a in args {
            full.push(a);
        }
        if !set_task_argv(pid, &full) {
            // argv is optional; a failure to write it degrades to no-args
            crate::println!("[sched] warning: argv block failed for {}", path);
        }
    }
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
    let pid = spawn_user_process(elf_data, Priority::High)?;

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
/// Loads all registers from memory and does eret.
/// Saves ptr to stack first to avoid it being clobbered by ldr.
pub unsafe fn restore_context(frame: &TrapFrame) {
    let ptr = frame as *const TrapFrame as *const u8;
    // Debug: print key values before eret (suppressed to reduce serial spam)
    // crate::println!(
    //     "[sched] restore_context: entry={:#018x} sp={:#018x} spsr={:#018x}",
    //     frame.elr, frame.sp, frame.spsr
    // );
    asm!(
        "sub sp, sp, #16",
        "str {p}, [sp]",
        // Load x0-x15
        "ldr x0, [{p}, #0]",
        "ldr x1, [{p}, #8]",
        "ldr x2, [{p}, #16]",
        "ldr x3, [{p}, #24]",
        "ldr x4, [{p}, #32]",
        "ldr x5, [{p}, #40]",
        "ldr x6, [{p}, #48]",
        "ldr x7, [{p}, #56]",
        "ldr x8, [{p}, #64]",
        "ldr x9, [{p}, #72]",
        "ldr x10, [{p}, #80]",
        "ldr x11, [{p}, #88]",
        "ldr x12, [{p}, #96]",
        "ldr x13, [{p}, #104]",
        "ldr x14, [{p}, #112]",
        "ldr x15, [{p}, #120]",
        // Load x16-x30
        "ldr x16, [{p}, #128]",
        "ldr x17, [{p}, #136]",
        "ldr x18, [{p}, #144]",
        "ldr x19, [{p}, #152]",
        "ldr x20, [{p}, #160]",
        "ldr x21, [{p}, #168]",
        "ldr x22, [{p}, #176]",
        "ldr x23, [{p}, #184]",
        "ldr x24, [{p}, #192]",
        "ldr x25, [{p}, #200]",
        "ldr x26, [{p}, #208]",
        "ldr x27, [{p}, #216]",
        "ldr x28, [{p}, #224]",
        "ldr x29, [{p}, #232]",
        "ldr x30, [{p}, #240]",
        // Now reload ptr from stack to a scratch reg for SP/ELR/SPSR
        "ldr x16, [sp]",            // x16 = saved ptr
        "ldr x17, [x16, #248]",     // sp (user stack)
        // Workaround: QEMU doesn't implement direct msr SP_EL0 at EL1.
        // Switch SP bank to SP_EL0, write SP, switch back to SP_EL1.
        // SPSel controls which SP bank is active.
        "mov x18, #0",              // x18 = 0 (select SP_EL0)
        "msr SPSel, x18",           // SP = SP_EL0 bank
        "mov sp, x17",              // Write user SP to SP_EL0
        "mov x18, #1",              // x18 = 1 (select SP_EL1)
        "msr SPSel, x18",           // SP = SP_EL1 bank (restore kernel SP)
        "ldr x17, [x16, #256]",     // elr
        "msr ELR_EL1, x17",
        "ldr x17, [x16, #264]",     // spsr
        "msr SPSR_EL1, x17",
        "add sp, sp, #16",
        "eret",
        p = in(reg) ptr,
        options(noreturn),
    );
}

/// Switch to a user process address space
unsafe fn switch_address_space(ttbr0_phys: u64) {
    // Set TTBR0_EL1 to the process's page table root
    asm!("msr TTBR0_EL1, {}", in(reg) ttbr0_phys);
    // Barrier only — no TLB invalidation needed for first context switch
    // since the kernel uses TTBR1 for its own pages
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
    // Check if time slice expired OR if current PID has no task table entry
    // (e.g. CURRENT_PID=0 before idle task runs) but ready tasks exist
    let should_switch = unsafe {
        if let Some(idx) = find_task_index(current_pid()) {
            // Current task exists in table — check timeslice
            if let Some(ref task) = TASK_TABLE[idx] {
                task.time_slice == 0
            } else {
                false
            }
        } else {
            // Current PID not in task table — switch if any ready task exists
            let mut found_ready = false;
            for i in 0..MAX_TASKS {
                if let Some(ref task) = TASK_TABLE[i] {
                    if task.state == TaskState::Ready {
                        found_ready = true;
                        break;
                    }
                }
            }
            found_ready
        }
    };

    // Check DHCP lease timers for renewal/rebinding
    dhcp::lease_tick();

    if should_switch {
        // Request a context switch (will happen on exception return)
        // In QEMU cooperative mode, schedule() is never called from IRQ handler,
        // so this just sets the flag. Don't spam serial with debug output.
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

/// Set when a user process exits and we want to return to cooperative mode
/// instead of looping forever in exit_current.
static mut COOPERATIVE_EXIT_REQUESTED: bool = false;

/// Saved kernel return address — when a user process exits in cooperative mode,
/// the exception handler sets ELR_EL1 to this and SPSR_EL1 to EL1h so eret
/// returns to the kernel code that called run_user_process().
static mut KERNEL_RETURN_ADDR: u64 = 0;

/// True while a cooperative run_and_wait trampoline owns the current
/// task — used by syscalls (e.g. sleep) to run in "blocking" style
/// while keeping the rest of the system alive.
pub fn coop_active() -> bool {
    unsafe { KERNEL_RETURN_ADDR != 0 }
}

/// Exit code of the most recently cooperatively-exited user process —
/// the exception handler copies it into the trap frame's x0 so the resume
/// trampoline returns it to run_and_wait's caller.
static mut COOP_EXIT_CODE: i64 = 0;

/// Saved kernel SP at the point run_and_wait was entered — the cooperative
/// exit path restores it so eret resumes the caller with a sane stack.
static mut KERNEL_SP: u64 = 0;

/// Saved kernel callee-saved registers (x19-x30) at run_and_wait entry.
/// eret-into-user clobbers all GPRs; the cooperative exit must restore these
/// before resuming the caller, or the caller branches through garbage.
/// (Consumed by the run_and_wait trampoline in the function below.)
///
/// Run a user process synchronously in cooperative mode.
///
/// This function:
/// 1. Saves its own return address (so we can come back after the process exits)
/// 2. Sets CURRENT_PID to the spawned process
/// 3. Restores the process's trap frame and does eret into EL0
/// 4. The user process runs, calls syscalls, eventually calls sys_exit
/// 5. The exception handler sees cooperative_exit_pending, sets SPSR=EL1h
/// 6. eret returns to the saved kernel return address — this function returns
///
/// Returns the exit code of the user process.
/// Entry trampoline for cooperative run-and-wait.
///
/// Saves the caller's callee-saved registers (x19-x30), SP and LR into
/// statics, then tail-calls the body. When the user process exits, the
/// exception handler performs eret back to `.Lcoop_resume` with the same
/// statics restored, so the caller sees a normal function return with the
/// exit code in x0.
///
/// # Safety
/// Called from kernel context at EL1h only.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub unsafe extern "C" fn run_and_wait(_pid: u64) -> i64 {
    core::arch::naked_asm!(
        // Stash callee-saved regs + lr into statics.
        "ldr x18, =KERNEL_CALLEE_SAVE",
        "stp x19, x20, [x18, #0]",
        "stp x21, x22, [x18, #16]",
        "stp x23, x24, [x18, #32]",
        "stp x25, x26, [x18, #48]",
        "stp x27, x28, [x18, #64]",
        "stp x29, x30, [x18, #80]",
        "mov x1, sp",
        "str x1, [x18, #96]", // save SP
        "bl run_and_wait_body",
        // Normal return path (body found no task etc.): restore and return.
        "ldr x18, =KERNEL_CALLEE_SAVE",
        "ldp x19, x20, [x18, #0]",
        "ldp x21, x22, [x18, #16]",
        "ldp x23, x24, [x18, #32]",
        "ldp x25, x26, [x18, #48]",
        "ldp x27, x28, [x18, #64]",
        "ldp x29, x30, [x18, #80]",
        "ret",
        // Cooperative-exit resume target: exception handler erets here with
        // exit code in x0. Restore regs/SP and return to the caller.
        ".global COOP_RESUME_ADDR",
        "COOP_RESUME_ADDR:",
        "ldr x18, =KERNEL_CALLEE_SAVE",
        "ldp x19, x20, [x18, #0]",
        "ldp x21, x22, [x18, #16]",
        "ldp x23, x24, [x18, #32]",
        "ldp x25, x26, [x18, #48]",
        "ldp x27, x28, [x18, #64]",
        "ldp x29, x30, [x18, #80]",
        "ldr x1, [x18, #96]",
        "mov sp, x1",
        "ret",
        ".align 3",
        "KERNEL_CALLEE_SAVE:",
        ".zero 104",
    )
}

/// Address of the cooperative-exit resume label (set at link time via
/// a small init that stores the label — done by `coop_resume_addr()`).
static mut COOP_RESUME: u64 = 0;

/// Body of run_and_wait: spawns nothing — the caller spawned the task; we
/// just eret into it and (on exit) the exception handler resumes the
/// trampoline. Returns -1 if the task vanished before we could enter it.
#[no_mangle]
pub unsafe extern "C" fn run_and_wait_body(_pid: u64) -> i64 {
    let pid = _pid as Pid;
    let idx = match find_task_index(pid) {
        Some(i) => i,
        None => return -1,
    };

    println!("[sched] run_and_wait: entered for PID {}", pid);

    // Record the resume address for the exception handler once.
    unsafe {
        if COOP_RESUME == 0 {
            COOP_RESUME = coop_resume_label();
        }
        KERNEL_RETURN_ADDR = COOP_RESUME;
        let sp: u64;
        core::arch::asm!("mov {}, sp", out(reg) sp);
        KERNEL_SP = sp;
    }

    unsafe {
        CURRENT_PID = pid;
    }

    unsafe {
        if let Some(ref mut task) = TASK_TABLE[idx] {
            task.state = TaskState::Running;
            task.time_slice = time_slice_for(task.priority);

            // User pages are dual-mapped into kernel TTBR0 at ELF load time,
            // so no TTBR0 switch is needed. eret into EL0 will work with the
            // kernel's page table still active.
            let frame = task.regs;

            // Restore context — this eret's into EL0 and never returns.
            // When the process exits, the exception handler erets to the
            // trampoline's resume label with the exit code in x0.
            restore_context(&frame);
        }
    }

    // Unreachable — restore_context does eret (noreturn).
    -1
}

/// Resolve the resume label address (asm helper).
#[cfg(target_arch = "aarch64")]
fn coop_resume_label() -> u64 {
    let addr: u64;
    unsafe {
        core::arch::asm!(
            "ldr {0}, =COOP_RESUME_ADDR",
            out(reg) addr,
        );
    }
    addr
}

/// Get the saved kernel return address (called from exception handler)
pub fn kernel_return_address() -> u64 {
    unsafe { KERNEL_RETURN_ADDR }
}

/// Get the saved kernel stack pointer (called from exception handler)
pub fn kernel_sp() -> u64 {
    unsafe { KERNEL_SP }
}

/// Restore saved kernel callee-saved registers (x19-x30) — kept for API
/// compatibility; the naked trampoline restores them in asm now.
pub unsafe fn restore_kernel_regs(_regs: *mut u64) {}

/// Check if a cooperative exit was requested (called from exception return path)
pub fn cooperative_exit_pending() -> bool {
    unsafe { COOPERATIVE_EXIT_REQUESTED }
}

/// Record the exit code to deliver through the resume trampoline (x0).
pub fn set_coop_exit_code(code: i64) {
    unsafe {
        COOP_EXIT_CODE = code;
    }
}

/// Read the recorded exit code.
pub fn coop_exit_code() -> i64 {
    unsafe { COOP_EXIT_CODE }
}

/// Clear the cooperative exit flag
pub fn clear_cooperative_exit() {
    unsafe {
        COOPERATIVE_EXIT_REQUESTED = false;
    }
}

/// Mark that we want cooperative exit — the next exception return should
/// go back to EL1 kernel mode instead of EL0.
pub fn request_cooperative_exit() {
    unsafe {
        COOPERATIVE_EXIT_REQUESTED = true;
    }
}

// ── Sliced cooperative multitasking (S2) ─────────────────────────────
//
// Background EL0 apps (spawned with `run <path> &`) never own the CPU
// outright. Each one runs until its next SYS_SLEEP; the sleep records a
// wake deadline and requests a SLICE YIELD — the exception handler erets
// back to the main loop exactly like a cooperative exit, but the task
// stays Ready instead of exiting. The main loop then picks the next
// ready background task whose wake time has passed and runs it.

/// Marker value returned through the resume trampoline when a slice ends
/// (as opposed to a process exit). Distinct from any plausible exit code.
/// 0x5L1CE = 'slice'-ish marker, outside plausible exit-code range.
pub const SLICE_YIELD_MARKER: i64 = 0x5E5_0000;

/// Set when a background task's slice should end at the next exception.
static mut SLICE_YIELD_REQUESTED: bool = false;

/// Set when a cooperative exit should ALSO end the slice loop (task exited).
pub fn request_slice_yield() {
    unsafe {
        SLICE_YIELD_REQUESTED = true;
    }
}

pub fn slice_yield_pending() -> bool {
    unsafe { SLICE_YIELD_REQUESTED }
}

pub fn clear_slice_yield() {
    unsafe {
        SLICE_YIELD_REQUESTED = false;
    }
}

/// True when the current cooperative run should return to the main loop
/// (either the task exited or its slice ended). Called by the exception
/// handler's centralized exit check — SPSR/ELR are already set for EL1
/// return there; we only decide whether to mark it as a slice vs exit.
pub fn slice_mode_active() -> bool {
    // Sliced only while a background task is the current task.
    let pid = unsafe { CURRENT_PID };
    match find_task_index(pid) {
        Some(idx) => unsafe {
            match &TASK_TABLE[idx] {
                Some(t) => t.bg,
                None => false,
            }
        },
        None => false,
    }
}

/// Pick the next background EL0 task to run: earliest wake_ms that has
/// arrived, round-robin among ties. Returns its PID, or None.
pub fn pick_bg_task(now_ms: u64) -> Option<Pid> {
    unsafe {
        let mut best: Option<(usize, u64, u64)> = None; // (idx, wake, ticket)
        for i in 0..MAX_TASKS {
            let Some(ref t) = TASK_TABLE[i] else { continue };
            if !t.bg || !t.is_user {
                continue;
            }
            if t.state != TaskState::Ready {
                continue;
            }
            if t.wake_ms > now_ms {
                continue; // still sleeping
            }
            let key = (t.wake_ms, t.rr_ticket);
            match best {
                Some((_, w, tk)) if (w, tk) <= key => {}
                _ => best = Some((i, t.wake_ms, t.rr_ticket)),
            }
        }
        let (idx, _, _) = best?;
        // Bump the round-robin ticket so others go first next time.
        static mut NEXT_TICKET: u64 = 1;
        if let Some(ref mut t) = TASK_TABLE[idx] {
            t.rr_ticket = NEXT_TICKET;
            NEXT_TICKET += 1;
            Some(t.pid)
        } else {
            None
        }
    }
}

/// Run one slice of every runnable background task (called from the main
/// loop). Returns true if any task ran. A slice ends at the task's next
/// SYS_SLEEP or its exit — both resume the main loop through the
/// cooperative-exit path.
pub fn run_bg_slices() -> bool {
    let now = crate::arch::ArchTimer::uptime_ms();
    let mut ran = false;
    // One task per call keeps main-loop latency bounded; round-robin
    // tickets ensure fairness across calls.
    if let Some(pid) = pick_bg_task(now) {
        let code = unsafe { run_and_wait(pid as u64) };
        if code as u64 != SLICE_YIELD_MARKER as u64 {
            println!("[sched] bg PID {} exited with code {}", pid, code);
        } else if let Some(idx) = find_task_index(pid) {
            // run_and_wait marked it Running; a slice end makes it Ready
            // again (wake_ms gates the next slice).
            unsafe {
                if let Some(ref mut t) = TASK_TABLE[idx] {
                    t.state = TaskState::Ready;
                }
            }
        }
        ran = true;
    }
    ran
}

/// Save the current EL0 trap frame into the task's register context so a
/// later run_and_wait resumes exactly where the slice ended.
/// `regs` is the exception-stack save block (x0-x30 at 0..240, SP_EL0 at
/// 248). `elr`/`spsr` are the exception-time values (SVC ELR already
/// points past the svc instruction).
pub unsafe fn save_slice_frame(pid: Pid, regs: *const u64, elr: u64, spsr: u64) {
    let Some(idx) = find_task_index(pid) else {
        return;
    };
    let Some(ref mut t) = TASK_TABLE[idx] else {
        return;
    };
    if regs.is_null() {
        return;
    }
    let f = &mut t.regs;
    f.x0 = *regs.add(0);
    f.x1 = *regs.add(1);
    f.x2 = *regs.add(2);
    f.x3 = *regs.add(3);
    f.x4 = *regs.add(4);
    f.x5 = *regs.add(5);
    f.x6 = *regs.add(6);
    f.x7 = *regs.add(7);
    f.x8 = *regs.add(8);
    f.x9 = *regs.add(9);
    f.x10 = *regs.add(10);
    f.x11 = *regs.add(11);
    f.x12 = *regs.add(12);
    f.x13 = *regs.add(13);
    f.x14 = *regs.add(14);
    f.x15 = *regs.add(15);
    f.x16 = *regs.add(16);
    f.x17 = *regs.add(17);
    f.x18 = *regs.add(18);
    f.x19 = *regs.add(19);
    f.x20 = *regs.add(20);
    f.x21 = *regs.add(21);
    f.x22 = *regs.add(22);
    f.x23 = *regs.add(23);
    f.x24 = *regs.add(24);
    f.x25 = *regs.add(25);
    f.x26 = *regs.add(26);
    f.x27 = *regs.add(27);
    f.x28 = *regs.add(28);
    f.x29 = *regs.add(29);
    f.x30 = *regs.add(30);
    t.regs.sp = *regs.add(31);
    t.regs.elr = elr;
    t.regs.spsr = spsr;
}

/// S3: true once `preempt on` has enabled the architectural-timer tick.
static mut PREEMPT_ON: bool = false;

pub fn set_preempt(on: bool) {
    unsafe {
        PREEMPT_ON = on;
    }
}

pub fn preempt_enabled() -> bool {
    unsafe { PREEMPT_ON }
}

/// Re-enable IRQs if the preempt tick is active. Called from the main
/// loop after every slice/cooperative return (which leave DAIF masked).
pub fn reenable_preempt_irqs() {
    if preempt_enabled() {
        crate::arch::cpu::enable_irqs();
    }
}

/// Mark a task as a background (sliced) EL0 task. It runs one slice per
/// main-loop iteration until its next SYS_SLEEP, then yields.
pub fn mark_bg(pid: Pid) -> bool {
    if let Some(idx) = find_task_index(pid) {
        unsafe {
            if let Some(ref mut t) = TASK_TABLE[idx] {
                if t.is_user {
                    t.bg = true;
                    t.wake_ms = 0;
                    return true;
                }
            }
        }
    }
    false
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
                    // S6: tear down any user windows the app left open
                    // (frames + WM chrome) before freeing the task.
                    crate::wm::user_windows::close_all_for_pid(task.pid);
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

/// Kill the current user process and force return to kernel cooperative mode.
/// This prevents re-entering EL0 after a fatal exception (kill-loop fix).
pub fn kill_current_and_exit(code: i64) {
    let pid = current_pid();
    if let Some(idx) = find_task_index(pid) {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.state = TaskState::Zombie;
                task.exit_code = code;
                println!("[sched] PID {} killed (exit code {})", pid, code);
            }
        }
    }
    // Request cooperative exit so eret returns to EL1h kernel mode
    request_cooperative_exit();
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

/// Find OOM victim — task with the most runtime ticks (excluding idle/init)
pub fn get_oom_victim() -> Option<Pid> {
    unsafe {
        let mut victim_pid: Option<Pid> = None;
        let mut max_ticks: u64 = 0;

        for i in 0..MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[i] {
                // Skip PID 0 (idle) and PID 1 (init)
                if task.pid <= 1 {
                    continue;
                }
                // Only consider Running or Ready tasks
                if task.state != TaskState::Running && task.state != TaskState::Ready {
                    continue;
                }
                if task.runtime_ticks > max_ticks {
                    max_ticks = task.runtime_ticks;
                    victim_pid = Some(task.pid);
                }
            }
        }
        victim_pid
    }
}

// ── Program break (brk) management ──
// Single-process cooperative model: one global brk is sufficient
static mut USER_BRK: u64 = 0x20000000; // Default heap base
static mut USER_BRK_LIMIT: u64 = 0x30000000; // Heap grows up to here

/// Get the current program break for the running user process
pub fn get_user_brk() -> u64 {
    unsafe { USER_BRK }
}

/// Set the program break (called from brk/mmap syscalls)
pub fn set_user_brk(new_brk: u64) -> bool {
    unsafe {
        if new_brk >= USER_BRK && new_brk <= USER_BRK_LIMIT {
            USER_BRK = new_brk;
            true
        } else {
            false
        }
    }
}

/// Get the parent PID of the current process
pub fn get_parent_pid() -> u64 {
    let pid = current_pid();
    if let Some(idx) = find_task_index(pid) {
        unsafe {
            if let Some(ref task) = TASK_TABLE[idx] {
                return task.parent_pid;
            }
        }
    }
    0 // No parent (init process)
}

/// Create a new session (setsid syscall)
pub fn create_session() -> u64 {
    let pid = current_pid();
    if let Some(idx) = find_task_index(pid) {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.sid = pid;
                task.pgid = pid;
                return pid;
            }
        }
    }
    u64::MAX // Error
}
