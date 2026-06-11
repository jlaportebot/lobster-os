//! x86_64 Local APIC (LAPIC) and I/O APIC driver
//!
//! Handles:
//! - LAPIC initialization and configuration
//! - I/O APIC initialization and IRQ routing
//! - Inter-processor interrupts (IPIs)
//! - Timer interrupts (LAPIC timer)
//! - Spurious interrupt handling

use crate::arch::x86_64::cpu;
use crate::println;
use core::sync::atomic::{AtomicU64, Ordering};

/// LAPIC base MSR
pub const LAPIC_BASE_MSR: u32 = 0x1B;
/// LAPIC base address (default: 0xFEE00000)
static mut LAPIC_BASE: u64 = 0xFEE0_0000;
/// LAPIC version register
pub const LAPIC_VERSION: u64 = 0x30;
/// LAPIC EOI register
pub const LAPIC_EOI: u64 = 0x0B0;
/// LAPIC SVR (Spurious Vector Register)
const LAPIC_SVR: u64 = 0x0F0;
/// LAPIC TPR (Task Priority Register)
const LAPIC_TPR: u64 = 0x080;
/// LAPIC LDR (Logical Destination Register)
const LAPIC_LDR: u64 = 0x0D0;
/// LAPIC DFR (Destination Format Register)
const LAPIC_DFR: u64 = 0x0E0;
/// LAPIC ICR (Interrupt Command Register) - low
pub const LAPIC_ICR_LOW: u64 = 0x300;
/// LAPIC ICR (Interrupt Command Register) - high
pub const LAPIC_ICR_HIGH: u64 = 0x310;
/// LAPIC LVT Timer
pub const LAPIC_LVT_TIMER: u64 = 0x320;
/// LAPIC LVT Thermal
pub const LAPIC_LVT_THERMAL: u64 = 0x330;
/// LAPIC LVT Perfmon
pub const LAPIC_LVT_PERFMON: u64 = 0x340;
/// LAPIC LVT LINT0
pub const LAPIC_LVT_LINT0: u64 = 0x350;
/// LAPIC LVT LINT1
pub const LAPIC_LVT_LINT1: u64 = 0x360;
/// LAPIC LVT Error
pub const LAPIC_LVT_ERROR: u64 = 0x370;
/// LAPIC Timer Initial Count
pub const LAPIC_TIMER_INIT_COUNT: u64 = 0x380;
/// LAPIC Timer Current Count
pub const LAPIC_TIMER_CUR_COUNT: u64 = 0x390;
/// LAPIC Timer Divide Configuration
pub const LAPIC_TIMER_DIV_CONF: u64 = 0x3E0;

/// I/O APIC base address (default: 0xFEC00000)
static mut IOAPIC_BASE: u64 = 0xFEC0_0000;
/// I/O APIC register select
const IOAPIC_REGSEL: u64 = 0x00;
/// I/O APIC register window
const IOAPIC_REGWIN: u64 = 0x10;
/// I/O APIC version register
const IOAPIC_VERSION: u32 = 0x01;
/// I/O APIC redirection table base
const IOAPIC_REDTBL: u32 = 0x10;

/// LAPIC timer frequency (calibrated at runtime)
static LAPIC_TIMER_FREQ: AtomicU64 = AtomicU64::new(0);

/// IPI vector numbers (matching ARM64 SGI numbers for compatibility)
pub const IPI_RESCHEDULE: u8 = 0xE0;
pub const IPI_CALL_FUNCTION: u8 = 0xE1;
pub const IPI_STOP: u8 = 0xE2;
pub const IPI_WAKE: u8 = 0xE3;

/// Timer vector
pub const TIMER_VECTOR: u8 = 0xEF;

/// Error vector
pub const ERROR_VECTOR: u8 = 0xFE;

/// Spurious vector
pub const SPURIOUS_VECTOR: u8 = 0xFF;

/// Read LAPIC register (memory-mapped)
#[inline]
pub unsafe fn lapic_read(reg: u64) -> u32 {
    let addr = LAPIC_BASE + reg;
    core::ptr::read_volatile(addr as *const u32)
}

