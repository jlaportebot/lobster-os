//! GIC-400 (GICv2) Interrupt Controller driver for BCM2712
//!
//! Memory map (from bcm2712.dtsi, offset from 0x10_0000_0000):
//!   GICD: 0x7FFF_9000 (4KB)
//!   GICC: 0x7FFFA000 (8KB)
//!   GICH: 0x7FFFC000 (8KB)
//!   GICV: 0x7FFFE000 (8KB)

/// SoC peripheral base offset (from device tree: ranges = <0x00 0x10 0x0000_0000>)
const SOC_BASE: u64 = 0x10_0000_0000;

/// GIC Distributor base address
const GICD_BASE: u64 = SOC_BASE + 0x7FFF_9000;

/// GIC CPU Interface base address
const GICC_BASE: u64 = SOC_BASE + 0x7FFFA000;

// ---- GICD (Distributor) Register Offsets ----
const GICD_CTLR: u64 = GICD_BASE + 0x000;    // Control Register
const GICD_TYPER: u64 = GICD_BASE + 0x004;   // Type Register
const GICD_IGROUPR: u64 = GICD_BASE + 0x080; // Interrupt Group
const GICD_ISENABLER: u64 = GICD_BASE + 0x100; // Interrupt Set-Enable
const GICD_ICENABLER: u64 = GICD_BASE + 0x180; // Interrupt Clear-Enable
const GICD_ISPENDR: u64 = GICD_BASE + 0x200;   // Interrupt Set-Pending
const GICD_ICPENDR: u64 = GICD_BASE + 0x280;   // Interrupt Clear-Pending
const GICD_ISACTIVER: u64 = GICD_BASE + 0x300;  // Interrupt Set-Active
const GICD_ICACTIVER: u64 = GICD_BASE + 0x380;  // Interrupt Clear-Active
const GICD_IPRIORITYR: u64 = GICD_BASE + 0x400; // Interrupt Priority
const GICD_ITARGETSR: u64 = GICD_BASE + 0x800;  // Interrupt Targets
const GICD_ICFGR: u64 = GICD_BASE + 0xC00;      // Interrupt Configuration
const GICD_SGIR: u64 = GICD_BASE + 0xF00;       // Software Generated Interrupt

// ---- GICC (CPU Interface) Register Offsets ----
const GICC_CTLR: u64 = GICC_BASE + 0x000;    // Control Register
const GICC_PMR: u64 = GICC_BASE + 0x004;     // Priority Mask Register
const GICC_BPR: u64 = GICC_BASE + 0x008;     // Binary Point Register
const GICC_IAR: u64 = GICC_BASE + 0x00C;     // Interrupt Acknowledge Register
const GICC_EOIR: u64 = GICC_BASE + 0x010;    // End of Interrupt Register
const GICC_RPR: u64 = GICC_BASE + 0x014;     // Running Priority Register
const GICC_HPPIR: u64 = GICC_BASE + 0x018;   // Highest Pending Interrupt Register

pub struct Gic;

impl Gic {
    /// Initialize the GIC Distributor and CPU Interface on core 0
    pub unsafe fn init() {
        // Disable the distributor while we configure it
        Self::write32(GICD_CTLR, 0);

        // Read GICD_TYPER to determine number of interrupts
        let typer = Self::read32(GICD_TYPER);
        let num_irqs = ((typer & 0x1F) + 1) * 32;
        crate::println!("[gic] Max interrupts: {}", num_irqs);

        // Set all SPIs (32+) to Group 1 (IRQ, not FIQ)
        // GICD_IGROUPR: 1 bit per IRQ, 32 bits per register
        for i in (32..num_irqs).step_by(32) {
            let reg = GICD_IGROUPR + ((i / 32) as u64 * 4);
            Self::write32(reg, 0xFFFFFFFF); // All Group 1
        }

        // Set all SPIs to default priority (0x80 = mid priority)
        // GICD_IPRIORITYR: 8 bits per IRQ, 4 IRQs per register (32 bits)
        for i in (32..num_irqs).step_by(4) {
            let reg = GICD_IPRIORITYR + ((i as u64) * 1);
            Self::write32(reg, 0x80808080);
        }

        // Route all SPIs to core 0 (target mask = 0b0001)
        // GICD_ITARGETSR: 8 bits per IRQ, 4 IRQs per register
        for i in (32..num_irqs).step_by(4) {
            let reg = GICD_ITARGETSR + ((i as u64) * 1);
            Self::write32(reg, 0x01010101); // All target core 0
        }

        // Set all SPIs to level-sensitive (bit 1 = 0 for level, = 1 for edge)
        // GICD_ICFGR: 2 bits per IRQ, 16 IRQs per register
        for i in (32..num_irqs).step_by(16) {
            let reg = GICD_ICFGR + ((i as u64 / 16) * 4);
            // Default is 0b00 per IRQ = level-sensitive, N-N model
            Self::write32(reg, 0x00000000);
        }

        // Disable all SPIs initially
        for i in (32..num_irqs).step_by(32) {
            let reg = GICD_ICENABLER + ((i as u64 / 32) * 4);
            Self::write32(reg, 0xFFFFFFFF);
        }

        // Enable the distributor (bit 0 = EnableGrp1, bit 1 = EnableGrp0)
        Self::write32(GICD_CTLR, 0x3);

        // Initialize CPU interface on this core
        Self::init_cpu_interface();
    }

