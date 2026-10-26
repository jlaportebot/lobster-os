//! ELF loader for user-space processes
//!
//! Parses ELF64 binaries and loads them into a process's address space.
//! Supports static executables (ET_EXEC, ET_DYN) for AArch64.
//!
//! Memory layout for user processes:
//!   0x0000_0000_1000_0000  Code + data (loaded from ELF)
//!   0x0000_0000_2000_0000  Heap (grows up via brk/mmap)
//!   0x0000_0000_7F00_0000  User stack (grows down)
//!   0x0000_0000_8000_0000  End of user address space

use crate::memory::frame_alloc;
use crate::memory::{phys_to_virt, PhysAddr, PAGE_SIZE};
use crate::println;

/// User code region start (in TTBR0 space)
pub const USER_CODE_START: u64 = 0x0000_0000_1000_0000;

/// User heap start
pub const USER_HEAP_START: u64 = 0x0000_0000_2000_0000;

/// User stack start (grows downward from here)
pub const USER_STACK_TOP: u64 = 0x0000_0000_7F00_0000;
pub const USER_STACK_SIZE: u64 = 0x0010_0000; // 1MB

/// User address space limit
pub const USER_ADDR_LIMIT: u64 = 0x0000_0000_8000_0000;

/// ELF magic number
pub const ELF_MAGIC: u32 = 0x464C457F; // "\x7fELF"

/// ELF class (64-bit)
pub const ELFCLASS64: u8 = 2;

/// ELF data encoding (little-endian)
pub const ELFDATA2LSB: u8 = 1;

/// ELF machine type (AArch64)
pub const EM_AARCH64: u16 = 183;

/// ELF type
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;

/// ELF segment types
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;

/// ELF segment flags
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

/// ELF64 file header
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

/// ELF64 program header
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

/// Result of loading an ELF binary
#[derive(Debug)]
pub struct LoadResult {
    /// Entry point virtual address
    pub entry_point: u64,
    /// TTBR0 physical address (page table root)
    pub ttbr0_phys: u64,
    /// Highest loaded address (for brk base)
    pub brk_base: u64,
}

/// Check if an ELF binary has a PT_DYNAMIC segment (i.e., is dynamically linked).
pub fn is_dynamic(elf_data: &[u8]) -> bool {
    if elf_data.len() < core::mem::size_of::<Elf64Ehdr>() {
        return false;
    }
    let ehdr = unsafe { &*(elf_data.as_ptr() as *const Elf64Ehdr) };

    for i in 0..ehdr.e_phnum as usize {
        let phdr_off = ehdr.e_phoff as usize + i * ehdr.e_phentsize as usize;
        if phdr_off + core::mem::size_of::<Elf64Phdr>() > elf_data.len() {
            break;
        }
        let phdr = unsafe { &*(elf_data.as_ptr().add(phdr_off) as *const Elf64Phdr) };
        if phdr.p_type == PT_DYNAMIC {
            return true;
        }
    }
    false
}

/// Determine the load base for a loaded ELF.
///
/// For ET_EXEC (type 2), load_base = 0 (absolute addresses).
/// For ET_DYN / PIE (type 3), load_base = USER_CODE_START (relocated).
pub fn load_base_for(elf_data: &[u8]) -> u64 {
    if elf_data.len() < core::mem::size_of::<Elf64Ehdr>() {
        return 0;
    }
    let ehdr = unsafe { &*(elf_data.as_ptr() as *const Elf64Ehdr) };
    if ehdr.e_type == ET_DYN {
        USER_CODE_START
    } else {
        0
    }
}