/// Write LAPIC register (memory-mapped)
#[inline]
pub unsafe fn lapic_write(reg: u64, value: u32) {
    let addr = LAPIC_BASE + reg;
    core::ptr::write_volatile(addr as *mut u32, value);
}

/// Read I/O APIC register
#[inline]
unsafe fn ioapic_read(reg: u32) -> u32 {
    core::ptr::write_volatile((IOAPIC_BASE + IOAPIC_REGSEL) as *mut u32, reg);
    core::ptr::read_volatile((IOAPIC_BASE + IOAPIC_REGWIN) as *const u32)
}

/// Write I/O APIC register
#[inline]
unsafe fn ioapic_write(reg: u32, value: u32) {
    core::ptr::write_volatile((IOAPIC_BASE + IOAPIC_REGSEL) as *mut u32, reg);
    core::ptr::write_volatile((IOAPIC_BASE + IOAPIC_REGWIN) as *mut u32, value);
}

/// Initialize the Local APIC on this CPU
pub unsafe fn init_lapic() {
    // Read LAPIC base MSR
    let base_msr = cpu::rdmsr(LAPIC_BASE_MSR);
    LAPIC_BASE = base_msr & 0xFFFF_F000;
    let enabled = base_msr & (1 << 11) != 0;

    if !enabled {
        // Enable LAPIC by setting the enable bit
        cpu::wrmsr(LAPIC_BASE_MSR, base_msr | (1 << 11));
    }

    // Read version
    let version = lapic_read(LAPIC_VERSION);
    let max_lvt = (version >> 16) & 0xFF;
    println!(
        "[lapic] LAPIC at {:#x}, version {:#x}, max LVT entries: {}",
        LAPIC_BASE,
        version & 0xFF,
        max_lvt
    );

    // Disable legacy PIC (if present) by masking all IRQs
    disable_pic();

    // Configure Spurious Vector Register: enable LAPIC, set spurious vector
    lapic_write(LAPIC_SVR, (1 << 8) | SPURIOUS_VECTOR as u32);

    // Set Task Priority Register to 0 (accept all priorities)
    lapic_write(LAPIC_TPR, 0);

    // Configure LVT Error vector
    lapic_write(LAPIC_LVT_ERROR, ERROR_VECTOR as u32);

    // Configure LVT LINT0/1 (NMI/INTR from legacy PIC) - mask them
    lapic_write(LAPIC_LVT_LINT0, (1 << 16)); // Mask
    lapic_write(LAPIC_LVT_LINT1, (1 << 16)); // Mask

    // Configure LVT Thermal and Perfmon - mask them
    lapic_write(LAPIC_LVT_THERMAL, (1 << 16));
    lapic_write(LAPIC_LVT_PERFMON, (1 << 16));

    // Initialize IPI vectors in LVT (we'll use ICR for IPIs, not LVT)

    println!("[lapic] Local APIC initialized on CPU {}", cpu::apic_id());
}

/// Initialize I/O APIC
pub unsafe fn init_ioapic() {
    // Read version
    let version = ioapic_read(IOAPIC_VERSION);
    let max_redir = (version >> 16) & 0xFF;
    println!(
        "[ioapic] I/O APIC at {:#x}, version {:#x}, max redirection entries: {}",
        IOAPIC_BASE,
        version & 0xFF,
        max_redir
    );

    // Mask all interrupts initially
    for i in 0..=max_redir {
        let redir_low = IOAPIC_REDTBL + i * 2;
        let redir_high = redir_low + 1;
        ioapic_write(redir_low, 1 << 16); // Mask bit
        ioapic_write(redir_high, 0);
    }

    println!("[ioapic] All interrupts masked");
}

/// Map an IRQ to a vector on a specific CPU
pub unsafe fn ioapic_set_irq(
    irq: u8,
    vector: u8,
    destination_cpu: u8,
    delivery_mode: u8,
    polarity: bool,
    trigger_mode: bool,
) {
    let redir_low = IOAPIC_REDTBL + (irq as u32) * 2;
    let redir_high = redir_low + 1;

    let mut low = vector as u32;
    low |= ((delivery_mode & 0x7) as u32) << 8; // Delivery mode
    low |= (1 << 11); // Destination mode: physical
    low |= (polarity as u32) << 13; // Polarity (0=high, 1=low)
    low |= (trigger_mode as u32) << 15; // Trigger mode (0=edge, 1=level)
    low |= (1 << 16); // Masked (we'll unmask when enabling)

    let high = (destination_cpu as u32) << 24; // Destination field

    ioapic_write(redir_low, low);
    ioapic_write(redir_high, high);
}

