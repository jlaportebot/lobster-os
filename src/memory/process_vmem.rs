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
use crate::memory::{PhysAddr, PAGE_SIZE};
use crate::println;

/// Re-export PTE flags
pub use super::mmu::{MT_NORMAL, PTE_ACCESS, PTE_ISH, PTE_NG, PTE_PAGE, PTE_TABLE, PTE_VALID};

/// AP[2:1] = 01: EL0 and EL1 read/write.
const AP_EL0_RW: u64 = 1 << 6;
/// AP[2:1] = 11: EL0 and EL1 read-only.
const AP_EL0_RO: u64 = 3 << 6;
/// AP[2] (bit 7): set on every read-only descriptor.
const AP_READ_ONLY_BIT: u64 = 1 << 7;
/// Both AP bits.
const AP_MASK: u64 = 3 << 6;

/// Descriptor bits every user page shares: valid page, non-global,
/// inner-shareable normal memory.
const PTE_USER_BASE: u64 = PTE_VALID | PTE_PAGE | PTE_ACCESS | PTE_NG | PTE_ISH | MT_NORMAL;

/// User read-only data: EL0 read, never executable at either level.
pub const PTE_USER_RO: u64 = PTE_USER_BASE | AP_EL0_RO | super::mmu::PTE_UXN | super::mmu::PTE_PXN;

/// User read-write data: never executable at either level.
pub const PTE_USER_RW: u64 = PTE_USER_BASE | AP_EL0_RW | super::mmu::PTE_UXN | super::mmu::PTE_PXN;

/// User read-execute code: read-only, EL0-executable, never EL1-executable.
pub const PTE_USER_RX: u64 = PTE_USER_BASE | AP_EL0_RO | super::mmu::PTE_PXN;

/// EL0 read/write/execute, privileged-never-execute. Only for ELF images whose
/// linker script emits one RWX PT_LOAD (MicroPython, early test programs):
/// splitting them is a toolchain change, so the loader keeps them working
/// while still denying EL1 execution of user pages.
pub const PTE_USER_RWX: u64 = PTE_USER_BASE | AP_EL0_RW | super::mmu::PTE_PXN;

/// COW flag — bit 55 (software-reserved; bit 52 is the hardware Contiguous hint)
pub const PTE_COW: u64 = 1 << 55;

/// Shared/refcounted flag — bit 56 (software-reserved; bit 53 is PXN)
pub const PTE_SHARED: u64 = 1 << 56;

/// Output-address mask of a descriptor.
const ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;

/// L1 slots (1 GiB each) that hold private user tables.
const USER_L1_SLOTS: usize = 2;

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