/// Verify the ELF header is valid for AArch64
pub fn verify_header(ehdr: &Elf64Ehdr) -> bool {
    // Check magic
    let magic = u32::from_le_bytes([
        ehdr.e_ident[0],
        ehdr.e_ident[1],
        ehdr.e_ident[2],
        ehdr.e_ident[3],
    ]);
    if magic != ELF_MAGIC {
        println!("[elf] Bad magic: {:#010x}", magic);
        return false;
    }

    // Check class (64-bit)
    if ehdr.e_ident[4] != ELFCLASS64 {
        println!("[elf] Not 64-bit ELF (class={})", ehdr.e_ident[4]);
        return false;
    }

    // Check endianness
    if ehdr.e_ident[5] != ELFDATA2LSB {
        println!("[elf] Not little-endian (data={})", ehdr.e_ident[5]);
        return false;
    }

    // Check machine
    if ehdr.e_machine != EM_AARCH64 {
        println!("[elf] Not AArch64 (machine={})", ehdr.e_machine);
        return false;
    }

    // Check type
    if ehdr.e_type != ET_EXEC && ehdr.e_type != ET_DYN {
        println!("[elf] Not executable (type={})", ehdr.e_type);
        return false;
    }

    true
}

/// User-space page table entry flags
/// AP[1]=1 (bit 6) is REQUIRED for EL0 access — without it, only EL1 can access
const PTE_USER_CODE: u64 = crate::memory::vmem::PTE_VALID
    | crate::memory::vmem::PTE_PAGE
    | crate::memory::vmem::PTE_ACCESS
    | crate::memory::vmem::PTE_NG    // non-Global (process-specific)
    | crate::memory::vmem::PTE_ISH
    | crate::memory::mmu::MT_NORMAL
    | crate::memory::mmu::PTE_AP_EL0_RW; // EL0 read/write access

const PTE_USER_DATA: u64 = crate::memory::vmem::PTE_VALID
    | crate::memory::vmem::PTE_PAGE
    | crate::memory::vmem::PTE_ACCESS
    | crate::memory::vmem::PTE_NG
    | crate::memory::vmem::PTE_ISH
    | crate::memory::mmu::MT_NORMAL
    | crate::memory::mmu::PTE_AP_EL0_RW; // EL0 read/write access

const PTE_USER_RO: u64 = crate::memory::vmem::PTE_VALID
    | crate::memory::vmem::PTE_PAGE
    | crate::memory::vmem::PTE_ACCESS
    | crate::memory::vmem::PTE_NG
    | crate::memory::vmem::PTE_ISH
    | crate::memory::mmu::MT_NORMAL
    | crate::memory::mmu::PTE_AP_EL0_RW; // EL0 read/write access

/// Allocate a user-space page table root
fn alloc_user_page_table() -> Option<PhysAddr> {
    let frame = frame_alloc::allocate_frame()?;
    // Zero the page table
    let virt = phys_to_virt(PhysAddr::new(frame.0));
    unsafe {
        let table = virt.0 as *mut u64;
        for i in 0..512 {
            core::ptr::write_volatile(table.add(i), 0);
        }
    }
    Some(frame)
}