/// Enable an IRQ on I/O APIC
pub unsafe fn ioapic_enable_irq(irq: u8) {
    let redir_low = IOAPIC_REDTBL + (irq as u32) * 2;
    let mut low = ioapic_read(redir_low);
    low &= !(1 << 16); // Clear mask bit
    ioapic_write(redir_low, low);
}

/// Disable an IRQ on I/O APIC
pub unsafe fn ioapic_disable_irq(irq: u8) {
    let redir_low = IOAPIC_REDTBL + (irq as u32) * 2;
    let mut low = ioapic_read(redir_low);
    low |= (1 << 16); // Set mask bit
    ioapic_write(redir_low, low);
}

/// Send an IPI (Inter-Processor Interrupt)
pub fn send_ipi(target_cpus: u32, vector: u8, delivery_mode: u8) {
    unsafe {
        // Wait for ICR to be ready
        while lapic_read(LAPIC_ICR_LOW) & (1 << 12) != 0 {
            cpu::nop();
        }

        let mut icr_low = vector as u32;
        icr_low |= ((delivery_mode & 0x7) as u32) << 8;
        icr_low |= (1 << 14); // Level = assert
        icr_low |= (1 << 11); // Destination mode: physical

        // Destination shorthand
        let dest_shorthand = if target_cpus == 0xFFFFFFFF {
            2 // All including self
        } else if target_cpus == 0xFFFFFFFE {
            3 // All excluding self
        } else {
            0 // Specific CPUs in ICR high
        };
        icr_low |= dest_shorthand << 18;

        let icr_high = (target_cpus & 0xFF) << 24;

        lapic_write(LAPIC_ICR_HIGH, icr_high);
        lapic_write(LAPIC_ICR_LOW, icr_low);
    }
}

/// Send IPI to all CPUs (including self)
pub fn send_ipi_all(vector: u8) {
    send_ipi(0xFFFFFFFF, vector, 0); // Fixed delivery
}

/// Send IPI to all other CPUs (excluding self)
pub fn send_ipi_others(vector: u8) {
    send_ipi(0xFFFFFFFE, vector, 0);
}

/// Send IPI to specific CPU
pub fn send_ipi_cpu(cpu_id: u8, vector: u8) {
    send_ipi(cpu_id as u32, vector, 0);
}

/// End of Interrupt - signal LAPIC that interrupt handling is complete
pub fn eoi() {
    unsafe {
        lapic_write(LAPIC_EOI, 0);
    }
}

/// Initialize LAPIC timer for periodic ticks
pub unsafe fn init_timer(vector: u8, hz: u32) {
    // Configure LVT Timer
    let mut lvt = lapic_read(LAPIC_LVT_TIMER);
    lvt &= !(0xFF); // Clear vector
    lvt |= vector as u32;
    lvt |= (1 << 17); // Periodic mode
                      // Mask initially
    lvt |= (1 << 16);
    lapic_write(LAPIC_LVT_TIMER, lvt);

    // Set divide configuration (divide by 16)
    lapic_write(LAPIC_TIMER_DIV_CONF, 0x3);

    // Calibrate timer frequency using TSC
    calibrate_timer(hz);

    // Unmask timer
    lvt &= !(1 << 16);
    lapic_write(LAPIC_LVT_TIMER, lvt);

    println!(
        "[lapic] Timer initialized at {} Hz, vector {:#x}",
        hz, vector
    );
}

