//! QEMU compatibility layer — provides alternative addresses and stubs
//! for BCM2712-specific hardware when building with the "qemu" feature
//! for raspi4b emulation.
//!
//! QEMU raspi4b emulates BCM2711 (Pi 4), which has different peripheral
//! addresses from BCM2712 (Pi 5). This module bridges the gap.

// ─── GIC-400 Addresses ───────────────────────────────────────────────
// QEMU raspi4b: GIC-400 at 0xFF841000/0xFF842000
// Real Pi 5:    GIC-400 at 0x10_7FFF_9000/0x10_7FFF_A000

#[cfg(feature = "qemu_raspi5b")]
pub const GICD_BASE: u64 = 0xFF84_1000;
#[cfg(feature = "qemu_raspi5b")]
pub const GICC_BASE: u64 = 0xFF84_2000;

#[cfg(not(feature = "qemu_raspi5b"))]
pub const GICD_BASE: u64 = 0x10_7FFF_9000;
#[cfg(not(feature = "qemu_raspi5b"))]
pub const GICC_BASE: u64 = 0x10_7FFF_A000;

// ─── System Timer ─────────────────────────────────────────────────────
// QEMU raspi4b: BCM2835-style system timer at 0xFE000300
// Real Pi 5:    BCM2712 system timer at 0x10_7C00_3000

#[cfg(feature = "qemu_raspi5b")]
pub const TIMER_BASE: u64 = 0xFE_000_300;
#[cfg(not(feature = "qemu_raspi5b"))]
pub const TIMER_BASE: u64 = 0x10_7C00_3000;

// ─── Peripheral Base ─────────────────────────────────────────────────
#[cfg(feature = "qemu_raspi5b")]
pub const PERIPHERAL_BASE: u64 = 0xFE_000_000; // BCM2711 I/O base
#[cfg(not(feature = "qemu_raspi5b"))]
pub const PERIPHERAL_BASE: u64 = 0x10_0000_0000; // BCM2712 I/O base

/// Check if running under QEMU
#[cfg(feature = "qemu_raspi5b")]
pub const fn is_qemu() -> bool { true }

#[cfg(not(feature = "qemu_raspi5b"))]
pub const fn is_qemu() -> bool { false }
