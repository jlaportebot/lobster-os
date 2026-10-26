//! QEMU MMIO Input Driver — reads input events from the custom QEMU
//! lobster-input device's MMIO ring buffer.
//!
//! When running under the custom QEMU raspi5b build with the lobster-input
//! device mapped at 0x7d008000, this driver polls the ring buffer and
//! converts events to the same HidEvent format used by the USB HID stack.
//!
//! MMIO Layout (must match hw/misc/lobster_input.c):
//!   0x00  MAGIC       — 0x4C4F4231 ("LOB1")
//!   0x04  VERSION     — 0x00010000
//!   0x08  BUF_SIZE    — 32
//!   0x0C  HEAD        — guest read index
//!   0x10  TAIL        — host write index
//!   0x14  EVENT_SIZE  — 16 bytes per slot
//!   0x18  CLEAR       — write to reset head=tail
//!   0x20+ EVENT_DATA  — 32 × 16-byte slots

use crate::usb::hid::HidEvent;

/// MMIO base address for the lobster-input device
const INPUT_MMIO_BASE: u64 = 0x7D00_8000;

/// Magic value identifying the device
const EXPECTED_MAGIC: u32 = 0x4C4F4231;

/// Ring buffer capacity
const RING_SIZE: usize = 32;

/// Event slot size in bytes
const SLOT_SIZE: usize = 16;

/// Event data region starts at this offset
const EVENT_DATA_OFFSET: u64 = 0x20;

/// Event type constants (match QEMU device)
const EV_MOUSE_MOVE: u8 = 1;
const EV_MOUSE_BTN: u8 = 2;
const EV_KEY_PRESS: u8 = 3;
const EV_KEY_RELEASE: u8 = 4;

/// Modifier flag bits
const MOD_SHIFT: u8 = 0x01;
const MOD_CTRL: u8 = 0x02;
const MOD_ALT: u8 = 0x04;
const MOD_GUI: u8 = 0x08;

/// Track whether we've verified the device is present
static mut DEVICE_PRESENT: bool = false;
static mut DEVICE_CHECKED: bool = false;

/// Current modifier state (tracked from key events)
static mut MODIFIERS: u8 = 0;

/// Previous absolute mouse position (for computing deltas)
static mut PREV_X: i32 = 960;
static mut PREV_Y: i32 = 540;
static mut FIRST_MOVE: bool = true;

/// Raw event slot — 16 bytes, matches the C struct layout
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct RawEvent {
    ev_type: u8,
    flags: u8,
    x: u16,
    y: u16,
    dx: i16,
    dy: i16,
    btn: u8,
    key: u8,
    reserved: [u8; 6],
}

/// Read a 32-bit MMIO register
unsafe fn mmio_read32(addr: u64) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

/// Write a 32-bit MMIO register
unsafe fn mmio_write32(addr: u64, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

/// Read a raw event slot from the ring buffer
unsafe fn read_event_slot(slot: usize) -> RawEvent {
    let addr = INPUT_MMIO_BASE + EVENT_DATA_OFFSET + (slot as u64) * (SLOT_SIZE as u64);
    let ptr = addr as *const RawEvent;
    core::ptr::read_volatile(ptr)
}

/// Check if the lobster-input MMIO device is present
pub fn is_present() -> bool {
    unsafe {
        if DEVICE_CHECKED {
            return DEVICE_PRESENT;
        }
        DEVICE_CHECKED = true;
        let magic = mmio_read32(INPUT_MMIO_BASE);
        DEVICE_PRESENT = magic == EXPECTED_MAGIC;
        DEVICE_PRESENT
    }
}

/// Poll the QEMU MMIO input device for one event.
/// Returns Some(HidEvent) if an event is available, None if the ring
/// buffer is empty or the device is not present.
pub fn poll() -> Option<HidEvent> {
    if !is_present() {
        return None;
    }

    unsafe {
        let head = mmio_read32(INPUT_MMIO_BASE + 0x0C) as usize;
        let tail = mmio_read32(INPUT_MMIO_BASE + 0x10) as usize;

        if head == tail {
            return None; // ring empty
        }

        let raw = read_event_slot(head);

        // Advance head (guest-side read pointer). Write to ADVANCE_HEAD
        // (0x1C), NOT CLEAR (0x18) — CLEAR resets the whole ring and
        // would drop every unconsumed event behind this one.
        let new_head = (head + 1) % RING_SIZE;
        mmio_write32(INPUT_MMIO_BASE + 0x1C, new_head as u32);

        // Update modifier tracking
        MODIFIERS = raw.flags;

        // Convert raw event to HidEvent
        // Read fields individually to avoid alignment issues with packed struct
        let ev_type = raw.ev_type;
        let flags = raw.flags;
        let x = raw.x;
        let y = raw.y;
        let dx_raw = raw.dx;
        let dy_raw = raw.dy;
        let btn = raw.btn;
        let key = raw.key;

        match ev_type {
            EV_MOUSE_MOVE => {
                // QEMU sends absolute x/y; compute delta for the WM
                let cur_x = x as i32;
                let cur_y = y as i32;
                // The QEMU device reports ABSOLUTE positions. On the first
                // event, jump the cursor to the absolute position (the WM
                // cursor starts at screen center, which would otherwise
                // eat the first coordinate as a zero-delta baseline).
                let (dx, dy) = if FIRST_MOVE {
                    FIRST_MOVE = false;
                    PREV_X = cur_x;
                    PREV_Y = cur_y;
                    // The WM cursor starts at screen center — compute a
                    // REAL delta so absolute positions take effect from the
                    // very first event.
                    let (wx, wy) = crate::wm::input::cursor_pos();
                    ((cur_x - wx as i32) as i16, (cur_y - wy as i32) as i16)
                } else {
                    let dx = cur_x - PREV_X;
                    let dy = cur_y - PREV_Y;
                    PREV_X = cur_x;
                    PREV_Y = cur_y;
                    (dx as i16, dy as i16)
                };
                Some(HidEvent::MouseMove { dx, dy })
            }
            EV_MOUSE_BTN => Some(HidEvent::MouseButton {
                left: (btn & 0x01) != 0,
                right: (btn & 0x02) != 0,
                middle: (btn & 0x04) != 0,
            }),
            EV_KEY_PRESS => Some(HidEvent::KeyPress {
                scancode: key,
                extended: false,
            }),
            EV_KEY_RELEASE => Some(HidEvent::KeyRelease {
                scancode: key,
                extended: false,
            }),
            _ => None,
        }
    }
}

/// Get current modifier state from the QEMU input device
pub fn modifiers() -> u8 {
    unsafe { MODIFIERS }
}

/// Check if shift is held
pub fn shift_held() -> bool {
    (modifiers() & MOD_SHIFT) != 0
}

/// Check if ctrl is held
pub fn ctrl_held() -> bool {
    (modifiers() & MOD_CTRL) != 0
}

/// Check if alt is held
pub fn alt_held() -> bool {
    (modifiers() & MOD_ALT) != 0
}

/// Check if gui (super/meta) is held
pub fn gui_held() -> bool {
    (modifiers() & MOD_GUI) != 0
}

/// Initialize the QEMU input device — verify presence and log
pub fn init() {
    if is_present() {
        crate::println!(
            "[qemu-input] LobsterOS MMIO input device detected at {:#010x}",
            INPUT_MMIO_BASE
        );
    } else {
        crate::println!(
            "[qemu-input] No MMIO input device at {:#010x} (magic={:#010x})",
            INPUT_MMIO_BASE,
            unsafe { mmio_read32(INPUT_MMIO_BASE) }
        );
    }
}
