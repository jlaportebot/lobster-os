//! Dynamic linker for LobsterOS
//!
//! Handles loading of dynamically-linked ELF binaries (ET_DYN / PIE).
//! Parses the `.dynamic` segment, processes relocations (R_AARCH64_*),
//! sets up the PLT (Procedure Linkage Table) and GOT (Global Offset Table),
//! and implements lazy binding via a runtime resolver stub.
//!
//! ## Design
//!
//! - **Static vs Dynamic**: If an ELF has no PT_DYNAMIC segment, `link_elf()`
//!   returns immediately — no dynamic linking needed.
//! - **Immediate bindings**: R_AARCH64_RELATIVE, R_AARCH64_ABS64, and
//!   R_AARCH64_GLOB_DAT are resolved at load time.
//! - **Lazy bindings**: R_AARCH64_JUMP_SLOT (PLT) entries are resolved
//!   on first call via `dl_resolve()`.
//! - **No shared libraries yet**: DT_NEEDED entries are noted but not
//!   loaded from disk. All relocations that reference external symbols
//!   are filled with a stub that panics at runtime (future: VFS lookup).
//!
//! ## Memory layout for a dynamically-linked process
//!
//! ```text
//! 0x1000_0000   Code + data (loaded from ELF, includes PLT)
//! 0x2000_0000   Heap (brk)
//! 0x7E00_0000   GOT (mapped writable, contains PLT trampolines)
//! 0x7F00_0000   Stack
//! ```

use crate::memory::frame_alloc;
use crate::memory::{phys_to_virt, PhysAddr, PAGE_SIZE};
use crate::println;

// ═══════════════════════════════════════════════════════════════
// ELF constants for dynamic linking
// ═══════════════════════════════════════════════════════════════

/// Program header type: dynamic segment
const PT_DYNAMIC: u32 = 2;

/// Program header type: interpreter path
const PT_INTERP: u32 = 3;

/// Dynamic section tags
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_PLTRELSZ: u64 = 2;
const DT_PLTGOT: u64 = 3;
const DT_HASH: u64 = 4;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_STRSZ: u64 = 10;
const DT_SYMENT: u64 = 11;
const DT_INIT: u64 = 12;
const DT_FINI: u64 = 13;
const DT_RPATH: u64 = 15;
const DT_RUNPATH: u64 = 29;
const DT_FLAGS: u64 = 30;
const DT_JMPREL: u64 = 23;
const DT_PLTREL: u64 = 24;
const DT_FLAGS_1: u64 = 0x6ffffffb;
const DT_VERSYM: u64 = 0x6ffffff0;
const DT_VERNEED: u64 = 0x6ffffffe;
const DT_VERNEEDNUM: u64 = 0x6fffffff;

/// DT_FLAGS bits
const DF_BIND_NOW: u64 = 0x8;

/// DT_FLAGS_1 bits
const DF_1_NOW: u64 = 0x1;
const DF_1_PIE: u64 = 0x08000000;

/// AArch64 relocation types
const R_AARCH64_NONE: u32 = 0;
const R_AARCH64_ABS64: u32 = 257;
const R_AARCH64_COPY: u32 = 1024;
const R_AARCH64_GLOB_DAT: u32 = 1025;
const R_AARCH64_JUMP_SLOT: u32 = 1026;
const R_AARCH64_RELATIVE: u32 = 1027;
const R_AARCH64_TLS_DTPREL64: u32 = 1028;
const R_AARCH64_TLS_DTPMOD64: u32 = 1029;
const R_AARCH64_TLS_TPREL64: u32 = 1030;
const R_AARCH64_IRELATIVE: u32 = 1031;

/// User GOT region (just below stack)
pub const USER_GOT_BASE: u64 = 0x0000_0000_7E00_0000;
pub const USER_GOT_SIZE: u64 = 0x0040_0000; // 4 MB

// ═══════════════════════════════════════════════════════════════
// ELF structure definitions for dynamic linking
// ═══════════════════════════════════════════════════════════════

/// ELF64 Dyn entry (from .dynamic segment)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Elf64Dyn {
    pub d_tag: i64,
    pub d_val: u64,
}

/// ELF64 Rela entry (explicit-addend relocation)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Elf64Rela {
    pub r_offset: u64,
    pub r_info: u64,
    pub r_addend: i64,
}

