//! PCIe root complex driver for BCM2712 (Raspberry Pi 5)
//!
//! BCM2712 has a PCIe 2.0 x4 root complex. On Pi 5, it's used for:
//! - xHCI USB controller (already handled by usb/xhci.rs)
//! - NVMe SSD (via M.2 HAT+)
//!
//! The PCIe controller registers are memory-mapped. We perform:
//! - Root complex initialization (link training, reset)
//! - Configuration space access (ECAM type 0/1)
//! - Bus enumeration (scan bus 0, assign bus numbers)
//! - BAR assignment and address translation
//! - Interrupt routing via GIC ITS
//!
//! References: BCM2712 ARM Peripherals, PCIe 2.0 Base Specification

use crate::arch::cpu::delay_us;
use crate::memory::{phys_to_virt, PhysAddr};
use crate::println;
use core::ptr::{read_volatile, write_volatile};

// ═══════════════════════════════════════════════════════════════════
// BCM2712 PCIe Controller Memory Map
// ═══════════════════════════════════════════════════════════════════

/// PCIe controller register base (BCM2712)
const PCIE_RC_BASE: u64 = 0x0001_00D0_0000;

/// PCIe ECAM (Enhanced Configuration Access Method) base
/// 256MB window: Bus[7:0] Dev[4:0] Fun[2:0] Reg[11:0]
const PCIE_ECAM_BASE: u64 = 0x0001_00E0_0000;

// MSI-X table base (allocated at runtime)
// We allocate a page for MSI-X tables if needed
// ── PCIe Root Complex Register Offsets ──

/// PCI Command Register (config space)
const PCI_CMD: u32 = 0x04;
/// PCI Status Register
const PCI_STS: u32 = 0x06;
/// PCI Class Code / Revision
const PCI_CLASS_REV: u32 = 0x08;
/// PCI BAR0
const PCI_BAR0: u32 = 0x10;
/// PCI BAR1
const PCI_BAR1: u32 = 0x14;
/// PCI BAR2
const PCI_BAR2: u32 = 0x18;
/// PCI BAR3
const PCI_BAR3: u32 = 0x1C;
/// PCI BAR4
const PCI_BAR4: u32 = 0x20;
/// PCI BAR5
const PCI_BAR5: u32 = 0x24;
/// PCI Capabilities Pointer
const PCI_CAP_PTR: u32 = 0x34;
/// PCI Interrupt Line
const PCI_INT_LINE: u32 = 0x3C;
/// PCI Interrupt Pin
const PCI_INT_PIN: u32 = 0x3D;

// ── PCIe Extended Config Space ──

/// PCIe Capabilities (extended) — offset 0x40 in config space
const PCIE_CAP_ID: u32 = 0x10; // PCI Express Capability ID
const PCIE_CAP_LINK: u32 = 0x12; // Link Capabilities
const PCIE_CAP_LINK_STS: u32 = 0x16; // Link Status

// ── BCM2712 PCIe RC Registers ──

/// RC Configuration Registers (local to the controller)
const RC_CFG_VENDOR: u64 = PCIE_RC_BASE + 0x0000;
const RC_CFG_CMD: u64 = PCIE_RC_BASE + 0x0004;
const RC_CFG_CLASS: u64 = PCIE_RC_BASE + 0x0008;

/// RC Reset and Status
const RC_RESET_CTRL: u64 = PCIE_RC_BASE + 0x1000;
const RC_RESET_STATUS: u64 = PCIE_RC_BASE + 0x1004;

/// RC Link Training
const RC_LINK_CTRL: u64 = PCIE_RC_BASE + 0x1010;
const RC_LINK_STS: u64 = PCIE_RC_BASE + 0x1014;

/// RC Bus Number Register
const RC_BUS_NUM: u64 = PCIE_RC_BASE + 0x1020;

/// RC I/O and Memory Base/Limit
const RC_MEM_BASE: u64 = PCIE_RC_BASE + 0x1030;
const RC_MEM_LIMIT: u64 = PCIE_RC_BASE + 0x1034;
const RC_PREFETCH_BASE: u64 = PCIE_RC_BASE + 0x1038;
const RC_PREFETCH_LIMIT: u64 = PCIE_RC_BASE + 0x103C;