/// Map a page into a user process's address space
///
/// This walks/creates page tables under the process's TTBR0 root.
/// Since we're in kernel mode, we access page tables via the direct phys map.
pub unsafe fn map_user_page(ttbr0_phys: u64, va: u64, pa: u64, flags: u64) {
    // We need to walk the user page tables via physical addresses
    // mapped through our direct phys map.
    let phys_map = crate::memory::PHYS_MAP_OFFSET;

    // Get the root table
    let root_virt = (phys_map + ttbr0_phys) as *mut [u64; 512];
    let mut table = root_virt;

    for level in 0..3 {
        let idx = ((va >> (39 - level * 9)) & 0x1FF) as usize;
        let entry = (*table)[idx];

        if entry & crate::memory::vmem::PTE_VALID == 0 {
            // Allocate next-level table
            let next_frame = match frame_alloc::allocate_frame() {
                Some(f) => f,
                None => {
                    println!("[elf] Out of frames for user page tables!");
                    return;
                }
            };
            // Zero the new table
            let next_virt = (phys_map + next_frame.0) as *mut u64;
            for i in 0..512 {
                core::ptr::write_volatile(next_virt.add(i), 0);
            }
            crate::arch::cpu::dsb();

            (*table)[idx] =
                next_frame.0 | crate::memory::vmem::PTE_VALID | crate::memory::vmem::PTE_PAGE; // Table descriptor

            table = (phys_map + next_frame.0) as *mut [u64; 512];
        } else if entry & crate::memory::mmu::PTE_TABLE != 0 {
            let next_phys = entry & !(PAGE_SIZE - 1);
            table = (phys_map + next_phys) as *mut [u64; 512];
        } else {
            // Block entry at L0/L1/L2 — split it into L(n+1) pages
            // This preserves the existing mapping while allowing finer granularity
            let _block_size = 1u64 << (30 - level * 9); // L0=1GB, L1=2MB, L2=2MB wait...
                                                        // Actually: L0 table→L1, L1 table→L2, L2 table→L3
                                                        // Block sizes: L1 block = 1GB, L2 block = 2MB
            let _block_size_shift = 30 - level * 9; // L0→1GB(30), L1→2MB(21), L2→4KB(12)?? No.
                                                    // For 4KB granule: L0→L1(512GB table), L1→L2(1GB block/table), L2→L3(2MB block/table)
                                                    // Block sizes: L1 block = 1GB (bits 30..), L2 block = 2MB (bits 21..)
            let bs = if level == 0 { 1u64 << 30 } else { 1u64 << 21 }; // L0 children are 1GB blocks, L1 children are 2MB blocks

            // Get the physical base and flags from the block entry
            let block_phys = entry & !0xFFF;
            let block_flags = entry & 0x3FF;

            // Allocate a new L(n+1) table
            let next_frame = match frame_alloc::allocate_frame() {
                Some(f) => f,
                None => {
                    println!("[elf] Out of frames for block split!");
                    return;
                }
            };
            let next_virt = (phys_map + next_frame.0) as *mut u64;
            // Fill the new table with entries that replicate the block mapping.
            // CRITICAL: L3 page descriptors need PTE_PAGE (bit 1) = 1,
            // but block descriptors have bit 1 = 0. Force it on.
            let child_flags = if level == 2 {
                block_flags | crate::memory::vmem::PTE_PAGE // L3 needs page descriptor
            } else {
                block_flags // L1/L2 children are blocks, keep bit 1 = 0
            };
            for i in 0..512 {
                let child_pa = block_phys + (i as u64) * (bs / 512);
                core::ptr::write_volatile(next_virt.add(i), child_pa | child_flags);
            }
            crate::arch::cpu::dsb();

            // Replace the block entry with a table entry
            (*table)[idx] =
                next_frame.0 | crate::memory::vmem::PTE_VALID | crate::memory::vmem::PTE_PAGE;
            table = (phys_map + next_frame.0) as *mut [u64; 512];

            // CRITICAL: Invalidate TLB for the VA range we just changed.
            // Without this, stale TLB entries for the old block mapping
            // cause permission faults on the new page entries.
            let tlb_va = va & !((1u64 << (30 - level * 9)) - 1);
            unsafe {
                core::arch::asm!(
                    "tlbi vaae1is, {}",
                    in(reg) tlb_va >> 12,
                );
            }
            crate::arch::cpu::dsb();
            crate::arch::cpu::isb();
        }
    }

    // Level 3 — write the page entry
    let idx = ((va >> 12) & 0x1FF) as usize;
    (*table)[idx] = pa | flags;
    crate::arch::cpu::dsb();

    // Invalidate TLB for this VA so the new page entry takes effect
    unsafe {
        core::arch::asm!("tlbi vaae1is, {}", in(reg) va >> 12);
    }
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Load an ELF binary from a byte slice into a new address space
///
/// Returns the entry point and TTBR0 for the new process.
pub fn load_elf(elf_data: &[u8]) -> Result<LoadResult, &'static str> {
    if elf_data.len() < core::mem::size_of::<Elf64Ehdr>() {
        return Err("ELF data too small for header");
    }

    // Parse the ELF header
    let ehdr = unsafe { &*(elf_data.as_ptr() as *const Elf64Ehdr) };

    if !verify_header(ehdr) {
        return Err("Invalid ELF header");
    }

    println!("[elf] Valid ELF64 AArch64 executable");
    println!("[elf] Entry point: {:#018x}", ehdr.e_entry);
    println!(
        "[elf] Program headers: {} (offset={:#x}, size={})",
        ehdr.e_phnum, ehdr.e_phoff, ehdr.e_phentsize
    );

    // Allocate a user page table
    let ttbr0 = alloc_user_page_table().ok_or("Failed to allocate user page table")?;
    println!("[elf] User TTBR0: {:#010x}", ttbr0.0);

    // Also read the kernel's TTBR0 so we can dual-map user pages
    let kernel_ttbr0: u64;
    unsafe {
        core::arch::asm!("mrs {}, TTBR0_EL1", out(reg) kernel_ttbr0);
    }
    println!("[elf] Kernel TTBR0: {:#010x}", kernel_ttbr0);

    let mut highest_addr: u64 = 0;

    // Load each PT_LOAD segment
    for i in 0..ehdr.e_phnum as usize {
        let phdr_offset = ehdr.e_phoff as usize + i * ehdr.e_phentsize as usize;
        if phdr_offset + core::mem::size_of::<Elf64Phdr>() > elf_data.len() {
            println!("[elf] Program header {} out of bounds", i);
            continue;
        }

        let phdr = unsafe { &*(elf_data.as_ptr().add(phdr_offset) as *const Elf64Phdr) };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        println!(
            "[elf] LOAD segment {}: vaddr={:#018x} filesz={:#x} memsz={:#x} flags={:#x}",
            i, phdr.p_vaddr, phdr.p_filesz, phdr.p_memsz, phdr.p_flags
        );

        if phdr.p_vaddr < USER_CODE_START || phdr.p_vaddr >= USER_ADDR_LIMIT {
            println!("[elf] Segment VA {:#018x} out of user range!", phdr.p_vaddr);
            return Err("Segment VA out of user address range");
        }

        // Determine page flags
        let flags = if phdr.p_flags & PF_X != 0 {
            PTE_USER_CODE
        } else if phdr.p_flags & PF_W != 0 {
            PTE_USER_DATA
        } else {
            PTE_USER_RO
        };

        // Map and populate the segment
        let segment_end = phdr.p_vaddr + phdr.p_memsz;
        let va_start = phdr.p_vaddr & !(PAGE_SIZE - 1);
        let va_end = (segment_end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        let mut va = va_start;
        let mut file_offset = phdr.p_offset;

        while va < va_end {
            // Allocate a physical frame
            let frame = frame_alloc::allocate_frame().ok_or("Out of frames for ELF loading")?;

            // Map the page in user space
            unsafe {
                map_user_page(ttbr0.0, va, frame.0, flags);
                // Also map into kernel TTBR0 so eret into EL0 works.
                // Clear nG bit for kernel mapping: the kernel runs with ASID 0
                // and nG=1 entries are ASID-tagged, so they'd be invisible to ASID 0.
                let kernel_flags = flags & !crate::memory::vmem::PTE_NG;
                map_user_page(kernel_ttbr0, va, frame.0, kernel_flags);
            }

            // Copy data from ELF file into the page
            let page_virt = phys_to_virt(PhysAddr::new(frame.0)).0;
            let page_ptr = page_virt as *mut u8;

            // Zero the page first (for BSS)
            unsafe {
                for b in 0..PAGE_SIZE as usize {
                    core::ptr::write_volatile(page_ptr.add(b), 0);
                }
            }

            // Copy file data into this page.
            // NOTE: when p_vaddr is not page-aligned, `va` starts BELOW
            // p_vaddr — handle with signed offsets so the first page
            // copies from the correct file offset (and unaligned vaddr
            // segments load correctly).
            let vaddr_delta = (va as i64) - (phdr.p_vaddr as i64);
            let seg_start_in_page = if vaddr_delta >= 0 {
                // Page fully inside the segment — copy to page start.
                0
            } else {
                // First page holds the segment start at (p_vaddr - va).
                (phdr.p_vaddr - va) as usize
            };
            let page_offset = if vaddr_delta >= 0 {
                vaddr_delta as usize
            } else {
                0
            };
            let copy_start = file_offset as usize;
            let copy_in_page = if page_offset < phdr.p_filesz as usize {
                let remaining = phdr.p_filesz as usize - page_offset;
                let avail = (PAGE_SIZE as usize).saturating_sub(seg_start_in_page);
                let n = remaining.min(avail);
                if copy_start + n <= elf_data.len() {
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            elf_data.as_ptr().add(copy_start),
                            page_ptr.add(seg_start_in_page),
                            n,
                        );
                    }
                    n
                } else {
                    0
                }
            } else {
                0
            };

            file_offset += copy_in_page as u64;
            va += PAGE_SIZE;
        }

        if segment_end > highest_addr {
            highest_addr = segment_end;
        }
    }

    // Map the user stack
    let stack_base = USER_STACK_TOP - USER_STACK_SIZE;
    for page_off in (0..USER_STACK_SIZE).step_by(PAGE_SIZE as usize) {
        let va = stack_base + page_off as u64;
        let frame = frame_alloc::allocate_frame().ok_or("Out of frames for user stack")?;
        unsafe {
            map_user_page(ttbr0.0, va, frame.0, PTE_USER_DATA);
            // Also map stack into kernel TTBR0 with nG cleared for kernel ASID 0
            let kernel_flags = PTE_USER_DATA & !crate::memory::vmem::PTE_NG;
            map_user_page(kernel_ttbr0, va, frame.0, kernel_flags);
        }
    }
    println!(
        "[elf] User stack mapped: {:#018x}..{:#018x}",
        stack_base, USER_STACK_TOP
    );

    // Align brk_base to page boundary
    let brk_base = (highest_addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

    println!(
        "[elf] Load complete! entry={:#018x} ttbr0={:#010x} brk={:#018x}",
        ehdr.e_entry, ttbr0.0, brk_base
    );

    Ok(LoadResult {
        entry_point: ehdr.e_entry,
        ttbr0_phys: ttbr0.0,
        brk_base,
    })
}

