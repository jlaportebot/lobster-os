//! HID (Human Interface Device) driver for LobsterOS
//!
//! USB HID class driver implementing boot protocol parsing
//! for keyboards and mice.
//!
//! - Keyboard boot protocol: byte 0 = modifier keys, bytes 2-7 = keycodes
//! - Mouse boot protocol: byte 0 = buttons, bytes 1-2 = X/Y displacement

use crate::println;
use crate::sync::spinlock::Spinlock;
use crate::usb::xhci;

// ─── HID Types ─────────────────────────────────────────────────────────

/// HID device type
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HidType {
    Keyboard,
    Mouse,
    Gamepad,
    Unknown,
}

/// HID event
#[derive(Clone, Copy, Debug)]
pub enum HidEvent {
    KeyPress {
        scancode: u8,
        extended: bool,
    },
    KeyRelease {
        scancode: u8,
        extended: bool,
    },
    MouseMove {
        dx: i16,
        dy: i16,
    },
    MouseButton {
        left: bool,
        right: bool,
        middle: bool,
    },
}

/// Maximum number of HID devices we track
const MAX_HID_DEVICES: usize = 8;

/// HID device state
struct HidDevice {
    active: bool,
    slot_id: u8,
    ep_addr: u8,
    dev_type: HidType,
    report_buf: [u8; 8],
    prev_report: [u8; 8],
    report_len: usize,
}

impl HidDevice {
    const ZERO: HidDevice = HidDevice {
        active: false,
        slot_id: 0,
        ep_addr: 0,
        dev_type: HidType::Unknown,
        report_buf: [0; 8],
        prev_report: [0; 8],
        report_len: 0,
    };
}

/// Thread-safe container for HID devices
static HID_DEVICES: Spinlock<[Option<HidDevice>; MAX_HID_DEVICES]> =
    Spinlock::new([const { None }; MAX_HID_DEVICES]);

// ─── Keyboard Modifier Bits ────────────────────────────────────────────

const MOD_LCTRL: u8 = 1 << 0;
const MOD_LSHIFT: u8 = 1 << 1;
const MOD_LALT: u8 = 1 << 2;
const MOD_LGUI: u8 = 1 << 3;
const MOD_RCTRL: u8 = 1 << 4;
const MOD_RSHIFT: u8 = 1 << 5;
const MOD_RALT: u8 = 1 << 6;
const MOD_RGUI: u8 = 1 << 7;

// ─── US Keyboard Scancode → ASCII Table ────────────────────────────────