/// RC ATU (Address Translation Unit) registers
const RC_ATU_VIEWPORT: u64 = PCIE_RC_BASE + 0x2000;
const RC_ATU_CTRL1: u64 = PCIE_RC_BASE + 0x2004;
const RC_ATU_CTRL2: u64 = PCIE_RC_BASE + 0x2008;
const RC_ATU_BASE_LOW: u64 = PCIE_RC_BASE + 0x200C;
const RC_ATU_BASE_HIGH: u64 = PCIE_RC_BASE + 0x2010;
const RC_ATU_LIMIT: u64 = PCIE_RC_BASE + 0x2014;
const RC_ATU_TARGET_LOW: u64 = PCIE_RC_BASE + 0x2018;
const RC_ATU_TARGET_HIGH: u64 = PCIE_RC_BASE + 0x201C;

// ── Reset Control bits ──
const RESET_PCIE_PHY: u32 = 1 << 0;
const RESET_PCIE_RC: u32 = 1 << 1;
const RESET_PCIE_LINK: u32 = 1 << 2;
const RESET_PCIE_ACTIVE: u32 = 1 << 3;

// ── Link Control bits ──
const LINK_TRAIN_EN: u32 = 1 << 0;
const LINK_RETRAIN: u32 = 1 << 1;
const LINK_DISABLE: u32 = 1 << 2;

// ── Link Status bits ──
const LINK_STS_TRAINING: u32 = 1 << 0;
const LINK_STS_UP: u32 = 1 << 1;
const LINK_STS_SPEED_MASK: u32 = 0x3 << 2;
const LINK_STS_WIDTH_MASK: u32 = 0x7 << 4;

// ── ATU Control bits ──
const ATU_TYPE_MEM: u32 = 0x0;
const ATU_TYPE_IO: u32 = 0x2;
const ATU_TYPE_CFG0: u32 = 0x4;
const ATU_TYPE_CFG1: u32 = 0x5;
const ATU_ENABLE: u32 = 1 << 31;

// ── PCI Command Register bits ──
const PCI_CMD_IO: u32 = 1 << 0;
const PCI_CMD_MEMORY: u32 = 1 << 1;
const PCI_CMD_BUS_MASTER: u32 = 1 << 2;

// ═══════════════════════════════════════════════════════════════════
// PCIe Configuration Space Access (ECAM)
// ═══════════════════════════════════════════════════════════════════

/// Compute ECAM address for Bus/Dev/Func/Register
#[inline]
fn ecam_addr(bus: u8, dev: u8, func: u8, reg: u32) -> u64 {
    PCIE_ECAM_BASE
        | ((bus as u64) << 20)
        | ((dev as u64) << 15)
        | ((func as u64) << 12)
        | ((reg as u64) & 0xFFC)
}

/// Read 32-bit PCI configuration space register via ECAM
pub unsafe fn config_read32(bus: u8, dev: u8, func: u8, reg: u32) -> u32 {
    let addr = ecam_addr(bus, dev, func, reg);
    read_volatile(addr as *const u32)
}

/// Write 32-bit PCI configuration space register via ECAM
pub unsafe fn config_write32(bus: u8, dev: u8, func: u8, reg: u32, val: u32) {
    let addr = ecam_addr(bus, dev, func, reg);
    write_volatile(addr as *mut u32, val);
}

/// Read 16-bit PCI config register
pub unsafe fn config_read16(bus: u8, dev: u8, func: u8, reg: u32) -> u16 {
    let val = config_read32(bus, dev, func, reg & !0x3);
    ((val >> ((reg & 0x2) * 8)) & 0xFFFF) as u16
}

/// Write 16-bit PCI config register
pub unsafe fn config_write16(bus: u8, dev: u8, func: u8, reg: u32, val: u16) {
    let old = config_read32(bus, dev, func, reg & !0x3);
    let shift = (reg & 0x2) * 8;
    let new = (old & !(0xFFFF << shift)) | ((val as u32) << shift);
    config_write32(bus, dev, func, reg & !0x3, new);
}

