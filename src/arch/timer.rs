//! ARM Generic Timer driver for BCM2712
//!
//! Uses the BCM2835 System Timer (1 MHz, SPIs 64-67) for the scheduler tick
//! as it's more reliable than the ARM generic timer whose frequency may not
//! be set correctly by all firmware versions.

use core::sync::atomic::{AtomicU64, Ordering};

/// System timer base (BCM2835-style, 1 MHz)
/// System timer base address
#[cfg(not(feature = "qemu_raspi5b"))]
const SYSTEM_TIMER_BASE: u64 = 0x10_7C00_3000; // BCM2712
#[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
const SYSTEM_TIMER_BASE: u64 = 0xFE00_3000; // BCM2711 (QEMU raspi4b)
#[cfg(feature = "qemu_raspi5b")]
const SYSTEM_TIMER_BASE: u64 = 0x7C00_3000; // BCM2711-style (QEMU raspi5b)
const TIMER_CS: u64 = SYSTEM_TIMER_BASE;
const TIMER_CLO: u64 = SYSTEM_TIMER_BASE + 0x04;
const TIMER_CHI: u64 = SYSTEM_TIMER_BASE + 0x08;
const TIMER_C1: u64 = SYSTEM_TIMER_BASE + 0x10;

/// Tick interval in microseconds (1 ms = 1000 µs) - default periodic tick
const TICK_INTERVAL_US: u32 = 1000;

/// Minimum tick interval for tickless idle (64 µs - shortest we can reliably program)
const TICKLESS_MIN_INTERVAL_US: u32 = 64;

/// Maximum tick interval for tickless idle (10 ms)
const TICKLESS_MAX_INTERVAL_US: u32 = 10_000;

/// Current tick interval (dynamically adjusted for tickless idle)
static mut CURRENT_TICK_INTERVAL_US: u32 = TICK_INTERVAL_US;

/// Whether tickless idle is enabled
static mut TICKLESS_ENABLED: bool = true;

/// Get the current tick interval
pub fn current_tick_interval() -> u32 {
    unsafe { CURRENT_TICK_INTERVAL_US }
}

/// Set the tick interval (for tickless idle)
pub fn set_tick_interval(us: u32) {
    unsafe {
        CURRENT_TICK_INTERVAL_US = us.clamp(TICKLESS_MIN_INTERVAL_US, TICKLESS_MAX_INTERVAL_US);
    }
}

/// Enable or disable tickless idle
pub fn set_tickless_enabled(enabled: bool) {
    unsafe {
        TICKLESS_ENABLED = enabled;
    }
}

/// Uptime counter (incremented on each timer tick)
static UPTIME_MS: AtomicU64 = AtomicU64::new(0);

/// Timer frequency (read from CNTFRQ_EL0 or assumed)
static mut TIMER_FREQ: u64 = 0;

pub struct Timer;

