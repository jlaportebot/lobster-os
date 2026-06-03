//! USB subsystem for LobsterOS
//!
//! Provides xHCI host controller driver and HID (keyboard/mouse) support
//! for the BCM2712 SoC (Raspberry Pi 5).
//!
//! - xhci: xHCI host controller driver (command/event rings, enumeration)
//! - hid: HID boot-protocol driver (keyboard, mouse)

pub mod xhci;
pub mod hid;

use crate::println;

/// USB subsystem configuration
pub struct UsbConfig {
    pub xhci_enabled: bool,
    pub usb2_enabled: bool,
}

impl UsbConfig {
    pub const DEFAULT: UsbConfig = UsbConfig {
        xhci_enabled: true,
        usb2_enabled: false,
    };
}

/// Global USB state
static mut USB_CONFIG: UsbConfig = UsbConfig {
    xhci_enabled: true,
    usb2_enabled: false,
};

static mut USB_INITIALIZED: bool = false;

/// Initialize the USB subsystem
pub fn init() {
    println!("[usb] Initializing USB subsystem...");

    unsafe {
        USB_CONFIG = UsbConfig::DEFAULT;
    }

    if unsafe { USB_CONFIG.xhci_enabled } {
        println!("[usb] Bringing up xHCI controller...");
        match xhci::init() {
            Ok(()) => {
                println!("[usb] xHCI controller initialized");
                // Enumerate devices on the bus
                xhci::scan_ports();
            }
            Err(e) => {
                println!("[usb] xHCI init failed: {}", e);
                return;
            }
        }
    }

    // Initialize HID driver (keyboard/mouse)
    hid::init();

    unsafe {
        USB_INITIALIZED = true;
    }
    println!("[usb] Subsystem ready");
}

/// Poll all HID devices for events
/// Returns Some(HidEvent) if there is input, None otherwise
pub fn poll_hids() -> Option<hid::HidEvent> {
    if !unsafe { USB_INITIALIZED } {
        return None;
    }
    hid::poll()
}

/// Handle xHCI IRQ (SPI 48)
/// Called from arch::exception when the xHCI interrupt fires
pub fn handle_irq() {
    xhci::handle_event();
}

/// List all connected USB devices (for shell command "usb list")
pub fn list_devices() {
    if !unsafe { USB_INITIALIZED } {
        println!("usb: not initialized");
        return;
    }
    xhci::list_slots();
}

/// Reset the USB bus (for shell command "usb reset")
pub fn reset_bus() {
    if !unsafe { USB_INITIALIZED } {
        println!("usb: not initialized");
        return;
    }
    println!("[usb] Resetting bus...");
    if let Err(e) = xhci::reset_controller() {
        println!("[usb] Reset failed: {}", e);
    } else {
        println!("[usb] Bus reset complete, re-enumerating...");
        xhci::scan_ports();
        hid::init();
    }
}
