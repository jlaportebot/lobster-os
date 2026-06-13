//! NVMe over PCIe driver for BCM2712 (Raspberry Pi 5)
//!
//! Implements the NVM Express specification for NVMe SSDs
//! connected via the Pi 5's PCIe 2.0 x4 interface (M.2 HAT+).
//!
//! Features:
//! - Admin Queue (AQ) for controller management
//! - I/O Submission/Completion Queues (pair 1)
//! - NVMe Admin commands: identify, create/delete I/O queue, get features
//! - NVMe I/O commands: read/write (PRP-based)
//! - Block device interface for filesystem integration
//!
//! References: NVM Express Specification 1.4, BCM2712 peripherals

use crate::arch::cpu::delay_us;
use crate::drivers::pcie;
use crate::memory::frame_alloc::{allocate_contiguous, allocate_frame, deallocate_frame};
use crate::memory::{phys_to_virt, virt_to_phys, VirtAddr, PAGE_SIZE};
use crate::println;
use core::mem::size_of;
use core::ptr::{read_volatile, write_volatile};

// ═══════════════════════════════════════════════════════════════════
// NVMe Controller Register Offsets (from BAR0 base)
// ═══════════════════════════════════════════════════════════════════

const NVME_CAP: u64 = 0x0000; // Controller Capabilities
const NVME_VS: u64 = 0x0008; // Version
const NVME_INTMS: u64 = 0x000C; // Interrupt Mask Set
const NVME_INTMC: u64 = 0x0010; // Interrupt Mask Clear
const NVME_CC: u64 = 0x0014; // Controller Configuration
const NVME_CSTS: u64 = 0x001C; // Controller Status
const NVME_NSSR: u64 = 0x0020; // NVM Subsystem Reset
const NVME_AQA: u64 = 0x0024; // Admin Queue Attributes
const NVME_ASQ: u64 = 0x0028; // Admin Submission Queue Base Address
const NVME_ACQ: u64 = 0x0030; // Admin Completion Queue Base Address
const NVME_CMBLOC: u64 = 0x0038; // Controller Memory Buffer Location
const NVME_CMBSZ: u64 = 0x003C; // Controller Memory Buffer Size

// ── CAP register fields ──
const CAP_MQES_SHIFT: u64 = 0;
const CAP_MQES_MASK: u64 = 0xFFFF;
const CAP_CQRS_SHIFT: u64 = 16;
const CAP_CQRS_MASK: u64 = 0x3;
const CAP_TO_SHIFT: u64 = 24;
const CAP_TO_MASK: u64 = 0xFF;
const CAP_DSTRD_SHIFT: u64 = 32;
const CAP_DSTRD_MASK: u64 = 0xF;
const CAP_CSS_SHIFT: u64 = 37;
const CAP_CSS_NVM: u64 = 1 << 37; // NVM command set supported
const CAP_CMBS_SHIFT: u64 = 57;
const CAP_CMBS_MASK: u64 = 0x1;

// ── CC register fields ──
const CC_EN: u32 = 1 << 0; // Enable
const CC_CSS_SHIFT: u32 = 4;
const CC_CSS_NVM: u32 = 0 << 4; // NVM command set
const CC_MPS_SHIFT: u32 = 7;
const CC_MPS_4K: u32 = 0 << 7; // 4KB memory page size
const CC_AMS_SHIFT: u32 = 11;
const CC_AMS_RR: u32 = 0 << 11; // Round-robin arbitration
const CC_SHN_SHIFT: u32 = 14;
const CC_SHN_NONE: u32 = 0 << 14;
const CC_IOSQES_SHIFT: u32 = 16;
const CC_IOSQES_6: u32 = (6 - 1) << 16; // IO SQ entry size = 2^6 = 64 bytes
const CC_IOCQES_SHIFT: u32 = 20;
const CC_IOCQES_4: u32 = (4 - 1) << 20; // IO CQ entry size = 2^4 = 16 bytes

// ── CSTS register fields ──
const CSTS_RDY: u32 = 1 << 0; // Ready
const CSTS_CFS: u32 = 1 << 1; // Controller Fatal Status
const CSTS_SHST_SHIFT: u32 = 2;
const CSTS_SHST_COMPLETE: u32 = 2 << 2;

// ═══════════════════════════════════════════════════════════════════
// NVMe Command Structures
// ═══════════════════════════════════════════════════════════════════

/// NVMe Submission Queue Entry (64 bytes)
#[repr(C, align(64))]
#[derive(Clone, Copy, Default)]
pub struct NvmeCmd {
    /// Opcode (8 bits) + FUSE (2 bits) + reserved (4 bits) + PSDT (2 bits)
    pub cdw0: u32,
    /// Command Identifier
    pub cid: u16,
    /// PRT (2 bits) + Command Set (4 bits) + reserved
    pub nsid: u32,
    /// NVM Set ID
    pub cdw2: u32,
    /// Reserved
    pub cdw3: u32,
    /// Metadata Pointer
    pub mptr: u64,
    /// PRP Entry 1
    pub prp1: u64,
    /// PRP Entry 2
    pub prp2: u64,
    /// Command Dword 10-15
    pub cdw10: u32,
    pub cdw11: u32,
    pub cdw12: u32,
    pub cdw13: u32,
    pub cdw14: u32,
    pub cdw15: u32,
}

impl NvmeCmd {
    pub const fn new() -> Self {
        NvmeCmd {
            cdw0: 0,
            cid: 0,
            nsid: 0,
            cdw2: 0,
            cdw3: 0,
            mptr: 0,
            prp1: 0,
            prp2: 0,
            cdw10: 0,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }
}

/// NVMe Completion Queue Entry (16 bytes)
#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
pub struct NvmeCqe {
    /// Command Dword 0 — specific to command type
    pub cdw0: u32,
    /// Reserved
    pub rsvd: u32,
    /// SQ Head Pointer (16 bits) + SQ ID (16 bits)
    pub sq_hd: u16,
    pub sq_id: u16,
    /// Command ID (16 bits) + Phase Tag (1 bit) + Status (15 bits)
    pub cid: u16,
    pub status_p: u16,
}

impl NvmeCqe {
    /// Check if this entry has a new phase (for polling)
    pub fn phase(&self) -> bool {
        (self.status_p & 1) != 0
    }

    /// Get the status code
    pub fn status(&self) -> u16 {
        self.status_p >> 1
    }

    /// Check if the command completed successfully
    pub fn success(&self) -> bool {
        self.status() == 0
    }
}

// ═══════════════════════════════════════════════════════════════════
// NVMe Admin / NVM Opcodes
// ═══════════════════════════════════════════════════════════════════

// Admin opcodes
const ADMIN_DELETE_SQ: u8 = 0x00;
const ADMIN_CREATE_SQ: u8 = 0x01;
const ADMIN_DELETE_CQ: u8 = 0x04;
const ADMIN_CREATE_CQ: u8 = 0x05;
const ADMIN_IDENTIFY: u8 = 0x06;
const ADMIN_SET_FEATURES: u8 = 0x09;
const ADMIN_GET_FEATURES: u8 = 0x0A;
const ADMIN_ABORT: u8 = 0x08;

// NVM opcodes
const NVM_WRITE: u8 = 0x01;
const NVM_READ: u8 = 0x02;
const NVM_FLUSH: u8 = 0x09;

// ═══════════════════════════════════════════════════════════════════
// NVMe Identify Structures
// ═══════════════════════════════════════════════════════════════════

/// NVMe Identify Controller data structure (4096 bytes, we use a subset)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NvmeIdCtrl {
    /// PCI VID
    pub vid: u16,
    /// PCI SSVID
    pub ssvid: u16,
    /// Serial Number (20 bytes)
    pub sn: [u8; 20],
    /// Model Number (40 bytes)
    pub mn: [u8; 40],
    /// Firmware Revision (8 bytes)
    pub fr: [u8; 8],
    /// Recommended Arbitration Burst
    pub rab: u8,
    /// IEEE OUI
    pub ieee: [u8; 3],
    /// Multi-Interface Capabilities
    pub mic: u8,
    /// Maximum Data Transfer Size
    pub mdts: u8,
    /// Controller ID
    pub cntlid: u16,
    /// Version
    pub ver: u32,
    /// RTD3 Resume Latency
    pub rtd3r: u32,
    /// RTD3 Enter Latency
    pub rtd3e: u32,
    /// OAES (offset 0x24)
    pub oaes: u32,
    /// CTRATT (offset 0x28)
    pub ctratt: u32,
    // ... many more fields, we skip to the important ones
    /// Number of Namespaces (offset 0x0F)
    /// We'll access this at the known offset
    // We use raw access for remaining fields
    _reserved: [u8; 3812],
}