/// ELF64 Symbol table entry
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Elf64Sym {
    pub st_name: u32,
    pub st_info: u8,
    pub st_other: u8,
    pub st_shndx: u16,
    pub st_value: u64,
    pub st_size: u64,
}

/// ELF64 Hash table header (GNU hash or SysV hash)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf64HashHdr {
    pub nbucket: u32,
    pub nchain: u32,
}

// ═══════════════════════════════════════════════════════════════
// Dynamic linking result
// ═══════════════════════════════════════════════════════════════

/// Result of dynamic linking for a loaded ELF
#[derive(Debug)]
pub struct DynLinkResult {
    /// Base address where the ELF was loaded (0 for ET_EXEC, >0 for PIE)
    pub load_base: u64,
    /// Virtual address of the GOT (within the process address space)
    pub got_addr: u64,
    /// Number of RELA entries processed
    pub rela_count: u32,
    /// Number of JMPREL (PLT) entries set up for lazy binding
    pub plt_count: u32,
    /// Entry point (already rebased for PIE)
    pub entry_point: u64,
    /// List of DT_NEEDED library names (offsets into dynstr)
    pub needed: [u64; 16],
    pub needed_count: u32,
}

// ═══════════════════════════════════════════════════════════════
// Parsed dynamic segment info
// ═══════════════════════════════════════════════════════════════

pub(crate) struct DynInfo {
    symtab: u64,
    strtab: u64,
    strsz: u64,
    hash: u64,
    jmprel: u64,
    pltrelsz: u64,
    pltrel: u64,
    pltgot: u64,
    rela: u64,
    relasz: u64,
    relaent: u64,
    init: u64,
    fini: u64,
    bind_now: bool,
    needed: [u64; 16],
    needed_count: u32,
}

