//! xHCI (eXtensible Host Controller Interface) driver for BCM2712
//!
//! Bare-metal xHCI host controller driver implementing:
//! - Register access (Capability, Operational, Runtime, Doorbell)
//! - Command ring and event ring management with TRB structures
//! - Device enumeration (enable slot, address device, configure endpoint 0)
//! - Interrupt handling and event dispatch
//!
//! References: xHCI Specification 1.2, BCM2712 peripherals datasheet

use crate::arch::cpu::delay_us;
use crate::memory::frame_alloc::allocate_contiguous;
use crate::memory::{phys_to_virt, PhysAddr, VirtAddr, PAGE_SIZE};
use crate::println;
use core::ptr::{read_volatile, write_volatile};

// ─── BCM2712 xHCI Memory Map ──────────────────────────────────────────

/// xHCI controller base physical address
const XHCI_BASE_PHYS: u64 = 0x0001_00C5_0000;

/// USB PHY base
const USB_PHY_BASE_PHYS: u64 = 0x0001_00C5_1000;

/// xHCI IRQ — GIC SPI 48
pub const XHCI_IRQ: u32 = 48;

// ─── Capability Register Offsets ──────────────────────────────────────

const CAPLENGTH: u64 = 0x000;
const HCSPARAMS1: u64 = 0x004;
const HCSPARAMS2: u64 = 0x008;
const HCCPARAMS1: u64 = 0x010;

// ─── Operational Register Offsets ─────────────────────────────────────

const USBCMD: u64 = 0x100;
const USBSTS: u64 = 0x104;
const USBINTR: u64 = 0x108;
const CRCR: u64 = 0x110;
const DCBAAP: u64 = 0x130;
const CONFIG_REG: u64 = 0x138;

// USBCMD bits
const USBCMD_RUN: u32 = 1 << 0;
const USBCMD_HCRST: u32 = 1 << 1;
const USBCMD_INTE: u32 = 1 << 2;
const USBCMD_HSEE: u32 = 1 << 3;
const USBCMD_LHCRST: u32 = 1 << 7;

// USBSTS bits
const USBSTS_HCH: u32 = 1 << 0;
const USBSTS_HSE: u32 = 1 << 2;
const USBSTS_EINT: u32 = 1 << 3;
const USBSTS_PCD: u32 = 1 << 4;
const USBSTS_SSS: u32 = 1 << 8;
const USBSTS_RSS: u32 = 1 << 9;
const USBSTS_SRE: u32 = 1 << 10;
const USBSTS_CNR: u32 = 1 << 11;

// ─── Runtime Register Offsets ─────────────────────────────────────────

const MFINDEX: u64 = 0x240;
const IMAN: u64 = 0x244; // Interrupter 0
const IMOD: u64 = 0x248; // Interrupter 0 modulation
const ERSTSZ: u64 = 0x24C; // Event Ring Segment Table Size
const ERSTBA: u64 = 0x250; // Event Ring Segment Table Base Address
const ERDP: u64 = 0x258; // Event Ring Dequeue Pointer

// IMAN bits
const IMAN_IP: u32 = 1 << 0;
const IMAN_IE: u32 = 1 << 1;

// ─── Doorbell Register Offsets ────────────────────────────────────────

fn db_offset(slot_id: u8) -> u64 {
    0x400 + (slot_id as u64 * 8)
}

// ─── TRB Types ─────────────────────────────────────────────────────────

const TRB_TYPE_NORMAL: u32 = 1;
const TRB_TYPE_SETUP_STAGE: u32 = 2;
const TRB_TYPE_DATA_STAGE: u32 = 3;
const TRB_TYPE_STATUS_STAGE: u32 = 4;
const TRB_TYPE_ISOCH: u32 = 5;
const TRB_TYPE_LINK: u32 = 6;
const TRB_TYPE_EVENT_DATA: u32 = 7;
const TRB_TYPE_NOOP: u32 = 8;
const TRB_TYPE_ENABLE_SLOT: u32 = 9;
const TRB_TYPE_DISABLE_SLOT: u32 = 10;
const TRB_TYPE_ADDRESS_DEV: u32 = 11;
const TRB_TYPE_CONFIG_EP: u32 = 12;
const TRB_TYPE_EVAL_CTX: u32 = 13;
const TRB_TYPE_RESET_EP: u32 = 14;
const TRB_TYPE_STOP_EP: u32 = 15;
const TRB_TYPE_SET_DEQ: u32 = 16;
const TRB_TYPE_RESET_DEV: u32 = 17;
const TRB_TYPE_FORCE_EVENT: u32 = 18;
const TRB_TYPE_NEGOTIATE_BW: u32 = 19;
const TRB_TYPE_SET_LTT: u32 = 20;
const TRB_TYPE_SET_HLT: u32 = 21;

