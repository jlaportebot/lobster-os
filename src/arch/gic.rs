//! GIC-400 (GICv2) Interrupt Controller driver
//!
//! Pi 5 (BCM2712): GICD at 0x10_7FFF_9000, GICC at 0x10_7FFF_A000
//! QEMU raspi4b:   GICD at 0xFF841000, GICC at 0xFF842000

/// GIC Distributor base address (physical)
#[cfg(not(feature = "qemu_raspi5b"))]
const GICD_PHYS_BASE: u64 = 0x10_7FFF_9000;
#[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
const GICD_PHYS_BASE: u64 = 0xFF84_1000;
#[cfg(feature = "qemu_raspi5b")]
const GICD_PHYS_BASE: u64 = 0x10_7FFF_9000;

/// GIC CPU Interface base address (physical)
#[cfg(not(feature = "qemu_raspi5b"))]
const GICC_PHYS_BASE: u64 = 0x10_7FFF_A000;
#[cfg(all(feature = "qemu", not(feature = "qemu_raspi5b")))]
const GICC_PHYS_BASE: u64 = 0xFF84_2000;
#[cfg(feature = "qemu_raspi5b")]
const GICC_PHYS_BASE: u64 = 0x10_7FFF_A000;

/// Virtual base addresses (set after MMU enable)
static mut GICD_VIRT_BASE: u64 = 0;
static mut GICC_VIRT_BASE: u64 = 0;
static mut USE_VIRT_ADDRESSES: bool = false;

/// Get the current GICD base (physical before MMU, virtual after)
#[inline]
fn gicd_base() -> u64 {
    unsafe {
        if USE_VIRT_ADDRESSES && GICD_VIRT_BASE != 0 {
            GICD_VIRT_BASE
        } else {
            GICD_PHYS_BASE
        }
    }
}

/// Get the current GICC base (physical before MMU, virtual after)
#[inline]
fn gicc_base() -> u64 {
    unsafe {
        if USE_VIRT_ADDRESSES && GICC_VIRT_BASE != 0 {
            GICC_VIRT_BASE
        } else {
            GICC_PHYS_BASE
        }
    }
}

/// Initialize virtual base addresses (call after MMU is enabled)
pub unsafe fn init_virtual(gicd_virt: u64, gicc_virt: u64) {
    GICD_VIRT_BASE = gicd_virt;
    GICC_VIRT_BASE = gicc_virt;
    USE_VIRT_ADDRESSES = true;
}

// ---- GICD (Distributor) Register Offsets ----
const GICD_CTLR_OFF: u64 = 0x000;
const GICD_TYPER_OFF: u64 = 0x004; // Type Register
const GICD_IGROUPR_OFF: u64 = 0x080; // Interrupt Group
const GICD_ISENABLER_OFF: u64 = 0x100; // Interrupt Set-Enable
const GICD_ICENABLER_OFF: u64 = 0x180; // Interrupt Clear-Enable
const GICD_ISPENDR_OFF: u64 = 0x200; // Interrupt Set-Pending
const GICD_ICPENDR_OFF: u64 = 0x280; // Interrupt Clear-Pending
const GICD_ISACTIVER_OFF: u64 = 0x300; // Interrupt Set-Active
const GICD_ICACTIVER_OFF: u64 = 0x380; // Interrupt Clear-Active
const GICD_IPRIORITYR_OFF: u64 = 0x400; // Interrupt Priority
const GICD_ITARGETSR_OFF: u64 = 0x800; // Interrupt Targets
const GICD_ICFGR_OFF: u64 = 0xC00; // Interrupt Configuration
const GICD_SGIR_OFF: u64 = 0xF00; // Software Generated Interrupt

// ---- GICC (CPU Interface) Register Offsets ----
const GICC_CTLR_OFF: u64 = 0x000;
const GICC_PMR_OFF: u64 = 0x004; // Priority Mask Register
const GICC_BPR_OFF: u64 = 0x008; // Binary Point Register
const GICC_IAR_OFF: u64 = 0x00C; // Interrupt Acknowledge Register
const GICC_EOIR_OFF: u64 = 0x010; // End of Interrupt Register
const GICC_RPR_OFF: u64 = 0x014; // Running Priority Register
const GICC_HPPIR_OFF: u64 = 0x018; // Highest Pending Interrupt Register

pub struct Gic;