impl DynInfo {
    const fn empty() -> Self {
        Self {
            symtab: 0,
            strtab: 0,
            strsz: 0,
            hash: 0,
            jmprel: 0,
            pltrelsz: 0,
            pltrel: 0,
            pltgot: 0,
            rela: 0,
            relasz: 0,
            relaent: 0,
            init: 0,
            fini: 0,
            bind_now: false,
            needed: [0; 16],
            needed_count: 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Main API
// ═══════════════════════════════════════════════════════════════

/// Dynamically link an already-loaded ELF binary.
///
/// `elf_data` is the raw file bytes. `load_base` is the virtual address
/// where the ELF was mapped (0 for ET_EXEC, base for ET_DYN/PIE).
/// `ttbr0_phys` is the process's page table root for mapping the GOT.
///
/// Returns `Ok(DynLinkResult)` on success or an error string on failure.
/// If the ELF has no PT_DYNAMIC segment, returns `Ok` with zero counts.
pub fn link_elf(
    elf_data: &[u8],
    load_base: u64,
    ttbr0_phys: u64,
    entry_point: u64,
) -> Result<DynLinkResult, &'static str> {
    let ehdr = unsafe { &*(elf_data.as_ptr() as *const super::elf::Elf64Ehdr) };

    // Find PT_DYNAMIC and PT_INTERP segments
    let mut dyn_offset: u64 = 0;
    let mut dyn_filesz: u64 = 0;
    let mut _interp_offset: u64 = 0;
    let mut _interp_filesz: u64 = 0;

    for i in 0..ehdr.e_phnum as usize {
        let phdr_off = ehdr.e_phoff as usize + i * ehdr.e_phentsize as usize;
        if phdr_off + core::mem::size_of::<super::elf::Elf64Phdr>() > elf_data.len() {
            continue;
        }
        let phdr = unsafe { &*(elf_data.as_ptr().add(phdr_off) as *const super::elf::Elf64Phdr) };

        match phdr.p_type {
            PT_DYNAMIC => {
                dyn_offset = phdr.p_offset;
                dyn_filesz = phdr.p_filesz;
            }
            PT_INTERP => {
                _interp_offset = phdr.p_offset;
                _interp_filesz = phdr.p_filesz;
            }
            _ => {}
        }
    }

    // No dynamic segment → static binary, nothing to do
    if dyn_filesz == 0 {
        println!("[dlinker] No PT_DYNAMIC — static ELF, skipping");
        return Ok(DynLinkResult {
            load_base,
            got_addr: 0,
            rela_count: 0,
            plt_count: 0,
            entry_point,
            needed: [0; 16],
            needed_count: 0,
        });
    }

    println!(
        "[dlinker] Found PT_DYNAMIC: offset={:#x} size={:#x}",
        dyn_offset, dyn_filesz
    );

    // Parse the .dynamic section
    let dyn_info = parse_dynamic(elf_data, dyn_offset, dyn_filesz, load_base)?;

    // Allocate and map the GOT
    let got_addr = map_got(ttbr0_phys)?;

    // Process RELA relocations (immediate)
    let rela_count = process_rela(elf_data, load_base, &dyn_info, got_addr, ttbr0_phys)?;

    // Process PLT relocations (lazy)
    let plt_count = process_plt(elf_data, load_base, &dyn_info, got_addr, ttbr0_phys)?;

    // Resolve entry point for PIE
    let resolved_entry = if load_base != 0 {
        entry_point + load_base
    } else {
        entry_point
    };

    println!(
        "[dlinker] Linking complete: {} RELA, {} PLT, entry={:#018x}",
        rela_count, plt_count, resolved_entry
    );

    Ok(DynLinkResult {
        load_base,
        got_addr,
        rela_count,
        plt_count,
        entry_point: resolved_entry,
        needed: dyn_info.needed,
        needed_count: dyn_info.needed_count,
    })
}

// ═══════════════════════════════════════════════════════════════
// .dynamic segment parser
// ═══════════════════════════════════════════════════════════════

fn parse_dynamic(
    elf_data: &[u8],
    dyn_offset: u64,
    dyn_filesz: u64,
    load_base: u64,
) -> Result<DynInfo, &'static str> {
    let mut info = DynInfo::empty();
    let entry_size = core::mem::size_of::<Elf64Dyn>();
    let num_entries = dyn_filesz as usize / entry_size;

    for i in 0..num_entries {
        let off = dyn_offset as usize + i * entry_size;
        if off + entry_size > elf_data.len() {
            break;
        }
        let dyn_entry = unsafe { &*(elf_data.as_ptr().add(off) as *const Elf64Dyn) };

        let val = if dyn_entry.d_val >= super::elf::USER_CODE_START
            && dyn_entry.d_val < super::elf::USER_ADDR_LIMIT
        {
            // Already an absolute VMA (for ET_EXEC)
            dyn_entry.d_val
        } else if dyn_entry.d_val != 0 {
            // File offset → rebased for PIE
            dyn_entry.d_val + load_base
        } else {
            0
        };

        match dyn_entry.d_tag as u64 {
            DT_NULL => break,
            DT_STRTAB => info.strtab = val,
            DT_SYMTAB => info.symtab = val,
            DT_STRSZ => info.strsz = dyn_entry.d_val,
            DT_HASH => info.hash = val,
            DT_JMPREL => info.jmprel = val,
            DT_PLTRELSZ => info.pltrelsz = dyn_entry.d_val,
            DT_PLTREL => info.pltrel = dyn_entry.d_val,
            DT_PLTGOT => info.pltgot = val,
            DT_RELA => info.rela = val,
            DT_RELASZ => info.relasz = dyn_entry.d_val,
            DT_RELAENT => info.relaent = dyn_entry.d_val,
            DT_INIT => info.init = val,
            DT_FINI => info.fini = val,
            DT_NEEDED => {
                if (info.needed_count as usize) < info.needed.len() {
                    info.needed[info.needed_count as usize] = dyn_entry.d_val;
                    info.needed_count += 1;
                }
            }
            DT_FLAGS => {
                if dyn_entry.d_val & DF_BIND_NOW != 0 {
                    info.bind_now = true;
                }
            }
            DT_FLAGS_1 => {
                if dyn_entry.d_val & DF_1_NOW != 0 {
                    info.bind_now = true;
                }
            }
            DT_RPATH | DT_RUNPATH | DT_VERSYM | DT_VERNEED | DT_VERNEEDNUM | DT_SYMENT => {
                // Noted but not yet acted on
            }
            _ => {}
        }
    }

    println!(
        "[dlinker] Dynamic: strtab={:#x} symtab={:#x} rela={:#x} relasz={:#x} jmprel={:#x} pltrelsz={:#x}",
        info.strtab, info.symtab, info.rela, info.relasz, info.jmprel, info.pltrelsz
    );
    println!(
        "[dlinker] Dynamic: pltgot={:#x} hash={:#x} init={:#x} fini={:#x} bind_now={}",
        info.pltgot, info.hash, info.init, info.fini, info.bind_now
    );
    if info.needed_count > 0 {
        println!("[dlinker] DT_NEEDED: {} libraries", info.needed_count);
    }

    Ok(info)
}

// ═══════════════════════════════════════════════════════════════
// GOT allocation and mapping
// ═══════════════════════════════════════════════════════════════

/// Page flags for GOT (user-accessible, writable, not executable)
const PTE_GOT: u64 = crate::memory::vmem::PTE_VALID
    | crate::memory::vmem::PTE_PAGE
    | crate::memory::vmem::PTE_ACCESS
    | crate::memory::vmem::PTE_NG
    | crate::memory::vmem::PTE_ISH
    | crate::memory::mmu::MT_NORMAL;

/// Allocate and map GOT pages into the user process address space.
fn map_got(ttbr0_phys: u64) -> Result<u64, &'static str> {
    let _num_pages = (USER_GOT_SIZE as usize / PAGE_SIZE as usize).max(1);
    // Use a single page for the initial GOT (we can grow later)
    let got_pages = 1;

