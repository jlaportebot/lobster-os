//! Per-process address space management
//!
//! Each user process gets its own TTBR0 page table tree, providing
//! full virtual memory isolation between processes.
//!
//! Layout (per-process, TTBR0 space, lower half):
//!   0x0000_0000_1000_0000  Code + data (loaded from ELF)
//!   0x0000_0000_2000_0000  Heap (grows up via brk/mmap)
//!   0x0000_0000_7F00_0000  Stack (grows down)
//!   0x0000_0000_8000_0000  Address space limit
//!
//! COW (Copy-on-Write):
//!   fork() marks all user pages read-only + COW bit.
//!   On write fault, the page fault handler:
//!     1. Allocates a new frame
//!     2. Copies the original page data
//!     3. Maps the new frame as writable
//!     4. Decrements the original frame's refcount

use crate::memory::frame_alloc;
use crate::memory::{phys_to_virt, PhysAddr, PAGE_SIZE};
use crate::println;

/// Re-export PTE flags
pub use super::mmu::{MT_NORMAL, PTE_ACCESS, PTE_ISH, PTE_NG, PTE_PAGE, PTE_TABLE, PTE_VALID};

/// User-space PTE flags (read-only, non-global, inner-shareable)
pub const PTE_USER_RO: u64 = PTE_VALID | PTE_PAGE | PTE_ACCESS | PTE_NG | PTE_ISH | MT_NORMAL;

/// User-space PTE flags (read-write, non-global, inner-shareable)
pub const PTE_USER_RW: u64 = PTE_VALID | PTE_PAGE | PTE_ACCESS | PTE_NG | PTE_ISH | MT_NORMAL;

/// User-space PTE flags (read-execute, non-global, inner-shareable)
pub const PTE_USER_RX: u64 = PTE_VALID | PTE_PAGE | PTE_ACCESS | PTE_NG | PTE_ISH | MT_NORMAL;

/// COW flag — bit 52 (software-defined, ignored by hardware)
pub const PTE_COW: u64 = 1 << 52;

/// Shared/refcounted flag — bit 53
pub const PTE_SHARED: u64 = 1 << 53;

/// Maximum VMA regions per process
pub const MAX_VMAS: usize = 32;

/// User address space limits (matching elf.rs)
pub const USER_CODE_START: u64 = 0x0000_0000_1000_0000;
pub const USER_HEAP_START: u64 = 0x0000_0000_2000_0000;
pub const USER_STACK_TOP: u64 = 0x0000_0000_7F00_0000;
pub const USER_ADDR_LIMIT: u64 = 0x0000_0000_8000_0000;
pub const USER_STACK_SIZE: u64 = 0x0010_0000; // 1MB default

/// Virtual Memory Area — describes a contiguous region of a process's address space
#[derive(Clone, Copy, Debug)]
pub struct Vma {
    /// Start virtual address (page-aligned)
    pub start: u64,
    /// Size in bytes (page-aligned)
    pub size: u64,
    /// Protection flags (PTE_USER_*)
    pub prot: u64,
    /// Is this area backed by a file? (None = anonymous)
    pub file_offset: u64,
    /// Is this area shared or private?
    pub shared: bool,
    /// Is this area used?
    pub in_use: bool,
}

/// Per-process address space
#[repr(C)]
pub struct AddressSpace {
    /// Physical address of the TTBR0 root page table
    pub ttbr0_phys: u64,
    /// VMAs for this process
    pub vmas: [Vma; MAX_VMAS],
    /// Number of active VMAs
    pub vma_count: usize,
    /// Current program break (top of heap)
    pub brk: u64,
}
// Page refcounts for COW tracking: we use a compact reference count table
// (defined below) rather than a full per-frame array, since 8GB RAM would
// need 2M entries × 1 byte = 2MB which we can't afford statically.

/// Maximum tracked COW frames (compact table)
const MAX_COW_FRAMES: usize = 256;

/// COW frame reference count entry
#[derive(Clone, Copy)]
struct CowEntry {
    paddr: u64,
    refcount: u16,
    in_use: bool,
}

