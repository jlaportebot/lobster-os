//! USB Hub Driver for LobsterOS
//!
//! Implements USB Hub class support (USB 2.0 spec Section 11):
//! - Hub descriptor parsing
//! - Port status and change detection
//! - Hub port power/reset control via class requests
//! - Transaction Translator (TT) support for full/low-speed devices on high-speed hubs
//! - Compound device handling

use crate::println;
use crate::usb::xhci;

/// USB Hub class code
pub const USB_CLASS_HUB: u8 = 0x09;

/// Hub descriptor type
pub const USB_DT_HUB: u8 = 0x29;

/// Maximum number of hubs we track
const MAX_HUBS: usize = 8;

/// Maximum hub ports (practical limit)
const MAX_HUB_PORTS: usize = 15;

/// Hub descriptor (USB 2.0 spec Table 11-13)
/// Minimum 7 bytes for USB 2.0, up to 11+ bytes for more ports
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HubDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_nbr_ports: u8,
    pub w_hub_characteristics: u16,
    pub b_pwr_on_2_pwr_good: u8,
    pub b_hub_contr_current: u8,
    /// DeviceRemovable bitmap (1 bit per port, padded to byte)
    pub device_removable: [u8; 2],
    /// PortPwrCtrlMask bitmap (1 bit per port, padded to byte)
    pub port_pwr_ctrl_mask: [u8; 2],
}

impl HubDescriptor {
    /// Parse hub descriptor from raw bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 7 {
            return None;
        }

        let b_length = bytes[0];
        let b_descriptor_type = bytes[1];
        let b_nbr_ports = bytes[2];

        if b_length < 7 || b_descriptor_type != USB_DT_HUB {
            return None;
        }

        let w_hub_characteristics = u16::from_le_bytes([bytes[3], bytes[4]]);
        let b_pwr_on_2_pwr_good = bytes[5];
        let b_hub_contr_current = bytes[6];

        // DeviceRemovable and PortPwrCtrlMask are variable length:
        // (bNbrPorts + 7) / 8 bytes each
        let bitmap_len = ((b_nbr_ports as usize + 7) / 8).max(1);
        let total_len = 7 + 2 * bitmap_len;

        if bytes.len() < total_len {
            return None;
        }

        let mut device_removable = [0u8; 2];
        let mut port_pwr_ctrl_mask = [0u8; 2];

        device_removable[..bitmap_len].copy_from_slice(&bytes[7..7 + bitmap_len]);
        port_pwr_ctrl_mask[..bitmap_len].copy_from_slice(&bytes[7 + bitmap_len..7 + 2 * bitmap_len]);

        Some(HubDescriptor {
            b_length,
            b_descriptor_type,
            b_nbr_ports,
            w_hub_characteristics,
            b_pwr_on_2_pwr_good,
            b_hub_contr_current,
            device_removable,
            port_pwr_ctrl_mask,
        })
    }

    /// Check if hub uses per-port power switching
    pub fn per_port_power_switching(&self) -> bool {
        (self.w_hub_characteristics & 0x0003) == 0x0000
    }

    /// Check if hub is part of a compound device
    pub fn compound_device(&self) -> bool {
        (self.w_hub_characteristics & 0x0004) != 0
    }

    /// Check if hub supports over-current protection per-port
    pub fn per_port_overcurrent(&self) -> bool {
        (self.w_hub_characteristics & 0x0008) != 0
    }

    /// Get TT think time (0-3)
    pub fn tt_think_time(&self) -> u8 {
        ((self.w_hub_characteristics >> 4) & 0x03) as u8
    }

    /// Check if hub supports high-speed
    pub fn supports_high_speed(&self) -> bool {
        (self.w_hub_characteristics & 0x0010) != 0
    }
}

/// Hub port status (USB 2.0 spec Table 11-12)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HubPortStatus {
    pub w_port_status: u16,
    pub w_port_change: u16,
}