/// Build a simple "hello world" user program as a static byte array
///
/// This creates a tiny ELF binary that writes "Hello from EL0!\n" to UART
/// via the write syscall. Used for testing EL0 transitions without
/// needing an external toolchain or heap allocation.
pub fn build_hello_elf() -> &'static [u8] {
    // Minimal AArch64 code:
    //   mov x8, #1      (SYS_WRITE)
    //   mov x0, #1      (stdout)
    //   adr x1, msg     (buffer pointer)
    //   mov x2, #16     (length)
    //   svc #0          (syscall)
    //   mov x8, #0      (SYS_EXIT)
    //   mov x0, #0      (exit code 0)
    //   svc #0          (syscall)
    // msg: .ascii "Hello from EL0!\n\0"
    //
    // We pre-construct a minimal ELF binary as a static array.
    // This avoids needing alloc::vec::Vec in a no_std kernel.

    // Hand-crafted ELF64 header + program header + AArch64 code + "Hello from EL0!\n"
    // Total size: 64 (ehdr) + 56 (phdr) + 32 (code) + 17 (msg) = 169 bytes
    static ELF: &[u8] = &[
        // === ELF Header (64 bytes) ===
        0x7f, 0x45, 0x4c, 0x46, // e_ident[0..4]: magic
        0x02, 0x01, 0x01, 0x00, // class=64, data=LE, version=1, osabi=0
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // padding
        0x02, 0x00, // e_type = ET_EXEC
        0xb7, 0x00, // e_machine = EM_AARCH64 (183)
        0x01, 0x00, 0x00, 0x00, // e_version = 1
        0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, // e_entry = 0x10000000
        0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // e_phoff = 64
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // e_shoff = 0
        0x00, 0x00, 0x00, 0x00, // e_flags = 0
        0x40, 0x00, // e_ehsize = 64
        0x38, 0x00, // e_phentsize = 56
        0x01, 0x00, // e_phnum = 1
        0x00, 0x00, // e_shentsize = 0
        0x00, 0x00, // e_shnum = 0
        0x00, 0x00, // e_shstrndx = 0
        // === Program Header (56 bytes) ===
        0x01, 0x00, 0x00, 0x00, // p_type = PT_LOAD
        0x05, 0x00, 0x00, 0x00, // p_flags = PF_R | PF_X
        0x78, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_offset = 120
        0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, // p_vaddr = 0x10000000
        0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, // p_paddr = 0x10000000
        0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_filesz = 49
        0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_memsz = 49
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // p_align = 0x10000
        // === Code (32 bytes = 8 instructions) ===
        0x28, 0x00, 0x80, 0xD2, // mov x8, #1
        0x20, 0x00, 0x80, 0xD2, // mov x0, #1
        0xC1, 0x00, 0x00, 0x10, // adr x1, +24 (address of msg at 0x10000020)
        0x02, 0x02, 0x80, 0xD2, // mov x2, #16
        0x01, 0x00, 0x00, 0xD4, // svc #0
        0x08, 0x00, 0x80, 0xD2, // mov x8, #0
        0x00, 0x00, 0x80, 0xD2, // mov x0, #0
        0x01, 0x00, 0x00, 0xD4, // svc #0
        // === Message (17 bytes) ===
        0x48, 0x65, 0x6c, 0x6c, 0x6f, 0x20, 0x66, 0x72, // "Hello fr"
        0x6f, 0x6d, 0x20, 0x45, 0x4c, 0x30, 0x21, 0x0a, // "om EL0!\n"
        0x00, // null terminator
    ];

    ELF
}

