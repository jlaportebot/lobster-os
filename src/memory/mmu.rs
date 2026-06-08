//! MMU (Memory Management Unit) driver for AArch64
//!
//! Sets up 4KB granule, 4-level page tables for 48-bit virtual addresses.
//! Uses TTBR0 for user space and TTBR1 for kernel space.
//!
//! BCM2712 (Cortex-A76) supports:
//! - 4KB / 16KB / 64KB pages
//! - 48-bit VA (4 levels), optional 52-bit with FEAT_LVA
//! - We use 4KB pages, 48-bit VA, 4-level page tables

use crate::memory::{PhysAddr, DEVICE_MAP_START, KERNEL_IMAGE_START, PAGE_SIZE, PHYS_MAP_OFFSET};
use crate::println;

/// Page table entry flags (lower attributes)
pub const PTE_VALID: u64 = 1 << 0;
pub const PTE_TABLE: u64 = 1 << 1; // Block (0) vs Table (1) at L1/L2
pub const PTE_PAGE: u64 = 1 << 1; // Page descriptor at L3
pub const PTE_ACCESS: u64 = 1 << 10; // AF (Access Flag)
pub const PTE_NG: u64 = 1 << 11; // nG (non-Global)
pub const PTE_DBM: u64 = 1 << 51; // Dirty Bit Modifier (hardware update)

/// Shareability attributes
pub const PTE_OSH: u64 = 2 << 8; // Outer Shareable
pub const PTE_ISH: u64 = 3 << 8; // Inner Shareable

/// Memory attributes index (MAIR_EL1)
pub const MT_NORMAL: u64 = 0 << 2; // Attr0: Normal WB RW-Alloc
pub const MT_DEVICE: u64 = 1 << 2; // Attr1: Device-nGnRnE
pub const MT_NORMAL_NC: u64 = 2 << 2; // Attr2: Normal Non-Cacheable

/// Combined attributes
pub const PTE_KERNEL_DATA: u64 = PTE_VALID | PTE_PAGE | PTE_ACCESS | PTE_ISH | MT_NORMAL;
pub const PTE_KERNEL_CODE: u64 = PTE_VALID | PTE_PAGE | PTE_ACCESS | PTE_ISH | MT_NORMAL;
pub const PTE_DEVICE: u64 = PTE_VALID | PTE_PAGE | PTE_ACCESS | PTE_OSH | MT_DEVICE;
pub const PTE_DEVICE_NG: u64 = PTE_VALID | PTE_PAGE | PTE_ACCESS | PTE_NG | PTE_OSH | MT_DEVICE;

/// Number of entries per page table (4096 / 8 = 512)
const PT_ENTRIES: usize = 512;

/// Page table levels (0 = L0, root)
const L0_SHIFT: u64 = 39;
const L1_SHIFT: u64 = 30;
const L2_SHIFT: u64 = 21;
const L3_SHIFT: u64 = 12;

const VA_INDEX_MASK: u64 = 0x1FF;

/// Number of page tables we pre-allocate (enough for early boot)
const MAX_EARLY_PAGE_TABLES: usize = 4;

/// Storage for page tables (must be page-aligned)
static mut PAGE_TABLE_STORAGE: [[u64; PT_ENTRIES]; MAX_EARLY_PAGE_TABLES] =
    [[0u64; PT_ENTRIES]; MAX_EARLY_PAGE_TABLES];
static mut NEXT_PT_INDEX: usize = 0;

/// Allocate a zeroed page table from our static storage
unsafe fn alloc_page_table() -> *mut [u64; PT_ENTRIES] {
    if NEXT_PT_INDEX >= MAX_EARLY_PAGE_TABLES {
        println!("[mmu] FATAL: out of early page tables!");
        loop {
            crate::arch::cpu::wfe()
        }
    }
    let idx = NEXT_PT_INDEX;
    NEXT_PT_INDEX += 1;
    // Zero the table
    for entry in &mut PAGE_TABLE_STORAGE[idx] {
        *entry = 0;
    }
    &mut PAGE_TABLE_STORAGE[idx]
}

/// Physical address of a page table
unsafe fn pt_phys(table: *mut [u64; PT_ENTRIES]) -> u64 {
    // In early boot before MMU is enabled, phys = virt
    table as u64
}

/// Extract the page table index at a given level
fn pt_index(va: u64, level: u32) -> usize {
    let shift = match level {
        0 => L0_SHIFT,
        1 => L1_SHIFT,
        2 => L2_SHIFT,
        3 => L3_SHIFT,
        _ => panic!("invalid PT level"),
    };
    ((va >> shift) & VA_INDEX_MASK) as usize
}

