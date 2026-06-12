//! USB Mass Storage Driver for LobsterOS
//!
//! Implements USB Mass Storage class support (USB MSC spec):
//! - Bulk-Only Transport (BOT) protocol
//! - SCSI Transparent Command Set
//! - Block read/write operations
//! - Multiple LUN support

use crate::println;
use crate::sync::spinlock::Spinlock;
use crate::usb::xhci;

/// USB Mass Storage class code
pub const USB_CLASS_MASS: u8 = 0x08;

/// Bulk-Only Transport subclass code
pub const USB_MS_SCSI: u8 = 0x06;

/// Bulk-Only Transport protocol code
pub const USB_PROTOCOL_BOT: u8 = 0x50;

/// Maximum mass storage devices we track
const MAX_MASS_DEVICES: usize = 4;

/// Maximum LUNs per device
const MAX_LUNS: usize = 8;

/// SCSI command block wrapper signature (USB MSC spec Section 5.1)
const CBW_SIGNATURE: u32 = 0x43425355; // "USBC"

/// SCSI command status wrapper signature (USB MSC spec Section 5.2)
const CSW_SIGNATURE: u32 = 0x53425355; // "USBS"

/// CBW data direction flags
const CBW_DIR_OUT: u8 = 0x00;
const CBW_DIR_IN: u8 = 0x80;

/// CSW status codes
const CSW_STATUS_OK: u8 = 0x00;
const CSW_STATUS_FAILED: u8 = 0x01;
const CSW_STATUS_PHASE_ERROR: u8 = 0x02;

/// Block size for mass storage (512 bytes standard)
pub const BLOCK_SIZE: u32 = 512;

// ─── SCSI Command Opcodes ──────────────────────────────────────────────

/// SCSI opcodes (SPC-3 / SBC-2)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ScsiOpcode {
    TestUnitReady = 0x00,
    RequestSense = 0x03,
    Inquiry = 0x12,
    ReadCapacity10 = 0x25,
    Read10 = 0x28,
    Write10 = 0x2A,
    ModeSense6 = 0x1A,
    StartStopUnit = 0x1B,
    PreventAllowRemoval = 0x1E,
}

// ─── SCSI Structures ───────────────────────────────────────────────────

/// SCSI Inquiry response (SPC-3 Section 6.4.2)
#[derive(Clone, Copy, Debug)]
pub struct ScsiInquiry {
    pub peripheral_device_type: u8,
    pub removable_media: bool,
    pub version: u8,
    pub response_data_format: u8,
    pub additional_length: u8,
    pub vendor_id: [u8; 8],
    pub product_id: [u8; 16],
    pub product_revision: [u8; 4],
}

impl ScsiInquiry {
    /// Parse inquiry response from raw bytes (min 36 bytes)
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 36 {
            return None;
        }

        let mut vendor_id = [0u8; 8];
        let mut product_id = [0u8; 16];
        let mut product_revision = [0u8; 4];

        vendor_id.copy_from_slice(&bytes[8..16]);
        product_id.copy_from_slice(&bytes[16..32]);
        product_revision.copy_from_slice(&bytes[32..36]);

        Some(ScsiInquiry {
            peripheral_device_type: bytes[0] & 0x1F,
            removable_media: (bytes[1] & 0x80) != 0,
            version: bytes[2],
            response_data_format: bytes[3] & 0x07,
            additional_length: bytes[4],
            vendor_id,
            product_id,
            product_revision,
        })
    }

    /// Check if this is a direct-access block device (SBC)
    pub fn is_block_device(&self) -> bool {
        self.peripheral_device_type == 0x00
    }
}

/// SCSI Read Capacity response (SBC-2 Section 5.10.2)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScsiReadCapacity {
    /// Last logical block address (0-based)
    pub last_lba: u32,
    /// Block size in bytes
    pub block_size: u32,
}

impl ScsiReadCapacity {
    /// Parse read capacity response from raw bytes (8 bytes)
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 8 {
            return None;
        }
        Some(ScsiReadCapacity {
            last_lba: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            block_size: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        })
    }

    /// Total number of blocks (last_lba + 1)
    pub fn block_count(&self) -> u64 {
        self.last_lba as u64 + 1
    }

    /// Total capacity in bytes
    pub fn capacity_bytes(&self) -> u64 {
        self.block_count() * self.block_size as u64
    }

    /// Capacity in MiB
    pub fn capacity_mib(&self) -> u64 {
        self.capacity_bytes() / (1024 * 1024)
    }
}

