//! Driver for the emulator's `lobster-sys` device (`0x7d50_0000`).
//!
//! The device is the guest's platform controller under the LobsterOS Pi 5
//! emulator: power off / reboot, the host's wall clock and timezone, a guest
//! lifecycle register the VM manager can read, and the resolution the VM
//! was configured with. Register layout: `hw/misc/lobster_sys.c` in the
//! QEMU fork.

const SYS_BASE: u64 = 0x7D50_0000;

const REG_MAGIC: u64 = 0x00;
const REG_POWER: u64 = 0x08;
const REG_STATE: u64 = 0x0C;
const REG_RTC_LO: u64 = 0x10;
const REG_RTC_HI: u64 = 0x14;
const REG_TZ_OFFSET: u64 = 0x18;
const REG_HINT_WIDTH: u64 = 0x20;
const REG_HINT_HEIGHT: u64 = 0x24;

const MAGIC: u32 = 0x4C53_5953; // "LSYS"
const POWER_OFF: u32 = 1;
const POWER_REBOOT: u32 = 2;

/// Guest lifecycle states published to the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum GuestState {
    Booting = 0,
    Desktop = 1,
    ShuttingDown = 2,
}

fn read32(offset: u64) -> u32 {
    // SAFETY: SYS_BASE..+0x1000 is the lobster-sys MMIO page, identity-mapped
    // as device memory with the rest of the 0x7dxx_xxxx peripheral window;
    // all registers are 32-bit and side-effect-free to read except RTC_LO,
    // whose only effect is latching RTC_HI.
    unsafe { core::ptr::read_volatile((SYS_BASE + offset) as *const u32) }
}

fn write32(offset: u64, value: u32) {
    // SAFETY: as in `read32`; writes are only made to POWER and STATE.
    unsafe { core::ptr::write_volatile((SYS_BASE + offset) as *mut u32, value) }
}

/// True when the emulator provides the device.
#[must_use]
pub fn detect() -> bool {
    read32(REG_MAGIC) == MAGIC
}

/// Host UTC time in epoch seconds and the host's UTC offset in seconds.
#[must_use]
pub fn wall_clock() -> Option<(u64, i32)> {
    if !detect() {
        return None;
    }
    let lo = u64::from(read32(REG_RTC_LO));
    let hi = u64::from(read32(REG_RTC_HI));
    let offset = read32(REG_TZ_OFFSET) as i32;
    Some(((hi << 32) | lo, offset))
}

/// The resolution the VM manager asked for, if any.
#[must_use]
pub fn preferred_resolution() -> Option<(u32, u32)> {
    if !detect() {
        return None;
    }
    let width = read32(REG_HINT_WIDTH);
    let height = read32(REG_HINT_HEIGHT);
    (width != 0 && height != 0).then_some((width, height))
}

/// Tell the host which lifecycle stage the guest reached.
pub fn publish_state(state: GuestState) {
    if detect() {
        write32(REG_STATE, state as u32);
    }
}

/// Ask the emulator to power the machine off. Returns only if the device is
/// absent (the request is asynchronous on the host, so callers should idle).
pub fn power_off() {
    if detect() {
        write32(REG_POWER, POWER_OFF);
    }
}

/// Ask the emulator to reset the machine.
pub fn reboot() {
    if detect() {
        write32(REG_POWER, POWER_REBOOT);
    }
}
