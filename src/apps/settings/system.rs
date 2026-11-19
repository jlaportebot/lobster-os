//! Read-only facts about the machine for the Storage, Network and About
//! pages.

use alloc::string::String;

/// Bytes per physical memory frame.
const FRAME_BYTES: u64 = 4096;
/// Hostname shown on the Network page (the sysfs `kernel/hostname` value).
pub const HOSTNAME: &str = "lobsteros";
/// Cores the BCM2712 has.
const CORE_COUNT: usize = 4;

/// Total and used physical memory in bytes.
#[must_use]
pub fn ram() -> (u64, u64) {
    let (total, used, _) = crate::memory::frame_alloc::stats();
    (total as u64 * FRAME_BYTES, used as u64 * FRAME_BYTES)
}

/// Number of CPU cores that are running.
#[must_use]
pub fn online_cores() -> usize {
    (0..CORE_COUNT)
        .filter(|core| crate::smp::is_core_online(*core))
        .count()
}

/// Raw `MIDR_EL1` (0 off aarch64).
#[must_use]
pub fn midr() -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        let value: u64;
        // SAFETY: MIDR_EL1 is a read-only ID register readable at EL1.
        unsafe { core::arch::asm!("mrs {}, MIDR_EL1", out(reg) value) };
        value
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        0
    }
}

/// Kernel version with the git commit when the build recorded one.
#[must_use]
pub fn build_id() -> String {
    let version = env!("CARGO_PKG_VERSION");
    match option_env!("LOBSTER_GIT_REV") {
        Some(rev) => alloc::format!("{version} ({rev})"),
        None => String::from(version),
    }
}

/// Which machine this kernel runs on.
#[must_use]
pub const fn machine() -> &'static str {
    if cfg!(feature = "qemu_raspi5b") {
        "QEMU Raspberry Pi 5 (raspi5b)"
    } else {
        "Raspberry Pi 5 hardware"
    }
}

/// Network link description for the Network page.
#[must_use]
pub fn link_status() -> &'static str {
    #[cfg(feature = "qemu_raspi5b")]
    {
        if crate::drivers::lobster_net::is_available() {
            return "Connected through the host";
        }
    }
    "No network device"
}

/// Whether there is a network device to test.
#[must_use]
pub fn has_network() -> bool {
    link_status() != "No network device"
}

/// Result line of fetching `url`.
#[must_use]
pub fn fetch_report(url: &str) -> String {
    match crate::net::fetch_url(url) {
        Some((status, body)) => {
            alloc::format!("OK: HTTP {} - {} bytes received", status, body.len())
        }
        None => String::from("Failed: no response from the server"),
    }
}
