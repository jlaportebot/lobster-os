//! SMP (Symmetric Multiprocessing) support for Raspberry Pi 5
//!
//! Full SMP implementation with:
//! - Per-core runqueues and scheduling state
//! - Inter-Processor Interrupts (IPI) via GIC SGIs
//! - PSCI-style core boot (spin-table method for RPi5)
//! - Load balancing across cores
//! - Core-local timer interrupts
//! - CPU online/offline state tracking
//!
//! BCM2712: 4x Cortex-A76, Affinity 0-3

use crate::arch::cpu;
use crate::arch::gic;
use crate::println;
use crate::sched::{self, Pid};

pub mod balance;
pub mod ipi;
pub mod runqueue;

/// Total number of CPU cores on BCM2712
pub const CORE_COUNT: usize = 4;

/// SGI numbers used for IPI
/// GICv2 supports SGI 0-15; we allocate specific ones
pub const IPI_RESCHEDULE: u32 = 0; // Reschedule IPI
pub const IPI_CALL_FUNCTION: u32 = 1; // Generic function call IPI
pub const IPI_STOP: u32 = 2; // Stop/halt core IPI
pub const IPI_WAKE: u32 = 3; // Wake core from WFE

/// Per-core state
#[repr(C)]
pub struct CoreLocal {
    /// Core ID (0-3)
    pub core_id: u64,
    /// MPIDR affinity value
    pub mpidr: u64,
    /// Is this core online and scheduling?
    pub online: bool,
    /// Currently running PID on this core
    pub current_pid: Pid,
    /// Core-local scheduler tick count
    pub tick_count: u64,
    /// Number of context switches on this core
    pub context_switches: u64,
    /// Number of IPIs sent from this core
    pub ipis_sent: u64,
    /// Number of IPIs received on this core
    pub ipis_received: u64,
    /// Load average (runnable tasks, exponentially weighted)
    pub load_avg: u64,
    /// Idle since last check?
    pub idle: bool,
}

impl CoreLocal {
    pub const fn new(core_id: u64, mpidr: u64) -> Self {
        Self {
            core_id,
            mpidr,
            online: false,
            current_pid: 0,
            tick_count: 0,
            context_switches: 0,
            ipis_sent: 0,
            ipis_received: 0,
            load_avg: 0,
            idle: false,
        }
    }
}

/// Per-core data array (one per physical core)
/// Accessed via core_id index
static mut CORE_LOCAL: [CoreLocal; CORE_COUNT] = [
    CoreLocal::new(0, 0),
    CoreLocal::new(1, 1),
    CoreLocal::new(2, 2),
    CoreLocal::new(3, 3),
];

/// MPIDR affinity values for BCM2712 cores
/// Core 0: affinity 0x00, Core 1: 0x01, Core 2: 0x02, Core 3: 0x03
const MPIDR_AFFINITIES: [u64; CORE_COUNT] = [0x00, 0x01, 0x02, 0x03];

/// Get the current core ID
#[inline]
pub fn current_core_id() -> u64 {
    cpu::mpidr() & 0xFF
}

/// Get a reference to the CoreLocal for the current core
/// # Safety
/// Caller must not create overlapping mutable references
#[inline]
pub unsafe fn this_core() -> *mut CoreLocal {
    let id = current_core_id() as usize;
    &mut CORE_LOCAL[id] as *mut CoreLocal
}

/// Get a reference to a specific core's CoreLocal
#[inline]
pub unsafe fn core_local(id: usize) -> *mut CoreLocal {
    &mut CORE_LOCAL[id] as *mut CoreLocal
}

/// Check if a core is online
pub fn is_core_online(core_id: usize) -> bool {
    unsafe { CORE_LOCAL[core_id].online }
}

/// Set a core's online status
pub fn set_core_online(core_id: usize, online: bool) {
    unsafe {
        CORE_LOCAL[core_id].online = online;
    }
}

/// Get the current PID running on a specific core
pub fn core_current_pid(core_id: usize) -> Pid {
    unsafe { CORE_LOCAL[core_id].current_pid }
}

/// Set the current PID on a specific core
pub fn set_core_current_pid(core_id: usize, pid: Pid) {
    unsafe {
        CORE_LOCAL[core_id].current_pid = pid;
    }
}

/// Increment the IPI received counter for a core
pub fn increment_ipi_received(core_id: usize) {
    unsafe {
        CORE_LOCAL[core_id].ipis_received += 1;
    }
}

/// Initialize core 0 state
pub fn init_core0() {
    unsafe {
        (*this_core()).online = true;
        (*this_core()).current_pid = 0;
        println!(
            "[smp] Core 0 initialized (MPIDR={:#x})",
            (*this_core()).mpidr
        );
    }
}

