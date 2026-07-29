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

// Block variants (for L1/L2 block mappings — bit 1 = 0, not PTE_PAGE)
pub const PTE_KERNEL_CODE_BLOCK: u64 = PTE_VALID | PTE_ACCESS | PTE_ISH | MT_NORMAL;
pub const PTE_KERNEL_DATA_BLOCK: u64 = PTE_VALID | PTE_ACCESS | PTE_ISH | MT_NORMAL;
/// Block entry (2MB) for device memory — bit 1 (Table/Block) must be 0
pub const PTE_DEVICE_BLOCK: u64 = PTE_VALID | PTE_ACCESS | PTE_OSH | MT_DEVICE;

/// Number of entries per page table (4096 / 8 = 512)
const PT_ENTRIES: usize = 512;

/// Page table levels (0 = L0, root)
const L0_SHIFT: u64 = 39;
const L1_SHIFT: u64 = 30;
const L2_SHIFT: u64 = 21;
const L3_SHIFT: u64 = 12;

const VA_INDEX_MASK: u64 = 0x1FF;

/// Number of page tables we pre-allocate (enough for early boot)
/// Physical map uses 1 L2 table per 1GB. For 2GB QEMU = 2, for 8GB Pi 5 = 8.
/// Kernel map: 1 L2. Identity map: 1 L1 + 1 L2. Device maps: 1 L1 + 1 L2.
/// 2 roots + 2 L1 + ~12 L2 = ~16. Use 64 for headroom.
const MAX_EARLY_PAGE_TABLES: usize = 64;

/// Storage for page tables — must be page-aligned (4KB) for TTBR registers.
#[repr(C, align(4096))]
struct PageTableStorage([[u64; PT_ENTRIES]; MAX_EARLY_PAGE_TABLES]);

static mut PAGE_TABLE_STORAGE: PageTableStorage =
    PageTableStorage([[0u64; PT_ENTRIES]; MAX_EARLY_PAGE_TABLES]);
static mut NEXT_PT_INDEX: usize = 0;

/// Allocate a zeroed page table from our static storage
unsafe fn alloc_page_table() -> *mut [u64; PT_ENTRIES] {
    if NEXT_PT_INDEX >= MAX_EARLY_PAGE_TABLES {
        println!(
            "[mmu] FATAL: out of early page tables! (allocated {})",
            NEXT_PT_INDEX
        );
        loop {
            crate::arch::cpu::wfe()
        }
    }
    let idx = NEXT_PT_INDEX;
    NEXT_PT_INDEX += 1;
    // Zero the table
    for entry in &mut PAGE_TABLE_STORAGE.0[idx] {
        *entry = 0;
    }
    &mut PAGE_TABLE_STORAGE.0[idx]
}