impl Gic {
    /// Initialize the GIC Distributor and CPU Interface on core 0
    pub unsafe fn init() {
        // Disable the distributor while we configure it
        Self::write32(gicd_base() + GICD_CTLR_OFF, 0);

        // Read GICD_TYPER to determine number of interrupts
        let typer = Self::read32(gicd_base() + GICD_TYPER_OFF);
        let num_irqs = ((typer & 0x1F) + 1) * 32;
        crate::println!("[gic] Max interrupts: {}", num_irqs);

        // Set all SPIs (32+) to Group 1 (IRQ, not FIQ)
        // GICD_IGROUPR: 1 bit per IRQ, 32 bits per register
        // NOTE: distributor registers are indexed by FULL irq ID (SGI+PPI+SPI),
        // so SPI numbering starts at ID 32 — this loop is already correct because
        // it iterates distributor IDs directly.
        for i in (32..num_irqs).step_by(32) {
            let reg = gicd_base() + GICD_IGROUPR_OFF + ((i / 32) as u64 * 4);
            Self::write32(reg, 0xFFFFFFFF); // All Group 1
        }

        // Set all SPIs to default priority (0x80 = mid priority)
        // GICD_IPRIORITYR: 8 bits per IRQ, 4 IRQs per register (32 bits)
        for i in (32..num_irqs).step_by(4) {
            let reg = gicd_base() + GICD_IPRIORITYR_OFF + (i as u64);
            Self::write32(reg, 0x80808080);
        }

        // Route all SPIs to core 0 (target mask = 0b0001)
        // GICD_ITARGETSR: 8 bits per IRQ, 4 IRQs per register
        for i in (32..num_irqs).step_by(4) {
            let reg = gicd_base() + GICD_ITARGETSR_OFF + (i as u64);
            Self::write32(reg, 0x01010101); // All target core 0
        }

        // Set all SPIs to level-sensitive (bit 1 = 0 for level, = 1 for edge)
        // GICD_ICFGR: 2 bits per IRQ, 16 IRQs per register
        for i in (32..num_irqs).step_by(16) {
            let reg = gicd_base() + GICD_ICFGR_OFF + ((i as u64 / 16) * 4);
            // Default is 0b00 per IRQ = level-sensitive, N-N model
            Self::write32(reg, 0x00000000);
        }

        // Disable all SPIs initially
        for i in (32..num_irqs).step_by(32) {
            let reg = gicd_base() + GICD_ICENABLER_OFF + ((i as u64 / 32) * 4);
            Self::write32(reg, 0xFFFFFFFF);
        }

        // Enable the distributor (bit 0 = EnableGrp1, bit 1 = EnableGrp0)
        Self::write32(gicd_base() + GICD_CTLR_OFF, 0x3);

        // Initialize CPU interface on this core
        Self::init_cpu_interface();
    }

    /// Initialize GIC CPU Interface (call on each core)
    pub unsafe fn init_cpu_interface() {
        // Disable CPU interface while configuring
        Self::write32(gicc_base() + GICC_CTLR_OFF, 0);

        // Set priority mask — allow all priorities (0xFF = lowest mask, allow all)
        Self::write32(gicc_base() + GICC_PMR_OFF, 0xFF);

        // Set binary point register (no priority grouping)
        Self::write32(gicc_base() + GICC_BPR_OFF, 0);

        // Enable CPU interface: EnableGrp1 (bit 1) + EnableGrp0 (bit 0)
        Self::write32(gicc_base() + GICC_CTLR_OFF, 0x3);
    }

    /// GICv2 distributor registers are indexed by the FULL interrupt ID:
    ///   [0..15] SGIs, [16..31] PPIs, [32..] SPIs.
    /// SPI N therefore lives at distributor register index N + 32.
    /// (The BCM2712 GIC-400 numbers its SPIs 0-based from the GIC's IRQ 32.)
    const SPI_TO_DIST: u32 = 32;

    /// Enable a PPI/SGI (GIC ID < 32): ISENABLERn bit (id % 32).
    /// GICv2 PPIs are NOT enabled by default — they live in ISENABLER0.
    pub unsafe fn enable_ppi(gic_id: u32) {
        if gic_id >= 32 {
            return;
        }
        let reg = gicd_base() + GICD_ISENABLER_OFF + ((gic_id as u64 / 32) * 4);
        let bit = 1u32 << (gic_id % 32);
        Self::write32(reg, bit);
    }

    /// Set the CPU-interface Priority Mask Register (GICC_PMR).
    /// QEMU's GIC resets PMR to 0 — which blocks ALL interrupt delivery
    /// (delivery requires prio < PMR). Set 0xFF to allow all.
    pub unsafe fn set_priority_mask(mask: u8) {
        const GICC_PMR_OFF: u64 = 0x104;
        Self::write32(gicc_base() + GICC_PMR_OFF, mask as u32);
    }