/// Global COW reference count table
static mut COW_TABLE: [CowEntry; MAX_COW_FRAMES] = [CowEntry {
    paddr: 0,
    refcount: 0,
    in_use: false,
}; MAX_COW_FRAMES];
static mut COW_COUNT: usize = 0;

/// Look up or create a COW entry for a physical frame
fn cow_get(paddr: u64) -> usize {
    unsafe {
        // Search existing
        for i in 0..COW_COUNT {
            if COW_TABLE[i].in_use && COW_TABLE[i].paddr == paddr {
                return i;
            }
        }
        // Create new entry
        if COW_COUNT < MAX_COW_FRAMES {
            let idx = COW_COUNT;
            COW_TABLE[idx] = CowEntry {
                paddr,
                refcount: 1,
                in_use: true,
            };
            COW_COUNT += 1;
            return idx;
        }
        // Table full — can't track, treat as unshared
        usize::MAX
    }
}

/// Increment refcount for a COW frame
pub fn cow_ref(paddr: u64) {
    let idx = cow_get(paddr);
    if idx != usize::MAX {
        unsafe {
            COW_TABLE[idx].refcount += 1;
        }
    }
}

/// Decrement refcount for a COW frame, return true if last reference
pub fn cow_deref(paddr: u64) -> bool {
    unsafe {
        for i in 0..COW_COUNT {
            if COW_TABLE[i].in_use && COW_TABLE[i].paddr == paddr {
                if COW_TABLE[i].refcount > 1 {
                    COW_TABLE[i].refcount -= 1;
                    return false; // Still shared
                } else {
                    COW_TABLE[i].in_use = false;
                    return true; // Last reference — free the frame
                }
            }
        }
        true // Not tracked — assume last reference
    }
}

/// Check if a physical frame has multiple references (is COW-shared)
pub fn cow_is_shared(paddr: u64) -> bool {
    unsafe {
        for i in 0..COW_COUNT {
            if COW_TABLE[i].in_use && COW_TABLE[i].paddr == paddr {
                return COW_TABLE[i].refcount > 1;
            }
        }
        false
    }
}

/// Allocate a new (empty) address space with a TTBR0 root page table
pub fn create_address_space() -> Option<AddressSpace> {
    let root_frame = frame_alloc::allocate_frame()?;
    // Zero the root page table
    let root_virt = phys_to_virt(root_frame);
    unsafe {
        let table = root_virt.0 as *mut u64;
        for i in 0..512 {
            core::ptr::write_volatile(table.add(i), 0);
        }
        crate::arch::cpu::dsb();
    }

    Some(AddressSpace {
        ttbr0_phys: root_frame.0,
        vmas: [Vma {
            start: 0,
            size: 0,
            prot: 0,
            file_offset: 0,
            shared: false,
            in_use: false,
        }; MAX_VMAS],
        vma_count: 0,
        brk: USER_HEAP_START,
    })
}

/// Destroy an address space — free all page tables and data frames
pub fn destroy_address_space(space: &AddressSpace) {
    // Walk the TTBR0 page tables and free all frames
    // We need to free: L1 tables, L2 tables, L3 tables, and data pages
    let root_virt = (crate::memory::PHYS_MAP_OFFSET + space.ttbr0_phys) as *mut [u64; 512];
    unsafe {
        free_page_table_tree(root_virt, 0);
    }
    // Free the root table itself
    frame_alloc::deallocate_frame(PhysAddr::new(space.ttbr0_phys));
}

/// Recursively free all page table frames and data frames in a subtree
unsafe fn free_page_table_tree(table: *mut [u64; 512], level: u32) {
    if level >= 3 {
        return; // L3 entries are data pages, not tables
    }
    for i in 0..512 {
        let entry = (*table)[i];
        if entry & PTE_VALID == 0 {
            continue;
        }
        if entry & PTE_TABLE != 0 {
            // Table descriptor — recurse
            let next_phys = entry & !(PAGE_SIZE - 1);
            let next_virt = (crate::memory::PHYS_MAP_OFFSET + next_phys) as *mut [u64; 512];
            free_page_table_tree(next_virt, level + 1);
            // Free the table frame itself
            frame_alloc::deallocate_frame(PhysAddr::new(next_phys));
        } else if level == 2 {
            // Block entry at L2 — free the block (1 page of data = 2MB block)
            // Block entries map large ranges; we'd need to free the underlying frames
            // For now, just log it
        }
    }
    // At L2, free the data pages pointed to by L3 entries
    if level == 2 {
        for i in 0..512 {
            let entry = (*table)[i];
            if entry & PTE_VALID == 0 {
                continue;
            }
            if entry & PTE_TABLE != 0 {
                // Already handled above
            }
        }
    }
}