/// Build a hello-world ELF binary into a provided mutable slice
/// Returns the number of bytes written.
pub fn build_hello_elf_into(buf: &mut [u8]) -> usize {
    // Code: AArch64 instructions
    let code: [u32; 8] = [
        0xD2800028, // mov x8, #1       (SYS_WRITE)
        0xD2800020, // mov x0, #1       (stdout fd)
        0x100000C1, // adr x1, msg     (PC+24 → address of string at 0x10000020)
        0xD2800202, // mov x2, #16      (length)
        0xD4000001, // svc #0
        0xD2800008, // mov x8, #0       (SYS_EXIT)
        0xD2800000, // mov x0, #0       (exit(0))
        0xD4000001, // svc #0
    ];

    let msg = b"Hello from EL0!\n\0";

    let ehdr_size = core::mem::size_of::<Elf64Ehdr>();
    let phdr_size = core::mem::size_of::<Elf64Phdr>();
    let load_offset = ehdr_size + phdr_size;
    let code_size = code.len() * 4;
    let total = load_offset + code_size + msg.len();

    if buf.len() < total {
        return 0;
    }

    // Build ELF header
    let mut ehdr = Elf64Ehdr {
        e_ident: [0; 16],
        e_type: ET_EXEC,
        e_machine: EM_AARCH64,
        e_version: 1,
        e_entry: USER_CODE_START,
        e_phoff: ehdr_size as u64,
        e_shoff: 0,
        e_flags: 0,
        e_ehsize: ehdr_size as u16,
        e_phentsize: phdr_size as u16,
        e_phnum: 1,
        e_shentsize: 0,
        e_shnum: 0,
        e_shstrndx: 0,
    };
    ehdr.e_ident[0] = 0x7f;
    ehdr.e_ident[1] = b'E';
    ehdr.e_ident[2] = b'L';
    ehdr.e_ident[3] = b'F';
    ehdr.e_ident[4] = ELFCLASS64;
    ehdr.e_ident[5] = ELFDATA2LSB;
    ehdr.e_ident[6] = 1;

    // Program header
    let phdr = Elf64Phdr {
        p_type: PT_LOAD,
        p_flags: PF_R | PF_X,
        p_offset: load_offset as u64,
        p_vaddr: USER_CODE_START,
        p_paddr: USER_CODE_START,
        p_filesz: (code_size + msg.len()) as u64,
        p_memsz: (code_size + msg.len()) as u64,
        p_align: PAGE_SIZE,
    };

    // Write ELF header
    unsafe {
        let ehdr_bytes =
            core::slice::from_raw_parts(&ehdr as *const Elf64Ehdr as *const u8, ehdr_size);
        buf[..ehdr_size].copy_from_slice(ehdr_bytes);
    }

    // Write program header
    unsafe {
        let phdr_bytes =
            core::slice::from_raw_parts(&phdr as *const Elf64Phdr as *const u8, phdr_size);
        buf[ehdr_size..load_offset].copy_from_slice(phdr_bytes);
    }

    // Write code
    let code_bytes = unsafe { core::slice::from_raw_parts(code.as_ptr() as *const u8, code_size) };
    buf[load_offset..load_offset + code_size].copy_from_slice(code_bytes);

    // Write message
    buf[load_offset + code_size..total].copy_from_slice(msg);

    total
}