    /// Raw distributor read (for diagnostics)
    pub unsafe fn dist_read32(offset: u64) -> u32 {
        Self::read32(gicd_base() + offset)
    }

    /// Enable a specific SPI (Shared Peripheral Interrupt)
    pub unsafe fn enable_spi(irq_num: u32) {
        if irq_num < 32 {
            return; // SGIs/PPIs are always enabled on GICv2
        }
        let id = irq_num + Self::SPI_TO_DIST;
        let reg = gicd_base() + GICD_ISENABLER_OFF + ((id as u64 / 32) * 4);
        let bit = 1u32 << (id % 32);
        Self::write32(reg, bit);
    }

    /// Disable a specific SPI
    pub unsafe fn disable_spi(irq_num: u32) {
        if irq_num < 32 {
            return;
        }
        let id = irq_num + Self::SPI_TO_DIST;
        let reg = gicd_base() + GICD_ICENABLER_OFF + ((id as u64 / 32) * 4);
        let bit = 1u32 << (id % 32);
        Self::write32(reg, bit);
    }

    /// Set priority for a specific IRQ (0 = highest, 0xFF = lowest).
    /// SGIs/PPIs (ids < 32) are indexed directly; SPIs are 0-based and
    /// offset by 32 to their GIC ID.
    pub unsafe fn set_priority(irq_num: u32, priority: u8) {
        let id = if irq_num < 32 {
            irq_num
        } else {
            irq_num + Self::SPI_TO_DIST
        };
        let shift = (id % 4) * 8;
        let reg_aligned = gicd_base() + GICD_IPRIORITYR_OFF + ((id as u64 / 4) * 4);
        let mut val = Self::read32(reg_aligned);
        val &= !(0xFF << shift);
        val |= (priority as u32) << shift;
        Self::write32(reg_aligned, val);
    }

    /// Set target cores for a specific IRQ (bit mask: bit 0 = core 0, etc.)
    pub unsafe fn set_target(irq_num: u32, target_mask: u8) {
        let id = irq_num + Self::SPI_TO_DIST;
        let reg = gicd_base() + GICD_ITARGETSR_OFF + ((id as u64 / 4) * 4);
        let shift = (id % 4) * 8;
        let mut val = Self::read32(reg);
        val &= !(0xFF << shift);
        val |= (target_mask as u32) << shift;
        Self::write32(reg, val);
    }

    /// Set interrupt configuration (edge vs level triggered)
    /// config: 0b00 = level-sensitive, 0b10 = edge-triggered.
    /// SGIs/PPIs (ids < 32) are indexed directly.
    pub unsafe fn set_config(irq_num: u32, config: u32) {
        let id = if irq_num < 32 {
            irq_num
        } else {
            irq_num + Self::SPI_TO_DIST
        };
        let reg = gicd_base() + GICD_ICFGR_OFF + ((id as u64 / 16) * 4);
        let shift = (id % 16) * 2;
        let mut val = Self::read32(reg);
        val &= !(0b11 << shift);
        val |= config << shift;
        Self::write32(reg, val);
    }

    /// Acknowledge the highest-priority pending interrupt
    /// Returns the IRQ ID (read from GICC_IAR)
    pub unsafe fn acknowledge() -> u32 {
        Self::read32(gicc_base() + GICC_IAR_OFF) & 0x3FF
    }

    /// Signal end of interrupt handling
    pub unsafe fn end_of_interrupt(irq_id: u32) {
        Self::write32(gicc_base() + GICC_EOIR_OFF, irq_id);
    }

    /// Send SGI (Software Generated Interrupt) to target cores
    pub unsafe fn send_sgi(sgi_num: u32, target_cores: u8) {
        // GICD_SGIR format:
        //   [3:0]   = SGIINTID (interrupt number 0-15)
        //   [15:4]  = TargetListFilter (0 = use target list)
        //   [23:16] = TargetList (bit per core)
        let val = ((target_cores as u32) << 16) | (sgi_num & 0xF);
        Self::write32(gicd_base() + GICD_SGIR_OFF, val);
    }

    /// Read a 32-bit MMIO register
    #[inline]
    unsafe fn read32(addr: u64) -> u32 {
        core::ptr::read_volatile(addr as *const u32)
    }

    /// Write a 32-bit MMIO register
    #[inline]
    unsafe fn write32(addr: u64, val: u32) {
        core::ptr::write_volatile(addr as *mut u32, val);
    }
}