/// Read 8-bit PCI config register
pub unsafe fn config_read8(bus: u8, dev: u8, func: u8, reg: u32) -> u8 {
    let val = config_read32(bus, dev, func, reg & !0x3);
    ((val >> ((reg & 0x3) * 8)) & 0xFF) as u8
}

// ═══════════════════════════════════════════════════════════════════
// PCIe Device Structure
// ═══════════════════════════════════════════════════════════════════

/// Maximum number of PCIe devices we track
const MAX_PCIE_DEVICES: usize = 8;

/// PCIe device information
#[derive(Clone, Copy)]
pub struct PcieDevice {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub bar: [u64; 6],
    pub bar_size: [u64; 6],
    pub bar_64bit: [bool; 6],
    pub bar_prefetch: [bool; 6],
    pub irq_pin: u8,
    pub irq_line: u8,
    pub msi_cap_off: u8,
    pub msix_cap_off: u8,
    pub pcie_cap_off: u8,
    pub in_use: bool,
}

impl Default for PcieDevice {
    fn default() -> Self {
        PcieDevice {
            bus: 0,
            dev: 0,
            func: 0,
            vendor_id: 0xFFFF,
            device_id: 0xFFFF,
            class_code: 0,
            subclass: 0,
            prog_if: 0,
            revision: 0,
            bar: [0; 6],
            bar_size: [0; 6],
            bar_64bit: [false; 6],
            bar_prefetch: [false; 6],
            irq_pin: 0,
            irq_line: 0,
            msi_cap_off: 0,
            msix_cap_off: 0,
            pcie_cap_off: 0,
            in_use: false,
        }
    }
}

impl PcieDevice {
    /// Check if this is a valid device (vendor != 0xFFFF)
    pub fn is_valid(&self) -> bool {
        self.vendor_id != 0xFFFF && self.in_use
    }

    /// Class code description
    pub fn class_name(&self) -> &'static str {
        match self.class_code {
            0x01 => "Storage",
            0x02 => "Network",
            0x03 => "Display",
            0x04 => "Multimedia",
            0x05 => "Memory",
            0x06 => "Bridge",
            0x07 => "Comm",
            0x08 => "System",
            0x09 => "Input",
            0x0A => "Docking",
            0x0B => "Processor",
            0x0C => "Serial Bus",
            0x0D => "Wireless",
            0xFF => "Other",
            _ => "Unknown",
        }
    }

    /// Is this an NVMe device? (class=0x01 mass storage, subclass=0x08 NVMe)
    pub fn is_nvme(&self) -> bool {
        self.class_code == 0x01 && self.subclass == 0x08 && self.prog_if == 0x02
    }

    /// Is this a USB xHCI controller? (class=0x0C serial, subclass=0x03 USB, prog=0x30 xHCI)
    pub fn is_xhci(&self) -> bool {
        self.class_code == 0x0C && self.subclass == 0x03 && self.prog_if == 0x30
    }
}

/// Global PCIe device list
static mut PCIE_DEVICES: [PcieDevice; MAX_PCIE_DEVICES] = [PcieDevice {
    bus: 0,
    dev: 0,
    func: 0,
    vendor_id: 0xFFFF,
    device_id: 0xFFFF,
    class_code: 0,
    subclass: 0,
    prog_if: 0,
    revision: 0,
    bar: [0; 6],
    bar_size: [0; 6],
    bar_64bit: [false; 6],
    bar_prefetch: [false; 6],
    irq_pin: 0,
    irq_line: 0,
    msi_cap_off: 0,
    msix_cap_off: 0,
    pcie_cap_off: 0,
    in_use: false,
}; MAX_PCIE_DEVICES];

/// Number of discovered PCIe devices
static mut PCIE_DEVICE_COUNT: usize = 0;

// ═══════════════════════════════════════════════════════════════════
// PCIe Memory Allocation Region
// ═══════════════════════════════════════════════════════════════════

/// PCIe memory window: 4MB starting at this physical address
/// We allocate from this pool for BAR assignment
const PCIE_MEM_POOL_START: u64 = 0x10_1C00_0000;
const PCIE_MEM_POOL_SIZE: u64 = 4 * 1024 * 1024; // 4MB

/// Next available PCIe memory address
static mut PCIE_MEM_NEXT: u64 = PCIE_MEM_POOL_START;