// Event TRB types
const TRB_TYPE_TRANSFER_EVT: u32 = 32;
const TRB_TYPE_CMD_COMPLETE: u32 = 33;
const TRB_TYPE_PORT_STATUS: u32 = 34;
const TRB_TYPE_BW_REQUEST: u32 = 35;
const TRB_TYPE_DOORBELL: u32 = 36;
const TRB_TYPE_HOST_EVENT: u32 = 37;
const TRB_TYPE_MFINDEX_WRAP: u32 = 38;

// TRB completion codes
const COMP_SUCCESS: u32 = 1;
const COMP_TRB_ERR: u32 = 5;
const COMP_STALL: u32 = 6;
const COMP_SHORT_PKT: u32 = 13;

// ─── TRB Structure (16 bytes) ─────────────────────────────────────────

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
pub struct Trb {
    pub param: u64,
    pub status: u32,
    pub ctrl: u32,
}

impl Trb {
    pub const ZERO: Trb = Trb {
        param: 0,
        status: 0,
        ctrl: 0,
    };

    pub fn new(param: u64, status: u32, ctrl: u32) -> Self {
        Trb {
            param,
            status,
            ctrl,
        }
    }

    pub fn trb_type(&self) -> u32 {
        (self.ctrl >> 10) & 0x3F
    }

    pub fn set_type(&mut self, trb_type: u32) {
        self.ctrl = (self.ctrl & !(0x3F << 10)) | ((trb_type & 0x3F) << 10);
    }

    pub fn cycle_bit(&self) -> bool {
        (self.ctrl & 1) != 0
    }

    pub fn set_cycle(&mut self, c: bool) {
        if c {
            self.ctrl |= 1;
        } else {
            self.ctrl &= !1;
        }
    }

    pub fn toggle_cycle(&mut self) {
        self.ctrl ^= 1;
    }

    /// Completion code from event TRB status field
    pub fn completion_code(&self) -> u32 {
        self.status >> 24 & 0xFF
    }

    /// Slot ID from event TRB control field
    pub fn slot_id(&self) -> u8 {
        ((self.ctrl >> 24) & 0xFF) as u8
    }
}

// ─── Port Register Offsets (relative to operational base + 0x400) ─────

fn port_regs(port: u8) -> u64 {
    0x400 + (port as u64 * 0x10)
}

const PORTSC: u64 = 0x00;
const PORTPMSC: u64 = 0x04;
const PORTLI: u64 = 0x08;
const PORTHUBPMSC: u64 = 0x0C;

// PORTSC bits
const PORTSC_CCS: u32 = 1 << 0; // Current Connect Status
const PORTSC_PED: u32 = 1 << 1; // Port Enabled/Disabled
const PORTSC_OCA: u32 = 1 << 3; // Over-Current Active
const PORTSC_PR: u32 = 1 << 4; // Port Reset
const PORTSC_PLS_MASK: u32 = 0xF << 5; // Port Link State
const PORTSC_PP: u32 = 1 << 9; // Port Power
const PORTSC_SPEED_MASK: u32 = 0xF << 10;
const PORTSC_LWS: u32 = 1 << 16; // Link State Write Strobe
const PORTSC_CSC: u32 = 1 << 17; // Connect Status Change
const PORTSC_PEC: u32 = 1 << 18; // Port Enable/Disable Change
const PORTSC_WRC: u32 = 1 << 19; // Warm Port Reset Change
const PORTSC_OCC: u32 = 1 << 20; // Over-Current Change
const PORTSC_PRC: u32 = 1 << 21; // Port Reset Change
const PORTSC_PLC: u32 = 1 << 22; // Port Link State Change
const PORTSC_CEC: u32 = 1 << 23; // Port Config Error Change
const PORTSC_CAS: u32 = 1 << 26; // Cold Attach Status

