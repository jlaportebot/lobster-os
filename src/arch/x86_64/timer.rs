//! x86_64 Timer subsystem — LAPIC timer and TSC
//!
//! Provides:
//! - High-resolution timestamp counter (TSC)
//! - LAPIC timer for scheduler ticks
//! - Uptime tracking

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::lapic;
use crate::println;
use core::sync::atomic::{AtomicU64, Ordering};

/// Uptime in milliseconds (updated by LAPIC timer interrupt)
static UPTIME_MS: AtomicU64 = AtomicU64::new(0);

/// TSC frequency in Hz (calibrated at init)
static TSC_FREQ: AtomicU64 = AtomicU64::new(0);

/// Timer structure (matches ARM64 Timer API for compatibility)
pub struct Timer;

impl Timer {
    /// Initialize the timer subsystem (called on BSP)
    pub unsafe fn init() {
        // Calibrate TSC frequency
        Self::calibrate_tsc();

        // Initialize LAPIC timer for 1000 Hz (1ms ticks)
        lapic::init_timer(lapic::TIMER_VECTOR, 1000);

        println!("[timer] x86_64 timer subsystem initialized (TSC + LAPIC)");
    }

    /// Initialize timer on secondary CPUs
    pub unsafe fn init_secondary() {
        // LAPIC timer is already configured globally
        // Just ensure it's unmasked on this CPU
        let mut lvt = lapic::lapic_read(lapic::LAPIC_LVT_TIMER);
        lvt &= !(1 << 16); // Unmask
        lapic::lapic_write(lapic::LAPIC_LVT_TIMER, lvt);
    }

    /// Calibrate TSC frequency using PIT or CPUID
    fn calibrate_tsc() {
        // Try CPUID leaf 0x15 first (TSC frequency info)
        let mut eax: u32;
        let mut ebx: u32;
        let mut ecx: u32;
        let mut edx: u32;

        unsafe {
            core::arch::asm!(
                "cpuid",
                in("eax") 0x15u32,
                out("eax") eax,
                out("ebx") ebx,
                out("ecx") ecx,
                out("edx") edx,
                options(nomem, nostack, preserves_flags)
            );
        }

        if eax != 0 || ebx != 0 || ecx != 0 || edx != 0 {
            // TSC frequency = (EBX/EAX) * ECX (crystal frequency)
            // Actually leaf 0x15: EAX = denominator, EBX = numerator, ECX = crystal Hz
            if eax != 0 && ecx != 0 {
                let freq = ((ebx as u64 * ecx as u64) / eax as u64) as u64;
                TSC_FREQ.store(freq, Ordering::Relaxed);
                println!("[timer] TSC frequency from CPUID 0x15: {} Hz", freq);
                return;
            }
        }

        // Try CPUID leaf 0x16 (processor frequency info)
        unsafe {
            core::arch::asm!(
                "cpuid",
                in("eax") 0x16u32,
                out("eax") eax,
                out("ebx") ebx,
                out("ecx") ecx,
                out("edx") edx,
                options(nomem, nostack, preserves_flags)
            );
        }

        if eax != 0 {
            // EAX = base frequency in MHz
            let freq = (eax as u64) * 1_000_000;
            TSC_FREQ.store(freq, Ordering::Relaxed);
            println!("[timer] TSC frequency from CPUID 0x16: {} Hz", freq);
            return;
        }

        // Fallback: calibrate using PIT or LAPIC timer
        // For now, assume 2.4 GHz (will be refined)
        TSC_FREQ.store(2_400_000_000, Ordering::Relaxed);
        println!("[timer] TSC frequency assumed: 2.4 GHz (fallback)");
    }

    /// Get TSC frequency in Hz
    pub fn frequency() -> u64 {
        TSC_FREQ.load(Ordering::Relaxed)
    }

    /// Get uptime in milliseconds
    pub fn uptime_ms() -> u64 {
        UPTIME_MS.load(Ordering::Relaxed)
    }

    /// Get high-resolution timestamp from TSC (nanoseconds)
    pub fn system_time_ns() -> u64 {
        let tsc = cpu::rdtsc();
        let freq = Self::frequency();
        if freq > 0 {
            // Convert TSC cycles to nanoseconds
            (tsc * 1_000_000_000) / freq
        } else {
            0
        }
    }

    /// Get high-resolution timestamp from TSC (microseconds)
    pub fn system_time_us() -> u64 {
        let tsc = cpu::rdtsc();
        let freq = Self::frequency();
        if freq > 0 {
            (tsc * 1_000_000) / freq
        } else {
            0
        }
    }

    /// Get raw TSC value
    pub fn raw_tsc() -> u64 {
        cpu::rdtsc()
    }

    /// Handle timer interrupt (called from LAPIC timer handler)
    pub fn handle_irq() {
        let uptime = UPTIME_MS.fetch_add(1, Ordering::Relaxed);

        if uptime.is_multiple_of(1000) {
            let secs = uptime / 1000;
            println!("[timer] Uptime: {}s", secs);
        }

        // Signal scheduler
        crate::sched::on_timer_tick();

        // Drive SMP per-core timer accounting + load balancing
        let core_id = crate::smp::current_core_id() as usize;
        crate::smp::runqueue::on_core_timer_tick(core_id);
        crate::smp::balance::on_timer_tick();
    }
}

/// Delay for microseconds using TSC
pub fn delay_us(us: u64) {
    let start = cpu::rdtsc();
    let freq = Timer::frequency();
    if freq > 0 {
        let cycles_per_us = freq / 1_000_000;
        let target = start + us * cycles_per_us;
        while cpu::rdtsc() < target {
            cpu::nop();
        }
    } else {
        // Fallback
        cpu::delay_us(us);
    }
}
