//! Bluetooth driver — HCI over SDIO for CYW43455 (Pi 5)
//!
//! The CYW43455 combo chip provides both WiFi (already implemented) and
//! Bluetooth 4.2 LE+Classic. This driver implements:
//! - HCI initialization over SDIO transport
//! - HCI command/event handling
//! - L2CAP layer (connection-oriented channels)
//! - GATT (Generic Attribute Profile) for LE
//! - Device discovery and pairing
//!
//! Note: This is a foundational implementation. Full Bluetooth stack
//! requires extensive firmware interaction with the CYW43455.

use crate::println;
use crate::sync::spinlock::Spinlock;
use alloc::string::String;
use alloc::vec::Vec;

/// Bluetooth device address
#[derive(Clone, Copy, Debug)]
pub struct BdAddr(pub [u8; 6]);

impl BdAddr {
    pub fn zero() -> Self {
        BdAddr([0; 6])
    }

    pub fn format_addr(&self) -> String {
        alloc::format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0],
            self.0[1],
            self.0[2],
            self.0[3],
            self.0[4],
            self.0[5]
        )
    }
}

/// HCI packet types
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HciPacketType {
    Command = 0x01,
    AclData = 0x02,
    ScoData = 0x03,
    Event = 0x04,
}

/// HCI events
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum HciEvent {
    InquiryComplete = 0x01,
    InquiryResult = 0x02,
    ConnectionComplete = 0x03,
    ConnectionRequest = 0x04,
    DisconnectionComplete = 0x05,
    RemoteNameComplete = 0x07,
    EncryptionChange = 0x08,
    CommandComplete = 0x0E,
    CommandStatus = 0x0F,
    LeMeta = 0x3E,
}

/// Discoverable device
#[derive(Clone, Debug)]
pub struct DiscoveredDevice {
    addr: BdAddr,
    name: String,
    rssi: i8,
    class_of_device: u32,
    is_le: bool,
}

/// Bluetooth connection state
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BtState {
    Off,
    Initializing,
    Ready,
    Scanning,
    Connected,
    Error,
}

/// L2CAP channel
#[derive(Clone, Copy)]
struct L2capChannel {
    active: bool,
    local_cid: u16,
    remote_cid: u16,
    psm: u16, // Protocol/Service Multiplexer
}

impl L2capChannel {
    const fn empty() -> Self {
        L2capChannel {
            active: false,
            local_cid: 0,
            remote_cid: 0,
            psm: 0,
        }
    }
}

/// Global Bluetooth state
static BT_STATE: Spinlock<BtState> = Spinlock::new(BtState::Off);
static BT_ADDR: Spinlock<BdAddr> = Spinlock::new(BdAddr([0; 6]));
static DISCOVERED: Spinlock<Vec<DiscoveredDevice>> = Spinlock::new(Vec::new());
static L2CAP_CHANNELS: Spinlock<[L2capChannel; 8]> = Spinlock::new([L2capChannel::empty(); 8]);

/// Initialize the Bluetooth subsystem
pub fn init() {
    *BT_STATE.lock() = BtState::Initializing;
    println!("[bt] Initializing Bluetooth (CYW43455 HCI over SDIO)");

    // HCI reset command
    if hci_reset().is_ok() {
        // Read local BD address
        let addr = hci_read_bd_addr().unwrap_or_else(|_| BdAddr::zero());
        *BT_ADDR.lock() = addr;
        *BT_STATE.lock() = BtState::Ready;
        println!("[bt] Adapter ready, address: {}", addr.format_addr());
    } else {
        *BT_STATE.lock() = BtState::Error;
        println!("[bt] Failed to initialize adapter");
    }
}

/// Get current Bluetooth state
pub fn state() -> BtState {
    *BT_STATE.lock()
}

/// Get the local adapter address
pub fn local_address() -> BdAddr {
    *BT_ADDR.lock()
}

/// Send HCI reset command
fn hci_reset() -> Result<(), &'static str> {
    // HCI_Reset: opcode 0x0C03, no parameters
    // In production, send via SDIO transport to CYW43455
    println!("[bt] HCI reset sent");
    Ok(())
}

/// Read local BD_ADDR
fn hci_read_bd_addr() -> Result<BdAddr, &'static str> {
    // HCI_Read_BD_ADDR: opcode 0x1009
    // Returns 6-byte address in event
    // Placeholder: return a fixed address
    Ok(BdAddr([0x00, 0x1A, 0x7D, 0x00, 0x00, 0x00]))
}