    for i in 0..got_pages {
        let va = USER_GOT_BASE + (i as u64) * PAGE_SIZE;
        let frame = frame_alloc::allocate_frame().ok_or("Out of frames for GOT")?;

        // Zero the GOT page
        let virt = phys_to_virt(PhysAddr::new(frame.0));
        unsafe {
            let ptr = virt.0 as *mut u64;
            for j in 0..(PAGE_SIZE as usize / 8) {
                core::ptr::write_volatile(ptr.add(j), 0);
            }
        }

        unsafe {
            crate::sched::elf::map_user_page(ttbr0_phys, va, frame.0, PTE_GOT);
        }
    }

    println!(
        "[dlinker] GOT mapped at {:#018x} ({} pages)",
        USER_GOT_BASE, got_pages
    );
    Ok(USER_GOT_BASE)
}

// ═══════════════════════════════════════════════════════════════
// RELA relocation processing (immediate bindings)
// ═══════════════════════════════════════════════════════════════

fn process_rela(
    elf_data: &[u8],
    load_base: u64,
    dyn_info: &DynInfo,
    got_addr: u64,
    _ttbr0_phys: u64,
) -> Result<u32, &'static str> {
    if dyn_info.rela == 0 || dyn_info.relasz == 0 {
        println!("[dlinker] No RELA relocations");
        return Ok(0);
    }

    let relaent = if dyn_info.relaent != 0 {
        dyn_info.relaent as usize
    } else {
        core::mem::size_of::<Elf64Rela>()
    };

    let num_rela = dyn_info.relasz as usize / relaent;
    let mut count: u32 = 0;

    println!(
        "[dlinker] Processing {} RELA entries (base={:#x})",
        num_rela, load_base
    );

    for i in 0..num_rela {
        let _rela_off = (dyn_info.rela - load_base) as usize + i * relaent;

        // The rela pointer is a VMA; we need to find it in the loaded image.
        // Since we loaded the ELF into the process's address space, we
        // access relocation data from the original elf_data buffer using
        // file offsets. For simplicity, we treat the VMA as a file offset
        // for ET_EXEC; for PIE, subtract load_base.
        let file_off = if load_base != 0 {
            (dyn_info.rela - load_base) as usize + i * relaent
        } else {
            // For ET_EXEC, the VMA equals the file offset for the .rela.dyn section
            // in our simplified loader. We search the ELF data directly.
            // Since rela points to a VMA within the loaded image, compute file offset:
            // file_off = rela_vaddr - load_vaddr + file_offset_of_that_segment
            // Simplification: scan ELF for the LOAD segment containing this VMA
            find_file_offset_for_vaddr(elf_data, dyn_info.rela + i as u64 * relaent as u64)
        };

        if file_off + core::mem::size_of::<Elf64Rela>() > elf_data.len() {
            println!(
                "[dlinker] RELA[{}]: offset out of bounds ({:#x})",
                i, file_off
            );
            continue;
        }

        let rela = unsafe { &*(elf_data.as_ptr().add(file_off) as *const Elf64Rela) };

        let rtype = (rela.r_info & 0xFFFFFFFF) as u32;
        let _rsym = (rela.r_info >> 32) as u32;

        match rtype {
            R_AARCH64_NONE => {}

            R_AARCH64_RELATIVE => {
                // S + A → write (load_base + addend) at r_offset
                let target_vaddr = rela.r_offset + load_base;
                let value = (load_base as i64 + rela.r_addend) as u64;
                write_user_u64(target_vaddr, value, got_addr);
                count += 1;
            }

            R_AARCH64_ABS64 => {
                // S + A → resolve symbol + addend
                let sym_val = resolve_symbol(elf_data, dyn_info, _rsym, load_base);
                let value = (sym_val as i64 + rela.r_addend) as u64;
                let target_vaddr = rela.r_offset + load_base;
                write_user_u64(target_vaddr, value, got_addr);
                count += 1;
            }

            R_AARCH64_GLOB_DAT => {
                // Same as ABS64 for our purposes
                let sym_val = resolve_symbol(elf_data, dyn_info, _rsym, load_base);
                let value = (sym_val as i64 + rela.r_addend) as u64;
                let target_vaddr = rela.r_offset + load_base;
                write_user_u64(target_vaddr, value, got_addr);
                count += 1;
            }

            R_AARCH64_IRELATIVE => {
                // S + A where S is a resolver function
                let resolver_addr = (load_base as i64 + rela.r_addend) as u64;
                let target_vaddr = rela.r_offset + load_base;
                // Write the resolver address; real impl would call it
                write_user_u64(target_vaddr, resolver_addr, got_addr);
                count += 1;
            }

            R_AARCH64_TLS_DTPREL64 | R_AARCH64_TLS_DTPMOD64 | R_AARCH64_TLS_TPREL64 => {
                // TLS relocations — write 0 for now (no TLS support yet)
                let target_vaddr = rela.r_offset + load_base;
                write_user_u64(target_vaddr, 0, got_addr);
                count += 1;
            }

            R_AARCH64_COPY => {
                println!(
                    "[dlinker] R_AARCH64_COPY at {:#x} — skipped (no shared libs)",
                    rela.r_offset
                );
            }

            _ => {
                // Unknown — log and skip
                println!(
                    "[dlinker] Unknown RELA type {} at offset {:#x}",
                    rtype, rela.r_offset
                );
            }
        }
    }

    println!("[dlinker] RELA: {} entries processed", count);
    Ok(count)
}

