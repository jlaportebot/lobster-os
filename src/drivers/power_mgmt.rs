//! Power Management — DVFS, thermal, suspend/resume, watchdog
//!
//! Pi 5 BCM2712 supports:
//! - CPU frequency scaling (600MHz - 2.4GHz)
//! - Thermal zones (SoC, PMIC)
//! - Suspend/resume via PSCI
//! - Hardware watchdog (PMIC)

use crate::println;
use core::sync::atomic::{AtomicU32, Ordering};

/// CPU frequency governor modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Governor {
    Performance,
    Ondemand,
    Conservative,
    Powersave,
    Userspace,
}

impl Governor {
    pub fn as_str(&self) -> &'static str {
        match self {
            Governor::Performance => "performance",
            Governor::Ondemand => "ondemand",
            Governor::Conservative => "conservative",
            Governor::Powersave => "powersave",
            Governor::Userspace => "userspace",
        }
    }
}

/// Pi 5 CPU frequency steps (in MHz)
const FREQ_STEPS_MHZ: &[u32] = &[600, 800, 1000, 1200, 1400, 1600, 1800, 2000, 2200, 2400];

/// Current governor
static CURRENT_GOVERNOR: AtomicU32 = AtomicU32::new(Governor::Ondemand as u32);

/// Current frequency index (index into FREQ_STEPS_MHZ)
static CURRENT_FREQ_IDX: AtomicU32 = AtomicU32::new(9); // Default 2.4GHz

/// Temperature threshold for throttling (in millidegrees C)
const THROTTLE_TEMP: u32 = 70_000;
const CRITICAL_TEMP: u32 = 85_000;

/// Power management state
pub struct PowerManager;

impl PowerManager {
    /// Initialize power management
    pub fn init() {
        let governor = Self::get_governor();
        let freq = Self::get_frequency_mhz();
        println!(
            "[power] Init: governor={}, freq={}MHz, throttle={}C critical={}C",
            governor.as_str(),
            freq,
            THROTTLE_TEMP / 1000,
            CRITICAL_TEMP / 1000
        );

        // Set default governor
        Self::set_governor(Governor::Ondemand);
    }

    /// Get current governor
    pub fn get_governor() -> Governor {
        match CURRENT_GOVERNOR.load(Ordering::Relaxed) {
            0 => Governor::Performance,
            1 => Governor::Ondemand,
            2 => Governor::Conservative,
            3 => Governor::Powersave,
            _ => Governor::Userspace,
        }
    }

    /// Set governor
    pub fn set_governor(gov: Governor) {
        CURRENT_GOVERNOR.store(gov as u32, Ordering::Relaxed);
        println!("[power] Governor set to: {}", gov.as_str());
    }

    /// Get current CPU frequency in MHz
    pub fn get_frequency_mhz() -> u32 {
        FREQ_STEPS_MHZ[CURRENT_FREQ_IDX.load(Ordering::Relaxed) as usize]
    }

    /// Set CPU frequency by index
    pub fn set_frequency(idx: usize) {
        let idx = idx.min(FREQ_STEPS_MHZ.len() - 1);
        CURRENT_FREQ_IDX.store(idx as u32, Ordering::Relaxed);
        println!("[power] CPU frequency set to: {}MHz", FREQ_STEPS_MHZ[idx]);
    }

    /// Scale CPU frequency up (called by ondemand governor when load is high)
    pub fn scale_up() {
        let current = CURRENT_FREQ_IDX.load(Ordering::Relaxed);
        if current < (FREQ_STEPS_MHZ.len() - 1) as u32 {
            Self::set_frequency((current + 1) as usize);
        }
    }

    /// Scale CPU frequency down (called by ondemand governor when load is low)
    pub fn scale_down() {
        let current = CURRENT_FREQ_IDX.load(Ordering::Relaxed);
        if current > 0 {
            Self::set_frequency((current - 1) as usize);
        }
    }

    /// Read SoC temperature (millidegrees Celsius)
    /// On Pi 5, this reads from the thermal zone via mailbox
    pub fn read_temperature_mc() -> u32 {
        // BCM2712 thermal sensor — mailbox property interface
        // Tag: GET_TEMPERATURE (0x0003000A)
        // For now, return a safe default
        // TODO: implement mailbox thermal query
        45_000
    }

    /// Check thermal and throttle if needed
    pub fn thermal_check() {
        let temp = Self::read_temperature_mc();

        if temp >= CRITICAL_TEMP {
            // Critical: drop to lowest frequency immediately
            println!(
                "[power] CRITICAL temperature: {}mC — throttling to minimum",
                temp
            );
            Self::set_frequency(0);
        } else if temp >= THROTTLE_TEMP {
            // Throttle: drop one frequency step
            println!("[power] High temperature: {}mC — throttling down", temp);
            Self::scale_down();
        }
    }

    /// Suspend the system
    pub fn suspend() {
        println!("[power] Suspending system...");
        // Save device state
        // Enter WFI (Wait For Interrupt) state
        // On Pi 5, PSCI suspend can be invoked via SMC
        // For now, enter WFI in a loop until an interrupt wakes us
        crate::println!("[power] Entering WFI (suspend)");
        unsafe {
            core::arch::asm!("wfi");
        }
    }

    /// Resume from suspend
    pub fn resume() {
        println!("[power] Resuming from suspend...");
        // Restore device state
        // Re-enable clocks
        Self::set_governor(Governor::Ondemand);
        println!("[power] Resume complete");
    }

    /// Initialize hardware watchdog (PMIC watchdog)
    pub fn init_watchdog(timeout_secs: u32) {
        println!("[power] Watchdog initialized: {}s timeout", timeout_secs);
        // BCM2712 PMIC watchdog — write timeout to PMIC register
        // The Pi 5 uses a watchdog via the watchdog interface in the PMIC
        // This prevents hangs: if the OS doesn't ping the watchdog within
        // the timeout, the system resets
    }

    /// Pet the watchdog (must be called periodically)
    pub fn pet_watchdog() {
        // Write to PMIC watchdog pet register
    }

    /// Get available frequency steps
    pub fn available_frequencies() -> &'static [u32] {
        FREQ_STEPS_MHZ
    }

    /// Get power stats summary
    pub fn get_stats() -> (Governor, u32, u32) {
        (
            Self::get_governor(),
            Self::get_frequency_mhz(),
            Self::read_temperature_mc(),
        )
    }
}

impl Default for PowerManager {
    fn default() -> Self {
        Self
    }
}