/// Physical address of a page table
unsafe fn pt_phys(table: *mut [u64; PT_ENTRIES]) -> u64 {
    // After MMU is enabled, static data lives at a high VA in the
    // phys-map region (VA = PA + PHYS_MAP_OFFSET). Convert back to PA.
    let va = table as u64;
    if va >= crate::memory::PHYS_MAP_OFFSET {
        va - crate::memory::PHYS_MAP_OFFSET
    } else {
        // Early boot (pre-MMU): phys == virt
        va
    }
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

/// Map a range of pages (4KB granularity) — for kernel/device mappings
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

/// Map a range using 2MB block entries at L2
unsafe fn map_range_blocks(
    root: *mut [u64; PT_ENTRIES],
    va_start: u64,
    pa_start: u64,
    size: u64,
    flags: u64,
) {
    // 2MB block size (L2 block)
    const BLOCK_SIZE_2M: u64 = 2 * 1024 * 1024;
    const BLOCK_ALIGN_2M: u64 = BLOCK_SIZE_2M - 1;

    let mut va = va_start & !BLOCK_ALIGN_2M;
    let mut pa = pa_start & !BLOCK_ALIGN_2M;
    let end = ((va_start + size) + BLOCK_ALIGN_2M) & !BLOCK_ALIGN_2M;

    while va < end {
        let idx_l0 = pt_index(va, 0);
        let idx_l1 = pt_index(va, 1);
        let idx_l2 = pt_index(va, 2);

        let table_l0 = root;
        let entry_l0 = (*table_l0)[idx_l0];

        // Allocate L1 table if needed
        let table_l1: *mut [u64; PT_ENTRIES];
        if entry_l0 & PTE_VALID == 0 {
            let next = alloc_page_table();
            let next_phys = pt_phys(next);
            (*table_l0)[idx_l0] = next_phys | PTE_VALID | PTE_TABLE;
            table_l1 = next;
        } else if entry_l0 & PTE_TABLE != 0 {
            let next_phys = entry_l0 & !(PAGE_SIZE - 1);
            table_l1 = core::ptr::with_exposed_provenance_mut(next_phys as usize);
        } else {
            panic!("[mmu] Block entry at L0 not expected");
        }

        // Allocate L2 table if needed
        let entry_l1 = (*table_l1)[idx_l1];
        let table_l2: *mut [u64; PT_ENTRIES];
        if entry_l1 & PTE_VALID == 0 {
            let next = alloc_page_table();
            let next_phys = pt_phys(next);
            (*table_l1)[idx_l1] = next_phys | PTE_VALID | PTE_TABLE;
            table_l2 = next;
        } else if entry_l1 & PTE_TABLE != 0 {
            let next_phys = entry_l1 & !(PAGE_SIZE - 1);
            table_l2 = core::ptr::with_exposed_provenance_mut(next_phys as usize);
        } else {
            panic!("[mmu] Block entry at L1 not expected (should be table for 2MB blocks)");
        }

        // Use 2MB block at L2
        let entry_l2 = pa | flags | PTE_VALID | PTE_ACCESS; // Block entry (no PTE_TABLE)
        (*table_l2)[idx_l2] = entry_l2;

        va += BLOCK_SIZE_2M;
        pa += BLOCK_SIZE_2M;
    }
}

/// Set up MAIR_EL1 (Memory Attribute Indirection Register)
unsafe fn setup_mair() {
    // Attr0: Normal, Write-Back, Read/Write Allocate
    let mair = 0xFF    // Attr0: Normal WB RW-Alloc
             | (0x04 << 8)    // Attr1: Device-nGnRnE
             | (0x44 << 16); // Attr2: Normal Non-Cacheable
    core::arch::asm!("msr MAIR_EL1, {0:x}", in(reg) mair);
}

/// Set up TCR_EL1 (Translation Control Register)
unsafe fn setup_tcr() {
    let tcr: u64 = (16u64 << 16)   // T1SZ = 16 (48-bit TTBR1 VA space)
            | (16u64 << 0)        // T0SZ = 16 (48-bit TTBR0 VA space)
            | (0b10u64 << 30)     // TG1 = 4KB granule for TTBR1 (0b10 = 4KB)
            | (0b00u64 << 14)     // TG0 = 4KB granule for TTBR0
            | (3u64 << 28)        // SH1 = Inner Shareable
            | (3u64 << 12)        // SH0 = Inner Shareable
            | (1u64 << 26)        // ORGN1 = Write-Back Cacheable
            | (1u64 << 10)        // ORGN0 = Write-Back Cacheable
            | (1u64 << 24)        // IRGN1 = Write-Back Cacheable
            | (1u64 << 8)         // IRGN0 = Write-Back Cacheable
            // EPD1 (bit 23) = 0 → ENABLE TTBR1 walks
            // EPD0 (bit 7)  = 0 → ENABLE TTBR0 walks
            // IPS (bits 34:32) = 2 → 40-bit PA (1TB), covers SoC peripherals at 0x10_xxxx_xxxx
            | (2u64 << 32);
    core::arch::asm!("msr TCR_EL1, {0}", in(reg) tcr);
}

/// Set up SCTLR_EL1 (System Control Register) — enable MMU + caches
unsafe fn enable_mmu_and_caches(ttbr0: u64, ttbr1: u64) {
    // Set TTBRs
    core::arch::asm!("msr TTBR0_EL1, {0}", in(reg) ttbr0);
    core::arch::asm!("msr TTBR1_EL1, {0}", in(reg) ttbr1);

    // Ensure all table writes are visible
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();

    // Invalidate all TLB entries
    core::arch::asm!("tlbi vmalle1is");
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();

    // Read SCTLR, set ONLY M (MMU enable). No caches, no features.
    // This is the minimal safest MMU enable for debugging.
    let mut sctlr: u64;
    core::arch::asm!("mrs {0}, SCTLR_EL1", out(reg) sctlr);

    // Debug: dump TCR value
    let mut tcr_val: u64;
    core::arch::asm!("mrs {0}, TCR_EL1", out(reg) tcr_val);
    crate::println!("[mmu] TCR_EL1 = {:#018x}", tcr_val);
    crate::println!("[mmu] SCTLR before = {:#018x}", sctlr);

    sctlr |= 1 << 0; // M: MMU enable ONLY

    // Clear WXN (write implies XN) and other restrictive bits
    sctlr &= !((1 << 19)  // WXN
             | (1 << 28)); // NTI

    crate::println!("[mmu] SCTLR after  = {:#018x}", sctlr);

    core::arch::asm!("msr SCTLR_EL1, {0}", in(reg) sctlr);
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

    // Identity map the first 128MB in TTBR0 (low VA range).
    // Covers kernel image (~0x80000-0x13e000), BSS, page tables, and
    // the 16MB stack area (up to ~0x41f2000). 128MB = 64 × 2MB blocks.
    map_range_blocks(
        ttbr0_root,
        0x0,
        0x0,
        128 * 1024 * 1024,
        PTE_KERNEL_CODE_BLOCK,
    );

    // Identity-map the full QEMU peripheral range in TTBR0 as device memory.
    // QEMU raspi5b puts peripherals at 0x7C00_0000 - 0x7E00_0000 (32MB).
    // These VAs have bit 55 = 0 → resolved via TTBR0, not TTBR1.
    // Without this, any device access (UART, timer, GIC GIC) faults.
    #[cfg(feature = "qemu_raspi5b")]
    {
        const QEMU_PERIPH_BASE: u64 = 0x7C00_0000;
        const QEMU_PERIPH_SIZE: u64 = 0x200_0000; // 32MB = 16 × 2MB blocks
        map_range_blocks(
            ttbr0_root,
            QEMU_PERIPH_BASE,
            QEMU_PERIPH_BASE,
            QEMU_PERIPH_SIZE,
            PTE_DEVICE_BLOCK,
        );

        // Identity-map GIC-400 at 0x10_7FFF_0000 in TTBR0.
        // The GIC is at PA 0x10_7FFF_9000 (GICD) / 0x10_7FFF_A000 (GICC).
        // VA bit 55 = 0, so TTBR0 handles it.
        const GIC_PERIPH_BASE: u64 = 0x10_7FFF_0000;
        const GIC_PERIPH_SIZE: u64 = 0x20_0000; // 2MB block
        println!(
            "[mmu] GIC identity map: base={:#010x} size={:#x}",
            GIC_PERIPH_BASE, GIC_PERIPH_SIZE
        );
        map_range_blocks(
            ttbr0_root,
            GIC_PERIPH_BASE,
            GIC_PERIPH_BASE,
            GIC_PERIPH_SIZE,
            PTE_DEVICE_BLOCK,
        );
        println!("[mmu] GIC identity map done, next_pt={}", unsafe {
            NEXT_PT_INDEX
        });

        // Debug: verify L1[65] was created by reading from the L1 table
        let l0_entry0 = (*ttbr0_root)[0];
        let l1_table_phys = (l0_entry0 & !(PAGE_SIZE - 1)) as *const [u64; PT_ENTRIES];
        let l1_65 = unsafe { (*l1_table_phys)[65] };
        println!(
            "[mmu] Debug: L0[0]={:#018x} L1[65]={:#018x}",
            l0_entry0, l1_65
        );
    }

    // Map kernel image at high VA (identity map only needed during boot transition, not in final page tables)
    let kernel_map_size = ((kernel_size + 0x1FFFFF) & !0x1FFFFF).max(2 * 1024 * 1024);
    map_range_blocks(
        ttbr1_root,
        KERNEL_IMAGE_START & !0x1FFFFF,
        kernel_phys_start & !0x1FFFFF,
        kernel_map_size,
        PTE_KERNEL_CODE_BLOCK,
    );

    // Direct physical memory map (all RAM) — use 2MB/1GB block mappings
    let _phys_map_end = PHYS_MAP_OFFSET + ram_size;
    map_range_blocks(
        ttbr1_root,
        PHYS_MAP_OFFSET,
        0x0,
        ram_size,
        PTE_KERNEL_DATA_BLOCK,
    );

    // Map BCM2712 peripherals (SOC range: 0x10_0000_0000 .. 0x11_0000_0000)
    // UART10, GIC, System Timer, etc.
    const SOC_PERIPH_BASE: u64 = 0x10_0000_0000;
    const SOC_PERIPH_SIZE: u64 = 0x1_0000_0000; // 256MB
    map_range_blocks(
        ttbr1_root,
        DEVICE_MAP_START,
        SOC_PERIPH_BASE,
        SOC_PERIPH_SIZE,
        PTE_DEVICE_BLOCK,
    );

    // GIC-400 is identity-mapped in TTBR0 for QEMU (physical address access).
    // No separate TTBR1 GIC mapping needed.
    #[cfg(feature = "qemu_raspi5b")]
    {
        // Force compile-time check
        const _: () = assert!(cfg!(feature = "qemu_raspi5b"));
        // Map BCM2711-style peripherals (0x7C00_0000 .. 0x7E00_0000) for QEMU raspi5b
        // System timer, mailbox, UARTs, clocks, GPIOs, etc.
        // NOTE: These are ALSO identity-mapped in TTBR0 for pre-virtual-address access.
        const BCM2711_PERIPH_BASE: u64 = 0x7C00_0000;
        const BCM2711_PERIPH_SIZE: u64 = 0x2000_0000; // 512MB
        map_range_blocks(
            ttbr1_root,
            DEVICE_MAP_START + 0x200_0000,
            BCM2711_PERIPH_BASE,
            BCM2711_PERIPH_SIZE,
            PTE_DEVICE_BLOCK,
        );
    }

    #[cfg(not(feature = "qemu_raspi5b"))]
    {
        // Map BCM2835-style peripherals (0x7C00_0000 .. 0x7D00_0000) for system timer
        // Only used on real Pi 5 and QEMU raspi4b
        const BCM_PERIPH_BASE: u64 = 0x10_7C00_0000;
        const BCM_PERIPH_SIZE: u64 = 0x0100_0000; // 1MB
        map_range_blocks(
            ttbr1_root,
            DEVICE_MAP_START + 0x100_0000,
            BCM_PERIPH_BASE,
            BCM_PERIPH_SIZE,
            PTE_DEVICE_BLOCK,
        );
    }

    // Map GPU bus memory (VC RAM) for QEMU framebuffer display
    // QEMU gpu_bus_mr has RAM aliased at 0x0, 0x40000000, 0x80000000, 0xC0000000 (each 4GB)
    // vcram_base = 0xFC000000 (GPU bus), framebuffer at 0xFC100000
    // This falls in the 4th alias (0xC0000000..0xFFFFFFFF) -> physical 0x3C000000..0x40000000
    // Map the 4th alias region (0xC0000000..0x1_0000_0000 GPU bus) as DEVICE (uncached)
    // so CPU writes are immediately visible to QEMU display.
    #[cfg(feature = "qemu_raspi5b")]
    {
        const GPU_BUS_4TH_ALIAS: u64 = 0xC000_0000; // 4th RAM alias in GPU bus MR
        const GPU_BUS_4TH_ALIAS_SIZE: u64 = 0x4000_0000; // 1GB (covers 0xC0000000..0xFFFFFFFF)
        map_range_blocks(
            ttbr1_root,
            DEVICE_MAP_START + 0x300_0000, // VA: 0xFFFF_C000_3000_0000
            GPU_BUS_4TH_ALIAS,             // PA: GPU bus 4th alias (0xC000_0000)
            GPU_BUS_4TH_ALIAS_SIZE,
            PTE_DEVICE_BLOCK,
        );
        println!(
            "[mmu] GPU bus 4th alias mapped: {:#x} -> GPU bus {:#x} ({} MB)",
            DEVICE_MAP_START + 0x300_0000,
            GPU_BUS_4TH_ALIAS,
            GPU_BUS_4TH_ALIAS_SIZE / 1024 / 1024
        );
    }

    println!("[mmu] After cfg blocks, before TTBR phys");

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

    println!("[mmu] About to enable MMU...");

    // Debug: dump first few page table entries for TTBR0
    let ttbr0_entries = PAGE_TABLE_STORAGE.0[0];
    println!(
        "[mmu] TTBR0 L0[0]: {:#018x} (expect valid=1, table=1)",
        ttbr0_entries[0]
    );
    if ttbr0_entries[0] & 1 != 0 {
        let l1_ptr = (ttbr0_entries[0] & !0xFFF) as *const [u64; PT_ENTRIES];
        println!("[mmu] L1[0]: {:#018x} (expect valid=1, table=1)", unsafe {
            (*l1_ptr)[0]
        });
        if unsafe { (*l1_ptr)[0] } & 1 != 0 {
            let l2_ptr = (unsafe { (*l1_ptr)[0] } & !0xFFF) as *const [u64; PT_ENTRIES];
            println!(
                "[mmu] L2[0]: {:#018x} (expect valid=1, block, AF=1)",
                unsafe { (*l2_ptr)[0] }
            );
        }
    }

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
            table =
                crate::memory::phys_to_virt(PhysAddr::new(next_phys)).0 as *mut [u64; PT_ENTRIES];
        } else if entry & PTE_TABLE != 0 {
            let next_phys = entry & !(PAGE_SIZE - 1);
            table =
                crate::memory::phys_to_virt(PhysAddr::new(next_phys)).0 as *mut [u64; PT_ENTRIES];
        } else {
            // Block entry — split it into a next-level table.
            // A block at level N covers (1 << shift_for_N) bytes and
            // splits into 512 entries at level N+1.
            let block_pa = entry & !(PAGE_SIZE - 1);
            let block_flags = entry & (PAGE_SIZE - 1);
            let current_shift = match level {
                0 => L0_SHIFT,
                1 => L1_SHIFT,
                2 => L2_SHIFT,
                _ => unreachable!(),
            };
            // Each sub-entry is 1/512 of the block = 1 << (shift - 9)
            let sub_stride: u64 = 1u64 << (current_shift - 9);
            let sub_level = level + 1;

            let next = alloc_page_table();
            let next_phys = pt_phys(next);
            let next_virt =
                crate::memory::phys_to_virt(PhysAddr::new(next_phys)).0 as *mut [u64; PT_ENTRIES];

            // Populate the new table BEFORE linking it into the parent.
            // If we link first, the old block mapping is gone and the CPU
            // faults when dereferencing next_virt (which lives inside the
            // region being remapped).
            for i in 0..PT_ENTRIES {
                let sub_pa = block_pa + (i as u64) * sub_stride;
                if sub_level == 3 {
                    // L3 entries are page descriptors (bit 1 = PTE_PAGE)
                    (*next_virt)[i] = sub_pa | block_flags | PTE_PAGE;
                } else {
                    // Sub-level is L1 or L2 — keep as blocks
                    (*next_virt)[i] = sub_pa | block_flags;
                }
            }

            // Now link — the new table is fully populated, so a TLB walk
            // after this replacement will find valid mappings.
            (*table)[idx] = next_phys | PTE_VALID | PTE_TABLE;
            let block_va = va & !((1u64 << current_shift) - 1);
            core::arch::asm!("tlbi vaae1is, {0}", in(reg) block_va >> 12);
            crate::arch::cpu::dsb();
            crate::arch::cpu::isb();
            table = next_virt;
        }
    }

    let idx = pt_index(va, 3);
    (*table)[idx] = pa | flags;
}