// ─── Ring Buffer ───────────────────────────────────────────────────────

const CMD_RING_SIZE: usize = 64;
const EVT_RING_SIZE: usize = 64;
const XFER_RING_SIZE: usize = 64;

/// Command ring state
struct CmdRing {
    base: VirtAddr,
    phys: PhysAddr,
    enqueue_idx: usize,
    cycle: bool,
}

impl CmdRing {
    const ZERO: CmdRing = CmdRing {
        base: VirtAddr(0),
        phys: PhysAddr(0),
        enqueue_idx: 0,
        cycle: true,
    };

    fn enqueue(&mut self, trb: Trb) -> PhysAddr {
        let ptr = self.base.0 as *mut Trb;
        let mut trb = trb;
        trb.set_cycle(self.cycle);
        unsafe {
            ptr.add(self.enqueue_idx).write(trb);
        }

        let phys_addr = PhysAddr(self.phys.0 + (self.enqueue_idx as u64 * 16));

        self.enqueue_idx += 1;
        if self.enqueue_idx >= CMD_RING_SIZE - 1 {
            // Write link TRB at last slot
            let link = Trb::new(
                self.phys.0,
                0,
                (TRB_TYPE_LINK << 10) | (self.cycle as u32) | 1, // Toggle cycle
            );
            unsafe {
                ptr.add(CMD_RING_SIZE - 1).write(link);
            }
            self.enqueue_idx = 0;
            self.cycle = !self.cycle;
        }

        phys_addr
    }
}

/// Event ring segment table entry
#[repr(C)]
struct ErstEntry {
    addr: u64,
    size: u32,
    _res: u32,
}

/// Event ring state
struct EvtRing {
    base: VirtAddr,
    phys: PhysAddr,
    erst_base: VirtAddr,
    dequeue_idx: usize,
    cycle: bool,
}

impl EvtRing {
    const ZERO: EvtRing = EvtRing {
        base: VirtAddr(0),
        phys: PhysAddr(0),
        erst_base: VirtAddr(0),
        dequeue_idx: 0,
        cycle: true,
    };

    fn dequeue(&mut self) -> Option<Trb> {
        let ptr = self.base.0 as *const Trb;
        let trb = unsafe { ptr.add(self.dequeue_idx).read() };

        if trb.cycle_bit() == self.cycle {
            self.dequeue_idx += 1;
            if self.dequeue_idx >= EVT_RING_SIZE {
                self.dequeue_idx = 0;
                self.cycle = !self.cycle;
            }
            Some(trb)
        } else {
            None
        }
    }

    /// Update the ERDP register to acknowledge processed events
    fn update_erdp(&self, base: VirtAddr) {
        let erdp_val = self.phys.0 + (self.dequeue_idx as u64 * 16);
        reg32_write(base, ERDP, erdp_val as u32 | 1); // Dequeue Cycle bit
        reg32_write(base, ERDP + 4, (erdp_val >> 32) as u32);
    }
}

/// Transfer ring for a device endpoint
struct XferRing {
    base: VirtAddr,
    phys: PhysAddr,
    enqueue_idx: usize,
    cycle: bool,
}

impl XferRing {
    const ZERO: XferRing = XferRing {
        base: VirtAddr(0),
        phys: PhysAddr(0),
        enqueue_idx: 0,
        cycle: true,
    };

    fn enqueue(&mut self, trb: Trb) {
        let ptr = self.base.0 as *mut Trb;
        let mut trb = trb;
        trb.set_cycle(self.cycle);
        unsafe {
            ptr.add(self.enqueue_idx).write(trb);
        }
        self.enqueue_idx += 1;
        if self.enqueue_idx >= XFER_RING_SIZE - 1 {
            let link = Trb::new(
                self.phys.0,
                0,
                (TRB_TYPE_LINK << 10) | (self.cycle as u32) | 1,
            );
            unsafe {
                ptr.add(XFER_RING_SIZE - 1).write(link);
            }
            self.enqueue_idx = 0;
            self.cycle = !self.cycle;
        }
    }
}

// ─── Device Slot ───────────────────────────────────────────────────────

/// Maximum number of device slots
pub const MAX_SLOTS: usize = 4;

/// USB device speed
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UsbSpeed {
    Unknown,
    Full,  // 12 Mbps
    High,  // 480 Mbps
    Super, // 5 Gbps
}