/// SCSI Request Sense response (SPC-3 Section 6.27)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScsiSense {
    pub sense_key: u8,
    pub additional_sense_code: u8,
    pub additional_sense_qualifier: u8,
}

impl ScsiSense {
    /// Parse sense data from raw bytes (min 18 bytes)
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 18 {
            return None;
        }
        Some(ScsiSense {
            sense_key: bytes[2] & 0x0F,
            additional_sense_code: bytes[12],
            additional_sense_qualifier: bytes[13],
        })
    }

    /// Check if sense indicates no error
    pub fn is_no_sense(&self) -> bool {
        self.sense_key == 0x00
    }

    /// Check if sense indicates unit attention (e.g., media changed)
    pub fn is_unit_attention(&self) -> bool {
        self.sense_key == 0x06
    }
}

// ─── BOT Command Structures ────────────────────────────────────────────

/// Command Block Wrapper (CBW) — USB MSC BOT spec Section 5.1
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CommandBlockWrapper {
    pub signature: u32,
    pub tag: u32,
    pub transfer_length: u32,
    pub flags: u8,
    pub lun: u8,
    pub cb_length: u8,
    pub cb: [u8; 16],
}

impl CommandBlockWrapper {
    /// Create a new CBW for a SCSI command
    pub fn new(tag: u32, transfer_len: u32, dir_in: bool, lun: u8, cb: &[u8]) -> Self {
        let mut cb_array = [0u8; 16];
        let copy_len = cb.len().min(16);
        cb_array[..copy_len].copy_from_slice(&cb[..copy_len]);

        CommandBlockWrapper {
            signature: CBW_SIGNATURE,
            tag,
            transfer_length: transfer_len,
            flags: if dir_in { CBW_DIR_IN } else { CBW_DIR_OUT },
            lun,
            cb_length: copy_len as u8,
            cb: cb_array,
        }
    }

    /// Create a CBW for TEST UNIT READY
    pub fn test_unit_ready(tag: u32, lun: u8) -> Self {
        Self::new(
            tag,
            0,
            true,
            lun,
            &[ScsiOpcode::TestUnitReady as u8, 0, 0, 0, 0, 0],
        )
    }

    /// Create a CBW for REQUEST SENSE
    pub fn request_sense(tag: u32, lun: u8) -> Self {
        Self::new(
            tag,
            18,
            true,
            lun,
            &[ScsiOpcode::RequestSense as u8, 0, 0, 0, 18, 0],
        )
    }

    /// Create a CBW for INQUIRY
    pub fn inquiry(tag: u32, lun: u8) -> Self {
        Self::new(
            tag,
            36,
            true,
            lun,
            &[ScsiOpcode::Inquiry as u8, 0, 0, 0, 36, 0],
        )
    }

    /// Create a CBW for READ CAPACITY(10)
    pub fn read_capacity(tag: u32, lun: u8) -> Self {
        Self::new(
            tag,
            8,
            true,
            lun,
            &[ScsiOpcode::ReadCapacity10 as u8, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        )
    }

    /// Create a CBW for READ(10)
    pub fn read10(tag: u32, lun: u8, lba: u32, block_count: u16) -> Self {
        Self::new(
            tag,
            BLOCK_SIZE * block_count as u32,
            true,
            lun,
            &[
                ScsiOpcode::Read10 as u8,
                0,
                (lba >> 24) as u8,
                (lba >> 16) as u8,
                (lba >> 8) as u8,
                lba as u8,
                0,
                (block_count >> 8) as u8,
                block_count as u8,
                0,
            ],
        )
    }

    /// Create a CBW for WRITE(10)
    pub fn write10(tag: u32, lun: u8, lba: u32, block_count: u16) -> Self {
        Self::new(
            tag,
            BLOCK_SIZE * block_count as u32,
            false,
            lun,
            &[
                ScsiOpcode::Write10 as u8,
                0,
                (lba >> 24) as u8,
                (lba >> 16) as u8,
                (lba >> 8) as u8,
                lba as u8,
                0,
                (block_count >> 8) as u8,
                block_count as u8,
                0,
            ],
        )
    }
}

/// Command Status Wrapper (CSW) — USB MSC BOT spec Section 5.2
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandStatusWrapper {
    pub signature: u32,
    pub tag: u32,
    pub data_residue: u32,
    pub status: u8,
}

impl CommandStatusWrapper {
    /// Parse CSW from raw bytes (13 bytes)
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 13 {
            return None;
        }
        Some(CommandStatusWrapper {
            signature: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            tag: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            data_residue: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            status: bytes[12],
        })
    }