/// Map a single 4KB page: va → pa with given flags
unsafe fn map_page(root: *mut [u64; PT_ENTRIES], va: u64, pa: u64, flags: u64) {
    let mut table = root;
    let _phys = pt_phys(root);

    for level in 0..3 {
        let idx = pt_index(va, level);
        let entry = (*table)[idx];

        if entry & PTE_VALID == 0 {
            // Allocate next-level table
            let next = alloc_page_table();
            let next_phys = pt_phys(next);
            (*table)[idx] = next_phys | PTE_VALID | PTE_TABLE;
            table = next;
        } else if entry & PTE_TABLE != 0 {
            // Follow the table pointer
            let next_phys = entry & !(PAGE_SIZE - 1);
            table = core::ptr::with_exposed_provenance_mut(next_phys as usize);
        } else {
            // Block entry — we can't split blocks in early boot
            panic!("[mmu] Block entry at L{} not supported yet", level);
        }
    }

    // Level 3 — write the page entry
    let idx = pt_index(va, 3);
    (*table)[idx] = pa | flags;
}

/// Map a range of pages
unsafe fn map_range(
    root: *mut [u64; PT_ENTRIES],
    va_start: u64,
    pa_start: u64,
    size: u64,
    flags: u64,
) {
    let mut va = va_start & !(PAGE_SIZE - 1);
    let pa = pa_start & !(PAGE_SIZE - 1);
    let end = ((va_start + size) + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

    while va < end {
        let offset = va - (va_start & !(PAGE_SIZE - 1));
        map_page(root, va, pa + offset, flags);
        va += PAGE_SIZE;
    }
}

/// Set up MAIR_EL1 (Memory Attribute Indirection Register)
unsafe fn setup_mair() {
    // Attr0: Normal, Write-Back, Read/Write Allocate
    let mair = 0xFF    // Attr0: Normal WB RW-Alloc
             | (0x04 << 8)    // Attr1: Device-nGnRnE
             | (0x44 << 16); // Attr2: Normal Non-Cacheable
    core::arch::asm!("msr MAIR_EL1, {mair:x}", mair = in(reg) mair);
}

/// Set up TCR_EL1 (Translation Control Register)
unsafe fn setup_tcr() {
    let tcr: u64 = (0xFFFFu64 << 48) // T1SZ = 16 (48-bit TTBR1 space)
            | (0xFFFFu64 << 32) // T0SZ = 16 (48-bit TTBR0 space)
            | (0b11u64 << 30)   // TG1 = 4KB granule for TTBR1
            | (0b00u64 << 14)   // TG0 = 4KB granule for TTBR0
            | (3u64 << 28)      // SH1 = Inner Shareable
            | (3u64 << 12)      // SH0 = Inner Shareable
            | (1u64 << 26)      // ORGN1 = Write-Back Cacheable
            | (1u64 << 10)      // ORGN0 = Write-Back Cacheable
            | (1u64 << 24)      // IRGN1 = Write-Back Cacheable
            | (1u64 << 8)       // IRGN0 = Write-Back Cacheable
            | (1u64 << 23)      // EPD1 = Enable TTBR1 walks
            | (1u64 << 7)       // EPD0 = Enable TTBR0 walks
            | (0u64 << 22)      // A1 = TTBR0 ASID
            | (0u64 << 6); // A1 = TTBR0 ASID
    core::arch::asm!("msr TCR_EL1, {}", in(reg) tcr);
}

/// Set up SCTLR_EL1 (System Control Register) — enable MMU + caches
unsafe fn enable_mmu_and_caches(ttbr0: u64, ttbr1: u64) {
    // Set TTBRs
    core::arch::asm!("msr TTBR0_EL1, {}", in(reg) ttbr0);
    core::arch::asm!("msr TTBR1_EL1, {}", in(reg) ttbr1);

    // Ensure all table writes are visible
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();

    // Invalidate all TLB entries
    core::arch::asm!("tlbi vmalle1is");
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();

    // Read SCTLR, set M (MMU), C (D-cache), I (I-cache)
    let mut sctlr: u64;
    core::arch::asm!("mrs {}, SCTLR_EL1", out(reg) sctlr);
    sctlr |= (1 << 0)  // M: MMU enable
           | (1 << 2)  // C: Data cache enable
           | (1 << 12) // I: Instruction cache enable
           | (1 << 3)  // SPAN: Privileged Access Never
           | (1 << 25) // UCT: EL0 access to CTR_EL0
           | (1 << 26) // DZE: EL0 DC ZVA
           | (1 << 27) // UMA: EL0 access to AMAIR
           ;
    // Clear WXN (write implies XN) and other restrictive bits
    sctlr &= !((1 << 19)  // WXN
             | (1 << 28)); // NTI — no trap on cache ins

    core::arch::asm!("msr SCTLR_EL1, {}", in(reg) sctlr);
    crate::arch::cpu::isb();
}

/// Initialize the MMU — set up page tables and enable virtual memory
pub unsafe fn init(kernel_phys_start: u64, kernel_size: u64, ram_size: u64) {
    println!("[mmu] Initializing MMU...");

    // Set up MAIR and TCR first (before enabling MMU)
    setup_mair();
    setup_tcr();

    // Allocate root page tables
    let ttbr0_root = alloc_page_table(); // User space (initially empty)
    let ttbr1_root = alloc_page_table(); // Kernel space

    // Identity map the first 1MB as device memory (for bootloader/DTB region)
    map_range(ttbr1_root, 0x0, 0x0, 0x10_0000, PTE_DEVICE);

    // Map kernel image: identity + high VA (both needed during transition)
    // Identity mapping (for boot transition)
    map_range(
        ttbr1_root,
        kernel_phys_start,
        kernel_phys_start,
        kernel_size,
        PTE_KERNEL_CODE,
    );
    // High VA mapping
    map_range(
        ttbr1_root,
        KERNEL_IMAGE_START,
        kernel_phys_start,
        kernel_size,
        PTE_KERNEL_CODE,
    );

    // Direct physical memory map (all RAM)
    let _phys_map_end = PHYS_MAP_OFFSET + ram_size;
    map_range(ttbr1_root, PHYS_MAP_OFFSET, 0x0, ram_size, PTE_KERNEL_DATA);

    // Map BCM2712 peripherals (SOC range: 0x10_0000_0000 .. 0x11_0000_0000)
    // UART10, GIC, System Timer, etc.
    const SOC_PERIPH_BASE: u64 = 0x10_0000_0000;
    const SOC_PERIPH_SIZE: u64 = 0x1_0000_0000; // 256MB
    map_range(
        ttbr1_root,
        DEVICE_MAP_START,
        SOC_PERIPH_BASE,
        SOC_PERIPH_SIZE,
        PTE_DEVICE,
    );

    // Map BCM2835-style peripherals (0x7C00_0000 .. 0x7D00_0000) for system timer
    const BCM_PERIPH_BASE: u64 = 0x10_7C00_0000;
    const BCM_PERIPH_SIZE: u64 = 0x0100_0000; // 1MB
    map_range(
        ttbr1_root,
        DEVICE_MAP_START + 0x100_0000,
        BCM_PERIPH_BASE,
        BCM_PERIPH_SIZE,
        PTE_DEVICE,
    );

    let ttbr0_phys = pt_phys(ttbr0_root);
    let ttbr1_phys = pt_phys(ttbr1_root);

    println!("[mmu] TTBR0 (user): {:#018x}", ttbr0_phys);
    println!("[mmu] TTBR1 (kernel): {:#018x}", ttbr1_phys);
    println!(
        "[mmu] Kernel mapped: phys {:#010x} → virt {:#018x} ({} KB)",
        kernel_phys_start,
        KERNEL_IMAGE_START,
        kernel_size / 1024
    );
    println!(
        "[mmu] Phys map: 0x0 → {:#018x} ({} MB)",
        PHYS_MAP_OFFSET,
        ram_size / (1024 * 1024)
    );
    println!("[mmu] Device map: {:#018x}", DEVICE_MAP_START);

    // Enable MMU + caches
    enable_mmu_and_caches(ttbr0_phys, ttbr1_phys);

    println!("[mmu] MMU enabled! Virtual memory active.");
    println!("[mmu] L1 D-cache + L1 I-cache enabled.");
}

/// Map a page using virtual addresses for page table access (after MMU is on)
unsafe fn map_page_via_virt(root_virt: *mut [u64; PT_ENTRIES], va: u64, pa: u64, flags: u64) {
    let mut table = root_virt;

    for level in 0..3 {
        let idx = pt_index(va, level);
        let entry = (*table)[idx];

        if entry & PTE_VALID == 0 {
            let next = alloc_page_table();
            let next_phys = pt_phys(next);
            (*table)[idx] = next_phys | PTE_VALID | PTE_TABLE;
            // Convert to virtual address for further walks
            table =
                crate::memory::phys_to_virt(PhysAddr::new(next_phys)).0 as *mut [u64; PT_ENTRIES];
        } else if entry & PTE_TABLE != 0 {
            let next_phys = entry & !(PAGE_SIZE - 1);
            table =
                crate::memory::phys_to_virt(PhysAddr::new(next_phys)).0 as *mut [u64; PT_ENTRIES];
        } else {
            panic!("[mmu] Block entry at L{} during virt map", level);
        }
    }

    let idx = pt_index(va, 3);
    (*table)[idx] = pa | flags;
}

/// Map a page after MMU is enabled (uses virtual addresses for table walks)
pub unsafe fn map_page_virt(va: u64, pa: u64, flags: u64) {
    // Get the TTBR1 root page table virtual address
    let ttbr1_root_phys = pt_phys(&mut PAGE_TABLE_STORAGE[1]);
    let ttbr1_root_virt = crate::memory::phys_to_virt(PhysAddr::new(ttbr1_root_phys)).0;
    let root = ttbr1_root_virt as *mut [u64; PT_ENTRIES];
    map_page_via_virt(root, va, pa, flags);
    // Flush TLB for this page
    core::arch::asm!("tlbi vaae1is, {}", in(reg) va >> 12);
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Unmap a page after MMU is enabled
pub unsafe fn unmap_page_virt(va: u64) {
    let ttbr1_root_phys = pt_phys(&mut PAGE_TABLE_STORAGE[1]);
    let ttbr1_root_virt = crate::memory::phys_to_virt(PhysAddr::new(ttbr1_root_phys)).0;
    let mut table = ttbr1_root_virt as *mut [u64; PT_ENTRIES];

    for level in 0..3 {
        let idx = pt_index(va, level);
        let entry = (*table)[idx];
        if entry & PTE_VALID == 0 {
            return;
        }
        let next_phys = entry & !(PAGE_SIZE - 1);
        table = crate::memory::phys_to_virt(PhysAddr::new(next_phys)).0 as *mut [u64; PT_ENTRIES];
    }

    let idx = pt_index(va, 3);
    (*table)[idx] = 0; // Invalidate
    core::arch::asm!("tlbi vaae1is, {}", in(reg) va >> 12);
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Change protection flags on a mapped page
pub unsafe fn protect_page_virt(va: u64, flags: u64) {
    let ttbr1_root_phys = pt_phys(&mut PAGE_TABLE_STORAGE[1]);
    let ttbr1_root_virt = crate::memory::phys_to_virt(PhysAddr::new(ttbr1_root_phys)).0;
    let mut table = ttbr1_root_virt as *mut [u64; PT_ENTRIES];

    for level in 0..3 {
        let idx = pt_index(va, level);
        let entry = (*table)[idx];
        if entry & PTE_VALID == 0 {
            return;
        }
        let next_phys = entry & !(PAGE_SIZE - 1);
        table = crate::memory::phys_to_virt(PhysAddr::new(next_phys)).0 as *mut [u64; PT_ENTRIES];
    }

    let idx = pt_index(va, 3);
    let old = (*table)[idx];
    if old & PTE_VALID != 0 {
        let pa = old & !(PAGE_SIZE - 1);
        (*table)[idx] = pa | flags;
    }
    core::arch::asm!("tlbi vaae1is, {}", in(reg) va >> 12);
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Software page table walk — translate VA → PA
pub unsafe fn translate(va: u64) -> Option<u64> {
    let ttbr1_root_phys = pt_phys(&mut PAGE_TABLE_STORAGE[1]);
    let ttbr1_root_virt = crate::memory::phys_to_virt(PhysAddr::new(ttbr1_root_phys)).0;
    let mut table = ttbr1_root_virt as *mut [u64; PT_ENTRIES];

    for level in 0..4 {
        let idx = pt_index(va, level);
        let entry = (*table)[idx];
        if entry & PTE_VALID == 0 {
            return None;
        }
        if level == 3 || (entry & PTE_TABLE == 0 && level < 3) {
            // Block or page entry
            let pa = entry & !(PAGE_SIZE - 1);
            let offset = va & (PAGE_SIZE - 1);
            return Some(pa | offset);
        }
        let next_phys = entry & !(PAGE_SIZE - 1);
        table = crate::memory::phys_to_virt(PhysAddr::new(next_phys)).0 as *mut [u64; PT_ENTRIES];
    }
    None
}

/// Get the physical address of TTBR1 root (for secondary cores)
pub unsafe fn get_ttbr1_phys() -> u64 {
    pt_phys(&mut PAGE_TABLE_STORAGE[1])
}