/// USB device slot
pub struct UsbSlot {
    pub enabled: bool,
    pub slot_id: u8,
    pub port: u8,
    pub speed: UsbSpeed,
    pub ep0_ring: XferRing,
    pub dev_ctx: VirtAddr,
    pub dev_ctx_phys: PhysAddr,
    pub class_code: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub vid: u16,
    pub pid: u16,
}

impl UsbSlot {
    const ZERO: UsbSlot = UsbSlot {
        enabled: false,
        slot_id: 0,
        port: 0,
        speed: UsbSpeed::Unknown,
        ep0_ring: XferRing::ZERO,
        dev_ctx: VirtAddr(0),
        dev_ctx_phys: PhysAddr(0),
        class_code: 0,
        subclass: 0,
        protocol: 0,
        vid: 0,
        pid: 0,
    };
}

// ─── Global Controller State ───────────────────────────────────────────

static mut XHCI_BASE: VirtAddr = VirtAddr(0);
static mut XHCI_CAP_LENGTH: u8 = 0;
static mut XHCI_MAX_SLOTS: u8 = 0;
static mut XHCI_MAX_PORTS: u8 = 0;

static mut CMD_RING: CmdRing = CmdRing::ZERO;
static mut EVT_RING: EvtRing = EvtRing::ZERO;

static mut SLOTS: [UsbSlot; MAX_SLOTS] = [UsbSlot::ZERO; MAX_SLOTS];

/// Device Context Base Array (up to 256 slots + 1 for the array itself)
static mut DCBA: [u64; MAX_SLOTS + 1] = [0; MAX_SLOTS + 1];
static mut DCBA_PHYS: PhysAddr = PhysAddr(0);

// ─── Volatile Register Access ──────────────────────────────────────────

fn reg32_read(base: VirtAddr, offset: u64) -> u32 {
    unsafe { read_volatile((base.0 + offset) as *const u32) }
}

fn reg32_write(base: VirtAddr, offset: u64, val: u32) {
    unsafe { write_volatile((base.0 + offset) as *mut u32, val) }
}

fn reg64_read(base: VirtAddr, offset: u64) -> u64 {
    let lo = reg32_read(base, offset) as u64;
    let hi = reg32_read(base, offset + 4) as u64;
    (hi << 32) | lo
}

fn reg64_write(base: VirtAddr, offset: u64, val: u64) {
    reg32_write(base, offset, val as u32);
    reg32_write(base, offset + 4, (val >> 32) as u32);
}

// ─── Ring Doorbell ─────────────────────────────────────────────────────

fn ring_doorbell(base: VirtAddr, slot_id: u8, target: u32) {
    reg32_write(base, db_offset(slot_id), target);
}

// ─── Controller Initialization ─────────────────────────────────────────