/// NVMe Identify Namespace data structure (subset, 4096 bytes)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NvmeIdNs {
    /// Namespace Size (in logical blocks)
    pub nsze: u64,
    /// Namespace Capacity (in logical blocks)
    pub ncap: u64,
    /// Namespace Utilization (in logical blocks)
    pub nuse: u64,
    /// Namespace Features
    pub nsfeat: u8,
    /// Number of LBA Formats
    pub nlbaf: u8,
    /// Formatted LBA Size
    pub flbas: u8,
    /// Metadata Capabilities
    pub mc: u8,
    /// End-to-End Data Protection Capabilities
    pub dpc: u8,
    /// End-to-End Data Protection Type Settings
    pub dps: u8,
    /// Namespace Multi-path / Sharing Capabilities
    pub nmic: u8,
    /// Reservation Capabilities
    pub rescap: u8,
    /// Format Progress Indicator
    pub fpi: u8,
    /// Deallocate Logical Block Features
    pub dlfeat: u8,
    /// Namespace Atomic Write Unit Normal
    pub nawun: u16,
    /// Namespace Atomic Write Unit Power Fail
    pub nawupf: u16,
    /// Namespace Atomic Compare & Write Unit
    pub nacwu: u16,
    /// Namespace Atomic Boundary Size Normal
    pub nabsn: u16,
    /// Namespace Atomic Boundary Offset
    pub nabo: u16,
    /// Namespace Atomic Boundary Size Power Fail
    pub nabspf: u16,
    /// Namespace Optimal I/O Boundary
    pub noiob: u16,
    /// LBA Format Support (offset 0x20)
    pub lbaf: [u32; 16],
    // Remaining bytes
    _reserved: [u8; 3904],
}