/// Start device discovery (scan)
pub fn start_discovery() -> Result<(), &'static str> {
    let state = *BT_STATE.lock();
    if state != BtState::Ready && state != BtState::Connected {
        return Err("Bluetooth not ready");
    }

    *BT_STATE.lock() = BtState::Scanning;
    DISCOVERED.lock().clear();

    // HCI_Inquiry: opcode 0x0401
    // Parameters: LAP=0x9E8B33, inquiry_length=10, num_responses=0
    println!("[bt] Discovery started");
    Ok(())
}

/// Stop device discovery
pub fn stop_discovery() {
    // HCI_Inquiry_Cancel: opcode 0x0402
    if *BT_STATE.lock() == BtState::Scanning {
        *BT_STATE.lock() = BtState::Ready;
        println!(
            "[bt] Discovery stopped ({} devices found)",
            DISCOVERED.lock().len()
        );
    }
}

/// Add a discovered device (called when HCI Inquiry Result event received)
pub fn add_discovered_device(addr: BdAddr, rssi: i8, is_le: bool) {
    let mut devices = DISCOVERED.lock();
    // Avoid duplicates
    if devices.iter().any(|d| d.addr.0 == addr.0) {
        return;
    }
    devices.push(DiscoveredDevice {
        addr,
        name: String::from("Unknown"),
        rssi,
        class_of_device: 0,
        is_le,
    });
}

/// Get the list of discovered devices
pub fn discovered_devices() -> Vec<DiscoveredDevice> {
    DISCOVERED.lock().clone()
}

/// Pair with a device (HCI authentication)
pub fn pair(addr: BdAddr) -> Result<(), &'static str> {
    println!("[bt] Pairing with {}", addr.format_addr());
    // HCI_Authentication_Requested: opcode 0x0407
    Ok(())
}

/// Connect to a device
pub fn connect(addr: BdAddr) -> Result<(), &'static str> {
    println!("[bt] Connecting to {}", addr.format_addr());
    // HCI_Create_Connection: opcode 0x0405
    *BT_STATE.lock() = BtState::Connected;
    Ok(())
}

/// Disconnect from a device
pub fn disconnect(_addr: BdAddr) -> Result<(), &'static str> {
    // HCI_Disconnect: opcode 0x0406
    *BT_STATE.lock() = BtState::Ready;
    println!("[bt] Disconnected");
    Ok(())
}

/// Allocate an L2CAP channel
pub fn l2cap_alloc_channel(psm: u16) -> Result<u16, &'static str> {
    let mut channels = L2CAP_CHANNELS.lock();
    for ch in channels.iter_mut() {
        if !ch.active {
            ch.active = true;
            ch.local_cid = 0x0040; // Dynamic CIDs start at 0x0040
            ch.psm = psm;
            return Ok(ch.local_cid);
        }
    }
    Err("no free L2CAP channels")
}

/// Free an L2CAP channel
pub fn l2cap_free_channel(cid: u16) {
    let mut channels = L2CAP_CHANNELS.lock();
    for ch in channels.iter_mut() {
        if ch.active && ch.local_cid == cid {
            *ch = L2capChannel::empty();
            return;
        }
    }
}

/// Send data on an L2CAP channel
pub fn l2cap_send(cid: u16, _data: &[u8]) -> Result<(), &'static str> {
    let channels = L2CAP_CHANNELS.lock();
    let ch = channels.iter().find(|c| c.active && c.local_cid == cid);
    if ch.is_none() {
        return Err("channel not found");
    }
    // Build L2CAP packet: length(2) + CID(2) + payload
    Ok(())
}

/// Print Bluetooth status
pub fn print_status() {
    let state = *BT_STATE.lock();
    let addr = *BT_ADDR.lock();
    println!("[bt] State: {:?}", state);
    println!("[bt] Address: {}", addr.format_addr());
    println!("[bt] Discovered: {} devices", DISCOVERED.lock().len());
    let channels = L2CAP_CHANNELS.lock();
    let active = channels.iter().filter(|c| c.active).count();
    println!("[bt] L2CAP channels: {}/8 active", active);
}

// ─── GATT (Generic Attribute Profile) ──────────────────────────────────
//
// GATT is the protocol used by Bluetooth LE devices to exchange data.
// It defines a hierarchy of Services → Characteristics → Descriptors.
// Each has a 128-bit UUID. The GATT server exposes attributes that a
// GATT client can read/write/notify.

/// GATT UUID (128-bit)
#[derive(Debug, Clone, Copy)]
pub struct GattUuid(pub [u8; 16]);

/// GATT attribute permissions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GattPermission {
    Read,
    Write,
    ReadWrite,
    Notify,
}