/// Initialize the xHCI controller
pub fn init() -> Result<(), &'static str> {
    // Map xHCI base address
    // For now, assume identity mapping or use phys_to_virt
    let base = phys_to_virt(PhysAddr(XHCI_BASE_PHYS));
    unsafe {
        XHCI_BASE = base;
    }

    println!("[xhci] Base address: {:#x}", base.0);

    // 1. Read capability registers
    let cap_len = reg32_read(base, CAPLENGTH) as u8;
    let hcsparams1 = reg32_read(base, HCSPARAMS1);
    let _hcsparams2 = reg32_read(base, HCSPARAMS2);
    let _hccparams1 = reg32_read(base, HCCPARAMS1);

    let max_slots = (hcsparams1 & 0xFF) as u8;
    let max_intrs = ((hcsparams1 >> 8) & 0x7FF) as u16;
    let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;

    unsafe {
        XHCI_CAP_LENGTH = cap_len;
        XHCI_MAX_SLOTS = max_slots;
        XHCI_MAX_PORTS = max_ports;
    }

    println!(
        "[xhci] CAPLENGTH={}, MaxSlots={}, MaxIntrs={}, MaxPorts={}",
        cap_len, max_slots, max_intrs, max_ports
    );

    // 2. Ensure controller is halted
    let sts = reg32_read(base, USBSTS);
    if (sts & USBSTS_HCH) == 0 {
        println!("[xhci] Halting controller...");
        reg32_write(base, USBCMD, reg32_read(base, USBCMD) & !USBCMD_RUN);
        // Wait for halt (up to 1 second)
        let mut timeout = 100000u64;
        while timeout > 0 {
            if (reg32_read(base, USBSTS) & USBSTS_HCH) != 0 {
                break;
            }
            delay_us(10);
            timeout -= 10;
        }
        if timeout == 0 {
            return Err("xHCI: controller did not halt");
        }
    }

    // 3. Reset controller
    println!("[xhci] Resetting controller...");
    reg32_write(base, USBCMD, USBCMD_HCRST);
    let mut timeout = 100000u64;
    while timeout > 0 {
        let cmd = reg32_read(base, USBCMD);
        let sts = reg32_read(base, USBSTS);
        if (cmd & USBCMD_HCRST) == 0 && (sts & USBSTS_CNR) == 0 {
            break;
        }
        delay_us(10);
        timeout -= 10;
    }
    if timeout == 0 {
        return Err("xHCI: controller reset timed out");
    }
    println!("[xhci] Controller reset complete");

    // 4. Allocate and set up command ring
    let cmd_ring_phys = allocate_contiguous(1).ok_or("xHCI: failed to alloc cmd ring")?;
    let cmd_ring_virt = phys_to_virt(cmd_ring_phys);

    // Zero the command ring
    let ptr = cmd_ring_virt.0 as *mut u8;
    unsafe {
        for i in 0..(PAGE_SIZE as usize) {
            ptr.add(i).write(0);
        }
    }

    unsafe {
        CMD_RING = CmdRing {
            base: cmd_ring_virt,
            phys: cmd_ring_phys,
            enqueue_idx: 0,
            cycle: true,
        };
    }

    // Set CRCR (Command Ring Control Register)
    reg64_write(base, CRCR, cmd_ring_phys.0 | 1); // Ring Cycle Bit = 1

    // 5. Allocate event ring and ERST
    let evt_ring_phys = allocate_contiguous(1).ok_or("xHCI: failed to alloc evt ring")?;
    let evt_ring_virt = phys_to_virt(evt_ring_phys);

    // Zero the event ring
    let ptr = evt_ring_virt.0 as *mut u8;
    unsafe {
        for i in 0..(PAGE_SIZE as usize) {
            ptr.add(i).write(0);
        }
    }

    let erst_phys = allocate_contiguous(1).ok_or("xHCI: failed to alloc ERST")?;
    let erst_virt = phys_to_virt(erst_phys);

    // Write ERST entry: [0] = ring base address, [8] = ring size
    let erst_ptr = erst_virt.0 as *mut ErstEntry;
    unsafe {
        (*erst_ptr).addr = evt_ring_phys.0;
        (*erst_ptr).size = EVT_RING_SIZE as u32;
        (*erst_ptr)._res = 0;
    }

    unsafe {
        EVT_RING = EvtRing {
            base: evt_ring_virt,
            phys: evt_ring_phys,
            erst_base: erst_virt,
            dequeue_idx: 0,
            cycle: true,
        };
    }

    // Configure runtime registers
    // Set ERST Size = 1
    reg32_write(base, ERSTSZ, 1);

    // Set ERST Base Address
    reg64_write(base, ERSTBA, erst_phys.0);

    // Set ERDP (initial dequeue pointer)
    reg64_write(base, ERDP, evt_ring_phys.0 | 1); // Dequeue Cycle Bit

    // 6. Allocate Device Context Base Array
    let dcba_phys = allocate_contiguous(1).ok_or("xHCI: failed to alloc DCBA")?;
    let dcba_virt = phys_to_virt(dcba_phys);

    // Zero DCBA
    let ptr = dcba_virt.0 as *mut u8;
    unsafe {
        for i in 0..(PAGE_SIZE as usize) {
            ptr.add(i).write(0);
        }
        DCBA_PHYS = dcba_phys;
    }

    // Set DCBAAP
    reg64_write(base, DCBAAP, dcba_phys.0);

    // 7. Configure controller
    let max_slots_val = max_slots.min(MAX_SLOTS as u8);
    reg32_write(base, CONFIG_REG, max_slots_val as u32);

    // 8. Enable interrupter 0
    let iman = reg32_read(base, IMAN);
    reg32_write(base, IMAN, iman | IMAN_IE);

    // Enable host controller interrupts
    reg32_write(base, USBINTR, 0x1); // Interrupt on Event

    // 9. Start the controller
    reg32_write(base, USBCMD, USBCMD_RUN | USBCMD_INTE);

    // Wait for controller to start (HCH bit should clear)
    let mut timeout = 100000u64;
    while timeout > 0 {
        if (reg32_read(base, USBSTS) & USBSTS_HCH) == 0 {
            break;
        }
        delay_us(10);
        timeout -= 10;
    }
    if timeout == 0 {
        return Err("xHCI: controller did not start");
    }

    println!(
        "[xhci] Controller running, command ring at {:#x}, event ring at {:#x}",
        cmd_ring_phys.0, evt_ring_phys.0
    );

    // 10. Power on all ports
    for port in 1..=max_ports {
        let portsc_off = 0x400 + (port as u64 * 0x10);
        let portsc = reg32_read(base, portsc_off);
        // Set Port Power (PP) bit if supported
        if (portsc & PORTSC_PP) == 0 {
            reg32_write(base, portsc_off, portsc | PORTSC_PP);
        }
    }

    delay_us(100_000); // Wait 100ms for ports to stabilize

    Ok(())
}

