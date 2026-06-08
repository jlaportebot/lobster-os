//! BCM2712 power management driver — enhanced with idle, DVFS, suspend/resume, watchdog
//!
//! Provides:
//! - CPU idle states (WFI-based)
//! - DVFS (dynamic voltage/frequency scaling)
//! - Suspend/resume framework
//! - RTC wake-on-timer
//! - Watchdog timer driver

use crate::arch::cpu::{disable_irqs, wfe};
use crate::arch::timer::Timer;
use crate::drivers::mailbox;
use crate::println;

/// Power management base address
const PM_BASE: u64 = 0x10_7C10_0000;

/// Power management registers
const PM_RSTC: u64 = PM_BASE + 0x1C;
const PM_WDOG: u64 = PM_BASE + 0x24;

/// Power management password
const PM_PASSWORD: u32 = 0x5A000000;

/// Reset commands
const PM_RSTC_WRCFG_CLR: u32 = 0xffffffcf;
const PM_RSTC_WRCFG_FULL_RESET: u32 = 0x00000020;

/// Watchdog commands
const PM_WDOG_TIME_SET: u32 = 0x000fffff;

/// CPU idle states
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CpuIdleState {
    /// Active - normal execution
    Active,
    /// Wait for interrupt (WFI) - light sleep
    Wfi,
    /// Deep sleep - requires external wake
    DeepSleep,
}

/// Global power state
static mut POWER_STATE: PowerState = PowerState {
    current_idle_state: CpuIdleState::Active,
    dvfs_enabled: false,
    current_frequency_mhz: 2400, // BCM2712 max 2.4 GHz
    suspend_requested: false,
    watchdog_enabled: false,
    watchdog_timeout_ms: 0,
};

/// Power management state
struct PowerState {
    current_idle_state: CpuIdleState,
    dvfs_enabled: bool,
    current_frequency_mhz: u32,
    suspend_requested: bool,
    watchdog_enabled: bool,
    watchdog_timeout_ms: u32,
}

/// Available CPU frequencies (MHz)
const CPU_FREQUENCIES: &[u32] = &[600, 1000, 1500, 2000, 2400];

/// Initialize power management
pub fn init() {
    println!("[power] Initializing power management...");
    
    // Initialize mailbox for DVFS
    mailbox::init();
    
    // Get current frequency
    let freq = get_current_frequency();
    unsafe {
        POWER_STATE.current_frequency_mhz = freq;
    }
    
    println!("[power] Current frequency: {} MHz", freq);
    println!("[power] Power management initialized");
}

/// Get current CPU frequency via mailbox
fn get_current_frequency() -> u32 {
    // Use mailbox property tag to get clock rate
    const TAG_GET_CLOCK_RATE: u32 = 0x00030002;
    const CLOCK_ARM: u32 = 3;
    
    let mut buf = [0u32; 8];
    buf[0] = 32; // buffer size
    buf[1] = 0;  // request
    buf[2] = TAG_GET_CLOCK_RATE;
    buf[3] = 8;  // value buffer size
    buf[4] = 0;  // request code
    buf[5] = CLOCK_ARM;
    buf[6] = 0;
    buf[7] = 0;  // end tag
    
    if mailbox::property_call(&mut buf) {
        let rate_hz = buf[6];
        (rate_hz / 1_000_000) as u32
    } else {
        2400 // fallback to max
    }
}

/// Set CPU frequency via mailbox (DVFS)
pub fn set_frequency(mhz: u32) -> Result<(), &'static str> {
    // Check if frequency is supported
    if !CPU_FREQUENCIES.contains(&mhz) {
        return Err("Unsupported frequency");
    }
    
    const TAG_SET_CLOCK_RATE: u32 = 0x00038002;
    const CLOCK_ARM: u32 = 3;
    
    let mut buf = [0u32; 8];
    buf[0] = 32;
    buf[1] = 0;
    buf[2] = TAG_SET_CLOCK_RATE;
    buf[3] = 12;
    buf[4] = 0;
    buf[5] = CLOCK_ARM;
    buf[6] = mhz * 1_000_000;
    buf[7] = 0;
    
    if mailbox::property_call(&mut buf) {
        unsafe {
            POWER_STATE.current_frequency_mhz = mhz;
        }
        println!("[power] Frequency set to {} MHz", mhz);
        Ok(())
    } else {
        Err("Failed to set frequency via mailbox")
    }
}

/// Get current CPU frequency
pub fn get_frequency() -> u32 {
    unsafe { POWER_STATE.current_frequency_mhz }
}

/// Enable/disable DVFS
pub fn set_dvfs_enabled(enabled: bool) {
    unsafe {
        POWER_STATE.dvfs_enabled = enabled;
    }
    println!("[power] DVFS {}", if enabled { "enabled" } else { "disabled" });
}

/// CPU idle loop - called from scheduler when no tasks are ready
/// Enters WFI (Wait For Interrupt) to save power
pub fn cpu_idle() {
    // If DVFS is enabled, consider lowering frequency when idle
    unsafe {
        if POWER_STATE.dvfs_enabled && POWER_STATE.current_frequency_mhz > 600 {
            let _ = set_frequency(600);
        }
        POWER_STATE.current_idle_state = CpuIdleState::Wfi;
    }
    
    // Wait for interrupt
    wfe();
    
    // Restore frequency when waking up
    unsafe {
        if POWER_STATE.dvfs_enabled {
            let target_freq = 2400; // Restore to max
            let _ = set_frequency(target_freq);
        }
        POWER_STATE.current_idle_state = CpuIdleState::Active;
    }
}