/// Allocate a new (empty) address space: a process root that already holds
/// the kernel's EL1-only descriptors (see `aspace`).
pub fn create_address_space() -> Option<AddressSpace> {
    let root = super::aspace::new_user_root()?;

    Some(AddressSpace {
        ttbr0_phys: root,
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

/// Destroy an address space: every user frame, user table and the root.
pub fn destroy_address_space(space: &AddressSpace) {
    super::aspace::free_user_root(space.ttbr0_phys);
}

/// Release the frame behind one valid user leaf descriptor, honouring COW
/// reference counts.
pub fn release_leaf(entry: u64) {
    let pa = entry & !(PAGE_SIZE - 1) & 0x0000_FFFF_FFFF_FFFF;
    if entry & PTE_COW != 0 && !cow_deref(pa) {
        return;
    }
    frame_alloc::deallocate_frame(PhysAddr::new(pa));
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

/// Call `f` with the physical address of every user L3 table of the tree
/// rooted at `root`. Only the two private L2 tables are walked, never the
/// kernel's copied descriptors.
unsafe fn for_each_user_l3(root: u64, mut f: impl FnMut(u64, usize, u64)) {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let l0 = (phys_map + root) as *const [u64; 512];
    let l1_entry = (*l0)[0];
    if l1_entry & PTE_TABLE == 0 {
        return;
    }
    let l1 = (phys_map + (l1_entry & ADDR_MASK)) as *const [u64; 512];
    for slot in 0..USER_L1_SLOTS {
        let l2_entry = (*l1)[slot];
        if l2_entry & PTE_TABLE == 0 {
            continue;
        }
        let l2_phys = l2_entry & ADDR_MASK;
        let l2 = (phys_map + l2_phys) as *const [u64; 512];
        for i in 0..512 {
            let e = (*l2)[i];
            if e & PTE_VALID != 0 && e & PTE_TABLE != 0 {
                f(l2_phys, i, e & ADDR_MASK);
            }
        }
    }
}

/// Mark all writable user pages as COW (read-only + COW bit)
/// Used by fork() to set up copy-on-write
pub unsafe fn cow_mark_all(ttbr0_phys: u64) {
    for_each_user_l3(ttbr0_phys, |_, _, l3| {
        cow_mark_leaves((crate::memory::PHYS_MAP_OFFSET + l3) as *mut [u64; 512]);
    });
}

/// Mark the writable leaves of one L3 table copy-on-write.
unsafe fn cow_mark_leaves(table: *mut [u64; 512]) {
    for i in 0..512 {
        let entry = (*table)[i];
        let writable = entry & AP_READ_ONLY_BIT == 0;
        if entry & PTE_VALID == 0 || entry & PTE_NG == 0 || entry & PTE_COW != 0 || !writable {
            continue;
        }
        cow_ref(entry & !(PAGE_SIZE - 1) & ADDR_MASK);
        (*table)[i] = (entry & !AP_MASK) | AP_EL0_RO | PTE_COW;
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

/// Copy every user leaf of `src_ttbr0` into `dst_ttbr0` (shallow: both share
/// the physical frames, COW refcounts are bumped).
fn copy_page_tables(src_ttbr0: u64, dst_ttbr0: u64) -> Option<()> {
    let mut ok = true;
    // SAFETY: both roots are live process roots owned by the caller.
    unsafe {
        for_each_user_l3(src_ttbr0, |l2_phys, idx, src_l3| {
            ok = ok && copy_l3(l2_phys, idx, src_l3, src_ttbr0, dst_ttbr0);
        });
    }
    ok.then_some(())
}

/// Copy one L3 table: find the matching destination L2 slot, give it a fresh
/// L3 and duplicate the leaves.
unsafe fn copy_l3(l2_phys: u64, idx: usize, src_l3: u64, src_root: u64, dst_root: u64) -> bool {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let Some(dst_l2) = matching_l2(l2_phys, src_root, dst_root) else {
        return false;
    };
    let Some(frame) = frame_alloc::allocate_frame() else {
        return false;
    };
    let dst = (phys_map + frame.0) as *mut [u64; 512];
    let src = (phys_map + src_l3) as *const [u64; 512];
    for i in 0..512 {
        let entry = (*src)[i];
        (*dst)[i] = entry;
        if entry & PTE_VALID != 0 {
            cow_ref(entry & !(PAGE_SIZE - 1) & ADDR_MASK);
        }
    }
    let l2 = (phys_map + dst_l2) as *mut [u64; 512];
    (*l2)[idx] = frame.0 | PTE_VALID | PTE_TABLE;
    crate::arch::cpu::dsb();
    true
}

/// The private L2 table of `dst_root` that sits at the same L1 slot as the
/// L2 table `src_l2` of `src_root`.
unsafe fn matching_l2(src_l2: u64, src_root: u64, dst_root: u64) -> Option<u64> {
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let l1_of = |root: u64| {
        let l0 = (phys_map + root) as *const [u64; 512];
        (phys_map + ((*l0)[0] & ADDR_MASK)) as *const [u64; 512]
    };
    let (src_l1, dst_l1) = (l1_of(src_root), l1_of(dst_root));
    (0..USER_L1_SLOTS)
        .find(|&s| (*src_l1)[s] & ADDR_MASK == src_l2)
        .map(|s| (*dst_l1)[s] & ADDR_MASK)
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