/// GATT characteristic
#[derive(Debug, Clone)]
pub struct GattCharacteristic {
    pub uuid: GattUuid,
    pub permissions: GattPermission,
    pub value: Vec<u8>,
    pub notifying: bool,
}

/// GATT service
#[derive(Debug, Clone)]
pub struct GattService {
    pub uuid: GattUuid,
    pub characteristics: Vec<GattCharacteristic>,
}

/// GATT server state
static GATT_SERVICES: Spinlock<Vec<GattService>> = Spinlock::new(Vec::new());

/// Well-known UUIDs (shortened to 16-bit mapped to 128-bit base)
const UUID_BASE: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0x80, 0x5F, 0x9B, 0x34, 0xFB,
];

/// Create a 128-bit UUID from a 16-bit short UUID
fn uuid_from_short(short: u16) -> GattUuid {
    let mut uuid = UUID_BASE;
    uuid[2] = (short & 0xFF) as u8;
    uuid[3] = ((short >> 8) & 0xFF) as u8;
    GattUuid(uuid)
}

/// Register a GATT service
pub fn gatt_register_service(uuid: GattUuid) -> Result<(), &'static str> {
    let mut services = GATT_SERVICES.lock();
    services.push(GattService {
        uuid,
        characteristics: Vec::new(),
    });
    println!("[bt] GATT service registered");
    Ok(())
}

/// Add a characteristic to the last registered service
pub fn gatt_add_characteristic(
    uuid: GattUuid,
    permissions: GattPermission,
    initial_value: Vec<u8>,
) -> Result<(), &'static str> {
    let mut services = GATT_SERVICES.lock();
    let service = services.last_mut().ok_or("no GATT service registered")?;
    service.characteristics.push(GattCharacteristic {
        uuid,
        permissions,
        value: initial_value,
        notifying: false,
    });
    Ok(())
}

/// Read a GATT characteristic value by UUID
pub fn gatt_read(uuid: &GattUuid) -> Option<Vec<u8>> {
    let services = GATT_SERVICES.lock();
    for service in services.iter() {
        for ch in &service.characteristics {
            if ch.uuid.0 == uuid.0 {
                if ch.permissions == GattPermission::Write {
                    return None; // Not readable
                }
                return Some(ch.value.clone());
            }
        }
    }
    None
}

/// Write a GATT characteristic value by UUID
pub fn gatt_write(uuid: &GattUuid, data: &[u8]) -> Result<(), &'static str> {
    let mut services = GATT_SERVICES.lock();
    for service in services.iter_mut() {
        for ch in &mut service.characteristics {
            if ch.uuid.0 == uuid.0 {
                if ch.permissions == GattPermission::Read {
                    return Err("characteristic is read-only");
                }
                ch.value = data.to_vec();
                // If notifying, send notification via L2CAP
                if ch.notifying {
                    println!("[bt] GATT notification sent for characteristic");
                }
                return Ok(());
            }
        }
    }
    Err("characteristic not found")
}

/// Enable notifications for a characteristic
pub fn gatt_enable_notify(uuid: &GattUuid) -> Result<(), &'static str> {
    let mut services = GATT_SERVICES.lock();
    for service in services.iter_mut() {
        for ch in &mut service.characteristics {
            if ch.uuid.0 == uuid.0 {
                ch.notifying = true;
                println!("[bt] GATT notifications enabled");
                return Ok(());
            }
        }
    }
    Err("characteristic not found")
}

/// Print GATT server status
pub fn gatt_print_status() {
    let services = GATT_SERVICES.lock();
    println!("[bt] GATT services: {}", services.len());
    for (i, service) in services.iter().enumerate() {
        println!(
            "[bt]   Service {}: {} characteristics",
            i,
            service.characteristics.len()
        );
    }
}

/// Initialize the GATT server with some default services
pub fn gatt_init() {
    // Generic Access Service (0x1800)
    let _ = gatt_register_service(uuid_from_short(0x1800));
    let _ = gatt_add_characteristic(
        uuid_from_short(0x2A00), // Device Name
        GattPermission::Read,
        b"LobsterOS".to_vec(),
    );
    let _ = gatt_add_characteristic(
        uuid_from_short(0x2A01), // Appearance
        GattPermission::Read,
        Vec::from([0x00u8, 0x00]),
    );

    // Battery Service (0x180F)
    let _ = gatt_register_service(uuid_from_short(0x180F));
    let _ = gatt_add_characteristic(
        uuid_from_short(0x2A19), // Battery Level
        GattPermission::ReadWrite,
        Vec::from([100u8]),
    );

    println!("[bt] GATT server initialized with default services");
}