/// Mirror user page mappings into the kernel's TTBR0.
///
/// Walks the user page table (3 levels: L0→L1→L2) and copies leaf entries
/// (L2 block or L3 page mappings) into the kernel's TTBR0 at the same VAs.
/// This allows eret into EL0 to access user code/stack without switching TTBR0.
pub fn mirror_user_mappings(kernel_ttbr0: u64, user_ttbr0: u64) {
    crate::drivers::uart::Uart10::puts("[elf] mirror: start\r\n");
    let phys_map = crate::memory::PHYS_MAP_OFFSET;
    let _pte_user_ro = PTE_USER_RO;
    let _pte_user_code = PTE_USER_CODE;
    let _pte_user_data = PTE_USER_DATA;

    // Walk L0 (512 entries)
    let l0_user = unsafe { &*((phys_map + user_ttbr0) as *const [u64; 512]) };
    for i in 0..512 {
        let l0e = l0_user[i];
        if l0e & crate::memory::vmem::PTE_VALID == 0 {
            continue;
        }
        // L0 entry points to L1 table
        let l1_phys = l0e & !0xFFF;
        let l1_user = unsafe { &*((phys_map + l1_phys) as *const [u64; 512]) };
        for j in 0..512 {
            let l1e = l1_user[j];
            if l1e & crate::memory::vmem::PTE_VALID == 0 {
                continue;
            }
            // Check if L1 is a block (1GB block)
            if l1e & 0x2 != 0 {
                // L1 block — copy directly (unlikely for user pages)
                unsafe {
                    map_user_page(
                        kernel_ttbr0,
                        (i as u64) << 39 | (j as u64) << 30,
                        l1e & !0xFFF,
                        l1e & 0x3FF,
                    );
                }
                continue;
            }
            // L1 entry points to L2 table
            let l2_phys = l1e & !0xFFF;
            let l2_user = unsafe { &*((phys_map + l2_phys) as *const [u64; 512]) };
            for k in 0..512 {
                let l2e = l2_user[k];
                if l2e & crate::memory::vmem::PTE_VALID == 0 {
                    continue;
                }
                let va = (i as u64) << 39 | (j as u64) << 30 | (k as u64) << 21;
                if l2e & 0x2 != 0 {
                    // L2 block (2MB) — copy as block
                    let flags = l2e & 0x3FF;
                    unsafe {
                        map_user_page(kernel_ttbr0, va, l2e & !0xFFF, flags);
                    }
                } else {
                    // L2 entry points to L3 table — walk L3
                    let l3_phys = l2e & !0xFFF;
                    let l3_user = unsafe { &*((phys_map + l3_phys) as *const [u64; 512]) };
                    for m in 0..512 {
                        let l3e = l3_user[m];
                        if l3e & crate::memory::vmem::PTE_VALID == 0 {
                            continue;
                        }
                        let page_va = va | (m as u64) << 12;
                        let page_flags = l3e & 0x3FF;
                        unsafe {
                            map_user_page(kernel_ttbr0, page_va, l3e & !0xFFF, page_flags);
                        }
                    }
                }
            }
        }
    }
}