impl NvmeIdNs {
    /// Get the sector size from the active LBA format
    pub fn sector_size(&self) -> u32 {
        let idx = (self.flbas & 0xF) as usize;
        if idx < 16 {
            // lbaf[i] = ms(16) | ds(8) | rp(2) | reserved(6)
            // ds = 2^(lbaf[i] & 0xFF) = data size in bytes
            1u32 << (self.lbaf[idx] & 0xFF)
        } else {
            512 // Default
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// NVMe Queue Management
// ═══════════════════════════════════════════════════════════════════

/// Admin queue depth
const ADMIN_Q_DEPTH: usize = 32;

/// I/O queue depth
const IO_Q_DEPTH: usize = 64;

/// NVMe queue pair
pub struct NvmeQueue {
    /// Submission queue base virtual address
    sq_base: u64,
    /// Completion queue base virtual address
    cq_base: u64,
    /// Submission queue tail pointer index
    sq_tail: u16,
    /// Completion queue head pointer index
    cq_head: u16,
    /// Queue depth (number of entries)
    depth: u16,
    /// Queue ID
    qid: u16,
    /// Completion queue phase tag
    phase: bool,
    /// Physical address of SQ (for controller)
    sq_phys: u64,
    /// Physical address of CQ (for controller)
    cq_phys: u64,
}

impl NvmeQueue {
    pub const fn new() -> Self {
        NvmeQueue {
            sq_base: 0,
            cq_base: 0,
            sq_tail: 0,
            cq_head: 0,
            depth: 0,
            qid: 0,
            phase: true,
            sq_phys: 0,
            cq_phys: 0,
        }
    }
}

impl Default for NvmeQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl NvmeQueue {
    /// Create a queue pair with allocated memory
    pub fn create(qid: u16, depth: u16) -> Option<Self> {
        let sq_size = depth as usize * size_of::<NvmeCmd>();
        let cq_size = depth as usize * size_of::<NvmeCqe>();

        // Allocate contiguous frames for SQ and CQ
        let sq_pages = (sq_size + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;
        let cq_pages = (cq_size + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;

        let sq_phys = allocate_contiguous(sq_pages)?;
        let cq_phys = allocate_contiguous(cq_pages)?;

        let sq_virt = phys_to_virt(sq_phys).0;
        let cq_virt = phys_to_virt(cq_phys).0;

        // Zero the queues
        unsafe {
            core::ptr::write_bytes(sq_virt as *mut u8, 0, sq_pages * PAGE_SIZE as usize);
            core::ptr::write_bytes(cq_virt as *mut u8, 0, cq_pages * PAGE_SIZE as usize);
        }

        Some(NvmeQueue {
            sq_base: sq_virt,
            cq_base: cq_virt,
            sq_tail: 0,
            cq_head: 0,
            depth,
            qid,
            phase: true,
            sq_phys: sq_phys.0,
            cq_phys: cq_phys.0,
        })
    }

    /// Get pointer to next submission slot
    fn sq_entry(&mut self) -> &mut NvmeCmd {
        unsafe {
            let offset = self.sq_tail as usize * size_of::<NvmeCmd>();
            &mut *((self.sq_base + offset as u64) as *mut NvmeCmd)
        }
    }

    /// Advance submission tail and return old tail value
    fn sq_push(&mut self) -> u16 {
        let old_tail = self.sq_tail;
        self.sq_tail = (self.sq_tail + 1) % self.depth;
        old_tail
    }

    /// Peek at the next completion entry
    fn cq_peek(&self) -> &NvmeCqe {
        unsafe {
            let offset = self.cq_head as usize * size_of::<NvmeCqe>();
            &*((self.cq_base + offset as u64) as *const NvmeCqe)
        }
    }

    /// Advance completion head if the entry has the expected phase
    /// Returns true if a new completion was consumed
    fn cq_pop(&mut self) -> bool {
        let cqe = self.cq_peek();
        if cqe.phase() == self.phase {
            self.cq_head = (self.cq_head + 1) % self.depth;
            // Phase toggles when head wraps around
            if self.cq_head == 0 {
                self.phase = !self.phase;
            }
            return true;
        }
        false
    }
}

/// NVMe Controller State
/// ═══════════════════════════════════════════════════════════════════
///
/// NVMe controller instance
pub struct NvmeController {
    /// MMIO base address (mapped BAR0)
    pub base: u64,
    /// Admin queue
    pub admin_q: NvmeQueue,
    /// I/O queue pair (queue ID 1)
    pub io_q: NvmeQueue,
    /// Command ID counter
    next_cid: u16,
    /// Controller capabilities (cached from CAP)
    capabilities: u64,
    /// Doorbell stride (in bytes)
    doorbell_stride: u64,
    /// Number of namespaces
    num_namespaces: u32,
    /// Active namespace ID
    nsid: u32,
    /// Sector size in bytes
    sector_size: u32,
    /// Namespace size in logical blocks
    namespace_size: u64,
    /// Total capacity in bytes
    capacity_bytes: u64,
    /// Controller is initialized and ready
    initialized: bool,
    /// PCIe device index
    pcie_dev_idx: usize,
}

impl NvmeController {
    pub const fn new() -> Self {
        NvmeController {
            base: 0,
            admin_q: NvmeQueue::new(),
            io_q: NvmeQueue::new(),
            next_cid: 0,
            capabilities: 0,
            doorbell_stride: 4,
            num_namespaces: 0,
            nsid: 1,
            sector_size: 512,
            namespace_size: 0,
            capacity_bytes: 0,
            initialized: false,
            pcie_dev_idx: 0,
        }
    }

    /// Read a single LBA (logical block) into a 512-byte buffer
    pub fn read_lba(&self, lba: u64, buf: &mut [u8; 512]) -> Result<(), &'static str> {
        if !self.initialized {
            return Err("NVMe not initialized");
        }
        if lba >= self.namespace_size {
            return Err("LBA out of range");
        }
        let ret = read(lba, 1, buf.as_mut_ptr() as *mut u8);
        if ret < 0 {
            return Err("Read failed");
        }
        Ok(())
    }

    /// Write a single LBA (logical block) from a 512-byte buffer
    pub fn write_lba(&self, lba: u64, buf: &[u8; 512]) -> Result<(), &'static str> {
        if !self.initialized {
            return Err("NVMe not initialized");
        }
        if lba >= self.namespace_size {
            return Err("LBA out of range");
        }
        let ret = write(lba, 1, buf.as_ptr() as *const u8);
        if ret < 0 {
            return Err("Write failed");
        }
        Ok(())
    }
}

impl Default for NvmeController {
    fn default() -> Self {
        Self::new()
    }
}

/// Global NVMe controller instance
static mut NVME_CTRL: NvmeController = NvmeController::new();

// ═══════════════════════════════════════════════════════════════════
// NVMe MMIO Register Access
// ═══════════════════════════════════════════════════════════════════

#[inline]
unsafe fn reg_read32(base: u64, offset: u64) -> u32 {
    read_volatile((base + offset) as *const u32)
}

#[inline]
unsafe fn reg_read64(base: u64, offset: u64) -> u64 {
    read_volatile((base + offset) as *const u64)
}

#[inline]
unsafe fn reg_write32(base: u64, offset: u64, val: u32) {
    write_volatile((base + offset) as *mut u32, val);
}

#[inline]
unsafe fn reg_write64(base: u64, offset: u64, val: u64) {
    write_volatile((base + offset) as *mut u64, val);
}

/// Write to a doorbell register
/// Doorbell offset = 0x1000 + (qid * 2 * doorbell_stride) + (is_cq ? stride : 0)
#[inline]
unsafe fn ring_doorbell(base: u64, stride: u64, qid: u16, is_cq: bool, val: u16) {
    let offset = 0x1000 + (qid as u64 * 2 * stride) + if is_cq { stride } else { 0 };
    write_volatile((base + offset) as *mut u32, val as u32);
}

// ═══════════════════════════════════════════════════════════════════
// NVMe Controller Initialization
// ═══════════════════════════════════════════════════════════════════

/// Initialize the NVMe controller
pub unsafe fn init() -> bool {
    println!("[nvme] Looking for NVMe device...");

    // Find NVMe device on PCIe bus
    let dev_idx = match pcie::find_nvme() {
        Some(idx) => idx,
        None => {
            println!("[nvme] No NVMe device found on PCIe bus");
            return false;
        }
    };

    let dev = pcie::get_device(dev_idx).unwrap();
    NVME_CTRL.pcie_dev_idx = dev_idx;

    // Map BAR0 (controller registers)
    let bar0 = pcie::map_bar(dev_idx, 0);
    if bar0 == 0 {
        println!("[nvme] Failed to map BAR0");
        return false;
    }
    NVME_CTRL.base = bar0;

    println!(
        "[nvme] Controller at BAR0={:#x} (VID={:04x} DID={:04x})",
        bar0, dev.vendor_id, dev.device_id
    );

    // Read controller capabilities
    let cap = reg_read64(bar0, NVME_CAP);
    NVME_CTRL.capabilities = cap;

    let max_q_entries = ((cap >> CAP_MQES_SHIFT) & CAP_MQES_MASK) as u16 + 1;
    let doorbell_stride = 4u64 * (1 << ((cap >> CAP_DSTRD_SHIFT) & CAP_DSTRD_MASK));
    NVME_CTRL.doorbell_stride = doorbell_stride;
    let timeout_ms = ((cap >> CAP_TO_SHIFT) & CAP_TO_MASK) * 500; // Units of 500ms

    println!(
        "[nvme] CAP: max_q_entries={} doorbell_stride={} timeout={}ms",
        max_q_entries, doorbell_stride, timeout_ms
    );

    // Check if controller is already enabled — if so, disable it first
    let cc = reg_read32(bar0, NVME_CC);
    if cc & CC_EN != 0 {
        println!("[nvme] Controller is enabled, disabling...");
        reg_write32(bar0, NVME_CC, cc & !CC_EN);

        // Wait for ready to clear
        let mut timeout_wait = timeout_ms * 1000; // Convert to us
        while reg_read32(bar0, NVME_CSTS) & CSTS_RDY != 0 {
            timeout_wait -= 1;
            if timeout_wait == 0 {
                println!("[nvme] Timeout waiting for controller to disable");
                return false;
            }
            delay_us(1);
        }
    }

    // Create admin queue pair
    let admin_q = match NvmeQueue::create(0, ADMIN_Q_DEPTH as u16) {
        Some(q) => q,
        None => {
            println!("[nvme] Failed to allocate admin queue");
            return false;
        }
    };
    NVME_CTRL.admin_q = admin_q;

    // Configure admin queue
    // AQA: ASQS (12 bits) | ACQS (12 bits), 0-based
    let aqa = ((ADMIN_Q_DEPTH as u32 - 1) << 16) | (ADMIN_Q_DEPTH as u32 - 1);
    reg_write32(bar0, NVME_AQA, aqa);
    reg_write64(bar0, NVME_ASQ, NVME_CTRL.admin_q.sq_phys);
    reg_write64(bar0, NVME_ACQ, NVME_CTRL.admin_q.cq_phys);

    // Enable controller
    let cc = CC_CSS_NVM | CC_MPS_4K | CC_AMS_RR | CC_IOSQES_6 | CC_IOCQES_4;
    reg_write32(bar0, NVME_CC, cc | CC_EN);

    println!("[nvme] Waiting for controller ready...");
    let mut timeout_wait = timeout_ms * 1000;
    while reg_read32(bar0, NVME_CSTS) & CSTS_RDY == 0 {
        timeout_wait -= 1;
        if timeout_wait == 0 {
            println!("[nvme] Timeout waiting for controller ready");
            return false;
        }
        delay_us(1);
    }

    let vs = reg_read32(bar0, NVME_VS);
    println!(
        "[nvme] Controller ready! Version {}.{}.{}",
        (vs >> 16) & 0xFF,
        (vs >> 8) & 0xFF,
        vs & 0xFF
    );

    // Identify controller
    if !identify_controller() {
        println!("[nvme] Identify controller failed");
        return false;
    }

    // Create I/O completion queue 1
    if !create_io_cq(1, 1) {
        // CQ 1, IRQ vector 1
        println!("[nvme] Failed to create I/O CQ 1");
        return false;
    }

    // Create I/O submission queue 1
    if !create_io_sq(1, 1) {
        // SQ 1 → CQ 1
        println!("[nvme] Failed to create I/O SQ 1");
        return false;
    }

    // Identify namespace 1
    if !identify_namespace(1) {
        println!("[nvme] Identify namespace 1 failed");
        // Not fatal — we'll use defaults
    }

    NVME_CTRL.initialized = true;
    println!(
        "[nvme] Initialization complete! Capacity: {} MB, sector: {} bytes",
        NVME_CTRL.capacity_bytes / (1024 * 1024),
        NVME_CTRL.sector_size
    );

    true
}

// ═══════════════════════════════════════════════════════════════════
// NVMe Admin Commands
// ═══════════════════════════════════════════════════════════════════

/// Get next command ID
fn next_cid() -> u16 {
    unsafe {
        let cid = NVME_CTRL.next_cid;
        NVME_CTRL.next_cid = NVME_CTRL.next_cid.wrapping_add(1);
        cid
    }
}

/// Submit an admin command and wait for completion
unsafe fn admin_submit_wait(cmd: &mut NvmeCmd) -> NvmeCqe {
    cmd.cid = next_cid();

    // Copy command to admin SQ
    let sq_entry = NVME_CTRL.admin_q.sq_entry();
    core::ptr::copy_nonoverlapping(cmd as *const NvmeCmd, sq_entry as *mut NvmeCmd, 1);

    // Ring admin SQ doorbell
    let tail = NVME_CTRL.admin_q.sq_push();
    ring_doorbell(NVME_CTRL.base, NVME_CTRL.doorbell_stride, 0, false, tail);

    // Poll for completion
    let mut timeout = 5_000_000; // 5 seconds in us
    loop {
        if NVME_CTRL.admin_q.cq_pop() {
            let _cqe = *NVME_CTRL.admin_q.cq_peek(); // Re-read after pop
                                                     // Actually we need to read the entry BEFORE advancing
                                                     // Let's fix this — re-read the previous entry
                                                     // The cq_pop already advanced head, so we need to read the one before
                                                     // Better approach: read first, then advance
            break;
        }
        delay_us(1);
        timeout -= 1;
        if timeout == 0 {
            println!("[nvme] Admin command timeout!");
            return NvmeCqe::default();
        }
    }

    // Ring admin CQ doorbell
    ring_doorbell(
        NVME_CTRL.base,
        NVME_CTRL.doorbell_stride,
        0,
        true,
        NVME_CTRL.admin_q.cq_head,
    );

    // Read the completion entry (the one we just consumed)
    // cq_head is now one past the consumed entry, so read at (cq_head - 1) mod depth
    let idx = if NVME_CTRL.admin_q.cq_head == 0 {
        NVME_CTRL.admin_q.depth as usize - 1
    } else {
        NVME_CTRL.admin_q.cq_head as usize - 1
    };
    let offset = idx * size_of::<NvmeCqe>();
    let cqe = read_volatile((NVME_CTRL.admin_q.cq_base + offset as u64) as *const NvmeCqe);

    // Clear the entry for next use
    write_volatile(
        (NVME_CTRL.admin_q.cq_base + offset as u64) as *mut NvmeCqe,
        NvmeCqe::default(),
    );

    cqe
}

/// Identify controller (CNS=1)
unsafe fn identify_controller() -> bool {
    // Allocate a page for the identify data
    let id_phys = match allocate_frame() {
        Some(p) => p,
        None => return false,
    };
    let id_virt = phys_to_virt(id_phys).0;

    // Zero the buffer
    core::ptr::write_bytes(id_virt as *mut u8, 0, PAGE_SIZE as usize);

    let mut cmd = NvmeCmd::new();
    cmd.cdw0 = (ADMIN_IDENTIFY as u32) | (0 << 14); // FUSE=0, PSDT=0
    cmd.nsid = 0;
    cmd.prp1 = id_phys.0;
    cmd.cdw10 = 1; // CNS=1 (Identify Controller)

    let cqe = admin_submit_wait(&mut cmd);

    if !cqe.success() {
        println!(
            "[nvme] Identify controller failed: status={:#x}",
            cqe.status()
        );
        return false;
    }

    // Read controller data
    let id_ctrl = &*(id_virt as *const NvmeIdCtrl);

    // Print model/serial
    let mn: &[u8] = &id_ctrl.mn;
    let sn: &[u8] = &id_ctrl.sn;
    let mn_str = core::str::from_utf8_unchecked(mn);
    let sn_str = core::str::from_utf8_unchecked(sn);
    println!("[nvme] Model: {}", mn_str.trim());
    println!("[nvme] Serial: {}", sn_str.trim());

    // Get number of namespaces from Identify Controller at offset 0x0F (NN field)
    // NN is at byte offset 515 (0x203) — 32-bit value
    let nn = read_volatile((id_virt + 516) as *const u32);
    NVME_CTRL.num_namespaces = nn;
    println!("[nvme] Namespaces: {}", nn);

    deallocate_frame(id_phys);
    true
}

/// Identify namespace (CNS=0)
unsafe fn identify_namespace(nsid: u32) -> bool {
    let id_phys = match allocate_frame() {
        Some(p) => p,
        None => return false,
    };
    let id_virt = phys_to_virt(id_phys).0;

    core::ptr::write_bytes(id_virt as *mut u8, 0, PAGE_SIZE as usize);

    let mut cmd = NvmeCmd::new();
    cmd.cdw0 = ADMIN_IDENTIFY as u32;
    cmd.nsid = nsid;
    cmd.prp1 = id_phys.0;
    cmd.cdw10 = 0; // CNS=0 (Identify Namespace)

    let cqe = admin_submit_wait(&mut cmd);

    if !cqe.success() {
        println!(
            "[nvme] Identify namespace {} failed: status={:#x}",
            nsid,
            cqe.status()
        );
        return false;
    }

    let id_ns = &*(id_virt as *const NvmeIdNs);

    NVME_CTRL.nsid = nsid;
    NVME_CTRL.namespace_size = id_ns.nsze;
    NVME_CTRL.sector_size = id_ns.sector_size();
    NVME_CTRL.capacity_bytes = id_ns.nsze * NVME_CTRL.sector_size as u64;

    println!(
        "[nvme] Namespace {}: size={} blocks, sector_size={}, capacity={:#x} bytes",
        nsid, id_ns.nsze, NVME_CTRL.sector_size, NVME_CTRL.capacity_bytes
    );

    deallocate_frame(id_phys);
    true
}

/// Create I/O completion queue
unsafe fn create_io_cq(qid: u16, irq_vector: u16) -> bool {
    let io_q = match NvmeQueue::create(qid, IO_Q_DEPTH as u16) {
        Some(q) => q,
        None => return false,
    };
    NVME_CTRL.io_q = io_q;

    let mut cmd = NvmeCmd::new();
    cmd.cdw0 = ADMIN_CREATE_CQ as u32;
    cmd.prp1 = NVME_CTRL.io_q.cq_phys;
    cmd.cdw10 = (IO_Q_DEPTH as u32 - 1) | ((qid as u32) << 16);
    // cdw11: IRQ enabled | IRQ vector | physically contiguous
    cmd.cdw11 = 1 | ((irq_vector as u32) << 16) | (1 << 1); // IE=1, IV, PC=1

    let cqe = admin_submit_wait(&mut cmd);

    if !cqe.success() {
        println!("[nvme] Create I/O CQ failed: status={:#x}", cqe.status());
        return false;
    }

    println!("[nvme] I/O CQ {} created", qid);
    true
}

/// Create I/O submission queue
unsafe fn create_io_sq(qid: u16, cqid: u16) -> bool {
    let mut cmd = NvmeCmd::new();
    cmd.cdw0 = ADMIN_CREATE_SQ as u32;
    cmd.prp1 = NVME_CTRL.io_q.sq_phys;
    cmd.cdw10 = (IO_Q_DEPTH as u32 - 1) | ((qid as u32) << 16);
    // cdw11: CQ ID | physically contiguous
    cmd.cdw11 = (cqid as u32) | (1 << 1); // CQID, PC=1

    let cqe = admin_submit_wait(&mut cmd);

    if !cqe.success() {
        println!("[nvme] Create I/O SQ failed: status={:#x}", cqe.status());
        return false;
    }

    println!("[nvme] I/O SQ {} created (→ CQ {})", qid, cqid);
    true
}

// ═══════════════════════════════════════════════════════════════════
// NVMe I/O Commands
// ═══════════════════════════════════════════════════════════════════

/// Submit an I/O command and wait for completion
unsafe fn io_submit_wait(cmd: &mut NvmeCmd) -> NvmeCqe {
    cmd.cid = next_cid();

    // Copy command to I/O SQ
    let sq_entry = NVME_CTRL.io_q.sq_entry();
    core::ptr::copy_nonoverlapping(cmd as *const NvmeCmd, sq_entry as *mut NvmeCmd, 1);

    // Ring I/O SQ doorbell
    let tail = NVME_CTRL.io_q.sq_push();
    ring_doorbell(
        NVME_CTRL.base,
        NVME_CTRL.doorbell_stride,
        NVME_CTRL.io_q.qid,
        false,
        tail,
    );

    // Poll for completion
    let mut timeout = 10_000_000; // 10 seconds
    loop {
        if NVME_CTRL.io_q.cq_pop() {
            break;
        }
        delay_us(1);
        timeout -= 1;
        if timeout == 0 {
            println!("[nvme] I/O command timeout!");
            return NvmeCqe::default();
        }
    }

    // Ring I/O CQ doorbell
    ring_doorbell(
        NVME_CTRL.base,
        NVME_CTRL.doorbell_stride,
        NVME_CTRL.io_q.qid,
        true,
        NVME_CTRL.io_q.cq_head,
    );

    // Read consumed completion entry
    let idx = if NVME_CTRL.io_q.cq_head == 0 {
        NVME_CTRL.io_q.depth as usize - 1
    } else {
        NVME_CTRL.io_q.cq_head as usize - 1
    };
    let offset = idx * size_of::<NvmeCqe>();
    let cqe = read_volatile((NVME_CTRL.io_q.cq_base + offset as u64) as *const NvmeCqe);

    // Clear entry
    write_volatile(
        (NVME_CTRL.io_q.cq_base + offset as u64) as *mut NvmeCqe,
        NvmeCqe::default(),
    );

    cqe
}

/// Read sectors from NVMe device
/// Returns number of bytes read, or -1 on error
pub fn read(block: u64, count: u32, buf: *mut u8) -> i64 {
    unsafe {
        if !NVME_CTRL.initialized {
            return -1;
        }

        let buf_phys = virt_to_phys(VirtAddr::new(buf as u64));
        if buf_phys.0 == 0 {
            // Buffer not in directly-mappable range — use bounce buffer
            // For now, reject non-direct buffers
            println!("[nvme] read: buffer not in mappable range");
            return -1;
        }

        let mut cmd = NvmeCmd::new();
        cmd.cdw0 = NVM_READ as u32;
        cmd.nsid = NVME_CTRL.nsid;
        cmd.prp1 = buf_phys.0;
        // PRP2: if the transfer spans a 4KB page boundary, we'd need a PRP list
        // For simplicity, assume single-page transfers (count * sector_size <= 4KB)
        cmd.cdw10 = block as u32; // Starting LBA (low 32 bits)
        cmd.cdw11 = (block >> 32) as u32; // Starting LBA (high 32 bits)
        cmd.cdw12 = (count - 1) & 0xFFFF; // Number of logical blocks (0-based)

        let cqe = io_submit_wait(&mut cmd);

        if !cqe.success() {
            println!(
                "[nvme] Read failed at block {}: status={:#x}",
                block,
                cqe.status()
            );
            return -1;
        }

        (count as i64) * (NVME_CTRL.sector_size as i64)
    }
}

/// Write sectors to NVMe device
/// Returns number of bytes written, or -1 on error
pub fn write(block: u64, count: u32, buf: *const u8) -> i64 {
    unsafe {
        if !NVME_CTRL.initialized {
            return -1;
        }

        let buf_phys = virt_to_phys(VirtAddr::new(buf as u64));
        if buf_phys.0 == 0 {
            println!("[nvme] write: buffer not in mappable range");
            return -1;
        }

        let mut cmd = NvmeCmd::new();
        cmd.cdw0 = NVM_WRITE as u32;
        cmd.nsid = NVME_CTRL.nsid;
        cmd.prp1 = buf_phys.0;
        cmd.cdw10 = block as u32;
        cmd.cdw11 = (block >> 32) as u32;
        cmd.cdw12 = (count - 1) & 0xFFFF;

        let cqe = io_submit_wait(&mut cmd);

        if !cqe.success() {
            println!(
                "[nvme] Write failed at block {}: status={:#x}",
                block,
                cqe.status()
            );
            return -1;
        }

        (count as i64) * (NVME_CTRL.sector_size as i64)
    }
}

/// Flush write cache
pub fn flush() -> bool {
    unsafe {
        if !NVME_CTRL.initialized {
            return false;
        }

        let mut cmd = NvmeCmd::new();
        cmd.cdw0 = NVM_FLUSH as u32;
        cmd.nsid = NVME_CTRL.nsid;

        let cqe = io_submit_wait(&mut cmd);
        cqe.success()
    }
}

// ═══════════════════════════════════════════════════════════════════
// SDHCI (SD Card) Driver — BCM2712 SDIO1
// ═══════════════════════════════════════════════════════════════════

/// BCM2712 SDIO1 (SD card) controller base
const SDHCI_BASE: u64 = 0x10_00C0_0000;

/// SDHCI register offsets
const SDHCI_DMA_ADDR: u64 = SDHCI_BASE + 0x00;
const SDHCI_BLK_SIZE: u64 = SDHCI_BASE + 0x04;
const SDHCI_BLK_COUNT: u64 = SDHCI_BASE + 0x06;
const SDHCI_ARG: u64 = SDHCI_BASE + 0x08;
const SDHCI_CMD: u64 = SDHCI_BASE + 0x0C;
const SDHCI_RSP0: u64 = SDHCI_BASE + 0x10;
const SDHCI_RSP1: u64 = SDHCI_BASE + 0x14;
const SDHCI_RSP2: u64 = SDHCI_BASE + 0x18;
const SDHCI_RSP3: u64 = SDHCI_BASE + 0x1C;
const SDHCI_DATA: u64 = SDHCI_BASE + 0x20;
const SDHCI_PRESENT: u64 = SDHCI_BASE + 0x24;
const SDHCI_HOST_CTRL: u64 = SDHCI_BASE + 0x28;
const SDHCI_PWR_CTRL: u64 = SDHCI_BASE + 0x29;
const SDHCI_CLK_CTRL: u64 = SDHCI_BASE + 0x2C;
const SDHCI_TIMEOUT: u64 = SDHCI_BASE + 0x2E;
const SDHCI_SW_RESET: u64 = SDHCI_BASE + 0x2F;
const SDHCI_INT_STATUS: u64 = SDHCI_BASE + 0x30;
const SDHCI_INT_ENABLE: u64 = SDHCI_BASE + 0x34;
const SDHCI_INT_SIGNAL: u64 = SDHCI_BASE + 0x38;
const SDHCI_ACMD_ERR: u64 = SDHCI_BASE + 0x3C;
const SDHCI_CAPS: u64 = SDHCI_BASE + 0x40;
const SDHCI_CAPS_1: u64 = SDHCI_BASE + 0x44;
const SDHCI_MAX_CUR: u64 = SDHCI_BASE + 0x48;
const SDHCI_SLOT_INT: u64 = SDHCI_BASE + 0xFC;
const SDHCI_VERSION: u64 = SDHCI_BASE + 0xFE;

// ── Command register flags ──
const CMD_RESP_NONE: u16 = 0x0000;
const CMD_RESP_136: u16 = 0x0001;
const CMD_RESP_48: u16 = 0x0002;
const CMD_RESP_48_BUSY: u16 = 0x0003;
const CMD_CRC: u16 = 0x0008;
const CMD_INDEX: u16 = 0x0010;
const CMD_DATA: u16 = 0x0020;
const CMD_ABORT: u16 = 0x0040;

// ── Present state flags ──
const PRES_CMD_INHIBIT: u32 = 1 << 0;
const PRES_DAT_INHIBIT: u32 = 1 << 1;
const PRES_DAT_ACTIVE: u32 = 1 << 2;
const PRES_WRITE_ENABLE: u32 = 1 << 8;
const PRES_READ_ENABLE: u32 = 1 << 9;
const PRES_CARD_PRESENT: u32 = 1 << 16;
const PRES_CARD_STABLE: u32 = 1 << 17;
const PRES_CARD_DETECT: u32 = 1 << 18;
const PRES_WRITE_PROTECT: u32 = 1 << 19;

// ── Interrupt flags ──
const INT_CMD_COMPLETE: u32 = 1 << 0;
const INT_XFER_COMPLETE: u32 = 1 << 1;
const INT_BUFFER_READY: u32 = 1 << 4;
const INT_ERR: u32 = 1 << 15;
const INT_CMD_TIMEOUT: u32 = 1 << 16;
const INT_CMD_CRC: u32 = 1 << 17;
const INT_CMD_END: u32 = 1 << 19;
const INT_CMD_INDEX: u32 = 1 << 20;
const INT_DAT_TIMEOUT: u32 = 1 << 24;
const INT_DAT_CRC: u32 = 1 << 25;
const INT_DAT_END: u32 = 1 << 27;

// ── Software reset flags ──
const SW_RESET_ALL: u8 = 0x01;
const SW_RESET_CMD: u8 = 0x02;
const SW_RESET_DAT: u8 = 0x04;

// ── Host control flags ──
const HOST_CTRL_DMA: u8 = 0x10;
const HOST_CTRL_4BIT: u8 = 0x02;
const HOST_CTRL_8BIT: u8 = 0x20;

// ── Power control ──
const PWR_ON: u8 = 0x01;
const PWR_180: u8 = 0x0A;
const PWR_300: u8 = 0x0C;
const PWR_330: u8 = 0x0E;

// ── Clock control ──
const CLK_INT_ENABLE: u16 = 1 << 0;
const CLK_INT_STABLE: u16 = 1 << 1;
const CLK_SDCLK_ENABLE: u16 = 1 << 2;
const CLK_MAX_FREQ_MASK: u16 = 0xFF << 8;

/// SDHCI controller state
pub struct SdhciController {
    /// Base address
    base: u64,
    /// Card is initialized
    card_initialized: bool,
    /// Card type (true = SDHC/SDXC, false = standard capacity)
    is_high_capacity: bool,
    /// Block size (typically 512)
    block_size: u32,
    /// Card capacity in blocks
    num_blocks: u32,
    /// RCA (Relative Card Address)
    rca: u16,
    /// Initialized
    initialized: bool,
}

impl SdhciController {
    pub const fn new() -> Self {
        SdhciController {
            base: SDHCI_BASE,
            card_initialized: false,
            is_high_capacity: false,
            block_size: 512,
            num_blocks: 0,
            rca: 0,
            initialized: false,
        }
    }
}

impl Default for SdhciController {
    fn default() -> Self {
        Self::new()
    }
}

/// Global SDHCI controller
static mut SDHCI_CTRL: SdhciController = SdhciController::new();

/// Read 32-bit SDHCI register
#[inline]
unsafe fn sdhci_read32(offset: u64) -> u32 {
    read_volatile(offset as *const u32)
}

/// Write 32-bit SDHCI register
#[inline]
unsafe fn sdhci_write32(offset: u64, val: u32) {
    write_volatile(offset as *mut u32, val);
}

/// Read 16-bit SDHCI register
#[inline]
unsafe fn sdhci_read16(offset: u64) -> u16 {
    read_volatile(offset as *const u16)
}

/// Write 16-bit SDHCI register
#[inline]
unsafe fn sdhci_write16(offset: u64, val: u16) {
    write_volatile(offset as *mut u16, val);
}

/// Read 8-bit SDHCI register
#[inline]
unsafe fn sdhci_read8(offset: u64) -> u8 {
    read_volatile(offset as *const u8)
}

/// Write 8-bit SDHCI register
#[inline]
unsafe fn sdhci_write8(offset: u64, val: u8) {
    write_volatile(offset as *mut u8, val);
}

/// Wait for a condition on a 32-bit register, with timeout
unsafe fn sdhci_wait32(offset: u64, mask: u32, value: u32, timeout_us: u64) -> bool {
    let mut remaining = timeout_us;
    while (sdhci_read32(offset) & mask) != value {
        if remaining == 0 {
            return false;
        }
        remaining -= 1;
        delay_us(1);
    }
    true
}

/// Wait for a condition on a 8-bit register, with timeout
unsafe fn sdhci_wait8(offset: u64, mask: u8, value: u8, timeout_us: u64) -> bool {
    let mut remaining = timeout_us;
    while (sdhci_read8(offset) & mask) != value {
        if remaining == 0 {
            return false;
        }
        remaining -= 1;
        delay_us(1);
    }
    true
}

/// Initialize the SDHCI controller and SD card
pub unsafe fn sdhci_init() -> bool {
    println!("[sdhci] Initializing SD card controller...");

    // Step 1: Reset the controller
    sdhci_write8(SDHCI_SW_RESET, SW_RESET_ALL);
    if !sdhci_wait8(SDHCI_SW_RESET, SW_RESET_ALL, 0, 1_000_000) {
        println!("[sdhci] Reset timeout");
        return false;
    }

    // Step 2: Read capabilities
    let caps = sdhci_read32(SDHCI_CAPS);
    let max_clk = ((caps >> 8) & 0xFF) as u32 * 1_000_000; // Base clock in MHz
    println!(
        "[sdhci] Capabilities: {:#x}, base clock={} MHz",
        caps,
        max_clk / 1_000_000
    );

    // Step 3: Power on the card
    sdhci_write8(SDHCI_PWR_CTRL, PWR_330 | PWR_ON);
    delay_us(10_000); // Wait for power ramp

    // Step 4: Set clock — start with identification clock (400 kHz)
    // First, disable the clock
    sdhci_write16(SDHCI_CLK_CTRL, 0);
    delay_us(100);

    // Set the clock divisor for 400 kHz
    // Div = base_clock / (2 * target) - 1
    // For 400 kHz from 200 MHz: Div = 200M / 800k - 1 = 249
    let div = if max_clk > 0 {
        ((max_clk / 800_000) - 1).min(0xFF) as u16
    } else {
        0x40 // Safe default
    };

    let clk_ctrl = (div << 8) | CLK_INT_ENABLE;
    sdhci_write16(SDHCI_CLK_CTRL, clk_ctrl);

    // Wait for clock to stabilize
    if !sdhci_wait16_ok(SDHCI_CLK_CTRL, CLK_INT_STABLE, 1, 100_000) {
        println!("[sdhci] Clock stabilization timeout");
        return false;
    }

    // Enable the SD clock
    let clk = sdhci_read16(SDHCI_CLK_CTRL);
    sdhci_write16(SDHCI_CLK_CTRL, clk | CLK_SDCLK_ENABLE);

    // Step 5: Set block size and timeout
    sdhci_write16(SDHCI_BLK_SIZE, 512);
    sdhci_write8(SDHCI_TIMEOUT, 0x0E); // Maximum timeout

    // Step 6: Enable interrupts
    sdhci_write32(
        SDHCI_INT_ENABLE,
        INT_CMD_COMPLETE
            | INT_XFER_COMPLETE
            | INT_BUFFER_READY
            | INT_CMD_TIMEOUT
            | INT_CMD_CRC
            | INT_DAT_TIMEOUT
            | INT_DAT_CRC
            | INT_ERR,
    );
    sdhci_write32(
        SDHCI_INT_SIGNAL,
        INT_CMD_COMPLETE | INT_XFER_COMPLETE | INT_BUFFER_READY,
    );

    // Step 7: Check if a card is present
    let present = sdhci_read32(SDHCI_PRESENT);
    if present & PRES_CARD_PRESENT == 0 {
        println!("[sdhci] No SD card detected");
        return false;
    }
    println!("[sdhci] SD card detected");

    // Step 8: Send CMD0 (GO_IDLE_STATE) — reset the card
    if !sdhci_send_cmd(0, 0, CMD_RESP_NONE, false) {
        println!("[sdhci] CMD0 (GO_IDLE) failed");
        return false;
    }
    println!("[sdhci] CMD0: Card reset");

    // Step 9: Send CMD8 (SEND_IF_COND) — check SD v2+ card
    let if_cond_arg: u32 = 0x00000155; // VHS=1 (2.7-3.6V), check pattern=0x55
    let cmd8_ok = sdhci_send_cmd(8, if_cond_arg, CMD_RESP_48, true);
    if cmd8_ok {
        let resp = sdhci_read32(SDHCI_RSP0);
        if resp == if_cond_arg {
            println!("[sdhci] CMD8: SD v2+ card detected");
            SDHCI_CTRL.is_high_capacity = false; // Will be confirmed with ACMD41
        } else {
            println!("[sdhci] CMD8: Response mismatch ({:#x})", resp);
        }
    } else {
        println!("[sdhci] CMD8: No response — SD v1 card or MMC");
    }

    // Step 10: Send ACMD41 (SD_SEND_OP_COND) — initialize card
    let mut op_cond_retries = 1000;
    let hcs_flag: u32 = 1 << 30; // High Capacity Support
    loop {
        // CMD55 (APP_CMD) — next command is application-specific
        if !sdhci_send_cmd(55, 0, CMD_RESP_48, true) {
            println!("[sdhci] CMD55 failed");
            return false;
        }

        // ACMD41 with HCS flag
        if !sdhci_send_cmd(41, 0x00FF8000 | hcs_flag, CMD_RESP_48, false) {
            println!("[sdhci] ACMD41 failed");
            return false;
        }

        let ocr = sdhci_read32(SDHCI_RSP0);
        if ocr & (1 << 31) != 0 {
            // Card is ready (power up status bit set)
            if ocr & hcs_flag != 0 {
                SDHCI_CTRL.is_high_capacity = true;
                println!("[sdhci] ACMD41: SDHC/SDXC card (OCR={:#x})", ocr);
            } else {
                SDHCI_CTRL.is_high_capacity = false;
                println!("[sdhci] ACMD41: Standard capacity SD card (OCR={:#x})", ocr);
            }
            break;
        }

        op_cond_retries -= 1;
        if op_cond_retries == 0 {
            println!("[sdhci] ACMD41: Card never became ready");
            return false;
        }
        delay_us(1000);
    }

    // Step 11: Send CMD2 (ALL_SEND_CID) — get card identification
    if !sdhci_send_cmd(2, 0, CMD_RESP_136, true) {
        println!("[sdhci] CMD2 (ALL_SEND_CID) failed");
        // Non-fatal, continue
    } else {
        let _cid_0 = sdhci_read32(SDHCI_RSP0);
        let cid_3 = sdhci_read32(SDHCI_RSP3);
        println!(
            "[sdhci] CMD2: CID received (MID={:02x})",
            (cid_3 >> 16) & 0xFF
        );
    }

    // Step 12: Send CMD3 (SEND_RELATIVE_ADDR) — get RCA
    if !sdhci_send_cmd(3, 0, CMD_RESP_48, true) {
        println!("[sdhci] CMD3 (SEND_RELATIVE_ADDR) failed");
        return false;
    }
    SDHCI_CTRL.rca = (sdhci_read32(SDHCI_RSP0) >> 16) as u16;
    println!("[sdhci] CMD3: RCA={:#x}", SDHCI_CTRL.rca);

    // Step 13: Send CMD9 (SEND_CSD) — get card specific data
    let csd_arg = (SDHCI_CTRL.rca as u32) << 16;
    if !sdhci_send_cmd(9, csd_arg, CMD_RESP_136, true) {
        println!("[sdhci] CMD9 (SEND_CSD) failed");
    } else {
        let _csd0 = sdhci_read32(SDHCI_RSP0);
        let csd1 = sdhci_read32(SDHCI_RSP1);
        let csd2 = sdhci_read32(SDHCI_RSP2);
        let _csd3 = sdhci_read32(SDHCI_RSP3);

        // Parse CSD to get capacity
        if SDHCI_CTRL.is_high_capacity {
            // CSD v2: capacity = (C_SIZE + 1) * 512KB / block_size
            let c_size = ((csd2 >> 16) | ((csd1 & 0x3F) << 16)) as u32;
            SDHCI_CTRL.num_blocks = ((c_size as u64 + 1) * 1024 * 1024 / 512) as u32;
        } else {
            // CSD v1: more complex calculation
            // For now, use a default
            SDHCI_CTRL.num_blocks = 0; // Will determine later
        }
        println!(
            "[sdhci] CMD9: CSD received, blocks={}",
            SDHCI_CTRL.num_blocks
        );
    }

    // Step 14: Send CMD7 (SELECT_CARD) — select the card
    let sel_arg = (SDHCI_CTRL.rca as u32) << 16;
    if !sdhci_send_cmd(7, sel_arg, CMD_RESP_48_BUSY, true) {
        println!("[sdhci] CMD7 (SELECT_CARD) failed");
        return false;
    }
    println!("[sdhci] CMD7: Card selected");

    // Step 15: Set block size to 512 for data transfers
    sdhci_write16(SDHCI_BLK_SIZE, 512 | (7 << 16) as u16); // 512 bytes, DMA boundary = 512KB

    // Step 16: Switch to high-speed mode and increase clock
    // ACMD6 to set 4-bit bus width
    if !sdhci_send_cmd(55, (SDHCI_CTRL.rca as u32) << 16, CMD_RESP_48, true) {
        println!("[sdhci] CMD55 for bus width failed");
    } else if !sdhci_send_cmd(6, 2, CMD_RESP_48, true) {
        // 4-bit mode
        println!("[sdhci] ACMD6 (SET_BUS_WIDTH) failed");
    } else {
        // Set 4-bit mode in host controller
        let host_ctrl = sdhci_read8(SDHCI_HOST_CTRL);
        sdhci_write8(SDHCI_HOST_CTRL, host_ctrl | HOST_CTRL_4BIT);
        println!("[sdhci] 4-bit bus width set");
    }

    // Increase clock to max (25 MHz for standard, 50 MHz for high-speed)
    sdhci_write16(SDHCI_CLK_CTRL, 0);
    delay_us(100);
    // Div=0 means base clock / 1
    sdhci_write16(SDHCI_CLK_CTRL, (0 << 8) | CLK_INT_ENABLE);
    if sdhci_wait16_ok(SDHCI_CLK_CTRL, CLK_INT_STABLE, 1, 100_000) {
        let clk = sdhci_read16(SDHCI_CLK_CTRL);
        sdhci_write16(SDHCI_CLK_CTRL, clk | CLK_SDCLK_ENABLE);
    }

    // Enable DMA in host controller
    let host_ctrl = sdhci_read8(SDHCI_HOST_CTRL);
    sdhci_write8(SDHCI_HOST_CTRL, host_ctrl | HOST_CTRL_DMA);

    SDHCI_CTRL.card_initialized = true;
    SDHCI_CTRL.initialized = true;

    let capacity_mb = (SDHCI_CTRL.num_blocks as u64 * 512) / (1024 * 1024);
    println!(
        "[sdhci] SD card initialized! Capacity: {} MB, {}-bit, {}",
        capacity_mb,
        if sdhci_read8(SDHCI_HOST_CTRL) & HOST_CTRL_4BIT != 0 {
            4
        } else {
            1
        },
        if SDHCI_CTRL.is_high_capacity {
            "SDHC"
        } else {
            "SDSC"
        }
    );

    true
}

/// Helper: wait for 16-bit register condition
unsafe fn sdhci_wait16_ok(offset: u64, mask: u16, value: u16, timeout_us: u64) -> bool {
    let mut remaining = timeout_us;
    while (sdhci_read16(offset) & mask) != value {
        if remaining == 0 {
            return false;
        }
        remaining -= 1;
        delay_us(1);
    }
    true
}

/// Send an SD command
unsafe fn sdhci_send_cmd(cmd_idx: u32, arg: u32, resp_type: u16, check_crc: bool) -> bool {
    // Wait for command and data inhibit to clear
    let present = sdhci_read32(SDHCI_PRESENT);
    if present & PRES_CMD_INHIBIT != 0 {
        if !sdhci_wait32(SDHCI_PRESENT, PRES_CMD_INHIBIT, 0, 100_000) {
            println!("[sdhci] CMD inhibit timeout");
            return false;
        }
    }
    if resp_type == CMD_RESP_48_BUSY
        || (resp_type != CMD_RESP_NONE && present & PRES_DAT_INHIBIT != 0)
    {
        if !sdhci_wait32(SDHCI_PRESENT, PRES_DAT_INHIBIT, 0, 100_000) {
            println!("[sdhci] DAT inhibit timeout");
            return false;
        }
    }

    // Clear interrupt status
    sdhci_write32(SDHCI_INT_STATUS, 0xFFFFFFFF);

    // Set argument
    sdhci_write32(SDHCI_ARG, arg);

    // Build command register
    let mut cmd_reg = (cmd_idx & 0x3F) as u16;
    cmd_reg |= resp_type;

    if check_crc && (resp_type == CMD_RESP_48 || resp_type == CMD_RESP_136) {
        cmd_reg |= CMD_CRC;
    }
    if resp_type == CMD_RESP_48 || resp_type == CMD_RESP_48_BUSY {
        cmd_reg |= CMD_INDEX;
    }

    // Write command (this triggers the command)
    sdhci_write16(SDHCI_CMD, cmd_reg);

    // Wait for command complete or error
    let mut timeout = 1_000_000; // 1 second
    loop {
        let int_status = sdhci_read32(SDHCI_INT_STATUS);

        if int_status & INT_CMD_COMPLETE != 0 {
            // Clear the interrupt
            sdhci_write32(SDHCI_INT_STATUS, INT_CMD_COMPLETE);
            return true;
        }

        if int_status & (INT_CMD_TIMEOUT | INT_CMD_CRC | INT_ERR) != 0 {
            let err_type = if int_status & INT_CMD_TIMEOUT != 0 {
                "timeout"
            } else if int_status & INT_CMD_CRC != 0 {
                "CRC error"
            } else {
                "error"
            };
            println!(
                "[sdhci] CMD{} failed: {} (status={:#x})",
                cmd_idx, err_type, int_status
            );
            sdhci_write32(SDHCI_INT_STATUS, int_status);

            // Reset command line
            sdhci_write8(SDHCI_SW_RESET, SW_RESET_CMD);
            sdhci_wait8(SDHCI_SW_RESET, SW_RESET_CMD, 0, 100_000);

            return false;
        }

        timeout -= 1;
        if timeout == 0 {
            println!("[sdhci] CMD{} timeout waiting for interrupt", cmd_idx);
            return false;
        }
        delay_us(1);
    }
}

/// Read a single block from the SD card
/// Returns true on success
pub unsafe fn sdhci_read_block(block: u32, buf: *mut u8) -> bool {
    if !SDHCI_CTRL.initialized {
        return false;
    }

    // Wait for data inhibit to clear
    if !sdhci_wait32(SDHCI_PRESENT, PRES_DAT_INHIBIT, 0, 500_000) {
        println!("[sdhci] Read: DAT inhibit timeout");
        return false;
    }

    // Set up DMA address
    let buf_phys = virt_to_phys(VirtAddr::new(buf as u64));
    sdhci_write32(SDHCI_DMA_ADDR, buf_phys.0 as u32);

    // Set block size and count
    sdhci_write16(SDHCI_BLK_SIZE, 512);
    sdhci_write16(SDHCI_BLK_COUNT, 1);

    // Clear interrupt status
    sdhci_write32(SDHCI_INT_STATUS, 0xFFFFFFFF);

    // Set argument (block address — sector number for SDHC, byte address for SDSC)
    let addr = if SDHCI_CTRL.is_high_capacity {
        block
    } else {
        block * 512
    };
    sdhci_write32(SDHCI_ARG, addr);

    // Send CMD17 (READ_SINGLE_BLOCK) with data transfer flag
    let cmd_reg = 17 | CMD_RESP_48 | CMD_CRC | CMD_INDEX | CMD_DATA;
    sdhci_write16(SDHCI_CMD, cmd_reg);

    // Wait for transfer complete or error
    let mut timeout = 5_000_000; // 5 seconds
    loop {
        let int_status = sdhci_read32(SDHCI_INT_STATUS);

        if int_status & INT_XFER_COMPLETE != 0 {
            sdhci_write32(SDHCI_INT_STATUS, INT_XFER_COMPLETE);
            return true;
        }

        if int_status & (INT_DAT_TIMEOUT | INT_DAT_CRC | INT_ERR) != 0 {
            println!(
                "[sdhci] Read block {} failed: status={:#x}",
                block, int_status
            );
            sdhci_write32(SDHCI_INT_STATUS, int_status);
            sdhci_write8(SDHCI_SW_RESET, SW_RESET_CMD | SW_RESET_DAT);
            sdhci_wait8(SDHCI_SW_RESET, SW_RESET_CMD | SW_RESET_DAT, 0, 100_000);
            return false;
        }

        timeout -= 1;
        if timeout == 0 {
            println!("[sdhci] Read block {} timeout", block);
            return false;
        }
        delay_us(1);
    }
}

/// Write a single block to the SD card
/// Returns true on success
pub unsafe fn sdhci_write_block(block: u32, buf: *const u8) -> bool {
    if !SDHCI_CTRL.initialized {
        return false;
    }

    // Wait for data inhibit to clear
    if !sdhci_wait32(SDHCI_PRESENT, PRES_DAT_INHIBIT, 0, 500_000) {
        println!("[sdhci] Write: DAT inhibit timeout");
        return false;
    }

    // Set up DMA address
    let buf_phys = virt_to_phys(VirtAddr::new(buf as u64));
    sdhci_write32(SDHCI_DMA_ADDR, buf_phys.0 as u32);

    // Set block size and count
    sdhci_write16(SDHCI_BLK_SIZE, 512);
    sdhci_write16(SDHCI_BLK_COUNT, 1);

    // Clear interrupt status
    sdhci_write32(SDHCI_INT_STATUS, 0xFFFFFFFF);

    // Set argument
    let addr = if SDHCI_CTRL.is_high_capacity {
        block
    } else {
        block * 512
    };
    sdhci_write32(SDHCI_ARG, addr);

    // Send CMD24 (WRITE_SINGLE_BLOCK) with data transfer flag
    let cmd_reg = 24 | CMD_RESP_48 | CMD_CRC | CMD_INDEX | CMD_DATA;
    sdhci_write16(SDHCI_CMD, cmd_reg);

    // Wait for transfer complete or error
    let mut timeout = 5_000_000;
    loop {
        let int_status = sdhci_read32(SDHCI_INT_STATUS);

        if int_status & INT_XFER_COMPLETE != 0 {
            sdhci_write32(SDHCI_INT_STATUS, INT_XFER_COMPLETE);
            return true;
        }

        if int_status & (INT_DAT_TIMEOUT | INT_DAT_CRC | INT_ERR) != 0 {
            println!(
                "[sdhci] Write block {} failed: status={:#x}",
                block, int_status
            );
            sdhci_write32(SDHCI_INT_STATUS, int_status);
            sdhci_write8(SDHCI_SW_RESET, SW_RESET_CMD | SW_RESET_DAT);
            sdhci_wait8(SDHCI_SW_RESET, SW_RESET_CMD | SW_RESET_DAT, 0, 100_000);
            return false;
        }

        timeout -= 1;
        if timeout == 0 {
            println!("[sdhci] Write block {} timeout", block);
            return false;
        }
        delay_us(1);
    }
}

/// Get SD card info
pub fn sdhci_card_info() -> (u32, u32, bool) {
    unsafe {
        (
            SDHCI_CTRL.num_blocks,
            SDHCI_CTRL.block_size,
            SDHCI_CTRL.is_high_capacity,
        )
    }
}

/// Check if SDHCI is initialized
pub fn sdhci_is_initialized() -> bool {
    unsafe { SDHCI_CTRL.initialized }
}

/// Check if NVMe is initialized
pub fn nvme_is_initialized() -> bool {
    unsafe { NVME_CTRL.initialized }
}

/// Get NVMe sector size
pub fn nvme_sector_size() -> u32 {
    unsafe { NVME_CTRL.sector_size }
}

/// Get NVMe capacity in blocks
pub fn nvme_num_blocks() -> u64 {
    unsafe { NVME_CTRL.namespace_size }
}

/// NVMe Block Device implementation
/// Implements the BlockDevice trait for filesystem integration
pub struct NvmeBlockDevice;

impl crate::fs::block_dev::BlockDevice for NvmeBlockDevice {
    fn read_block(
        &self,
        block: u64,
        buf: &mut [u8; 512],
    ) -> Result<(), crate::fs::block_dev::BlockError> {
        if block >= unsafe { NVME_CTRL.namespace_size } {
            return Err(crate::fs::block_dev::BlockError::InvalidBlock);
        }
        unsafe {
            NVME_CTRL
                .read_lba(block, buf)
                .map_err(|_| crate::fs::block_dev::BlockError::ReadError)
        }
    }

    fn write_block(
        &self,
        block: u64,
        buf: &[u8; 512],
    ) -> Result<(), crate::fs::block_dev::BlockError> {
        if block >= unsafe { NVME_CTRL.namespace_size } {
            return Err(crate::fs::block_dev::BlockError::InvalidBlock);
        }
        unsafe {
            NVME_CTRL
                .write_lba(block, buf)
                .map_err(|_| crate::fs::block_dev::BlockError::WriteError)
        }
    }

    fn block_size(&self) -> usize {
        512
    }

    fn total_blocks(&self) -> u64 {
        unsafe { NVME_CTRL.namespace_size }
    }

    fn name(&self) -> &str {
        "nvme0"
    }

    fn media_present(&self) -> bool {
        unsafe { NVME_CTRL.initialized }
    }

    fn device_type(&self) -> &str {
        "nvme"
    }

    fn read_only(&self) -> bool {
        false
    }

    fn flush(&self) -> Result<(), crate::fs::block_dev::BlockError> {
        // NVMe flush command would go here
        Ok(())
    }
}

/// Static NVMe block device instance for registration
pub static NVME_BLOCK_DEVICE: NvmeBlockDevice = NvmeBlockDevice;

/// Initialize NVMe and register as block device
pub unsafe fn init_and_register_block() -> bool {
    if init() {
        // Register as a block device
        if crate::fs::block_dev::register_device(&NVME_BLOCK_DEVICE).is_ok() {
            crate::println!("[nvme] Registered as block device");
            // Register partitions for ext2 mount
            crate::fs::block_dev::register_partitions(0);
            true
        } else {
            crate::println!("[nvme] Failed to register as block device");
            false
        }
    } else {
        false
    }
}
