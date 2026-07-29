//! CPU-level utilities: EL checking, system register access, barriers
//!
//! This module contains ARM64-specific implementations. For other targets,
//! stub implementations are provided.

#[cfg(target_arch = "aarch64")]
mod arm64 {
    /// Read the current Exception Level (0-3)
    #[inline]
    pub fn current_el() -> u32 {
        let el: u64;
        unsafe { core::arch::asm!("mrs {}, CurrentEL", out(reg) el) }
        ((el >> 2) & 0b11) as u32
    }

    /// Read the stack pointer
    #[inline]
    pub fn read_sp() -> u64 {
        let sp: u64;
        unsafe { core::arch::asm!("mov {}, sp", out(reg) sp) }
        sp
    }

    /// Read MPIDR (core ID)
    #[inline]
    pub fn mpidr() -> u64 {
        let val: u64;
        unsafe { core::arch::asm!("mrs {}, MPIDR_EL1", out(reg) val) }
        val & 0xFF
    }

    /// Enable IRQs (unmask in DAIF)
    #[inline]
    pub fn enable_irqs() {
        unsafe { core::arch::asm!("msr DAIFClr, #2") }
    }

    /// Disable IRQs (mask in DAIF)
    #[inline]
    pub fn disable_irqs() {
        unsafe { core::arch::asm!("msr DAIFSet, #2") }
    }

    /// Enable FIQs
    #[inline]
    #[allow(dead_code)]
    pub fn enable_fiqs() {
        unsafe { core::arch::asm!("msr DAIFClr, #4") }
    }

    /// Wait For Event (low-power idle)
    #[inline]
    pub fn wfe() {
        unsafe { core::arch::asm!("wfe") }
    }

    /// Send Event (wake other cores)
    #[inline]
    #[allow(dead_code)]
    pub fn sev() {
        unsafe { core::arch::asm!("sev") }
    }

    /// Data Synchronization Barrier
    #[inline]
    pub fn dsb() {
        unsafe { core::arch::asm!("dsb sy") }
    }

    /// Instruction Synchronization Barrier
    #[inline]
    pub fn isb() {
        unsafe { core::arch::asm!("isb") }
    }

    /// No-op
    #[inline]
    pub fn nop() {
        unsafe { core::arch::asm!("nop") }
    }

    /// Relax/yield for busy-wait loops (same as nop for ARM64)
    #[inline]
    pub fn relax() {
        nop();
    }

    /// Delay for approximately `us` microseconds using a busy-wait loop.
    /// Calibrated for ~1 GHz CPU: each iteration ≈ 10 ns (3–4 instructions).
    #[inline]
    pub fn delay_us(us: u64) {
        let iterations = us * 100; // 100 iterations per µs at ~1 GHz
        for _ in 0..iterations {
            nop();
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub use arm64::*;

#[cfg(not(target_arch = "aarch64"))]
mod stub {
    /// Read the current Exception Level (0-3) - stub returns 1 (EL1)
    #[inline]
    pub fn current_el() -> u32 {
        1
    }

    /// Read the stack pointer - stub returns 0
    #[inline]
    pub fn read_sp() -> u64 {
        0
    }

    /// Read MPIDR (core ID) - stub returns 0
    #[inline]
    pub fn mpidr() -> u64 {
        0
    }

    /// Enable IRQs - no-op on non-ARM64
    #[inline]
    pub fn enable_irqs() {}

    /// Disable IRQs - no-op on non-ARM64
    #[inline]
    pub fn disable_irqs() {}

    /// Enable FIQs - no-op on non-ARM64
    #[inline]
    #[allow(dead_code)]
    pub fn enable_fiqs() {}

    /// Wait For Event - no-op on non-ARM64
    #[inline]
    pub fn wfe() {}

    /// Send Event - no-op on non-ARM64
    #[inline]
    #[allow(dead_code)]
    pub fn sev() {}

    /// Data Synchronization Barrier - no-op on non-ARM64
    #[inline]
    pub fn dsb() {}

    /// Instruction Synchronization Barrier - no-op on non-ARM64
    #[inline]
    pub fn isb() {}

    /// No-op
    #[inline]
    pub fn nop() {}

    /// Relax/yield for busy-wait loops - no-op on non-ARM64
    #[inline]
    pub fn relax() {}

    /// Delay for approximately `us` microseconds - stub does nothing
    #[inline]
    pub fn delay_us(_us: u64) {}
}

#[cfg(not(target_arch = "aarch64"))]
pub use stub::*;