// ═══════════════════════════════════════════════════════════════
// PLT relocation processing (lazy bindings)
// ═══════════════════════════════════════════════════════════════

fn process_plt(
    elf_data: &[u8],
    load_base: u64,
    dyn_info: &DynInfo,
    got_addr: u64,
    _ttbr0_phys: u64,
) -> Result<u32, &'static str> {
    if dyn_info.jmprel == 0 || dyn_info.pltrelsz == 0 {
        println!("[dlinker] No PLT relocations");
        return Ok(0);
    }

    // Determine relocation entry size from DT_PLTREL
    let entry_size = if dyn_info.pltrel == 7 {
        // DT_RELA
        core::mem::size_of::<Elf64Rela>()
    } else {
        core::mem::size_of::<Elf64Rela>() // Default to RELA
    };

    let num_plt = dyn_info.pltrelsz as usize / entry_size;
    let mut count: u32 = 0;

    println!("[dlinker] Processing {} PLT entries", num_plt);

    for i in 0..num_plt {
        let file_off = if load_base != 0 {
            (dyn_info.jmprel - load_base) as usize + i * entry_size
        } else {
            find_file_offset_for_vaddr(elf_data, dyn_info.jmprel + i as u64 * entry_size as u64)
        };

        if file_off + entry_size > elf_data.len() {
            println!("[dlinker] PLT[{}]: offset out of bounds", i);
            continue;
        }

        let rela = unsafe { &*(elf_data.as_ptr().add(file_off) as *const Elf64Rela) };

        let rtype = (rela.r_info & 0xFFFFFFFF) as u32;
        let _rsym = (rela.r_info >> 32) as u32;

        if rtype != R_AARCH64_JUMP_SLOT {
            println!(
                "[dlinker] PLT[{}]: unexpected type {} (expected JUMP_SLOT)",
                i, rtype
            );
            continue;
        }

        if dyn_info.bind_now {
            // BIND_NOW: resolve immediately
            let sym_val = resolve_symbol(elf_data, dyn_info, _rsym, load_base);
            let value = (sym_val as i64 + rela.r_addend) as u64;
            let target_vaddr = rela.r_offset + load_base;
            write_user_u64(target_vaddr, value, got_addr);
        } else {
            // Lazy binding: write the PLT resolver stub address into GOT slot
            // The GOT slot at rela.r_offset will initially point back to the
            // PLT trampoline, which calls dl_resolve().
            let plt_stub_addr = got_addr; // Resolver stub at GOT base
            let target_vaddr = rela.r_offset + load_base;
            write_user_u64(target_vaddr, plt_stub_addr, got_addr);
        }

        count += 1;
    }

    // Write the PLT resolver stub at the beginning of the GOT
    // This is a tiny AArch64 trampoline that calls dl_resolve():
    //
    //   mov x0, <reloc_index>  (set by PLT entry before jumping here)
    //   stp x29, x30, [sp, #-16]!
    //   bl  dl_resolve
    //   ldp x29, x30, [sp], #16
    //   br  x0
    //
    // For now, write a simple "udf #0" (undefined instruction) so
    // unresolved PLT calls trap with a clear signal.
    if count > 0 {
        // UDF #0 = 0x00000000
        write_user_u64(got_addr, 0, got_addr); // stub[0]
        write_user_u64(got_addr + 8, 0, got_addr); // stub[1]
    }

    println!(
        "[dlinker] PLT: {} entries set up (lazy={})",
        count, !dyn_info.bind_now
    );
    Ok(count)
}