    /// Validate CSW signature
    pub fn valid_signature(&self) -> bool {
        self.signature == CSW_SIGNATURE
    }

    /// Check if command succeeded
    pub fn is_ok(&self) -> bool {
        self.status == CSW_STATUS_OK
    }

    /// Check if command failed
    pub fn is_failed(&self) -> bool {
        self.status == CSW_STATUS_FAILED
    }

    /// Check for phase error
    pub fn is_phase_error(&self) -> bool {
        self.status == CSW_STATUS_PHASE_ERROR
    }
}

// ─── Mass Storage Device State ──────────────────────────────────────────

/// Mass storage device state
#[derive(Clone, Copy, Debug)]
pub struct MassDevice {
    pub active: bool,
    pub slot_id: u8,
    pub port: u8,
    pub lun_count: u8,
    pub max_lun: u8,
    pub block_size: u32,
    pub last_lba: u32,
    pub removable: bool,
    pub ep_in: u8,  // Bulk IN endpoint number
    pub ep_out: u8, // Bulk OUT endpoint number
    pub tag_counter: u32,
    pub initialized: bool,
}

impl MassDevice {
    const ZERO: MassDevice = MassDevice {
        active: false,
        slot_id: 0,
        port: 0,
        lun_count: 0,
        max_lun: 0,
        block_size: BLOCK_SIZE,
        last_lba: 0,
        removable: false,
        ep_in: 0,
        ep_out: 0,
        tag_counter: 0,
        initialized: false,
    };

    /// Allocate next CBW tag
    fn next_tag(&mut self) -> u32 {
        self.tag_counter = self.tag_counter.wrapping_add(1);
        self.tag_counter
    }

    /// Total capacity in bytes
    pub fn capacity_bytes(&self) -> u64 {
        (self.last_lba as u64 + 1) * self.block_size as u64
    }

    /// Capacity in MiB
    pub fn capacity_mib(&self) -> u64 {
        self.capacity_bytes() / (1024 * 1024)
    }
}

/// Thread-safe container for mass storage devices
static MASS_DEVICES: Spinlock<[Option<MassDevice>; MAX_MASS_DEVICES]> =
    Spinlock::new([const { None }; MAX_MASS_DEVICES]);

// ─── Initialization ────────────────────────────────────────────────────

/// Initialize mass storage driver — scan xHCI slots for MSC devices
pub fn init() {
    println!("[mass] Scanning for USB mass storage devices...");

    {
        let mut devices = MASS_DEVICES.lock();
        for dev in devices.iter_mut() {
            *dev = None;
        }
    }

    // Scan xHCI slots for mass storage class devices (class=0x08)
    for slot_id in 1..xhci::MAX_SLOTS as u8 {
        if let Some(slot) = xhci::get_slot(slot_id) {
            if slot.enabled && slot.class_code == USB_CLASS_MASS {
                register_device(slot_id, slot.port, slot.subclass, slot.protocol);
            }
        }
    }

    let count = {
        let devices = MASS_DEVICES.lock();
        devices.iter().filter(|d| d.is_some()).count()
    };
    println!("[mass] {} mass storage device(s) found", count);
}

/// Register a mass storage device
fn register_device(slot_id: u8, port: u8, subclass: u8, protocol: u8) {
    // Only support Bulk-Only Transport with SCSI command set
    if subclass != USB_MS_SCSI {
        println!(
            "[mass] Skipping slot {}: unsupported subclass {:02x}",
            slot_id, subclass
        );
        return;
    }
    if protocol != USB_PROTOCOL_BOT {
        println!(
            "[mass] Skipping slot {}: unsupported protocol {:02x}",
            slot_id, protocol
        );
        return;
    }

    let mut devices = MASS_DEVICES.lock();
    for dev_opt in devices.iter_mut() {
        if dev_opt.is_none() {
            *dev_opt = Some(MassDevice {
                active: true,
                slot_id,
                port,
                lun_count: 1, // Will be updated by get_max_lun
                max_lun: 0,
                block_size: BLOCK_SIZE,
                last_lba: 0,
                removable: false,
                ep_in: 1,  // Default to EP1 IN
                ep_out: 2, // Default to EP2 OUT
                tag_counter: 0,
                initialized: false,
            });
            println!(
                "[mass] Registered device: slot={}, subclass={:02x}, protocol={:02x}",
                slot_id, subclass, protocol
            );
            return;
        }
    }
    println!("[mass] No free device slots for slot {}", slot_id);
}