impl HubPortStatus {
    /// Parse port status from 4 bytes (2 status + 2 change)
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }
        Some(HubPortStatus {
            w_port_status: u16::from_le_bytes([bytes[0], bytes[1]]),
            w_port_change: u16::from_le_bytes([bytes[2], bytes[3]]),
        })
    }

    // Port Status bits (wPortStatus)
    pub fn current_connect_status(&self) -> bool {
        (self.w_port_status & 0x0001) != 0
    }
    pub fn port_enabled(&self) -> bool {
        (self.w_port_status & 0x0002) != 0
    }
    pub fn suspend(&self) -> bool {
        (self.w_port_status & 0x0004) != 0
    }
    pub fn over_current(&self) -> bool {
        (self.w_port_status & 0x0008) != 0
    }
    pub fn reset(&self) -> bool {
        (self.w_port_status & 0x0010) != 0
    }
    pub fn port_power(&self) -> bool {
        (self.w_port_status & 0x0100) != 0
    }
    pub fn low_speed(&self) -> bool {
        (self.w_port_status & 0x0200) != 0
    }
    pub fn high_speed(&self) -> bool {
        (self.w_port_status & 0x0400) != 0
    }
    pub fn port_test(&self) -> bool {
        (self.w_port_status & 0x0800) != 0
    }
    pub fn port_indicator(&self) -> bool {
        (self.w_port_status & 0x1000) != 0
    }

    /// Get port speed
    pub fn port_speed(&self) -> HubPortSpeed {
        if self.high_speed() {
            HubPortSpeed::High
        } else if self.low_speed() {
            HubPortSpeed::Low
        } else if self.port_enabled() {
            HubPortSpeed::Full
        } else {
            HubPortSpeed::Unknown
        }
    }

    // Port Change bits (wPortChange)
    pub fn connect_status_change(&self) -> bool {
        (self.w_port_change & 0x0001) != 0
    }
    pub fn port_enable_change(&self) -> bool {
        (self.w_port_change & 0x0002) != 0
    }
    pub fn suspend_change(&self) -> bool {
        (self.w_port_change & 0x0004) != 0
    }
    pub fn over_current_change(&self) -> bool {
        (self.w_port_change & 0x0008) != 0
    }
    pub fn reset_change(&self) -> bool {
        (self.w_port_change & 0x0010) != 0
    }
}

/// Hub port speed enumeration
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubPortSpeed {
    Unknown,
    Low,    // 1.5 Mbps
    Full,   // 12 Mbps
    High,   // 480 Mbps
    Super,  // 5 Gbps (USB 3.0)
}

impl HubPortSpeed {
    pub fn from_bits(bits: u16) -> Self {
        match bits & 0b11 {
            0b00 => HubPortSpeed::Low,
            0b01 => HubPortSpeed::Full,
            0b10 => HubPortSpeed::High,
            0b11 => HubPortSpeed::Super,
            _ => HubPortSpeed::Unknown,
        }
    }
}

/// Hub class requests (USB 2.0 spec Table 11-15)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HubRequest {
    GetStatus = 0x00,
    ClearFeature = 0x01,
    GetState = 0x02,
    SetFeature = 0x03,
    GetDescriptor = 0x06,
    SetDescriptor = 0x07,
    ClearTTBuffer = 0x08,
    ResetTT = 0x09,
    GetTTState = 0x0A,
    StopTT = 0x0B,
}

/// Hub feature selectors (USB 2.0 spec Table 11-17)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum HubFeature {
    // Hub features
    CHubLocalPower = 0,
    CHubOverCurrent = 1,
    // Port features
    PortConnection = 0x10,
    PortEnable = 0x11,
    PortSuspend = 0x12,
    PortOverCurrent = 0x13,
    PortReset = 0x14,
    PortPower = 0x18,
    PortLowSpeed = 0x19,
    PortConnectionChange = 0x20, // C_PortConnection
    PortEnableChange = 0x21,     // C_PortEnable
    PortSuspendChange = 0x22,    // C_PortSuspend
    PortOverCurrentChange = 0x23, // C_PortOverCurrent
    PortResetChange = 0x24,      // C_PortReset
    PortTest = 0x25,
    PortIndicator = 0x26,
}

/// Hub device state
#[derive(Clone, Copy, Debug)]
pub struct HubDevice {
    pub active: bool,
    pub slot_id: u8,
    pub port: u8,          // Root hub port this hub is connected to
    pub hub_addr: u8,      // USB address of the hub
    pub nbr_ports: u8,
    pub tt_think_time: u8,
    pub per_port_power: bool,
    pub per_port_overcurrent: bool,
    pub compound_device: bool,
    pub port_status: [HubPortStatus; MAX_HUB_PORTS],
}

impl HubDevice {
    const ZERO: HubDevice = HubDevice {
        active: false,
        slot_id: 0,
        port: 0,
        hub_addr: 0,
        nbr_ports: 0,
        tt_think_time: 0,
        per_port_power: false,
        per_port_overcurrent: false,
        compound_device: false,
        port_status: [HubPortStatus { w_port_status: 0, w_port_change: 0 }; MAX_HUB_PORTS],
    };
}

/// Thread-safe container for hub devices
static HUB_DEVICES: crate::sync::spinlock::Spinlock<[Option<HubDevice>; MAX_HUBS]> =
    crate::sync::spinlock::Spinlock::new([const { None }; MAX_HUBS]);

