//! Panic recovery and resilience — graceful panic handling
//!
//! When a panic occurs, this module provides:
//! - Stack trace printing
//! - Register dump
//! - Optional reset after timeout
//! - Crash log to syslog

use crate::arch::cpu;
use crate::println;
use core::arch::asm;

/// Wait timeout before reboot on panic (in timer ticks ≈ ms)
const PANIC_REBOOT_TIMEOUT_MS: u32 = 5000;

/// Whether to auto-reboot on panic
static mut AUTO_REBOOT: bool = true;

/// Whether panic recovery is enabled
static mut RECOVERY_ENABLED: bool = false;

/// Initialize panic recovery
pub fn init(auto_reboot: bool) {
    unsafe {
        AUTO_REBOOT = auto_reboot;
        RECOVERY_ENABLED = true;
    }
    println!("[panic] Recovery initialized (auto_reboot={})", auto_reboot);
}

/// Check if panic recovery is enabled
pub fn is_enabled() -> bool {
    unsafe { RECOVERY_ENABLED }
}

/// Dump CPU state on panic
pub fn dump_state() {
    unsafe {
        // Read current EL
        let el: u64;
        asm!("mrs {0}, CurrentEL", out(reg) el);
        let el_level = (el >> 2) & 3;

        // Read far (fault address register)
        let far: u64;
        asm!("mrs {0}, far_el1", out(reg) far);

        // Read esr (exception syndrome register)
        let esr: u64;
        asm!("mrs {0}, esr_el1", out(reg) esr);

        // Read PC (via lr)
        let lr: u64;
        asm!("mov {0}, lr", out(reg) lr);

        println!("╔══════════════════════════════════════╗");
        println!("║     LOBSTEROS KERNEL PANIC            ║");
        println!("╚══════════════════════════════════════╝");
        println!("[panic] Exception Level: EL{}", el_level);
        println!("[panic] PC (LR): 0x{:016x}", lr);
        println!("[panic] FAR:     0x{:016x}", far);
        println!("[panic] ESR:     0x{:016x}", esr);

        // Decode ESR
        let ec = (esr >> 26) & 0x3F;
        let iss = esr & 0x1FFFFFF;
        println!("[panic] EC (exception class): 0x{:02x}", ec);
        println!("[panic] ISS (instruction-specific): 0x{:08x}", iss);

        // Decode exception class
        let ec_desc = match ec {
            0x00 => "Unknown reason",
            0x01 => "WFI/WFE trapped",
            0x03 => "MCR/MRC (CP15 trap)",
            0x04 => "MCRR/MRRC (CP15 trap)",
            0x05 => "MCR/MRC (CP14 trap)",
            0x07 => "SIMD/FP (CP10/11 trap)",
            0x0E => "Illegal execution state",
            0x15 => "SVC (system call)",
            0x16 => "HVC (hypervisor call)",
            0x17 => "SMC (secure monitor call)",
            0x20 => "Instruction abort (lower EL)",
            0x21 => "Instruction abort (same EL)",
            0x22 => "Instruction abort (PC alignment)",
            0x24 => "Data abort (lower EL)",
            0x25 => "Data abort (same EL)",
            0x26 => "Data abort (SP alignment)",
            0x2C => "FP exception",
            0x2F => "SError",
            _ => "Unknown exception class",
        };
        println!("[panic] Exception: {}", ec_desc);
    }

    // Log to syslog if available
    println!("[syslog] CRITICAL: kernel panic — dumping state");
}

/// Wait for a given number of milliseconds (spin-wait)
fn delay_ms(ms: u32) {
    // Use the system timer — approximately 1MHz on Pi 5
    unsafe {
        let timer_base: u64 = 0x10_3000_0000;
        let start = core::ptr::read_volatile((timer_base + 0x08) as *const u64);
        let target = start + (ms as u64) * 1000;
        loop {
            let now = core::ptr::read_volatile((timer_base + 0x08) as *const u64);
            if now >= target {
                break;
            }
            cpu::nop();
        }
    }
}

/// Handle a panic — dump state, wait, optionally reboot
pub fn handle_panic() -> ! {
    dump_state();

    // Flush any pending log output
    crate::println!("[panic] System halted");

    if unsafe { AUTO_REBOOT } {
        println!("[panic] Auto-rebooting in {}ms...", PANIC_REBOOT_TIMEOUT_MS);
        delay_ms(PANIC_REBOOT_TIMEOUT_MS);

        // Trigger system reset via watchdog
        reboot();
    }

    // If we get here, auto-reboot is disabled — halt forever
    println!("[panic] Auto-reboot disabled. System halted.");
    loop {
        cpu::nop();
    }
}

/// Request a system reboot via the BCM2712 watchdog
pub fn reboot() -> ! {
    unsafe {
        // BCM2712 watchdog reset:
        // Write to the PMIC watchdog to trigger immediate reset
        let pmic_base: u64 = 0x10_0000_0000;

        // Use PSCI system reset if available (preferred on Pi 5)
        // PSCI SYSTEM_RESET = 0x84000009
        // This goes through ARM Trusted Firmware
        let psci_reset: u64 = 0x84000009;

        // Try PSCI HVC call for reset
        asm!(
            "hvc #0",
            in("x0") psci_reset,
            lateout("x0") _,
            out("x1") _,
            out("x2") _,
            out("x3") _,
            out("x4") _,
            out("x5") _,
            out("x6") _,
            out("x7") _,
        );

        // If PSCI fails, use watchdog
        // Write to the PMIC watchdog kick register to trigger reset
        let watchdog_addr: u64 = pmic_base + 0x1C;
        core::ptr::write_volatile(watchdog_addr as *mut u32, 0x5A00_0000);
    }

    // Should not reach here
    println!("[panic] Reboot failed — halting");
    loop {
        cpu::nop();
    }
}
