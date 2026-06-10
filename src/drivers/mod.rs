//! Drivers module — hardware peripherals
//! Drivers module — hardware peripherals

#[cfg(target_arch = "aarch64")]
pub mod emmc;
#[cfg(target_arch = "aarch64")]
pub mod gpio;
#[cfg(target_arch = "aarch64")]
pub mod gpu;
#[cfg(target_arch = "aarch64")]
pub mod mailbox;
#[cfg(target_arch = "aarch64")]
pub mod nvme;
#[cfg(target_arch = "aarch64")]
pub mod pcie;
#[cfg(target_arch = "aarch64")]
pub mod power;
#[cfg(target_arch = "aarch64")]
pub mod sdio;
#[cfg(target_arch = "aarch64")]
pub mod serial_input;
#[cfg(target_arch = "aarch64")]
pub mod uart;
#[cfg(target_arch = "aarch64")]
pub mod wifi;
pub mod audio;