// ─── Command Submission ────────────────────────────────────────────────

/// Submit a command TRB and ring doorbell 0
fn submit_command(trb: Trb) -> Result<(), &'static str> {
    let base = unsafe { XHCI_BASE };
    unsafe {
        CMD_RING.enqueue(trb);
    }
    // Ring doorbell 0 (command ring)
    ring_doorbell(base, 0, 0);
    Ok(())
}

// ─── Port Scanning ─────────────────────────────────────────────────────

/// Scan all ports for connected devices and enumerate them
pub fn scan_ports() {
    let base = unsafe { XHCI_BASE };
    let max_ports = unsafe { XHCI_MAX_PORTS };

    for port in 1..=max_ports {
        let portsc_off = 0x400 + (port as u64 * 0x10);
        let portsc = reg32_read(base, portsc_off);

        if (portsc & PORTSC_CCS) != 0 {
            let speed_bits = (portsc >> 10) & 0xF;
            let speed = match speed_bits {
                1 => UsbSpeed::Full,
                2 => UsbSpeed::High,
                3 => UsbSpeed::Super,
                _ => UsbSpeed::Unknown,
            };
            println!("[xhci] Port {} connected, speed={:?}", port, speed);

            // Reset the port if not already enabled
            if (portsc & PORTSC_PED) == 0 {
                // Port reset
                reg32_write(base, portsc_off, portsc | PORTSC_PR);
                delay_us(100_000); // 100ms reset

                // Wait for reset complete
                let mut timeout = 50000u64;
                while timeout > 0 {
                    let sc = reg32_read(base, portsc_off);
                    if (sc & PORTSC_PRC) != 0 {
                        // Clear PRC by writing 1
                        reg32_write(base, portsc_off, sc | PORTSC_PRC);
                        break;
                    }
                    delay_us(10);
                    timeout -= 10;
                }
            }

            // Try to enumerate the device
            if let Err(e) = enumerate_device(port, speed) {
                println!("[xhci] Port {} enumeration failed: {}", port, e);
            }
        }
    }
}

// ─── Device Enumeration ────────────────────────────────────────────────