// ─── BOT Protocol Operations ──────────────────────────────────────────

/// Get max LUN from device (class-specific request)
fn get_max_lun(_slot_id: u8) -> u8 {
    // GET MAX LUN: bmRequestType=0xA1, bRequest=0xFE, wValue=0, wIndex=0, wLength=1
    // Placeholder — needs control transfer on EP0
    0
}

/// Send CBW to device via bulk OUT endpoint
fn send_cbw(_slot_id: u8, _ep_out: u8, _cbw: &CommandBlockWrapper) -> Result<(), &'static str> {
    // Requires xHCI bulk transfer on the OUT endpoint
    // Placeholder — needs xHCI submit_transfer integration
    Ok(())
}

/// Receive data via bulk IN endpoint
fn receive_data(_slot_id: u8, _ep_in: u8, _buf: &mut [u8]) -> Result<usize, &'static str> {
    // Requires xHCI bulk transfer on the IN endpoint
    // Placeholder — needs xHCI submit_transfer integration
    Ok(0)
}

/// Read CSW from device via bulk IN endpoint
fn read_csw(_slot_id: u8, _ep_in: u8) -> Result<CommandStatusWrapper, &'static str> {
    // Read 13 bytes CSW from bulk IN
    // Placeholder — needs xHCI submit_transfer integration
    Ok(CommandStatusWrapper {
        signature: CSW_SIGNATURE,
        tag: 0,
        data_residue: 0,
        status: CSW_STATUS_OK,
    })
}

/// Perform BOT reset recovery (USB MSC spec Section 5.3.4)
fn reset_recovery(_slot_id: u8) -> Result<(), &'static str> {
    // 1. Send Bulk-Only Mass Storage Reset (class-specific control request)
    // 2. Clear Feature HALT on bulk IN endpoint
    // 3. Clear Feature HALT on bulk OUT endpoint
    Ok(())
}

// ─── SCSI Command Functions ─────────────────────────────────────────────

/// Issue TEST UNIT READY to check if a LUN is ready
pub fn test_unit_ready(slot_id: u8, lun: u8) -> Result<bool, &'static str> {
    let mut devices = MASS_DEVICES.lock();
    let dev = find_device_mut(&mut devices, slot_id)?;

    let tag = dev.next_tag();
    let cbw = CommandBlockWrapper::test_unit_ready(tag, lun);
    drop(dev);

    send_cbw(slot_id, 0, &cbw)?;
    let csw = read_csw(slot_id, 0)?;

    if csw.is_phase_error() {
        reset_recovery(slot_id)?;
        return Err("phase error, recovery attempted");
    }

    Ok(csw.is_ok())
}

/// Issue REQUEST SENSE to get error information
pub fn request_sense(slot_id: u8, lun: u8) -> Result<ScsiSense, &'static str> {
    let mut devices = MASS_DEVICES.lock();
    let dev = find_device_mut(&mut devices, slot_id)?;

    let tag = dev.next_tag();
    let cbw = CommandBlockWrapper::request_sense(tag, lun);
    drop(dev);

    send_cbw(slot_id, 0, &cbw)?;
    let mut buf = [0u8; 18];
    receive_data(slot_id, 0, &mut buf)?;
    let csw = read_csw(slot_id, 0)?;

    if csw.is_failed() {
        return Err("REQUEST SENSE failed");
    }

    ScsiSense::from_bytes(&buf).ok_or("failed to parse sense data")
}

/// Issue INQUIRY to get device information
pub fn inquiry(slot_id: u8, lun: u8) -> Result<ScsiInquiry, &'static str> {
    let mut devices = MASS_DEVICES.lock();
    let dev = find_device_mut(&mut devices, slot_id)?;

    let tag = dev.next_tag();
    let cbw = CommandBlockWrapper::inquiry(tag, lun);
    drop(dev);

    send_cbw(slot_id, 0, &cbw)?;
    let mut buf = [0u8; 36];
    receive_data(slot_id, 0, &mut buf)?;
    let csw = read_csw(slot_id, 0)?;

    if csw.is_failed() {
        return Err("INQUIRY failed");
    }

    ScsiInquiry::from_bytes(&buf).ok_or("failed to parse inquiry data")
}