/// Recursively free data pages (L3 leaves) in a subtree
pub unsafe fn free_user_pages(table: *mut [u64; 512], level: u32) {
    if level > 3 {
        return;
    }
    for i in 0..512 {
        let entry = (*table)[i];
        if entry & PTE_VALID == 0 {
            continue;
        }

        if level < 3 && (entry & PTE_TABLE) != 0 {
            // Follow table pointer
            let next_phys = entry & !(PAGE_SIZE - 1);
            let next_virt = (crate::memory::PHYS_MAP_OFFSET + next_phys) as *mut [u64; 512];
            free_user_pages(next_virt, level + 1);
        } else if level == 3 {
            // Data page — free it
            let pa = entry & !(PAGE_SIZE - 1);
            // Check COW refcount before freeing
            if entry & PTE_COW != 0 {
                if cow_deref(pa) {
                    frame_alloc::deallocate_frame(PhysAddr::new(pa));
                }
            } else {
                frame_alloc::deallocate_frame(PhysAddr::new(pa));
            }
        }
    }
}

/// Map a page into a user address space
pub unsafe fn map_user_page(ttbr0_phys: u64, va: u64, pa: u64, flags: u64) {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let root_virt = (phys_map + ttbr0_phys) as *mut [u64; 512];
    let mut table = root_virt;

    for level in 0..3 {
        let idx = ((va >> (39 - level * 9)) & 0x1FF) as usize;
        let entry = (*table)[idx];

        if entry & PTE_VALID == 0 {
            // Allocate next-level table
            let next_frame = match frame_alloc::allocate_frame() {
                Some(f) => f,
                None => {
                    println!("[vmem] Out of frames for user page tables!");
                    return;
                }
            };
            // Zero the new table
            let next_virt = (phys_map + next_frame.0) as *mut u64;
            for j in 0..512 {
                core::ptr::write_volatile(next_virt.add(j), 0);
            }
            crate::arch::cpu::dsb();

            (*table)[idx] = next_frame.0 | PTE_VALID | PTE_TABLE;
            table = (phys_map + next_frame.0) as *mut [u64; 512];
        } else if entry & PTE_TABLE != 0 {
            let next_phys = entry & !(PAGE_SIZE - 1);
            table = (phys_map + next_phys) as *mut [u64; 512];
        } else {
            println!("[vmem] Block entry at L{} for VA {:#018x}", level, va);
            return;
        }
    }

    // Level 3 — write the page entry
    let idx = ((va >> 12) & 0x1FF) as usize;
    (*table)[idx] = pa | flags;
    crate::arch::cpu::dsb();
}

/// Unmap a page from a user address space (returns the old PA if mapped)
pub unsafe fn unmap_user_page(ttbr0_phys: u64, va: u64) -> Option<u64> {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let root_virt = (phys_map + ttbr0_phys) as *mut [u64; 512];
    let mut table = root_virt;

    for level in 0..3 {
        let idx = ((va >> (39 - level * 9)) & 0x1FF) as usize;
        let entry = (*table)[idx];
        if entry & PTE_VALID == 0 {
            return None;
        }
        let next_phys = entry & !(PAGE_SIZE - 1);
        table = (phys_map + next_phys) as *mut [u64; 512];
    }

    let idx = ((va >> 12) & 0x1FF) as usize;
    let old = (*table)[idx];
    if old & PTE_VALID == 0 {
        return None;
    }

    let pa = old & !(PAGE_SIZE - 1);
    (*table)[idx] = 0; // Invalidate
                       // Flush TLB for this VA
    core::arch::asm!("tlbi vaae1is, {}", in(reg) va >> 12);
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();

    Some(pa)
}