    /// Initialize GIC CPU Interface (call on each core)
    pub unsafe fn init_cpu_interface() {
        // Disable CPU interface while configuring
        Self::write32(GICC_CTLR, 0);

        // Set priority mask — allow all priorities (0xFF = lowest mask, allow all)
        Self::write32(GICC_PMR, 0xFF);

        // Set binary point register (no priority grouping)
        Self::write32(GICC_BPR, 0);

        // Enable CPU interface: EnableGrp1 (bit 1) + EnableGrp0 (bit 0)
        Self::write32(GICC_CTLR, 0x3);
    }

    /// Enable a specific SPI (Shared Peripheral Interrupt)
    pub unsafe fn enable_spi(irq_num: u32) {
        if irq_num < 32 {
            return; // SGIs/PPIs are always enabled on GICv2
        }
        let reg = GICD_ISENABLER + ((irq_num as u64 / 32) * 4);
        let bit = 1u32 << (irq_num % 32);
        Self::write32(reg, bit);
    }

    /// Disable a specific SPI
    pub unsafe fn disable_spi(irq_num: u32) {
        if irq_num < 32 {
            return;
        }
        let reg = GICD_ICENABLER + ((irq_num as u64 / 32) * 4);
        let bit = 1u32 << (irq_num % 32);
        Self::write32(reg, bit);
    }

    /// Set priority for a specific IRQ (0 = highest, 0xFF = lowest)
    pub unsafe fn set_priority(irq_num: u32, priority: u8) {
        let shift = (irq_num % 4) * 8;
        let reg_aligned = GICD_IPRIORITYR + ((irq_num as u64 / 4) * 4);
        let mut val = Self::read32(reg_aligned);
        val &= !(0xFF << shift);
        val |= (priority as u32) << shift;
        Self::write32(reg_aligned, val);
    }

    /// Set target cores for a specific IRQ (bit mask: bit 0 = core 0, etc.)
    pub unsafe fn set_target(irq_num: u32, target_mask: u8) {
        let reg = GICD_ITARGETSR + ((irq_num as u64 / 4) * 4);
        let shift = (irq_num % 4) * 8;
        let mut val = Self::read32(reg);
        val &= !(0xFF << shift);
        val |= (target_mask as u32) << shift;
        Self::write32(reg, val);
    }

    /// Set interrupt configuration (edge vs level triggered)
    /// config: 0b00 = level-sensitive, 0b10 = edge-triggered
    pub unsafe fn set_config(irq_num: u32, config: u32) {
        let reg = GICD_ICFGR + ((irq_num as u64 / 16) * 4);
        let shift = (irq_num % 16) * 2;
        let mut val = Self::read32(reg);
        val &= !(0b11 << shift);
        val |= config << shift;
        Self::write32(reg, val);
    }

    /// Acknowledge the highest-priority pending interrupt
    /// Returns the IRQ ID (read from GICC_IAR)
    pub unsafe fn acknowledge() -> u32 {
        Self::read32(GICC_IAR) & 0x3FF
    }

    /// Signal end of interrupt handling
    pub unsafe fn end_of_interrupt(irq_id: u32) {
        Self::write32(GICC_EOIR, irq_id);
    }

    /// Send SGI (Software Generated Interrupt) to target cores
    pub unsafe fn send_sgi(sgi_num: u32, target_cores: u8) {
        // GICD_SGIR format:
        //   [3:0]   = SGIINTID (interrupt number 0-15)
        //   [15:4]  = TargetListFilter (0 = use target list)
        //   [23:16] = TargetList (bit per core)
        let val = ((target_cores as u32) << 16) | (sgi_num & 0xF);
        Self::write32(GICD_SGIR, val);
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