/// Allocate PCIe memory for BAR assignment
fn pcie_mem_alloc(size: u64) -> u64 {
    unsafe {
        // Align to 64KB (typical BAR alignment)
        let align = 0x10000u64;
        let aligned_next = (PCIE_MEM_NEXT + align - 1) & !(align - 1);
        if aligned_next + size > PCIE_MEM_POOL_START + PCIE_MEM_POOL_SIZE {
            println!("[pcie] ERROR: PCIe memory pool exhausted!");
            return 0;
        }
        let addr = aligned_next;
        PCIE_MEM_NEXT = aligned_next + size;
        addr
    }
}

// ═══════════════════════════════════════════════════════════════════
// PCIe Root Complex Initialization
// ═══════════════════════════════════════════════════════════════════

/// Initialize the PCIe root complex
pub unsafe fn init() {
    println!("[pcie] Initializing PCIe root complex...");

    // Step 1: Deassert PCIe reset
    write_volatile(RC_RESET_CTRL as *mut u32, 0);
    delay_us(100);

    // Step 2: Wait for reset to clear
    let mut timeout = 1000;
    while (read_volatile(RC_RESET_STATUS as *const u32) & RESET_PCIE_ACTIVE) != 0 {
        timeout -= 1;
        if timeout == 0 {
            println!("[pcie] WARNING: PCIe reset timeout");
            break;
        }
        delay_us(10);
    }

    // Step 3: Configure bus numbers (primary=0, secondary=1, subordinate=1)
    write_volatile(RC_BUS_NUM as *mut u32, 0x00_01_01_00); // Sub=1, Sec=1, Pri=0

    // Step 4: Configure memory window
    write_volatile(RC_MEM_BASE as *mut u32, (PCIE_MEM_POOL_START >> 16) as u32);
    write_volatile(
        RC_MEM_LIMIT as *mut u32,
        ((PCIE_MEM_POOL_START + PCIE_MEM_POOL_SIZE - 1) >> 16) as u32,
    );

    // Step 5: Start link training
    write_volatile(RC_LINK_CTRL as *mut u32, LINK_TRAIN_EN);
    delay_us(1000);

    // Step 6: Wait for link to come up
    timeout = 50000;
    loop {
        let sts = read_volatile(RC_LINK_STS as *const u32);
        if sts & LINK_STS_UP != 0 {
            let speed = (sts & LINK_STS_SPEED_MASK) >> 2;
            let width = (sts & LINK_STS_WIDTH_MASK) >> 4;
            println!("[pcie] Link up! Speed=Gen{}, Width=x{}", speed + 1, width);
            break;
        }
        timeout -= 1;
        if timeout == 0 {
            println!("[pcie] WARNING: PCIe link training timeout (no devices?)");
            break;
        }
        delay_us(10);
    }

    // Step 7: Enable the root complex as bus master
    let cmd = read_volatile(RC_CFG_CMD as *const u32);
    write_volatile(
        RC_CFG_CMD as *mut u32,
        cmd | PCI_CMD_IO | PCI_CMD_MEMORY | PCI_CMD_BUS_MASTER,
    );

    // Step 8: Enumerate devices
    enumerate_bus(0);

    println!(
        "[pcie] Initialization complete. {} device(s) found.",
        PCIE_DEVICE_COUNT
    );
}

// ═══════════════════════════════════════════════════════════════════
// PCIe Bus Enumeration
// ═══════════════════════════════════════════════════════════════════

/// Scan a PCI bus for devices
unsafe fn enumerate_bus(bus: u8) {
    for dev in 0..32 {
        scan_device(bus, dev, 0);
    }
}