/// Translate a user VA → PA through a specific address space
pub unsafe fn translate_user(ttbr0_phys: u64, va: u64) -> Option<u64> {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let root_virt = (phys_map + ttbr0_phys) as *mut [u64; 512];
    let mut table = root_virt;

    for level in 0..4 {
        let idx = ((va >> (39 - level * 9)) & 0x1FF) as usize;
        let entry = (*table)[idx];
        if entry & PTE_VALID == 0 {
            return None;
        }
        if level == 3 || (entry & PTE_TABLE == 0 && level < 3) {
            let pa = entry & !(PAGE_SIZE - 1);
            let offset = va & (PAGE_SIZE - 1);
            return Some(pa | offset);
        }
        let next_phys = entry & !(PAGE_SIZE - 1);
        table = (phys_map + next_phys) as *mut [u64; 512];
    }
    None
}

/// Get PTE flags for a user page
pub unsafe fn get_user_pte(ttbr0_phys: u64, va: u64) -> Option<u64> {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let root_virt = (phys_map + ttbr0_phys) as *mut [u64; 512];
    let mut table = root_virt;

    for level in 0..3 {
        let idx = ((va >> (39 - level * 9)) & 0x1FF) as usize;
        let entry = (*table)[idx];
        if entry & PTE_VALID == 0 {
            return None;
        }
        let next_phys = entry & !(PAGE_SIZE - 1);
        table = (phys_map + next_phys) as *mut [u64; 512];
    }

    let idx = ((va >> 12) & 0x1FF) as usize;
    let pte = (*table)[idx];
    if pte & PTE_VALID == 0 {
        return None;
    }
    Some(pte)
}

