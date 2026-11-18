//! Byte-wise MMIO helpers that stay decodable under hardware virtualisation.
//!
//! A KVM guest's access to an emulated device traps to the host, which can
//! only emulate it if the CPU reports the faulting register and size in the
//! abort syndrome (ISV=1). Loads and stores that update their base register
//! (`ldrb w, [x], #1`) report ISV=0, so QEMU injects an external abort into the
//! guest. Rust's volatile byte loops compile to exactly those post-indexed
//! forms, so each access here is a single plain `ldrb`/`strb` instead.

/// Read one byte from a device register or window.
///
/// # Safety
/// `addr` must be a mapped, readable MMIO address.
#[inline]
pub unsafe fn read8(addr: u64) -> u8 {
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    {
        let value: u32;
        // SAFETY: caller guarantees `addr` is mapped; a plain `ldrb` has no base update.
        core::arch::asm!("ldrb {v:w}, [{a}]", a = in(reg) addr, v = out(reg) value, options(nostack, preserves_flags));
        value as u8
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "std"))))]
    {
        core::ptr::read_volatile(addr as *const u8)
    }
}

/// Write one byte to a device register or window.
///
/// # Safety
/// `addr` must be a mapped, writable MMIO address.
#[inline]
pub unsafe fn write8(addr: u64, value: u8) {
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    {
        // SAFETY: caller guarantees `addr` is mapped; a plain `strb` has no base update.
        core::arch::asm!("strb {v:w}, [{a}]", a = in(reg) addr, v = in(reg) value as u32, options(nostack, preserves_flags));
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "std"))))]
    {
        core::ptr::write_volatile(addr as *mut u8, value)
    }
}

/// Copy `buf.len()` bytes out of the device window starting at `addr`.
///
/// # Safety
/// `addr..addr + buf.len()` must be mapped, readable MMIO.
pub unsafe fn read_bytes(addr: u64, buf: &mut [u8]) {
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = read8(addr + i as u64);
    }
}

/// Copy `buf` into the device window starting at `addr`.
///
/// # Safety
/// `addr..addr + buf.len()` must be mapped, writable MMIO.
pub unsafe fn write_bytes(addr: u64, buf: &[u8]) {
    for (i, &byte) in buf.iter().enumerate() {
        write8(addr + i as u64, byte);
    }
}