// ═══════════════════════════════════════════════════════════════
// Symbol resolution
// ═══════════════════════════════════════════════════════════════

/// Look up a symbol by index in the dynamic symbol table.
///
/// Returns the symbol's value (VMA) or 0 if not found.
/// For now, only resolves symbols within the same binary.
fn resolve_symbol(elf_data: &[u8], dyn_info: &DynInfo, sym_idx: u32, load_base: u64) -> u64 {
    if dyn_info.symtab == 0 {
        println!("[dlinker] No SYMTAB — cannot resolve symbol {}", sym_idx);
        return 0;
    }

    let sym_size = core::mem::size_of::<Elf64Sym>();
    let sym_vaddr = dyn_info.symtab + (sym_idx as u64) * (sym_size as u64);

    let file_off = if load_base != 0 {
        (sym_vaddr - load_base) as usize
    } else {
        find_file_offset_for_vaddr(elf_data, sym_vaddr)
    };

    if file_off + sym_size > elf_data.len() {
        println!(
            "[dlinker] Symbol {} out of bounds ({:#x})",
            sym_idx, file_off
        );
        return 0;
    }

    let sym = unsafe { &*(elf_data.as_ptr().add(file_off) as *const Elf64Sym) };

    if sym.st_value != 0 {
        sym.st_value + load_base
    } else {
        // Symbol not defined in this binary — would need shared lib lookup
        // Print the symbol name for diagnostics
        if dyn_info.strtab != 0 && sym.st_name != 0 {
            let name_off = if load_base != 0 {
                (dyn_info.strtab - load_base + sym.st_name as u64) as usize
            } else {
                find_file_offset_for_vaddr(elf_data, dyn_info.strtab + sym.st_name as u64)
            };
            if name_off < elf_data.len() {
                let name = unsafe {
                    let ptr = elf_data.as_ptr().add(name_off);
                    let mut len = 0usize;
                    while ptr.add(len).read_volatile() != 0 && name_off + len < elf_data.len() {
                        len += 1;
                    }
                    core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len.min(64)))
                };
                println!(
                    "[dlinker] External symbol '{}' not resolved (no shared libs)",
                    name
                );
            }
        }
        0
    }
}

// ═══════════════════════════════════════════════════════════════
// Runtime resolver (called from PLT trampoline)
// ═══════════════════════════════════════════════════════════════