/// Calibrate LAPIC timer using TSC
unsafe fn calibrate_timer(hz: u32) {
    if !cpu::has_invariant_tsc() {
        println!("[lapic] WARNING: Invariant TSC not available, using approximate calibration");
    }

    // Wait for TSC to stabilize
    cpu::delay_us(10000);

    let tsc_start = cpu::rdtsc();
    let lapic_start = lapic_read(LAPIC_TIMER_CUR_COUNT);

    // Wait 10ms
    cpu::delay_us(10000);

    let tsc_end = cpu::rdtsc();
    let lapic_end = lapic_read(LAPIC_TIMER_CUR_COUNT);

    let tsc_delta = tsc_end - tsc_start;
    let lapic_delta = lapic_start.wrapping_sub(lapic_end); // Timer counts down

    // LAPIC timer frequency = (lapic_delta / tsc_delta) * TSC_frequency
    // For target Hz, initial_count = lapic_timer_freq / hz
    // lapic_timer_freq = lapic_delta * (TSC_freq / tsc_delta)
    // So initial_count = lapic_delta * TSC_freq / (tsc_delta * hz)

    // Assume TSC frequency ~2.4 GHz (should be read from CPUID leaf 0x15 or 0x16)
    let tsc_freq = 2_400_000_000u64;

    let initial_count = (lapic_delta as u64 * tsc_freq) / (tsc_delta * hz as u64);
    let initial_count = initial_count.min(0xFFFFFFFF) as u32;

    lapic_write(LAPIC_TIMER_INIT_COUNT, initial_count);
    LAPIC_TIMER_FREQ.store(hz as u64, Ordering::Relaxed);

    println!(
        "[lapic] Calibrated: TSC delta={}, LAPIC delta={}, initial_count={}",
        tsc_delta, lapic_delta, initial_count
    );
}

/// Get LAPIC timer frequency
pub fn timer_frequency() -> u64 {
    LAPIC_TIMER_FREQ.load(Ordering::Relaxed)
}

/// Handle LAPIC timer interrupt
pub fn handle_timer_irq() {
    // The timer is periodic, so we just need to acknowledge it
    eoi();

    // Update uptime
    static UPTIME_MS: AtomicU64 = AtomicU64::new(0);
    let uptime = UPTIME_MS.fetch_add(1, Ordering::Relaxed);

    if uptime.is_multiple_of(1000) {
        println!("[timer] Uptime: {}s", uptime / 1000);
    }

    // Drive scheduler
    crate::sched::on_timer_tick();

    // Drive SMP per-core timer accounting
    let core_id = crate::smp::current_core_id() as usize;
    crate::smp::runqueue::on_core_timer_tick(core_id);
    crate::smp::balance::on_timer_tick();
}

/// Handle spurious interrupt
pub fn handle_spurious_irq() {
    // Spurious interrupts don't require EOI
    println!("[lapic] Spurious interrupt");
}

/// Handle error interrupt
pub fn handle_error_irq() {
    eoi();
    let esr = unsafe { lapic_read(0x280) }; // Error Status Register
    println!("[lapic] Error interrupt, ESR={:#x}", esr);
}

/// Disable legacy 8259 PIC
unsafe fn disable_pic() {
    // Master PIC command port
    core::ptr::write_volatile(0x20 as *mut u8, 0x11); // ICW1
    core::ptr::write_volatile(0x21 as *mut u8, 0x20); // ICW2: vector offset 0x20
    core::ptr::write_volatile(0x21 as *mut u8, 0x04); // ICW3: slave on IRQ2
    core::ptr::write_volatile(0x21 as *mut u8, 0x01); // ICW4: 8086 mode

    // Slave PIC command port
    core::ptr::write_volatile(0xA0 as *mut u8, 0x11); // ICW1
    core::ptr::write_volatile(0xA1 as *mut u8, 0x28); // ICW2: vector offset 0x28
    core::ptr::write_volatile(0xA1 as *mut u8, 0x02); // ICW3: cascade on IRQ2
    core::ptr::write_volatile(0xA1 as *mut u8, 0x01); // ICW4: 8086 mode

    // Mask all interrupts on both PICs
    core::ptr::write_volatile(0x21 as *mut u8, 0xFF);
    core::ptr::write_volatile(0xA1 as *mut u8, 0xFF);

    println!("[lapic] Legacy PIC disabled");
}

/// Initialize LAPIC and I/O APIC for this CPU
pub unsafe fn init() {
    init_lapic();
    // I/O APIC only needs to be initialized once (on BSP)
    if cpu::current_cpl() == 0 {
        init_ioapic();
    }
    println!("[lapic] APIC subsystem initialized");
}

/// Get the current CPU's APIC ID
pub fn current_apic_id() -> u32 {
    cpu::apic_id()
}