/// Scan a specific Bus/Dev/Func
unsafe fn scan_device(bus: u8, dev: u8, func: u8) {
    let vendor_id = config_read16(bus, dev, func, 0x00);
    if vendor_id == 0xFFFF {
        return; // No device
    }

    let device_id = config_read16(bus, dev, func, 0x02);
    let class_rev = config_read32(bus, dev, func, PCI_CLASS_REV);
    let revision = (class_rev & 0xFF) as u8;
    let prog_if = ((class_rev >> 8) & 0xFF) as u8;
    let subclass = ((class_rev >> 16) & 0xFF) as u8;
    let class_code = ((class_rev >> 24) & 0xFF) as u8;
    let irq_pin = config_read8(bus, dev, func, PCI_INT_PIN);
    let irq_line = config_read8(bus, dev, func, PCI_INT_LINE);

    println!(
        "[pcie] Found: {:02x}:{:02x}.{} VID={:04x} DID={:04x} Class={:02x}:{:02x}:{:02x} Rev={:02x}",
        bus, dev, func, vendor_id, device_id, class_code, subclass, prog_if, revision
    );

    // Add to device list
    let idx = PCIE_DEVICE_COUNT;
    if idx >= MAX_PCIE_DEVICES {
        println!("[pcie] WARNING: Too many PCIe devices, skipping");
        return;
    }

    let mut pcie_dev = PcieDevice {
        bus,
        dev,
        func,
        vendor_id,
        device_id,
        class_code,
        subclass,
        prog_if,
        revision,
        bar: [0; 6],
        bar_size: [0; 6],
        bar_64bit: [false; 6],
        bar_prefetch: [false; 6],
        irq_pin,
        irq_line,
        msi_cap_off: 0,
        msix_cap_off: 0,
        pcie_cap_off: 0,
        in_use: true,
    };

    // Read and assign BARs
    assign_bars(&mut pcie_dev);

    // Scan capabilities
    scan_capabilities(&mut pcie_dev);

    // Enable the device (bus master, memory space, I/O space)
    let cmd = config_read16(bus, dev, func, PCI_CMD);
    config_write16(
        bus,
        dev,
        func,
        PCI_CMD,
        cmd | PCI_CMD_IO as u16 | PCI_CMD_MEMORY as u16 | PCI_CMD_BUS_MASTER as u16,
    );

    // If this is a bridge, enumerate behind it
    if class_code == 0x06 {
        // PCI Bridge — scan secondary bus
        let sec_bus = config_read8(bus, dev, func, 0x19);
        if sec_bus != 0 {
            println!(
                "[pcie] Bridge at {:02x}:{:02x}.{} → secondary bus {}",
                bus, dev, func, sec_bus
            );
            enumerate_bus(sec_bus);
        }
    }

    // If multi-function device header, scan other functions
    if func == 0 {
        let header_type = config_read8(bus, dev, func, 0x0E);
        if header_type & 0x80 != 0 {
            // Multi-function device
            for f in 1..8 {
                scan_device(bus, dev, f);
            }
        }
    }

    PCIE_DEVICES[idx] = pcie_dev;
    PCIE_DEVICE_COUNT += 1;
}

// ═══════════════════════════════════════════════════════════════════
// BAR Assignment
// ═══════════════════════════════════════════════════════════════════