/// Map a page after MMU is enabled (uses virtual addresses for table walks)
pub unsafe fn map_page_virt(va: u64, pa: u64, flags: u64) {
    // Get the TTBR1 root page table virtual address
    let ttbr1_root_phys = pt_phys(&mut PAGE_TABLE_STORAGE.0[1]);
    let ttbr1_root_virt = crate::memory::phys_to_virt(PhysAddr::new(ttbr1_root_phys)).0;
    let root = ttbr1_root_virt as *mut [u64; PT_ENTRIES];
    map_page_via_virt(root, va, pa, flags);
    // Flush TLB for this page
    core::arch::asm!("tlbi vaae1is, {0}", in(reg) va >> 12);
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Unmap a page after MMU is enabled
pub unsafe fn unmap_page_virt(va: u64) {
    let ttbr1_root_phys = pt_phys(&mut PAGE_TABLE_STORAGE.0[1]);
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
    core::arch::asm!("tlbi vaae1is, {0}", in(reg) va >> 12);
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Change protection flags on a mapped page
pub unsafe fn protect_page_virt(va: u64, flags: u64) {
    let ttbr1_root_phys = pt_phys(&mut PAGE_TABLE_STORAGE.0[1]);
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
    core::arch::asm!("tlbi vaae1is, {0}", in(reg) va >> 12);
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Software page table walk — translate VA → PA
pub unsafe fn translate(va: u64) -> Option<u64> {
    let ttbr1_root_phys = pt_phys(&mut PAGE_TABLE_STORAGE.0[1]);
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
    pt_phys(&mut PAGE_TABLE_STORAGE.0[1])
}