impl Timer {
    /// Initialize the system timer for periodic ticks
    pub unsafe fn init() {
        let freq: u64;
        core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) freq);
        TIMER_FREQ = if freq == 0 { 54_000_000 } else { freq };

        Self::disable_el1_timer();

        // Clear any pending match on channel 1
        mmio_write32(TIMER_CS, 1 << 1);

        // Set compare value = current + tick interval
        let current = mmio_read32(TIMER_CLO);
        let tick_interval = current_tick_interval();
        mmio_write32(TIMER_C1, current.wrapping_add(tick_interval));

        // NOTE: GIC SPI 65 is NOT enabled here. Enabling it at boot made the
        // first timer tick fire during MMU page-table setup (IRQs get unmasked
        // by the console writer after every print), faulting the vector fetch.
        // sched::enable() calls Timer::enable_irq_source() once boot is done.
        crate::println!("[timer] Using BCM2835 System Timer (1 MHz, SPI 65)");
    }

    /// Enable the ARM architectural EL1 physical timer (CNTP) with a
    /// periodic TVAL. This is the only timer QEMU raspi5b actually
    /// implements (the BCM system timer MMIO does not exist there).
    /// `period_ms` is the tick interval. IRQ = GIC PPI 30.
    pub unsafe fn enable_arch_timer(period_ms: u64) {
        let freq: u64;
        core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) freq);
        if freq == 0 {
            return;
        }
        let tval = freq.saturating_mul(period_ms) / 1000;
        core::arch::asm!("msr CNTP_TVAL_EL0, {0}", in(reg) tval);
        // Enable + unmask: CTL = 0b01 (Imask=0, Enable=1, status RO)
        core::arch::asm!("msr CNTP_CTL_EL0, {0}", in(reg) 1u64);
        // Route physical timer IRQs to EL1 (all cores, no NS bits needed
        // at Secure EL1 on this platform)
        core::arch::asm!("msr CNTP_CTL_EL0, {0}", in(reg) 1u64);
    }

    /// Enable the VIRTUAL timer (CNTV, PPI 30 on bcm2712 wiring).
    /// Linux uses CNTV for its tick on this class of hardware; the
    /// virtual-timer IRQ path in QEMU TCG differs from the physical one.
    pub unsafe fn enable_virt_timer(period_ms: u64) {
        let freq: u64;
        core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) freq);
        if freq == 0 {
            return;
        }
        let tval = freq.saturating_mul(period_ms) / 1000;
        core::arch::asm!("msr CNTV_TVAL_EL0, {0}", in(reg) tval);
        core::arch::asm!("msr CNTV_CTL_EL0, {0}", in(reg) 1u64);
    }

    /// Handle a CNTV IRQ: re-arm + unmask.
    pub fn handle_virt_timer_irq() {
        unsafe {
            let freq: u64;
            core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) freq);
            if freq > 0 {
                let tval = freq.saturating_mul(TICK_INTERVAL_US as u64) / 1_000_000;
                core::arch::asm!("msr CNTV_TVAL_EL0, {0}", in(reg) tval);
            }
            core::arch::asm!("msr CNTV_CTL_EL0, {0}", in(reg) 1u64);
        }
    }

    /// Read CNTFRQ_EL0 (for diagnostics)
    pub fn cntfrq() -> u64 {
        let freq: u64;
        unsafe { core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) freq); }
        freq
    }

    /// Handle the architectural timer IRQ: re-arm TVAL + clear status.
    /// Re-arms at the preempt slice interval (10ms).
    pub fn handle_arch_timer_irq() {
        unsafe {
            let freq: u64;
            core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) freq);
            if freq > 0 {
                const SLICE_MS: u64 = 10;
                let tval = freq.saturating_mul(SLICE_MS) / 1000;
                core::arch::asm!("msr CNTP_TVAL_EL0, {0}", in(reg) tval);
            }
            // Writing CTL with Imask=0,Enable=1 clears the pending state
            core::arch::asm!("msr CNTP_CTL_EL0, {0}", in(reg) 1u64);
        }
    }

    /// Enable the timer IRQ source in the GIC (call after boot completes).
    pub unsafe fn enable_irq_source() {
        crate::arch::gic::Gic::enable_spi(65);
        crate::arch::gic::Gic::set_config(65, 0b10);
        crate::arch::gic::Gic::set_priority(65, 0x80);
        // Re-arm the compare in case the original deadline passed while masked
        let current = mmio_read32(TIMER_CLO);
        mmio_write32(TIMER_C1, current.wrapping_add(current_tick_interval()));
        mmio_write32(TIMER_CS, 1 << 1);
    }

    /// Initialize timer for secondary cores
    pub unsafe fn init_secondary() {
        // Secondary cores use the same system timer
    }

    /// Get timer frequency
    pub fn frequency() -> u64 {
        unsafe { TIMER_FREQ }
    }

    /// Get uptime in milliseconds
    pub fn uptime_ms() -> u64 {
        // The ARMv8 architectural counter (CNTVCT/CNTFRQ) is always
        // available on Cortex-A76 — including under QEMU, where the
        // BCM2711-style MMIO system timer is not implemented on raspi5b.
        // Prefer the architectural counter; fall back to the MMIO timer
        // on real hardware where the 1ms tick already runs.
        let hw = UPTIME_MS.load(Ordering::Relaxed);
        if hw > 0 {
            return hw;
        }
        Self::generic_timer_ms()
    }

    /// Milliseconds from the ARMv8 generic virtual counter.
    fn generic_timer_ms() -> u64 {
        let freq: u64;
        let count: u64;
        unsafe {
            core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq);
            core::arch::asm!("mrs {}, cntvct_el0", out(reg) count);
        }
        if freq == 0 {
            return Self::system_time_us() / 1000;
        }
        count / (freq / 1000)
    }

    /// Get high-resolution timestamp from BCM2835 system timer (microseconds)
    pub fn system_time_us() -> u64 {
        unsafe {
            let lo = mmio_read32(TIMER_CLO);
            let hi = mmio_read32(TIMER_CHI);
            ((hi as u64) << 32) | (lo as u64)
        }
    }

    /// Disable the EL1 Physical Timer
    unsafe fn disable_el1_timer() {
        core::arch::asm!("msr CNTP_CTL_EL0, xzr");
    }

    /// Handle BCM2835 system timer IRQ
    pub fn handle_bcm_timer_irq(irq_id: u32) {
        match irq_id {
            65 => {
                let uptime = UPTIME_MS.fetch_add(1, Ordering::Relaxed);

                // Tickless idle: dynamically adjust tick interval based on system load
                let next_interval = if Timer::is_tickless_idle() {
                    // All cores idle - use longer interval to save power
                    TICKLESS_MAX_INTERVAL_US
                } else {
                    // At least one core has work - use minimum interval for responsiveness
                    TICK_INTERVAL_US
                };
                set_tick_interval(next_interval);

                unsafe {
                    let current = mmio_read32(TIMER_CLO);
                    mmio_write32(TIMER_C1, current.wrapping_add(current_tick_interval()));
                    mmio_write32(TIMER_CS, 1 << 1);
                }
                if uptime.is_multiple_of(1000) {
                    let secs = uptime / 1000;
                    crate::println!("[timer] Uptime: {}s", secs);
                }
            }
            64 | 66 | 67 => unsafe {
                let ch_bit = 1u32 << (irq_id - 64);
                mmio_write32(TIMER_CS, ch_bit);
            },
            _ => {}
        }
    }

    /// Handle ARM Generic Timer PPI
    pub fn handle_irq() {
        unsafe {
            let freq = TIMER_FREQ;
            core::arch::asm!("msr CNTP_TVAL_EL0, {}", in(reg) freq / 1000);
        }
    }

    /// Check if all cores are idle (only idle task running)
    /// Used for tickless idle optimization
    pub fn is_tickless_idle() -> bool {
        // Check if tickless is enabled
        if !Self::tickless_enabled() {
            return false;
        }

        // Check if all cores are idle (only idle task running)
        for i in 0..crate::smp::CORE_COUNT {
            if crate::smp::is_core_online(i) {
                let load = crate::smp::runqueue::runnable_count(i);
                if load > 1 {
                    // More than just idle task
                    return false;
                }
            }
        }
        true
    }

    /// Check if tickless idle is enabled
    fn tickless_enabled() -> bool {
        unsafe { TICKLESS_ENABLED }
    }
}

#[inline]
unsafe fn mmio_read32(addr: u64) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn mmio_write32(addr: u64, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}