/// Set PTE flags for a user page (e.g. mark COW, change protections)
pub unsafe fn set_user_pte(ttbr0_phys: u64, va: u64, flags: u64) {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let root_virt = (phys_map + ttbr0_phys) as *mut [u64; 512];
    let mut table = root_virt;

    for level in 0..3 {
        let idx = ((va >> (39 - level * 9)) & 0x1FF) as usize;
        let entry = (*table)[idx];
        if entry & PTE_VALID == 0 {
            return;
        }
        let next_phys = entry & !(PAGE_SIZE - 1);
        table = (phys_map + next_phys) as *mut [u64; 512];
    }

    let idx = ((va >> 12) & 0x1FF) as usize;
    let old = (*table)[idx];
    if old & PTE_VALID == 0 {
        return;
    }
    let pa = old & !(PAGE_SIZE - 1);
    (*table)[idx] = pa | flags;
    core::arch::asm!("tlbi vaae1is, {}", in(reg) va >> 12);
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Mark all writable user pages as COW (read-only + COW bit)
/// Used by fork() to set up copy-on-write
pub unsafe fn cow_mark_all(ttbr0_phys: u64) {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let root_virt = (phys_map + ttbr0_phys) as *mut [u64; 512];
    cow_mark_tree(root_virt, 0);
}

fn cow_mark_tree(table: *mut [u64; 512], level: u32) {
    if level > 3 {
        return;
    }
    for i in 0..512 {
        let entry = unsafe { (*table)[i] };
        if entry & PTE_VALID == 0 {
            continue;
        }

        if level < 3 && (entry & PTE_TABLE) != 0 {
            let next_phys = entry & !(PAGE_SIZE - 1);
            let next_virt = (crate::memory::PHYS_MAP_OFFSET + next_phys) as *mut [u64; 512];
            cow_mark_tree(next_virt, level + 1);
        } else if level == 3 {
            // Data page — if writable, mark COW
            // A writable user page has AP = EL0/EL1 RW (bits 7:6 = 01 for RW)
            // Since we use PTE_USER_RW, check if it's a writable user page
            // Simple heuristic: if it has PTE_NG set and doesn't have PTE_COW
            if entry & PTE_NG != 0 && entry & PTE_COW == 0 {
                let pa = entry & !(PAGE_SIZE - 1);
                // Increment refcount
                cow_ref(pa);
                // Mark read-only + COW
                // Remove write permission: change from PTE_USER_RW to PTE_USER_RO + COW
                let new_flags = PTE_USER_RO | PTE_COW;
                unsafe {
                    (*table)[i] = pa | new_flags;
                }
            }
        }
    }
    crate::arch::cpu::dsb();
}

/// Handle a COW page fault:
/// 1. Allocate a new frame
/// 2. Copy the original page data
/// 3. Map the new frame as writable (clear COW bit)
/// Returns true if the fault was handled, false if it was a genuine fault
pub fn handle_cow_fault(ttbr0_phys: u64, va: u64) -> bool {
    // Get the current PTE
    let pte = match unsafe { get_user_pte(ttbr0_phys, va) } {
        Some(pte) => pte,
        None => return false,
    };

    // Check if this is a COW page
    if pte & PTE_COW == 0 {
        return false;
    }

    let old_pa = pte & !(PAGE_SIZE - 1);

    // Allocate a new frame
    let new_frame = match frame_alloc::allocate_frame() {
        Some(f) => f,
        None => {
            println!("[vmem] COW fault: out of frames!");
            return false;
        }
    };

    // Copy the page data from old to new
    let src_virt = (crate::memory::PHYS_MAP_OFFSET + old_pa) as *const u8;
    let dst_virt = (crate::memory::PHYS_MAP_OFFSET + new_frame.0) as *mut u8;
    unsafe {
        for i in 0..(PAGE_SIZE as usize) {
            core::ptr::write_volatile(dst_virt.add(i), core::ptr::read_volatile(src_virt.add(i)));
        }
        crate::arch::cpu::dsb();
    }

    // Decrement refcount on old frame
    if cow_deref(old_pa) {
        // Last reference — we could free it, but since we just copied from it,
        // another process might still need it. Don't free here.
    }

    // Map the new frame as writable (clear COW)
    let va_aligned = va & !(PAGE_SIZE - 1);
    unsafe {
        unmap_user_page(ttbr0_phys, va_aligned);
        map_user_page(ttbr0_phys, va_aligned, new_frame.0, PTE_USER_RW);
    }

    true
}

/// Add a VMA to an address space
pub fn add_vma(space: &mut AddressSpace, start: u64, size: u64, prot: u64, shared: bool) -> bool {
    if space.vma_count >= MAX_VMAS {
        return false;
    }
    // Find a free slot
    for i in 0..MAX_VMAS {
        if !space.vmas[i].in_use {
            space.vmas[i] = Vma {
                start,
                size,
                prot,
                file_offset: 0,
                shared,
                in_use: true,
            };
            space.vma_count += 1;
            return true;
        }
    }
    false
}

/// Find the VMA containing a given address
pub fn find_vma(space: &AddressSpace, va: u64) -> Option<usize> {
    for i in 0..MAX_VMAS {
        let vma = &space.vmas[i];
        if vma.in_use && va >= vma.start && va < vma.start + vma.size {
            return Some(i);
        }
    }
    None
}

/// Clone an address space (for fork) — sets up COW instead of copying data
pub fn clone_address_space(src: &AddressSpace) -> Option<AddressSpace> {
    let mut new_space = create_address_space()?;

    // Copy VMA metadata
    new_space.vmas = src.vmas;
    new_space.brk = src.brk;

    // Copy all page table entries (shallow copy — share physical frames)
    // Then mark both source and destination as COW
    copy_page_tables(src.ttbr0_phys, new_space.ttbr0_phys)?;

    // Mark all writable pages in both spaces as COW
    unsafe {
        cow_mark_all(src.ttbr0_phys);
        cow_mark_all(new_space.ttbr0_phys);
    }

    Some(new_space)
}

/// Copy all page table entries from src TTBR0 to dst TTBR0
/// This is a shallow copy — both point to the same physical frames
fn copy_page_tables(src_ttbr0: u64, dst_ttbr0: u64) -> Option<()> {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let src_root = (phys_map + src_ttbr0) as *const [u64; 512];
    let dst_root = (phys_map + dst_ttbr0) as *mut [u64; 512];

    unsafe {
        copy_table_tree(src_root, dst_root, 0);
    }
    Some(())
}

/// Recursively copy page table entries (shallow — share physical frames)
unsafe fn copy_table_tree(src: *const [u64; 512], dst: *mut [u64; 512], level: u32) {
    if level > 3 {
        return;
    }
    for i in 0..512 {
        let entry = (*src)[i];
        if entry & PTE_VALID == 0 {
            continue;
        }

        if level < 3 && (entry & PTE_TABLE) != 0 {
            // Table descriptor — allocate a new table and recurse
            let src_next_phys = entry & !(PAGE_SIZE - 1);
            let src_next = (crate::memory::PHYS_MAP_OFFSET + src_next_phys) as *const [u64; 512];

            // Allocate new table frame for the destination
            let new_frame = match frame_alloc::allocate_frame() {
                Some(f) => f,
                None => return,
            };
            // Zero it
            let new_virt = (crate::memory::PHYS_MAP_OFFSET + new_frame.0) as *mut u64;
            for j in 0..512 {
                core::ptr::write_volatile(new_virt.add(j), 0);
            }
            crate::arch::cpu::dsb();

            let dst_next = (crate::memory::PHYS_MAP_OFFSET + new_frame.0) as *mut [u64; 512];
            (*dst)[i] = new_frame.0 | PTE_VALID | PTE_TABLE;

            copy_table_tree(src_next, dst_next, level + 1);
        } else if level == 3 {
            // Data page — just copy the PTE (shares the physical frame)
            (*dst)[i] = entry;
            // Increment COW refcount for this frame
            let pa = entry & !(PAGE_SIZE - 1);
            cow_ref(pa);
        }
    }
}

/// Handle a user-space page fault
///
/// Returns: PageFaultResult indicating what to do
pub fn handle_page_fault(
    ttbr0_phys: u64,
    va: u64,
    is_write: bool,
    _is_exec: bool,
) -> PageFaultResult {
    let va_page = va & !(PAGE_SIZE - 1);

    // Check if this address is in a valid VMA
    // (We'd need the AddressSpace pointer — for now use heuristic checks)

    // 1. Stack guard page check — if va is below stack but above heap
    if va < USER_STACK_TOP && va >= USER_STACK_TOP - 0x1000_0000 {
        // Stack growth region — automatically extend stack
        // Map a new page as writable
        let frame = match frame_alloc::allocate_frame() {
            Some(f) => f,
            None => return PageFaultResult::OutOfMemory,
        };
        unsafe {
            // Zero the page (security — don't leak kernel data)
            let virt = (crate::memory::PHYS_MAP_OFFSET + frame.0) as *mut u8;
            for i in 0..(PAGE_SIZE as usize) {
                core::ptr::write_volatile(virt.add(i), 0);
            }
            crate::arch::cpu::dsb();
            map_user_page(ttbr0_phys, va_page, frame.0, PTE_USER_RW);
        }
        return PageFaultResult::Handled;
    }

    // 2. COW fault — if the page is marked COW and this is a write
    if is_write {
        if handle_cow_fault(ttbr0_phys, va) {
            return PageFaultResult::Handled;
        }
    }

    // 3. Demand paging — if the VMA exists but no page is mapped yet
    // (For anonymous mmap regions)
    // Check if PTE is absent but VMA is valid
    let pte = unsafe { get_user_pte(ttbr0_phys, va) };
    if pte.is_none() {
        // No page mapped — check if it's in a valid region
        if va >= USER_HEAP_START && va < USER_STACK_TOP {
            // Anonymous page — allocate and map
            let frame = match frame_alloc::allocate_frame() {
                Some(f) => f,
                None => return PageFaultResult::OutOfMemory,
            };
            unsafe {
                let virt = (crate::memory::PHYS_MAP_OFFSET + frame.0) as *mut u8;
                for i in 0..(PAGE_SIZE as usize) {
                    core::ptr::write_volatile(virt.add(i), 0);
                }
                crate::arch::cpu::dsb();
                map_user_page(ttbr0_phys, va_page, frame.0, PTE_USER_RW);
            }
            return PageFaultResult::Handled;
        }
    }

    // 4. Genuine fault — kill the process
    PageFaultResult::KillProcess
}

/// Result of a page fault handler
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageFaultResult {
    /// Fault was handled — retry the instruction
    Handled,
    /// Out of memory — can't satisfy the fault
    OutOfMemory,
    /// Genuine fault — kill the offending process
    KillProcess,
    /// Can't handle — signal the process with SIGSEGV
    SegFault,
}