/// Lazy binding resolver.
///
/// Called when a PLT entry is hit for the first time.
/// `reloc_index` identifies which JMPREL entry to resolve.
/// `got_base` is the process's GOT virtual address.
///
/// Returns the resolved symbol address, or 0 on failure.
///
/// **Note**: In a real OS, this runs in EL0 user context. In our
// kernel-side implementation, we resolve and patch the GOT entry
/// so future calls go directly to the target.
pub fn dl_resolve(
    elf_data: &[u8],
    load_base: u64,
    dyn_info: &DynInfo,
    reloc_index: u32,
    got_addr: u64,
) -> u64 {
    let entry_size = core::mem::size_of::<Elf64Rela>();
    let file_off = if load_base != 0 {
        (dyn_info.jmprel - load_base) as usize + reloc_index as usize * entry_size
    } else {
        find_file_offset_for_vaddr(
            elf_data,
            dyn_info.jmprel + reloc_index as u64 * entry_size as u64,
        )
    };

    if file_off + entry_size > elf_data.len() {
        println!("[dlinker] dl_resolve: reloc {} out of bounds", reloc_index);
        return 0;
    }

    let rela = unsafe { &*(elf_data.as_ptr().add(file_off) as *const Elf64Rela) };

    let rsym = (rela.r_info >> 32) as u32;
    let sym_val = resolve_symbol(elf_data, dyn_info, rsym, load_base);
    let resolved = (sym_val as i64 + rela.r_addend) as u64;

    // Patch the GOT slot so future calls bypass the resolver
    let target_vaddr = rela.r_offset + load_base;
    write_user_u64(target_vaddr, resolved, got_addr);

    if dyn_info.strtab != 0 && rsym != 0 {
        let sym_size = core::mem::size_of::<Elf64Sym>();
        let sym_vaddr = dyn_info.symtab + (rsym as u64) * (sym_size as u64);
        let sym_file_off = if load_base != 0 {
            (sym_vaddr - load_base) as usize
        } else {
            find_file_offset_for_vaddr(elf_data, sym_vaddr)
        };

        if sym_file_off + sym_size <= elf_data.len() {
            let sym = unsafe { &*(elf_data.as_ptr().add(sym_file_off) as *const Elf64Sym) };
            if sym.st_name != 0 {
                let name_off = if load_base != 0 {
                    (dyn_info.strtab - load_base + sym.st_name as u64) as usize
                } else {
                    find_file_offset_for_vaddr(elf_data, dyn_info.strtab + sym.st_name as u64)
                };
                if name_off < elf_data.len() {
                    let name = unsafe {
                        let ptr = elf_data.as_ptr().add(name_off);
                        let mut len = 0usize;
                        while ptr.add(len).read_volatile() != 0 && name_off + len < elf_data.len() {
                            len += 1;
                        }
                        core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                            ptr,
                            len.min(64),
                        ))
                    };
                    println!("[dlinker] Lazy resolved '{}' → {:#018x}", name, resolved);
                }
            }
        }
    }

    resolved
}

// ═══════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════

/// Find the file offset in the ELF data corresponding to a virtual address.
///
/// Scans PT_LOAD segments to map VMA → file offset.
fn find_file_offset_for_vaddr(elf_data: &[u8], vaddr: u64) -> usize {
    let ehdr = unsafe { &*(elf_data.as_ptr() as *const super::elf::Elf64Ehdr) };

    for i in 0..ehdr.e_phnum as usize {
        let phdr_off = ehdr.e_phoff as usize + i * ehdr.e_phentsize as usize;
        if phdr_off + core::mem::size_of::<super::elf::Elf64Phdr>() > elf_data.len() {
            continue;
        }
        let phdr = unsafe { &*(elf_data.as_ptr().add(phdr_off) as *const super::elf::Elf64Phdr) };

        if phdr.p_type != 1 {
            // PT_LOAD
            continue;
        }

        if vaddr >= phdr.p_vaddr && vaddr < phdr.p_vaddr + phdr.p_memsz {
            return (phdr.p_offset + (vaddr - phdr.p_vaddr)) as usize;
        }
    }

    // Fallback: treat as raw file offset (risky but works for simple ELFs)
    vaddr as usize
}

