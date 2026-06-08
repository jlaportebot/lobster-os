//! ARM Generic Timer driver for BCM2712
//!
//! Uses the BCM2835 System Timer (1 MHz, SPIs 64-67) for the scheduler tick
//! as it's more reliable than the ARM generic timer whose frequency may not
//! be set correctly by all firmware versions.

use core::sync::atomic::{AtomicU64, Ordering};

/// System timer base (BCM2835-style, 1 MHz)
/// System timer base address
#[cfg(not(feature = "qemu"))]
const SYSTEM_TIMER_BASE: u64 = 0x10_7C00_3000; // BCM2712
#[cfg(feature = "qemu")]
const SYSTEM_TIMER_BASE: u64 = 0xFE_000_300; // BCM2711 (QEMU raspi4b)
const TIMER_CS: u64 = SYSTEM_TIMER_BASE;
const TIMER_CLO: u64 = SYSTEM_TIMER_BASE + 0x04;
const TIMER_CHI: u64 = SYSTEM_TIMER_BASE + 0x08;
const TIMER_C1: u64 = SYSTEM_TIMER_BASE + 0x10;

/// Default tick interval in microseconds (1 ms = 1000 µs)
const TICK_INTERVAL_US: u32 = 1000;

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
        mmio_write32(TIMER_C1, current.wrapping_add(TICK_INTERVAL_US));

        // Enable system timer channel 1 in GIC (SPI 65)
        crate::arch::gic::Gic::enable_spi(65);
        crate::arch::gic::Gic::set_config(65, 0b10);
        crate::arch::gic::Gic::set_priority(65, 0x80);

        crate::println!("[timer] Using BCM2835 System Timer (1 MHz, SPI 65)");
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
        UPTIME_MS.load(Ordering::Relaxed)
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
                unsafe {
                    let current = mmio_read32(TIMER_CLO);
                    mmio_write32(TIMER_C1, current.wrapping_add(TICK_INTERVAL_US));
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
}

#[inline]
unsafe fn mmio_read32(addr: u64) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn mmio_write32(addr: u64, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}
