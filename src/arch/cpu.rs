//! CPU-level utilities: EL checking, system register access, barriers

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

/// Delay for approximately `us` microseconds using a busy-wait loop.
/// Calibrated for ~1 GHz CPU: each iteration ≈ 10 ns (3–4 instructions).
#[inline]
pub fn delay_us(us: u64) {
    let iterations = us * 100; // 100 iterations per µs at ~1 GHz
    for _ in 0..iterations {
        nop();
    }
}
