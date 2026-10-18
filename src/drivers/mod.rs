//! Drivers module — hardware peripherals

pub mod audio;
#[cfg(target_arch = "aarch64")]
pub mod audio_mixer;
#[cfg(target_arch = "aarch64")]
pub mod bluetooth;
#[cfg(target_arch = "aarch64")]
pub mod emmc;
#[cfg(target_arch = "aarch64")]
pub mod gpio;
#[cfg(target_arch = "aarch64")]
pub mod gpio_user;
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
pub mod power_mgmt;
#[cfg(target_arch = "aarch64")]
pub mod qemu_input;
#[cfg(target_arch = "aarch64")]
pub mod sdio;
pub mod serial_input;
pub mod uart;

#[cfg(all(target_arch = "aarch64", feature = "qemu"))]
pub mod fast_serial;

#[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
pub mod lobster_storage;

#[cfg(target_arch = "aarch64")]
pub mod lobster_net;
#[cfg(target_arch = "aarch64")]
pub mod wifi;
