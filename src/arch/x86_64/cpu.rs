//! x86_64 CPU-level utilities: CPUID, MSR access, barriers, feature detection
//!
//! This module provides low-level x86_64 primitives for kernel operations.

use core::arch::asm;

/// Read the current CPU vendor string
pub fn cpu_vendor() -> [u8; 12] {
    #[cfg(not(test))]
    {
        let mut vendor = [0u8; 12];
        unsafe {
            asm!(
                "cpuid",
                in("eax") 0u32,
                out("ebx") vendor[0..4].as_mut_ptr() as *mut u32,
                out("edx") vendor[4..8].as_mut_ptr() as *mut u32,
                out("ecx") vendor[8..12].as_mut_ptr() as *mut u32,
                options(nomem, nostack, preserves_flags)
            );
        }
        vendor
    }
    #[cfg(test)]
    {
        // Stub for tests - return a dummy vendor string
        *b"TestVendor  "
    }
}

/// Check if a specific CPUID feature is supported
pub fn cpuid_feature(eax: u32, ecx: u32, bit: u32) -> bool {
    #[cfg(not(test))]
    {
        let mut edx: u32;
        let mut ecx_out: u32;
        unsafe {
            asm!(
                "cpuid",
                in("eax") eax,
                in("ecx") ecx,
                out("edx") edx,
                out("ecx") ecx_out,
                options(nomem, nostack, preserves_flags)
            );
        }
        (edx & (1 << bit)) != 0 || (ecx_out & (1 << bit)) != 0
    }
    #[cfg(test)]
    {
        // Stub for tests - return false
        false
    }
}

/// Check if APIC is present (CPUID.1:EDT[9])
pub fn has_apic() -> bool {
    cpuid_feature(1, 0, 9)
}

/// Check if x2APIC is supported (CPUID.1:ECP[21])
pub fn has_x2apic() -> bool {
    cpuid_feature(1, 0, 21)
}

/// Check if invariant TSC is supported (CPUID.0x80000007:EDT[8])
pub fn has_invariant_tsc() -> bool {
    cpuid_feature(0x80000007, 0, 8)
}

/// Read Time Stamp Counter (TSC)
#[inline]
pub fn rdtsc() -> u64 {
    let tsc: u64;
    unsafe {
        asm!("rdtsc", out("rax") tsc, options(nomem, nostack, preserves_flags));
    }
    tsc
}

/// Read TSC with auxiliary (requires RDTSCP support)
#[inline]
pub fn rdtscp() -> (u64, u32) {
    let (tsc, aux): (u64, u32);
    unsafe {
        asm!("rdtscp", out("rax") tsc, out("rcx") aux, options(nomem, nostack, preserves_flags));
    }
    (tsc, aux)
}

/// Read a Model Specific Register (MSR)
#[inline]
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") lo,
        out("edx") hi,
        options(nomem, nostack, preserves_flags)
    );
    ((hi as u64) << 32) | (lo as u64)
}

/// Write a Model Specific Register (MSR)
#[inline]
pub unsafe fn wrmsr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
        options(nomem, nostack, preserves_flags)
    );
}

/// Enable interrupts (STI)
#[inline]
pub fn enable_irqs() {
    unsafe { asm!("sti", options(nomem, nostack, preserves_flags)) }
}

/// Disable interrupts (CLI)
#[inline]
pub fn disable_irqs() {
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) }
}

/// Check if interrupts are enabled
#[inline]
pub fn irqs_enabled() -> bool {
    let flags: u64;
    unsafe {
        asm!("pushfq; pop {}", out(reg) flags, options(nomem, nostack));
    }
    (flags & (1 << 9)) != 0 // IF flag
}

/// Data Synchronization Barrier (MFENCE)
#[inline]
pub fn dsb() {
    unsafe { asm!("mfence", options(nomem, nostack, preserves_flags)) }
}

/// Instruction Synchronization Barrier (LFENCE)
#[inline]
pub fn isb() {
    unsafe { asm!("lfence", options(nomem, nostack, preserves_flags)) }
}

/// Full barrier (MFENCE + LFENCE)
#[inline]
pub fn dmb() {
    dsb();
    isb();
}

/// No-op (PAUSE - good for spin loops)
#[inline]
pub fn nop() {
    unsafe { asm!("pause", options(nomem, nostack, preserves_flags)) }
}

/// Wait For Event (HLT - halt until interrupt)
#[inline]
pub fn wfe() {
    unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) }
}

/// Send Event (no direct equivalent, but we can use IPI)
#[inline]
pub fn sev() {
    // On x86, we'd send an IPI instead
}

/// Read the stack pointer
#[inline]
pub fn read_sp() -> u64 {
    let sp: u64;
    unsafe { asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags)) }
    sp
}

/// Read the current privilege level (CPL)
#[inline]
pub fn current_cpl() -> u8 {
    let cs: u16;
    unsafe { asm!("mov {}, cs", out(reg) cs, options(nomem, nostack, preserves_flags)) }
    (cs & 0b11) as u8
}

/// Get APIC ID (requires APIC enabled)
pub fn apic_id() -> u32 {
    if has_x2apic() {
        unsafe { (rdmsr(0x802) & 0xFFFFFFFF) as u32 } // x2APIC ID register
    } else {
        // Legacy APIC - read from memory-mapped APIC
        0 // Placeholder
    }
}

/// Delay for approximately `us` microseconds using TSC
/// Requires invariant TSC for accuracy
pub fn delay_us(us: u64) {
    if has_invariant_tsc() {
        let start = rdtsc();
        // Assume ~2.4 GHz TSC frequency (calibrate at runtime in real implementation)
        let cycles_per_us = 2400;
        let target = start + us * cycles_per_us;
        while rdtsc() < target {
            nop();
        }
    } else {
        // Fallback: busy wait with rough estimate
        for _ in 0..us * 1000 {
            nop();
        }
    }
}

/// Get the number of logical CPUs (from CPUID leaf 0xB or 0x1F)
pub fn cpu_count() -> u32 {
    #[cfg(not(test))]
    {
        let mut count = 1u32;
        // Try extended topology leaf (0x1F)
        let mut eax: u32;
        let mut ebx: u32;
        unsafe {
            asm!(
                "cpuid",
                in("eax") 0x1Fu32,
                in("ecx") 0u32,
                out("eax") eax,
                out("ebx") ebx,
                out("ecx") _,
                out("edx") _,
                options(nomem, nostack, preserves_flags)
            );
        }
        if eax != 0 || ebx != 0 {
            count = ebx & 0xFFFF;
        } else {
            // Fallback to legacy leaf 0xB
            unsafe {
                asm!(
                    "cpuid",
                    in("eax") 0xBu32,
                    in("ecx") 0u32,
                    out("eax") eax,
                    out("ebx") ebx,
                    out("ecx") _,
                    out("edx") _,
                    options(nomem, nostack, preserves_flags)
                );
            }
            if eax != 0 || ebx != 0 {
                count = ebx & 0xFFFF;
            }
        }
        count
    }
    #[cfg(test)]
    {
        // Stub for tests
        1
    }
}

/// Port I/O: read byte from port
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let mut data: u8;
    asm!("in al, dx", in("dx") port, out("al") data, options(nomem, nostack, preserves_flags));
    data
}

/// Port I/O: write byte to port
#[inline]
pub unsafe fn outb(port: u16, data: u8) {
    asm!("out dx, al", in("dx") port, in("al") data, options(nomem, nostack, preserves_flags));
}