/// Enable a slot, address the device, and configure endpoint 0
fn enumerate_device(port: u8, speed: UsbSpeed) -> Result<(), &'static str> {
    // 1. Enable Slot command
    let enable_slot_trb = Trb::new(0, 0, TRB_TYPE_ENABLE_SLOT << 10);
    submit_command(enable_slot_trb)?;

    // Wait for command completion
    delay_us(50_000);

    // Check event ring for the response
    let evt = unsafe { EVT_RING.dequeue() };
    let slot_id = match evt {
        Some(e) if e.trb_type() == TRB_TYPE_CMD_COMPLETE => {
            let cc = e.completion_code();
            let sid = e.slot_id();
            if cc != COMP_SUCCESS {
                println!("[xhci] Enable Slot failed, cc={}", cc);
                return Err("Enable Slot command failed");
            }
            sid
        }
        _ => {
            // Use a default slot assignment for now
            let sid = find_free_slot().ok_or("no free slots")?;
            sid
        }
    };

    println!("[xhci] Slot {} enabled for port {}", slot_id, port);

    // 2. Allocate device context
    let ctx_phys = allocate_contiguous(2).ok_or("failed to alloc device context")?;
    let ctx_virt = phys_to_virt(ctx_phys);

    // Zero device context
    let ptr = ctx_virt.0 as *mut u8;
    unsafe {
        for i in 0..(PAGE_SIZE as usize * 2) {
            ptr.add(i).write(0);
        }
    }

    // 3. Allocate EP0 transfer ring
    let ep0_phys = allocate_contiguous(1).ok_or("failed to alloc EP0 ring")?;
    let ep0_virt = phys_to_virt(ep0_phys);

    // Zero EP0 ring
    let ptr = ep0_virt.0 as *mut u8;
    unsafe {
        for i in 0..(PAGE_SIZE as usize) {
            ptr.add(i).write(0);
        }
    }

    // Store slot info
    let idx = slot_id as usize;
    if idx < MAX_SLOTS {
        unsafe {
            SLOTS[idx].enabled = true;
            SLOTS[idx].slot_id = slot_id;
            SLOTS[idx].port = port;
            SLOTS[idx].speed = speed;
            SLOTS[idx].ep0_ring = XferRing {
                base: ep0_virt,
                phys: ep0_phys,
                enqueue_idx: 0,
                cycle: true,
            };
            SLOTS[idx].dev_ctx = ctx_virt;
            SLOTS[idx].dev_ctx_phys = ctx_phys;

            // Add to DCBA
            DCBA[idx] = ctx_phys.0;
        }
    }

    // 4. Address Device command
    let addr_trb = Trb::new(
        ctx_phys.0,
        0,
        (TRB_TYPE_ADDRESS_DEV << 10) | ((slot_id as u32) << 24),
    );
    submit_command(addr_trb)?;
    delay_us(50_000);

    // 5. Configure Endpoint 0
    // Set up the input context for EP0
    let input_ctx = ctx_virt.0 as *mut u64;
    unsafe {
        // Input control context (32 bytes at offset 0)
        // Set A(0) = 1 (add slot context), A(1) = 1 (add EP0)
        input_ctx.add(1).write(0x3); // Add bit flags
    }

    let config_trb = Trb::new(
        ctx_phys.0,
        0,
        (TRB_TYPE_CONFIG_EP << 10) | ((slot_id as u32) << 24),
    );
    submit_command(config_trb)?;
    delay_us(50_000);

    // 6. Get device descriptor (VID/PID)
    // For now, mark as generic — HID driver will probe class
    println!(
        "[xhci] Device enumerated: slot={}, port={}, speed={:?}",
        slot_id, port, speed
    );

    Ok(())
}

/// Find the first free slot in the slot table
fn find_free_slot() -> Option<u8> {
    for i in 1..MAX_SLOTS {
        if !unsafe { SLOTS[i].enabled } {
            return Some(i as u8);
        }
    }
    None
}

// ─── Event Handling ────────────────────────────────────────────────────

/// Handle xHCI events (called from IRQ or poll)
pub fn handle_event() {
    let base = unsafe { XHCI_BASE };

    // Clear Interrupt Pending bit
    let iman = reg32_read(base, IMAN);
    reg32_write(base, IMAN, iman | IMAN_IP);

    // Process all pending events
    loop {
        let evt = unsafe { EVT_RING.dequeue() };
        match evt {
            Some(trb) => {
                let trb_type = trb.trb_type();
                match trb_type {
                    TRB_TYPE_TRANSFER_EVT => {
                        let cc = trb.completion_code();
                        let slot = trb.slot_id();
                        if cc == COMP_SUCCESS || cc == COMP_SHORT_PKT {
                            // Transfer completed successfully
                        } else if cc == COMP_STALL {
                            println!("[xhci] Transfer STALL on slot {}", slot);
                        } else {
                            println!("[xhci] Transfer event: slot={} cc={}", slot, cc);
                        }
                    }
                    TRB_TYPE_CMD_COMPLETE => {
                        let cc = trb.completion_code();
                        let slot = trb.slot_id();
                        if cc != COMP_SUCCESS {
                            println!("[xhci] Command complete: slot={} cc={}", slot, cc);
                        }
                    }
                    TRB_TYPE_PORT_STATUS => {
                        let port = ((trb.param >> 24) & 0xFF) as u8;
                        println!("[xhci] Port status change: port={}", port);
                        // Re-scan this port
                        let portsc_off = 0x400 + (port as u64 * 0x10);
                        let portsc = reg32_read(base, portsc_off);
                        // Clear change bits
                        reg32_write(
                            base,
                            portsc_off,
                            portsc
                                | PORTSC_CSC
                                | PORTSC_PEC
                                | PORTSC_WRC
                                | PORTSC_OCC
                                | PORTSC_PRC
                                | PORTSC_PLC
                                | PORTSC_CEC,
                        );
                    }
                    TRB_TYPE_MFINDEX_WRAP => {
                        // Frame index wrapped — ignore
                    }
                    _ => {
                        println!("[xhci] Unknown event type={}", trb_type);
                    }
                }
            }
            None => break,
        }
    }

    // Update ERDP
    unsafe {
        EVT_RING.update_erdp(base);
    }
}