/// Scan code set 2 → ASCII (unshifted)
const SC_UNSHIFTED: [u8; 128] = [
    0x00, 0x1B, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x30, 0x2D, 0x3D, 0x08, 0x09,
    0x71, 0x77, 0x65, 0x72, 0x74, 0x79, 0x75, 0x69, 0x6F, 0x70, 0x5B, 0x5D, 0x0A, 0x00, 0x61, 0x73,
    0x64, 0x66, 0x67, 0x68, 0x6A, 0x6B, 0x6C, 0x3B, 0x27, 0x60, 0x00, 0x5C, 0x7A, 0x78, 0x63, 0x76,
    0x62, 0x6E, 0x6D, 0x2C, 0x2E, 0x2F, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Scan code set 2 → ASCII (shifted)
const SC_SHIFTED: [u8; 128] = [
    0x00, 0x1B, 0x21, 0x40, 0x23, 0x24, 0x25, 0x5E, 0x26, 0x2A, 0x28, 0x29, 0x5F, 0x2B, 0x08, 0x09,
    0x51, 0x57, 0x45, 0x52, 0x54, 0x59, 0x55, 0x49, 0x4F, 0x50, 0x7B, 0x7D, 0x0A, 0x00, 0x41, 0x53,
    0x44, 0x46, 0x47, 0x48, 0x4A, 0x4B, 0x4C, 0x3A, 0x22, 0x7E, 0x00, 0x7C, 0x5A, 0x58, 0x43, 0x56,
    0x42, 0x4E, 0x4D, 0x3C, 0x3E, 0x3F, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

// ─── Initialization ────────────────────────────────────────────────────

/// Initialize HID driver — scan xHCI slots for HID class devices
pub fn init() {
    println!("[hid] Scanning for HID devices...");

    // Reset device table
    {
        let mut devices = HID_DEVICES.lock();
        for dev in devices.iter_mut() {
            *dev = None;
        }
    }

    // Scan xHCI slots for HID class devices (USB class = 0x03)
    for slot_id in 1..xhci::MAX_SLOTS as u8 {
        if let Some(slot) = xhci::get_slot(slot_id) {
            if slot.enabled && slot.class_code == 0x03 {
                register_device(slot_id, slot.subclass, slot.protocol);
            }
        }
    }

    let count = {
        let devices = HID_DEVICES.lock();
        devices.iter().filter(|d| d.is_some()).count()
    };
    println!("[hid] {} HID device(s) found", count);
}

/// Register an HID device by class/subclass/protocol
fn register_device(slot_id: u8, subclass: u8, protocol: u8) {
    let dev_type = match (subclass, protocol) {
        (0x01, 0x01) => HidType::Keyboard, // Boot keyboard
        (0x01, 0x02) => HidType::Mouse,    // Boot mouse
        (0x01, _) => HidType::Keyboard,    // Other keyboard
        (_, 0x02) => HidType::Mouse,       // Other mouse
        _ => HidType::Unknown,
    };

    // Find a free slot in HID_DEVICES
    let mut devices = HID_DEVICES.lock();
    for dev in devices.iter_mut() {
        if dev.is_none() {
            *dev = Some(HidDevice {
                active: true,
                slot_id,
                ep_addr: 0x81, // Default EP1 IN
                dev_type,
                report_buf: [0; 8],
                prev_report: [0; 8],
                report_len: match dev_type {
                    HidType::Keyboard => 8,
                    HidType::Mouse => 4,
                    _ => 8,
                },
            });
            println!("[hid] Registered {:?} at slot={}", dev_type, slot_id);
            return;
        }
    }
    println!("[hid] No free HID device slots for slot {}", slot_id);
}

// ─── Polling ───────────────────────────────────────────────────────────

/// Poll all HID devices for input events
/// Returns Some(HidEvent) if there is new input, None otherwise
pub fn poll() -> Option<HidEvent> {
    let mut devices = HID_DEVICES.lock();
    for dev_opt in devices.iter_mut() {
        let dev = match dev_opt.as_mut() {
            Some(d) => d,
            None => continue,
        };

        // Save previous report
        let prev = dev.prev_report;

        // In a real implementation, we would read from the interrupt
        // endpoint via xHCI transfer. For now, check if the report
        // buffer has changed (would be updated by transfer completion).
        // This is a placeholder — actual data comes from USB transfers.

        match dev.dev_type {
            HidType::Keyboard => {
                if let Some(evt) = process_keyboard(dev, &prev) {
                    return Some(evt);
                }
            }
            HidType::Mouse => {
                if let Some(evt) = process_mouse(dev, &prev) {
                    return Some(evt);
                }
            }
            _ => {}
        }
    }

    // Fallback: check QEMU MMIO input device (custom lobster-input device)
    // On QEMU raspi5b, there are no real USB HID devices, so all input
    // comes through this MMIO ring buffer.
    #[cfg(target_arch = "aarch64")]
    {
        if let Some(evt) = crate::drivers::qemu_input::poll() {
            return Some(evt);
        }
    }

    None
}

/// Process a keyboard report and generate events
fn process_keyboard(dev: &mut HidDevice, prev: &[u8; 8]) -> Option<HidEvent> {
    let report = &dev.report_buf;

    // Modifier keys changed?
    let mod_changed = report[0] != prev[0];

    // Check for new key presses (keycodes in bytes 2-7)
    for i in 2..8 {
        let keycode = report[i];
        if keycode == 0 {
            continue;
        }
        // Check if this keycode was NOT in the previous report
        let was_pressed = prev[2..8].iter().any(|&k| k == keycode);
        if !was_pressed && keycode < 128 {
            return Some(HidEvent::KeyPress {
                scancode: keycode,
                extended: mod_changed,
            });
        }
    }

    // Check for key releases
    for i in 2..8 {
        let prev_keycode = prev[i];
        if prev_keycode == 0 {
            continue;
        }
        let still_pressed = report[2..8].iter().any(|&k| k == prev_keycode);
        if !still_pressed && prev_keycode < 128 {
            return Some(HidEvent::KeyRelease {
                scancode: prev_keycode,
                extended: false,
            });
        }
    }

    None
}

/// Process a mouse report and generate events
fn process_mouse(dev: &mut HidDevice, prev: &[u8; 8]) -> Option<HidEvent> {
    let report = &dev.report_buf;

    // Check for button changes
    let prev_buttons = prev[0] & 0x07;
    let curr_buttons = report[0] & 0x07;
    if prev_buttons != curr_buttons {
        return Some(HidEvent::MouseButton {
            left: (curr_buttons & 1) != 0,
            right: (curr_buttons & 2) != 0,
            middle: (curr_buttons & 4) != 0,
        });
    }

    // Check for movement (X and Y as signed bytes)
    let dx = report[1] as i8 as i16;
    let dy = report[2] as i8 as i16;
    if dx != 0 || dy != 0 {
        return Some(HidEvent::MouseMove { dx, dy });
    }

    None
}

// ─── Scancode → ASCII ──────────────────────────────────────────────────

/// Convert a USB HID scancode to ASCII character
/// Returns None for non-printable keys (function keys, arrows, etc.)
pub fn scancode_to_ascii(scancode: u8, shift: bool) -> Option<char> {
    if scancode as usize >= SC_UNSHIFTED.len() {
        return None;
    }
    let byte = if shift {
        SC_SHIFTED[scancode as usize]
    } else {
        SC_UNSHIFTED[scancode as usize]
    };
    if byte == 0 {
        None
    } else {
        Some(byte as char)
    }
}

// ─── Update Report Buffer ──────────────────────────────────────────────

/// Update a device's report buffer (called from USB transfer completion)
pub fn update_report(slot_id: u8, data: &[u8]) {
    let mut devices = HID_DEVICES.lock();
    for dev_opt in devices.iter_mut() {
        let dev = match dev_opt.as_mut() {
            Some(d) => d,
            None => continue,
        };
        if dev.slot_id == slot_id {
            // Save previous report
            dev.prev_report = dev.report_buf;
            // Copy new data
            let len = data.len().min(dev.report_len).min(8);
            dev.report_buf[..len].copy_from_slice(&data[..len]);
            return;
        }
    }
}

// ─── Query Functions ───────────────────────────────────────────────────

/// Check if a keyboard is connected
pub fn has_keyboard() -> bool {
    let devices = HID_DEVICES.lock();
    devices.iter().any(|d| {
        d.as_ref()
            .map_or(false, |dev| dev.active && dev.dev_type == HidType::Keyboard)
    })
}

/// Check if a mouse is connected
pub fn has_mouse() -> bool {
    let devices = HID_DEVICES.lock();
    devices.iter().any(|d| {
        d.as_ref()
            .map_or(false, |dev| dev.active && dev.dev_type == HidType::Mouse)
    })
}

/// Get the modifier keys currently held (from the first keyboard)
pub fn get_modifiers() -> u8 {
    let devices = HID_DEVICES.lock();
    for dev_opt in devices.iter() {
        if let Some(dev) = dev_opt.as_ref() {
            if dev.active && dev.dev_type == HidType::Keyboard {
                return dev.report_buf[0];
            }
        }
    }
    // Fallback: QEMU MMIO input modifier state
    #[cfg(target_arch = "aarch64")]
    {
        if crate::drivers::qemu_input::is_present() {
            return crate::drivers::qemu_input::modifiers();
        }
    }
    0
}

/// Check if Shift is held
pub fn shift_held() -> bool {
    let mods = get_modifiers();
    (mods & (MOD_LSHIFT | MOD_RSHIFT)) != 0
}

/// Check if Ctrl is held
pub fn ctrl_held() -> bool {
    let mods = get_modifiers();
    (mods & (MOD_LCTRL | MOD_RCTRL)) != 0
}

/// Check if Alt is held
pub fn alt_held() -> bool {
    let mods = get_modifiers();
    (mods & (MOD_LALT | MOD_RALT)) != 0
}

/// Check if GUI (Super/Windows) key is held
pub fn gui_held() -> bool {
    let mods = get_modifiers();
    (mods & (MOD_LGUI | MOD_RGUI)) != 0
}