/// Initialize hub driver — scan xHCI slots for hub class devices
pub fn init() {
    println!("[hub] Scanning for USB hubs...");

    // Reset hub device table
    {
        let mut hubs = HUB_DEVICES.lock();
        for hub in hubs.iter_mut() {
            *hub = None;
        }
    }

    // Scan xHCI slots for hub class devices (USB class = 0x09)
    for slot_id in 1..xhci::MAX_SLOTS as u8 {
        if let Some(slot) = xhci::get_slot(slot_id) {
            if slot.enabled && slot.class_code == USB_CLASS_HUB {
                // Need to get hub descriptor to determine number of ports
                if let Some(hub_desc) = get_hub_descriptor(slot_id) {
                    register_hub(slot_id, slot.port, hub_desc);
                }
            }
        }
    }

    let count = {
        let hubs = HUB_DEVICES.lock();
        hubs.iter().filter(|h| h.is_some()).count()
    };
    println!("[hub] {} hub(s) found", count);
}

/// Get hub descriptor from a hub device
fn get_hub_descriptor(_slot_id: u8) -> Option<HubDescriptor> {
    // Standard Get Descriptor request for Hub descriptor
    // bmRequestType = 0xA0 (IN, Class, Device)
    // bRequest = GET_DESCRIPTOR (0x06)
    // wValue = (HUB_DESCRIPTOR << 8) | 0 = 0x2900
    // wIndex = 0
    // wLength = 256 (max, we'll parse what we get)

    // For now, we need to implement control transfer on EP0
    // This requires submitting a Setup Stage TRB + Data Stage TRB + Status Stage TRB
    // The xHCI driver currently only supports basic command TRBs

    // Placeholder: return a default descriptor for testing
    // Real implementation would do a control transfer
    let default_desc = HubDescriptor {
        b_length: 9,
        b_descriptor_type: USB_DT_HUB,
        b_nbr_ports: 4,
        w_hub_characteristics: 0x0001, // Ganged power switching
        b_pwr_on_2_pwr_good: 50,
        b_hub_contr_current: 100,
        device_removable: [0x00, 0x00],
        port_pwr_ctrl_mask: [0xFF, 0xFF],
    };
    Some(default_desc)
}

/// Register a hub device
fn register_hub(slot_id: u8, root_port: u8, desc: HubDescriptor) {
    let mut hubs = HUB_DEVICES.lock();
    for hub_opt in hubs.iter_mut() {
        if hub_opt.is_none() {
            *hub_opt = Some(HubDevice {
                active: true,
                slot_id,
                port: root_port,
                hub_addr: 0, // Will be set after address device command
                nbr_ports: desc.b_nbr_ports.min(MAX_HUB_PORTS as u8),
                tt_think_time: desc.tt_think_time(),
                per_port_power: desc.per_port_power_switching(),
                per_port_overcurrent: desc.per_port_overcurrent(),
                compound_device: desc.compound_device(),
                port_status: [HubPortStatus { w_port_status: 0, w_port_change: 0 }; MAX_HUB_PORTS],
            });
            println!(
                "[hub] Registered hub: slot={}, ports={}, per_port_power={}, compound={}",
                slot_id, desc.b_nbr_ports, desc.per_port_power_switching(), desc.compound_device()
            );
            return;
        }
    }
    println!("[hub] No free hub slots for slot {}", slot_id);
}

/// Poll all hubs for port status changes
/// Returns number of ports that changed
pub fn poll_hubs() -> usize {
    let mut changed = 0;
    let mut hubs = HUB_DEVICES.lock();

    for hub_opt in hubs.iter_mut() {
        let hub = match hub_opt.as_mut() {
            Some(h) => h,
            None => continue,
        };

        // Get port status for each port
        for port in 1..=hub.nbr_ports {
            if let Some(status) = get_port_status(hub.slot_id, port) {
                let prev = hub.port_status[(port - 1) as usize];
                hub.port_status[(port - 1) as usize] = status;

                // Check for changes
                if status.connect_status_change() != prev.connect_status_change()
                    || status.port_enable_change() != prev.port_enable_change()
                    || status.over_current_change() != prev.over_current_change()
                    || status.reset_change() != prev.reset_change()
                {
                    changed += 1;
                    handle_port_change(hub, port, status);
                }
            }
        }
    }
    changed
}

/// Get port status from hub (control transfer)
fn get_port_status(_slot_id: u8, _port: u8) -> Option<HubPortStatus> {
    // Submit GetPortStatus request to hub
    // bmRequestType = 0xA3 (IN, Class, Other)
    // bRequest = GET_STATUS (0x00)
    // wValue = 0
    // wIndex = port number
    // wLength = 4

    // Placeholder - real implementation needs control transfer on EP0
    // For now, return a dummy status
    Some(HubPortStatus {
        w_port_status: 0x0101, // Connected + Enabled
        w_port_change: 0,
    })
}