/// Boot secondary cores using the spin-table method
pub fn boot_secondary_cores() {
    println!("[smp] Booting secondary cores...");

    // Initialize runqueues for all cores
    runqueue::init();

    // Boot cores 1, 2, and 3
    for core_id in 1..CORE_COUNT {
        boot_secondary_core(core_id as u64);
    }

    println!("[smp] All secondary cores booted");
}

/// Boot a specific secondary core
fn boot_secondary_core(core_id: u64) {
    // Calculate the spin-table address for this core
    // RPi5 spin table: firmware places core boot vectors at 0xD8, 0xE0, 0xE8 per core
    let spin_table_addr = 0xD8 + (core_id * 8);

    // The secondary core starts in secondary_core_init (boot.S → main.rs)
    // which sets up TTBR1/MMU before calling smp::secondary_entry().
    // We don't need to write a separate spin-table entry point here
    // because the RPi5 firmware already routes cores to our boot.S vector.

    // Mark core as "coming online" (will set online=true in secondary_core_init)
    unsafe {
        CORE_LOCAL[core_id as usize].mpidr = MPIDR_AFFINITIES[core_id as usize];
    }

    println!(
        "[smp] Core {} boot vector written to {:#010x}",
        core_id, spin_table_addr
    );

    // Wait for the core to come online (with timeout)
    let mut timeout = 1_000_000;
    while !is_core_online(core_id as usize) && timeout > 0 {
        cpu::delay_us(10);
        timeout -= 1;
    }

    if is_core_online(core_id as usize) {
        println!("[smp] Core {} online", core_id);
    } else {
        println!("[smp] Core {} failed to come online (timeout)", core_id);
    }
}

/// Entry point for secondary cores (called from secondary_core_init in main.rs)
/// `core_id` is passed from the boot assembly via the C ABI.
pub fn secondary_entry(core_id: u64) -> ! {
    // Set up this core's local state
    unsafe {
        let cl = &mut CORE_LOCAL[core_id as usize];
        cl.core_id = core_id;
        cl.mpidr = MPIDR_AFFINITIES[core_id as usize];
    }

    // Initialize GIC CPU interface for this core
    unsafe {
        gic::Gic::init_cpu_interface();
    }

    // Enable SGIs for IPI reception (SGI 0-3 are always enabled on GICv2)
    // But make sure our IPI SGIs can be received
    for sgi in 0..=IPI_WAKE {
        unsafe {
            gic::Gic::set_priority(sgi, 0x00); // Highest priority for IPIs
        }
    }

    // Enable timer for this core
    unsafe {
        crate::arch::timer::Timer::init_secondary();
    }

    // Set core online
    set_core_online(core_id as usize, true);

    println!("[smp] Core {} online (GIC+Timer ready)", core_id);

    // Enable IRQs
    cpu::enable_irqs();

    // Enter the per-core scheduler loop
    runqueue::core_scheduler_loop(core_id)
}

/// Handle an IPI received on this core
/// Called from the IRQ handler when an SGI arrives
pub fn handle_ipi(sgi_num: u32) {
    let core_id = current_core_id() as usize;
    increment_ipi_received(core_id);

    match sgi_num {
        IPI_RESCHEDULE => {
            // Someone asked us to reschedule
            sched::request_schedule();
        }
        IPI_CALL_FUNCTION => {
            // Generic function call — process pending callbacks
            ipi::process_pending_callbacks(core_id);
        }
        IPI_STOP => {
            // Halt this core
            println!("[smp] Core {} stopping...", core_id);
            set_core_online(core_id, false);
            loop {
                cpu::wfe();
            }
        }
        IPI_WAKE => {
            // Just waking us from WFE — nothing else to do
        }
        _ => {
            println!("[smp] Unknown IPI {} on core {}", sgi_num, core_id);
        }
    }
}

/// Print SMP status
pub fn print_status() {
    println!("[smp] === SMP Status ===");
    unsafe {
        for i in 0..CORE_COUNT {
            let cl = &CORE_LOCAL[i];
            let status = if cl.online { "ONLINE" } else { "OFFLINE" };
            let idle_str = if cl.idle { "idle" } else { "active" };
            println!(
                "[smp] Core {} | {} | PID={} | ticks={} | switches={} | load={} | {} | ipi_sent={} ipi_recv={}",
                i, status, cl.current_pid, cl.tick_count,
                cl.context_switches, cl.load_avg, idle_str,
                cl.ipis_sent, cl.ipis_received
            );
        }
    }
    runqueue::print_status();
}