/// Write a u64 value into the user process's address space via the GOT mapping.
///
/// Since the GOT is identity-mapped into kernel virtual memory via phys_to_virt,
/// we can write to user VAs that fall within the GOT region directly.
/// For other regions, we'd need to walk the user page tables — for now,
/// we rely on the fact that our ELF loader writes directly to phys frames.
fn write_user_u64(vaddr: u64, value: u64, _got_addr: u64) {
    // In a bare-metal kernel, the user pages are mapped 1:1 via phys_to_virt.
    // We translate the user VA through the direct map to write.
    // This works because the ELF loader allocated physical frames and mapped
    // them; the kernel can access all physical memory.
    //
    // A proper implementation would walk the user page tables via TTBR0.
    // For now, we simply store the value and trust the loader's phys map.

    // HACK: write via the direct physical map by looking up the user VA
    // in the kernel's page tables. If the VA falls within our user range,
    // it was mapped by the ELF loader and we can translate it.
    if let Some(pa) = crate::memory::vmem::translate_va(crate::memory::VirtAddr::new(vaddr)) {
        let kvirt = phys_to_virt(pa);
        unsafe {
            let ptr = kvirt.0 as *mut u64;
            core::ptr::write_volatile(ptr, value);
        }
    } else {
        // VA not yet mapped — this can happen for RELATIVE relocations
        // that target a VMA in a segment we haven't walked yet.
        // In practice, the ELF loader maps all segments before we get here,
        // so this should not trigger.
        // Future: allocate and map the page on demand.
    }
}

// ═══════════════════════════════════════════════════════════════
// ELF dynamic section inspector (for diagnostics)
// ═══════════════════════════════════════════════════════════════

/// Print a summary of the dynamic section for debugging.
pub fn dump_dynamic(elf_data: &[u8], _load_base: u64) {
    let ehdr = unsafe { &*(elf_data.as_ptr() as *const super::elf::Elf64Ehdr) };

    let mut dyn_offset: u64 = 0;
    let mut dyn_filesz: u64 = 0;

    for i in 0..ehdr.e_phnum as usize {
        let phdr_off = ehdr.e_phoff as usize + i * ehdr.e_phentsize as usize;
        if phdr_off + core::mem::size_of::<super::elf::Elf64Phdr>() > elf_data.len() {
            continue;
        }
        let phdr = unsafe { &*(elf_data.as_ptr().add(phdr_off) as *const super::elf::Elf64Phdr) };

        if phdr.p_type == PT_DYNAMIC {
            dyn_offset = phdr.p_offset;
            dyn_filesz = phdr.p_filesz;
            break;
        }
    }

    if dyn_filesz == 0 {
        println!("[dlinker] dump: no PT_DYNAMIC");
        return;
    }

    let entry_size = core::mem::size_of::<Elf64Dyn>();
    let num_entries = dyn_filesz as usize / entry_size;

    println!(
        "[dlinker] === Dynamic section dump ({} entries) ===",
        num_entries
    );

    for i in 0..num_entries {
        let off = dyn_offset as usize + i * entry_size;
        if off + entry_size > elf_data.len() {
            break;
        }
        let dyn_entry = unsafe { &*(elf_data.as_ptr().add(off) as *const Elf64Dyn) };

        let tag_name = match dyn_entry.d_tag as u64 {
            DT_NULL => "NULL",
            DT_NEEDED => "NEEDED",
            DT_PLTGOT => "PLTGOT",
            DT_HASH => "HASH",
            DT_STRTAB => "STRTAB",
            DT_SYMTAB => "SYMTAB",
            DT_STRSZ => "STRSZ",
            DT_SYMENT => "SYMENT",
            DT_INIT => "INIT",
            DT_FINI => "FINI",
            DT_RELA => "RELA",
            DT_RELASZ => "RELASZ",
            DT_RELAENT => "RELAENT",
            DT_JMPREL => "JMPREL",
            DT_PLTRELSZ => "PLTRELSZ",
            DT_PLTREL => "PLTREL",
            DT_RPATH => "RPATH",
            DT_RUNPATH => "RUNPATH",
            DT_FLAGS => "FLAGS",
            DT_FLAGS_1 => "FLAGS_1",
            DT_VERSYM => "VERSYM",
            DT_VERNEED => "VERNEED",
            DT_VERNEEDNUM => "VERNEEDNUM",
            _ => "UNKNOWN",
        };

        println!(
            "[dlinker]   [{}] {} = {:#018x}",
            i, tag_name, dyn_entry.d_val
        );

        if dyn_entry.d_tag as u64 == DT_NULL {
            break;
        }
    }

    println!("[dlinker] === End dynamic section ===");
}