/// Read and assign all BARs for a device
unsafe fn assign_bars(dev: &mut PcieDevice) {
    let mut bar_idx = 0;
    let mut reg = PCI_BAR0;

    while bar_idx < 6 && reg <= PCI_BAR5 {
        // Write all 1s to BAR to determine size
        config_write32(dev.bus, dev.dev, dev.func, reg, 0xFFFFFFFF);
        let mask = config_read32(dev.bus, dev.dev, dev.func, reg);

        if mask == 0 {
            // Not implemented BAR
            config_write32(dev.bus, dev.dev, dev.func, reg, 0);
            bar_idx += 1;
            reg += 4;
            continue;
        }

        // Check if 64-bit BAR
        let is_64bit = (mask & 0x4) != 0;
        let is_prefetch = (mask & 0x8) != 0;

        // Determine BAR size
        let size: u64;
        if is_64bit {
            // Write all 1s to upper BAR too
            config_write32(dev.bus, dev.dev, dev.func, reg + 4, 0xFFFFFFFF);
            let mask_hi = config_read32(dev.bus, dev.dev, dev.func, reg + 4);

            let full_mask = ((mask_hi as u64) << 32) | ((mask as u64) & !0xF);
            size = !(full_mask) + 1;

            dev.bar_64bit[bar_idx] = true;
            dev.bar_prefetch[bar_idx] = is_prefetch;

            // Allocate from PCIe memory pool
            let assigned = pcie_mem_alloc(size);
            if assigned != 0 {
                config_write32(
                    dev.bus,
                    dev.dev,
                    dev.func,
                    reg,
                    (assigned & 0xFFFFFFF0) as u32 | (mask & 0xF),
                );
                config_write32(dev.bus, dev.dev, dev.func, reg + 4, (assigned >> 32) as u32);
                dev.bar[bar_idx] = assigned;
                dev.bar_size[bar_idx] = size;
                println!(
                    "[pcie]   BAR{}: 64-bit {:#018x} size={:#x}{}{}",
                    bar_idx,
                    assigned,
                    size,
                    if is_prefetch { " prefetch" } else { "" },
                    if is_64bit { " 64bit" } else { "" }
                );
            }

            // Skip next BAR register (used by upper 32 bits)
            bar_idx += 1;
            reg += 8; // Skip two registers
            continue;
        } else {
            // 32-bit BAR
            let bar_mask = (mask as u64) & !0xF;
            size = !(bar_mask) + 1;

            dev.bar_64bit[bar_idx] = false;
            dev.bar_prefetch[bar_idx] = is_prefetch;

            let assigned = pcie_mem_alloc(size);
            if assigned != 0 {
                config_write32(
                    dev.bus,
                    dev.dev,
                    dev.func,
                    reg,
                    ((assigned & 0xFFFFFFF0) as u32) | (mask & 0xF),
                );
                dev.bar[bar_idx] = assigned;
                dev.bar_size[bar_idx] = size;
                println!(
                    "[pcie]   BAR{}: 32-bit {:#010x} size={:#x}{}",
                    bar_idx,
                    assigned as u32,
                    size,
                    if is_prefetch { " prefetch" } else { "" }
                );
            }

            bar_idx += 1;
            reg += 4;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Capability Scanning
// ═══════════════════════════════════════════════════════════════════

/// Scan PCI capabilities linked list
unsafe fn scan_capabilities(dev: &mut PcieDevice) {
    let cap_ptr = config_read8(dev.bus, dev.dev, dev.func, PCI_CAP_PTR);
    if cap_ptr == 0 {
        return;
    }

    let mut offset = cap_ptr as u32;
    let mut max_iter = 48; // Safety limit

    while offset != 0 && max_iter > 0 {
        let cap_id = config_read8(dev.bus, dev.dev, dev.func, offset);
        let next = config_read8(dev.bus, dev.dev, dev.func, offset + 1);

        match cap_id {
            0x10 => {
                // PCIe Capability
                dev.pcie_cap_off = offset as u8;
                let link_sts = config_read16(dev.bus, dev.dev, dev.func, offset + 0x12);
                let speed = link_sts & 0xF;
                let width = (link_sts >> 4) & 0x3F;
                println!("[pcie]   PCIe Cap: Gen{} x{}", speed, width);
            }
            0x05 => {
                // MSI Capability
                dev.msi_cap_off = offset as u8;
                println!("[pcie]   MSI Cap at offset {:#x}", offset);
            }
            0x11 => {
                // MSI-X Capability
                dev.msix_cap_off = offset as u8;
                println!("[pcie]   MSI-X Cap at offset {:#x}", offset);
            }
            _ => {
                // Unknown capability — skip
            }
        }

        offset = next as u32;
        max_iter -= 1;
    }
}

// ═══════════════════════════════════════════════════════════════════
// ATU (Address Translation Unit) Configuration
// ═══════════════════════════════════════════════════════════════════

/// Configure an ATU region for outbound memory access
/// Maps CPU physical addresses → PCIe bus addresses
pub unsafe fn configure_atu_outbound(
    index: u32,
    type_: u32,
    cpu_base: u64,
    pci_base: u64,
    size: u64,
) {
    // Select ATU region
    write_volatile(RC_ATU_VIEWPORT as *mut u32, index);

    // Configure region
    write_volatile(RC_ATU_CTRL1 as *mut u32, type_);
    write_volatile(RC_ATU_CTRL2 as *mut u32, ATU_ENABLE);
    write_volatile(RC_ATU_BASE_LOW as *mut u32, (cpu_base & 0xFFFFFFFF) as u32);
    write_volatile(RC_ATU_BASE_HIGH as *mut u32, (cpu_base >> 32) as u32);
    write_volatile(
        RC_ATU_LIMIT as *mut u32,
        ((cpu_base + size - 1) & 0xFFFFFFFF) as u32,
    );
    write_volatile(
        RC_ATU_TARGET_LOW as *mut u32,
        (pci_base & 0xFFFFFFFF) as u32,
    );
    write_volatile(RC_ATU_TARGET_HIGH as *mut u32, (pci_base >> 32) as u32);

    // Flush with a dummy read
    read_volatile(RC_ATU_CTRL2 as *const u32);
}

// ═══════════════════════════════════════════════════════════════════
// Public Query API
// ═══════════════════════════════════════════════════════════════════

/// Get number of discovered PCIe devices
pub fn device_count() -> usize {
    unsafe { PCIE_DEVICE_COUNT }
}

/// Get a reference to a PCIe device by index
pub fn get_device(idx: usize) -> Option<&'static PcieDevice> {
    unsafe {
        if idx < PCIE_DEVICE_COUNT && PCIE_DEVICES[idx].in_use {
            Some(&PCIE_DEVICES[idx])
        } else {
            None
        }
    }
}

/// Find a device by class/subclass/prog_if
pub fn find_device(class: u8, subclass: u8, prog_if: u8) -> Option<usize> {
    unsafe {
        for i in 0..PCIE_DEVICE_COUNT {
            if PCIE_DEVICES[i].class_code == class
                && PCIE_DEVICES[i].subclass == subclass
                && (prog_if == 0xFF || PCIE_DEVICES[i].prog_if == prog_if)
            {
                return Some(i);
            }
        }
        None
    }
}

/// Find an NVMe device
pub fn find_nvme() -> Option<usize> {
    // NVMe: class=0x01 (mass storage), subclass=0x08 (NVM), prog_if=0x02 (NVMe)
    find_device(0x01, 0x08, 0x02).or_else(|| {
        // Some NVMe drives report as class=0xFF (other) with vendor-specific
        // Try matching by vendor/device ID if known
        unsafe {
            for i in 0..PCIE_DEVICE_COUNT {
                if !PCIE_DEVICES[i].in_use {
                    continue;
                }
                // Check if it has an NVMe admin queue BAR
                // Heuristic: mass storage class with non-zero BAR0
                if PCIE_DEVICES[i].class_code == 0x01 && PCIE_DEVICES[i].bar[0] != 0 {
                    return Some(i);
                }
            }
            None
        }
    })
}

/// Dump all PCIe devices
pub fn dump_devices() {
    unsafe {
        println!("[pcie] === Device List ===");
        for i in 0..PCIE_DEVICE_COUNT {
            let d = &PCIE_DEVICES[i];
            println!(
                "[pcie] {}: {:02x}:{:02x}.{} {} VID={:04x} DID={:04x} {} BAR0={:#x}",
                i,
                d.bus,
                d.dev,
                d.func,
                d.class_name(),
                d.vendor_id,
                d.device_id,
                if d.is_nvme() {
                    "[NVMe]"
                } else if d.is_xhci() {
                    "[xHCI]"
                } else {
                    ""
                },
                d.bar[0]
            );
        }
    }
}

/// Map a device BAR into the kernel's virtual address space
/// Returns the virtual address of the mapped BAR region
pub fn map_bar(dev_idx: usize, bar_idx: usize) -> u64 {
    unsafe {
        if dev_idx >= PCIE_DEVICE_COUNT || bar_idx >= 6 {
            return 0;
        }
        let dev = &PCIE_DEVICES[dev_idx];
        if dev.bar[bar_idx] == 0 || dev.bar_size[bar_idx] == 0 {
            return 0;
        }

        let phys = dev.bar[bar_idx];
        let size = dev.bar_size[bar_idx];

        // BAR addresses on BCM2712 are already in the physical address space
        // that the kernel has 1:1 mapped via phys_to_virt
        let virt = phys_to_virt(PhysAddr::new(phys)).0;

        println!(
            "[pcie] Mapped BAR{} of dev {} → phys={:#x} virt={:#x} size={:#x}",
            bar_idx, dev_idx, phys, virt, size
        );
        virt
    }
}