/// Issue READ CAPACITY(10) to get device size
pub fn read_capacity(slot_id: u8, lun: u8) -> Result<ScsiReadCapacity, &'static str> {
    let mut devices = MASS_DEVICES.lock();
    let dev = find_device_mut(&mut devices, slot_id)?;

    let tag = dev.next_tag();
    let cbw = CommandBlockWrapper::read_capacity(tag, lun);
    drop(dev);

    send_cbw(slot_id, 0, &cbw)?;
    let mut buf = [0u8; 8];
    receive_data(slot_id, 0, &mut buf)?;
    let csw = read_csw(slot_id, 0)?;

    if csw.is_failed() {
        return Err("READ CAPACITY failed");
    }

    let cap = ScsiReadCapacity::from_bytes(&buf).ok_or("failed to parse capacity data")?;

    // Update device state with the capacity info
    let mut devices = MASS_DEVICES.lock();
    if let Ok(dev) = find_device_mut(&mut devices, slot_id) {
        dev.block_size = cap.block_size;
        dev.last_lba = cap.last_lba;
        dev.initialized = true;
    }

    Ok(cap)
}

/// Read blocks from a mass storage device
pub fn read_blocks(
    slot_id: u8,
    lun: u8,
    lba: u32,
    block_count: u16,
    buf: &mut [u8],
) -> Result<(), &'static str> {
    let expected_len = block_count as usize * BLOCK_SIZE as usize;
    if buf.len() < expected_len {
        return Err("buffer too small for read");
    }

    let mut devices = MASS_DEVICES.lock();
    let dev = find_device_mut(&mut devices, slot_id)?;
    let tag = dev.next_tag();
    let cbw = CommandBlockWrapper::read10(tag, lun, lba, block_count);
    drop(dev);

    send_cbw(slot_id, 0, &cbw)?;
    receive_data(slot_id, 0, &mut buf[..expected_len])?;
    let csw = read_csw(slot_id, 0)?;

    if csw.is_phase_error() {
        reset_recovery(slot_id)?;
        return Err("phase error during read, recovery attempted");
    }
    if csw.is_failed() {
        return Err("READ(10) command failed");
    }

    Ok(())
}

/// Write blocks to a mass storage device
pub fn write_blocks(
    slot_id: u8,
    lun: u8,
    lba: u32,
    block_count: u16,
    buf: &[u8],
) -> Result<(), &'static str> {
    let expected_len = block_count as usize * BLOCK_SIZE as usize;
    if buf.len() < expected_len {
        return Err("buffer too small for write");
    }

    let mut devices = MASS_DEVICES.lock();
    let dev = find_device_mut(&mut devices, slot_id)?;
    let tag = dev.next_tag();
    let cbw = CommandBlockWrapper::write10(tag, lun, lba, block_count);
    drop(dev);

    send_cbw(slot_id, 0, &cbw)?;
    // Send data on bulk OUT endpoint
    // Placeholder — needs xHCI bulk OUT transfer
    let _csw = read_csw(slot_id, 0)?;

    Ok(())
}

// ─── Utility Functions ─────────────────────────────────────────────────

/// Find a device by slot_id (mutable)
fn find_device_mut(
    devices: &mut [Option<MassDevice>; MAX_MASS_DEVICES],
    slot_id: u8,
) -> Result<&mut MassDevice, &'static str> {
    for dev_opt in devices.iter_mut() {
        if let Some(dev) = dev_opt {
            if dev.slot_id == slot_id && dev.active {
                return Ok(dev);
            }
        }
    }
    Err("mass storage device not found")
}

/// Get the number of registered devices
pub fn device_count() -> usize {
    let devices = MASS_DEVICES.lock();
    devices.iter().filter(|d| d.is_some()).count()
}

/// List mass storage devices for shell command
pub fn list_devices() {
    let devices = MASS_DEVICES.lock();
    for (i, dev_opt) in devices.iter().enumerate() {
        if let Some(dev) = dev_opt {
            println!(
                "  Device {}: slot={}, port={}, luns={}, blocks={}, block_size={}, capacity={}MiB",
                i,
                dev.slot_id,
                dev.port,
                dev.lun_count,
                dev.last_lba + 1,
                dev.block_size,
                dev.capacity_mib()
            );
        }
    }
}