/// Request system suspend
pub fn request_suspend() {
    unsafe {
        POWER_STATE.suspend_requested = true;
    }
    println!("[power] Suspend requested");
}

/// Check if suspend was requested
pub fn is_suspend_requested() -> bool {
    unsafe { POWER_STATE.suspend_requested }
}

/// Enter suspend mode
pub fn suspend() {
    println!("[power] Entering suspend...");
    
    // Save current state
    let freq = get_frequency();
    
    // Disable DVFS
    set_dvfs_enabled(false);
    
    // Lower to minimum frequency
    let _ = set_frequency(600);
    
    // Disable non-essential peripherals
    // TODO: Implement peripheral power gating
    
    // Wait for wake event
    loop {
        wfe();
        
        // Check if wake requested
        if !is_suspend_requested() {
            break;
        }
    }
    
    // Resume
    resume();
}

/// Resume from suspend
pub fn resume() {
    println!("[power] Resuming from suspend...");
    
    // Restore frequency
    let _ = set_frequency(2400);
    
    // Re-enable DVFS
    set_dvfs_enabled(true);
    
    unsafe {
        POWER_STATE.suspend_requested = false;
        POWER_STATE.current_idle_state = CpuIdleState::Active;
    }
    
    println!("[power] Resume complete");
}

/// Watchdog timer initialization
pub fn watchdog_init(timeout_ms: u32) {
    unsafe {
        POWER_STATE.watchdog_enabled = true;
        POWER_STATE.watchdog_timeout_ms = timeout_ms;
    }
    
    // Configure hardware watchdog
    let pm_wdog_val = PM_PASSWORD | (timeout_ms & PM_WDOG_TIME_SET);
    let pm_rstc_val = PM_PASSWORD | PM_RSTC_WRCFG_FULL_RESET;
    
    unsafe {
        core::ptr::write_volatile(PM_WDOG as *mut u32, pm_wdog_val);
        core::ptr::write_volatile(PM_RSTC as *mut u32, pm_rstc_val);
    }
    
    println!("[power] Watchdog enabled with {} ms timeout", timeout_ms);
}

/// Feed the watchdog (reset timeout)
pub fn watchdog_feed() {
    unsafe {
        if POWER_STATE.watchdog_enabled {
            // Reload watchdog timeout
            let pm_wdog_val = PM_PASSWORD | (POWER_STATE.watchdog_timeout_ms & PM_WDOG_TIME_SET);
            core::ptr::write_volatile(PM_WDOG as *mut u32, pm_wdog_val);
        }
    }
}

/// Disable watchdog
pub fn watchdog_disable() {
    unsafe {
        POWER_STATE.watchdog_enabled = false;
    }
    println!("[power] Watchdog disabled");
}

/// RTC wake alarm - set wake time
pub fn rtc_set_wake_alarm(seconds_from_now: u64) {
    // Use mailbox to set RTC alarm
    const TAG_SET_WAKE_TIME: u32 = 0x00038047;
    
    let wake_time = Timer::system_time_us() / 1_000_000 + seconds_from_now;
    
    let mut buf = [0u32; 8];
    buf[0] = 32;
    buf[1] = 0;
    buf[2] = TAG_SET_WAKE_TIME;
    buf[3] = 8;
    buf[4] = 0;
    buf[5] = (wake_time & 0xFFFFFFFF) as u32;
    buf[6] = (wake_time >> 32) as u32;
    buf[7] = 0;
    
    if mailbox::property_call(&mut buf) {
        println!("[power] RTC wake alarm set for {} seconds", seconds_from_now);
    } else {
        println!("[power] Failed to set RTC wake alarm");
    }
}

/// Reset the system
pub fn reset() -> ! {
    let wdog = 100; // usecs to reset

    disable_irqs();

    let pm_rstc_val = unsafe { core::ptr::read_volatile(PM_RSTC as *const u32) }
        & PM_RSTC_WRCFG_CLR
        | PM_PASSWORD
        | PM_RSTC_WRCFG_FULL_RESET;
    let pm_wdog_val = PM_PASSWORD | (wdog & PM_WDOG_TIME_SET);

    unsafe {
        core::ptr::write_volatile(PM_WDOG as *mut u32, pm_wdog_val);
        core::ptr::write_volatile(PM_RSTC as *mut u32, pm_rstc_val);
    }

    loop {
        wfe();
    }
}

/// Shutdown the system
pub fn shutdown() -> ! {
    disable_irqs();
    
    // Try to shutdown via mailbox
    const TAG_SET_POWER_STATE: u32 = 0x00028001;
    
    let mut buf = [0u32; 8];
    buf[0] = 32;
    buf[1] = 0;
    buf[2] = TAG_SET_POWER_STATE;
    buf[3] = 8;
    buf[4] = 0;
    buf[5] = 0; // device id
    buf[6] = 0; // state: off
    buf[7] = 0;
    
    mailbox::property_call(&mut buf);
    
    loop {
        wfe();
    }
}

/// Power management statistics
pub fn print_stats() {
    unsafe {
        println!("[power] Stats:");
        println!("  Idle state: {:?}", POWER_STATE.current_idle_state);
        println!("  DVFS enabled: {}", POWER_STATE.dvfs_enabled);
        println!("  Current frequency: {} MHz", POWER_STATE.current_frequency_mhz);
        println!("  Suspend requested: {}", POWER_STATE.suspend_requested);
        println!("  Watchdog enabled: {}", POWER_STATE.watchdog_enabled);
        if POWER_STATE.watchdog_enabled {
            println!("  Watchdog timeout: {} ms", POWER_STATE.watchdog_timeout_ms);
        }
    }
}