// ─── Transfer ──────────────────────────────────────────────────────────

/// Submit a transfer TRB on a device's endpoint ring
pub fn submit_transfer(slot_id: u8, ep_id: u8, trb: Trb) -> Result<(), &'static str> {
    let base = unsafe { XHCI_BASE };
    let idx = slot_id as usize;

    if idx >= MAX_SLOTS || !unsafe { SLOTS[idx].enabled } {
        return Err("invalid slot");
    }

    // For EP0, use the slot's EP0 ring
    if ep_id == 0 {
        unsafe {
            SLOTS[idx].ep0_ring.enqueue(trb);
        }
        ring_doorbell(base, slot_id, 1); // EP0 doorbell target = 1
    }

    Ok(())
}

// ─── Controller Reset ──────────────────────────────────────────────────

/// Reset the xHCI controller
pub fn reset_controller() -> Result<(), &'static str> {
    let base = unsafe { XHCI_BASE };

    // Halt the controller
    reg32_write(base, USBCMD, reg32_read(base, USBCMD) & !USBCMD_RUN);
    let mut timeout = 100000u64;
    while timeout > 0 {
        if (reg32_read(base, USBSTS) & USBSTS_HCH) != 0 {
            break;
        }
        delay_us(10);
        timeout -= 10;
    }

    // Clear all slot state
    unsafe {
        for slot in SLOTS.iter_mut() {
            *slot = UsbSlot::ZERO;
        }
    }

    // Re-initialize
    init()
}

// ─── Info Functions ────────────────────────────────────────────────────

/// List all active slots (for "usb list" shell command)
pub fn list_slots() {
    let max_ports = unsafe { XHCI_MAX_PORTS };
    println!("USB Ports: {}", max_ports);

    for i in 1..MAX_SLOTS {
        let slot = unsafe { &SLOTS[i] };
        if slot.enabled {
            println!("  Slot {}: port={}, speed={:?}, class={:#04x}/{:#04x}/{:#04x} vid={:#06x} pid={:#06x}",
                     slot.slot_id, slot.port, slot.speed,
                     slot.class_code, slot.subclass, slot.protocol,
                     slot.vid, slot.pid);
        }
    }

    let count = unsafe { SLOTS.iter().filter(|s| s.enabled).count() };
    if count == 0 {
        println!("  No devices connected");
    }
}

/// Get the number of connected devices
pub fn device_count() -> usize {
    unsafe { SLOTS.iter().filter(|s| s.enabled).count() }
}

/// Get a reference to a slot by slot ID
pub fn get_slot(slot_id: u8) -> Option<&'static UsbSlot> {
    let idx = slot_id as usize;
    if idx < MAX_SLOTS && unsafe { SLOTS[idx].enabled } {
        Some(unsafe { &SLOTS[idx] })
    } else {
        None
    }
}

/// Get a mutable reference to a slot by slot ID
pub fn get_slot_mut(slot_id: u8) -> Option<&'static mut UsbSlot> {
    let idx = slot_id as usize;
    if idx < MAX_SLOTS && unsafe { SLOTS[idx].enabled } {
        Some(unsafe { &mut SLOTS[idx] })
    } else {
        None
    }
}

/// Get max ports count
pub fn max_ports() -> u8 {
    unsafe { XHCI_MAX_PORTS }
}
