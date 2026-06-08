//! Drivers module — hardware peripherals

#[cfg(not(feature = "qemu"))]
pub mod audio;
#[cfg(not(feature = "qemu"))]
pub mod emmc;
pub mod gpio;
#[cfg(not(feature = "qemu"))]
pub mod gpu;
#[cfg(not(feature = "qemu"))]
pub mod mailbox;
#[cfg(not(feature = "qemu"))]
pub mod nvme;
#[cfg(not(feature = "qemu"))]
pub mod pcie;
pub mod power;
#[cfg(not(feature = "qemu"))]
pub mod sdio;
pub mod serial_input;
pub mod uart;
#[cfg(not(feature = "qemu"))]
pub mod wifi;
