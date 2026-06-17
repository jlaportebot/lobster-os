//! Virtual memory manager
//!
//! High-level API for mapping/unmapping virtual pages.
//! Wraps the MMU page table manipulation with a cleaner interface.

use crate::memory::{PhysAddr, VirtAddr};
use crate::println;

// Re-export PTE flags from mmu for external use
pub use super::mmu::MT_DEVICE;
pub use super::mmu::MT_NORMAL_NC;
pub use super::mmu::PTE_ACCESS;
pub use super::mmu::PTE_DEVICE;
pub use super::mmu::PTE_ISH;
pub use super::mmu::PTE_KERNEL_CODE;
pub use super::mmu::PTE_KERNEL_DATA;
pub use super::mmu::PTE_NG;
pub use super::mmu::PTE_PAGE;
pub use super::mmu::PTE_VALID;

/// Map a single kernel page (in TTBR1 space)
///
/// This is a convenience wrapper around the MMU map_page function.
/// After the MMU is initialized, new mappings must use the virtual
/// addresses of the page tables (via phys_to_virt).
pub unsafe fn map_kernel_page(va: u64, pa: u64, flags: u64) {
    // We need to walk the page tables via virtual addresses now
    // since the MMU is already enabled with the direct phys map
    super::mmu::map_page_virt(va, pa, flags);
}

/// Unmap a kernel page (mark as invalid)
pub unsafe fn unmap_kernel_page(va: u64) {
    super::mmu::unmap_page_virt(va);
}

/// Change protection flags on a mapped page
pub unsafe fn protect_kernel_page(va: u64, flags: u64) {
    super::mmu::protect_page_virt(va, flags);
}

/// Translate a kernel virtual address to physical
pub fn translate_va(va: VirtAddr) -> Option<PhysAddr> {
    unsafe { super::mmu::translate(va.0).map(PhysAddr::new) }
}

/// Information about a memory region
#[derive(Clone, Copy, Debug)]
pub struct MemoryRegion {
    pub virt_start: VirtAddr,
    pub phys_start: PhysAddr,
    pub size: u64,
    pub flags: u64,
    pub name: &'static str,
}

/// The kernel's memory map (populated during boot)
static mut KERNEL_MEMORY_MAP: [Option<MemoryRegion>; 16] = [None; 16];
static mut NUM_REGIONS: usize = 0;

/// Register a memory region in the kernel map
pub unsafe fn register_region(region: MemoryRegion) {
    if NUM_REGIONS < 16 {
        KERNEL_MEMORY_MAP[NUM_REGIONS] = Some(region);
        NUM_REGIONS += 1;
    }
}

/// Print the kernel memory map
pub fn print_memory_map() {
    println!("[vmem] print_memory_map() called");
    println!("[vmem] Kernel memory map:");
    unsafe {
        for i in 0..NUM_REGIONS {
            if let Some(r) = &KERNEL_MEMORY_MAP[i] {
                let size_kb = r.size / 1024;
                let perm = if r.flags & PTE_NG != 0 {
                    "user"
                } else {
                    "kern"
                };
                let cache = if r.flags & MT_DEVICE != 0 {
                    "device"
                } else if r.flags & MT_NORMAL_NC != 0 {
                    "uncached"
                } else {
                    "cached"
                };
                println!(
                    "  {:20} {:#018x}..{:#018x} → {:#010x} ({} KB, {}, {})",
                    r.name,
                    r.virt_start.0,
                    r.virt_start.0 + r.size,
                    r.phys_start.0,
                    size_kb,
                    perm,
                    cache
                );
            }
        }
    }
    println!();
}