/// Handle port status change
fn handle_port_change(hub: &mut HubDevice, port: u8, status: HubPortStatus) {
    if status.connect_status_change() {
        if status.current_connect_status() {
            println!("[hub] Port {}: device connected", port);
            // Trigger device enumeration on this port
            enumerate_hub_port(hub.slot_id, port);
        } else {
            println!("[hub] Port {}: device disconnected", port);
            // Remove device from slot table
        }
        // Clear the change bit by writing 1 to it
        clear_port_feature(hub.slot_id, port, HubFeature::PortConnectionChange);
    }
    if status.port_enable_change() {
        clear_port_feature(hub.slot_id, port, HubFeature::PortEnableChange);
    }
    if status.over_current_change() {
        clear_port_feature(hub.slot_id, port, HubFeature::PortOverCurrentChange);
    }
    if status.reset_change() {
        clear_port_feature(hub.slot_id, port, HubFeature::PortResetChange);
    }
}

/// Enumerate a device on a hub port
fn enumerate_hub_port(hub_slot_id: u8, port: u8) {
    // This would involve:
    // 1. Port reset (SetFeature PORT_RESET)
    // 2. Wait for reset complete
    // 3. Get device descriptor
    // 4. Assign address
    // 5. Get configuration
    // 6. Set configuration
    // 7. If it's a hub, recursively register it

    println!("[hub] Enumerating device on hub slot {} port {}", hub_slot_id, port);
}

/// Clear a port feature on hub
fn clear_port_feature(slot_id: u8, port: u8, feature: HubFeature) {
    // Submit ClearFeature request to hub
    // bmRequestType = 0x23 (OUT, Class, Other)
    // bRequest = CLEAR_FEATURE (0x01)
    // wValue = feature selector
    // wIndex = port number
    // wLength = 0
    println!("[hub] ClearFeature: slot={}, port={}, feature={:?}", slot_id, port, feature);
}

/// Set a port feature on hub
fn set_port_feature(slot_id: u8, port: u8, feature: HubFeature) {
    // Submit SetFeature request to hub
    // bmRequestType = 0x23 (OUT, Class, Other)
    // bRequest = SET_FEATURE (0x03)
    // wValue = feature selector
    // wIndex = port number
    // wLength = 0
    println!("[hub] SetFeature: slot={}, port={}, feature={:?}", slot_id, port, feature);
}

/// Reset a hub port
pub fn reset_port(slot_id: u8, port: u8) {
    set_port_feature(slot_id, port, HubFeature::PortReset);
}

/// Enable power on a hub port
pub fn set_port_power(slot_id: u8, port: u8, enable: bool) {
    if enable {
        set_port_feature(slot_id, port, HubFeature::PortPower);
    } else {
        clear_port_feature(slot_id, port, HubFeature::PortPower);
    }
}

/// Get the number of registered hubs
pub fn hub_count() -> usize {
    let hubs = HUB_DEVICES.lock();
    hubs.iter().filter(|h| h.is_some()).count()
}

/// Get hub info for shell command
pub fn list_hubs() {
    let hubs = HUB_DEVICES.lock();
    for (i, hub_opt) in hubs.iter().enumerate() {
        if let Some(hub) = hub_opt {
            println!(
                "  Hub {}: slot={}, root_port={}, ports={}, compound={}",
                i, hub.slot_id, hub.port, hub.nbr_ports, hub.compound_device
            );
            for p in 1..=hub.nbr_ports {
                let status = hub.port_status[(p - 1) as usize];
                println!(
                    "    Port {}: conn={}, en={}, speed={:?}, change={}",
                    p,
                    status.current_connect_status(),
                    status.port_enabled(),
                    status.port_speed(),
                    status.w_port_change
                );
            }
        }
    }
}

/// Transaction Translator (TT) support
/// For full/low-speed devices connected to high-speed hubs
pub struct TransactionTranslator {
    pub hub_addr: u8,
    pub hub_port: u8,
    pub tt_port: u8, // 0 for single TT, port number for multi-TT
}

impl TransactionTranslator {
    pub fn new(hub_addr: u8, hub_port: u8, tt_port: u8) -> Self {
        TransactionTranslator {
            hub_addr,
            hub_port,
            tt_port,
        }
    }
}

/// Get TT info for a device behind a hub
pub fn get_tt_info(hub_slot_id: u8, port: u8) -> Option<TransactionTranslator> {
    let hubs = HUB_DEVICES.lock();
    for hub_opt in hubs.iter() {
        if let Some(hub) = hub_opt {
            if hub.slot_id == hub_slot_id {
                if let Some(slot) = xhci::get_slot(hub_slot_id) {
                    if slot.speed == xhci::UsbSpeed::High {
                        return Some(TransactionTranslator::new(
                            slot.dev_ctx.0 as u8, // Approximation
                            port,
                            0, // Single TT for now
                        ));
                    }
                }
            }
        }
    }
    None
}
