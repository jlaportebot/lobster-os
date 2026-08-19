/* Phase 3: Dynamic Linker Enhancement
 * 
 * This file implements shared library loading for LobsterOS.
 * Key components:
 * 1. DT_NEEDED library loading from VFS
 * 2. Global symbol table for inter-library symbol resolution
 * 3. dlopen/dlsym/dlclose implementation
 */

use crate::fs::vfs;
use crate::println;
use alloc::string::String;
use alloc::vec::Vec;
use core::mem::size_of;

/// Maximum number of loaded shared libraries
const MAX_SHARED_LIBS: usize = 16;

/// Shared library handle
#[derive(Clone, Copy)]
pub struct SharedLib {
    pub name: [u8; 64],
    pub name_len: usize,
    pub base_vaddr: u64,
    pub load_base: u64,
    pub dyn_info: Option<SharedLibDynInfo>,
    pub ref_count: u32,
}

/// Dynamic info for a shared library
#[derive(Clone, Copy)]
pub struct SharedLibDynInfo {
    pub strtab: u64,
    pub symtab: u64,
    pub strsz: u64,
    pub hash: u64,
    pub jmprel: u64,
    pub pltrelsz: u64,
    pub pltrel: u64,
    pub pltgot: u64,
    pub rela: u64,
    pub relasz: u64,
    pub relaent: u64,
    pub init: u64,
    pub fini: u64,
    pub bind_now: bool,
}

/// Global shared library state
static mut SHARED_LIBS: [Option<SharedLib>; MAX_SHARED_LIBS] = [None; MAX_SHARED_LIBS];
static mut SHARED_LIBS_COUNT: usize = 0;

/// Find a shared library by name
pub fn find_shared_lib(name: &str) -> Option<usize> {
    unsafe {
        for i in 0..SHARED_LIBS_COUNT {
            if let Some(lib) = SHARED_LIBS[i] {
                let lib_name = core::str::from_utf8(&lib.name[..lib.name_len]).unwrap_or("");
                if lib_name == name {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Load a shared library from VFS
pub fn load_shared_lib(name: &str) -> Result<u64, &'static str> {
    // Check if already loaded
    if let Some(idx) = find_shared_lib(name) {
        unsafe {
            SHARED_LIBS[idx].as_mut().unwrap().ref_count += 1;
            return Ok(SHARED_LIBS[idx].unwrap().base_vaddr);
        }
    }
    
    // Search in library paths
    let search_paths = ["/usr/lib", "/lib", "/usr/local/lib"];
    let mut elf_data: Option<Vec<u8>> = None;
    
    for path in &search_paths {
        let full_path = alloc::format!("{}/{}", path, name);
        match vfs::read_file(&full_path) {
            Ok(data) => {
                elf_data = Some(data);
                break;
            }
            Err(_) => continue,
        }
    }
    
    let elf_data = elf_data.ok_or("Library not found in search paths")?;
    
    // Parse ELF header
    if elf_data.len() < size_of::<super::elf::Elf64Ehdr>() {
        return Err("ELF too small");
    }
    
    let ehdr = unsafe { &*(elf_data.as_ptr() as *const super::elf::Elf64Ehdr) };
    if !super::elf::verify_header(ehdr) {
        return Err("Invalid ELF header");
    }
    
    if ehdr.e_type != super::elf::ET_DYN {
        return Err("Not a shared library (ET_DYN)");
    }
    
    // Find load base (simplified: use first PT_LOAD vaddr)
    let mut load_base = 0u64;
    for i in 0..ehdr.e_phnum as usize {
        let phdr_off = ehdr.e_phoff as usize + i * ehdr.e_phentsize as usize;
        if phdr_off + size_of::<super::elf::Elf64Phdr>() > elf_data.len() {
            continue;
        }
        let phdr = unsafe { &*(elf_data.as_ptr().add(phdr_off) as *const super::elf::Elf64Phdr) };
        if phdr.p_type == 1 { // PT_LOAD
            load_base = phdr.p_vaddr;
            break;
        }
    }
    
    // For now, just map the library into the current process
    // A real implementation would:
    // 1. Allocate virtual address range
    // 2. Map all PT_LOAD segments
    // 3. Process relocations
    // 4. Run .init
    // 5. Add to global symbol table
    
    // Store in shared libs array
    unsafe {
        if SHARED_LIBS_COUNT >= MAX_SHARED_LIBS {
            return Err("Too many shared libraries");
        }
        
        let mut lib_name = [0u8; 64];
        let name_bytes = name.as_bytes();
        lib_name[..name_bytes.len().min(63)].copy_from_slice(name_bytes);
        
        SHARED_LIBS[SHARED_LIBS_COUNT] = Some(SharedLib {
            name: lib_name,
            name_len: name_bytes.len().min(63),
            base_vaddr: load_base,
            load_base: 0, // Not yet mapped
            dyn_info: None,
            ref_count: 1,
        });
        
        let idx = SHARED_LIBS_COUNT;
        SHARED_LIBS_COUNT += 1;
        
        println!("[shared] Loaded {} at index {}", name, idx);
        Ok(load_base)
    }
}

/// Unload a shared library
pub fn unload_shared_lib(name: &str) -> Result<(), &'static str> {
    unsafe {
        if let Some(idx) = find_shared_lib(name) {
            let lib = SHARED_LIBS[idx].as_mut().unwrap();
            lib.ref_count -= 1;
            if lib.ref_count == 0 {
                // TODO: Unmap pages, run .fini
                // Remove from array (swap with last)
                SHARED_LIBS[idx] = SHARED_LIBS[SHARED_LIBS_COUNT - 1];
                SHARED_LIBS[SHARED_LIBS_COUNT - 1] = None;
                SHARED_LIBS_COUNT -= 1;
            }
            Ok(())
        } else {
            Err("Library not loaded")
        }
    }
}

/// Lookup a symbol in all loaded shared libraries
pub fn lookup_symbol_in_libs(name: &str) -> u64 {
    unsafe {
        for i in 0..SHARED_LIBS_COUNT {
            if let Some(lib) = SHARED_LIBS[i] {
                if let Some(dyn_info) = lib.dyn_info {
                    if let Some(addr) = find_symbol_in_lib(&lib, dyn_info, name) {
                        return addr;
                    }
                }
            }
        }
    }
    0
}

fn find_symbol_in_lib(lib: &SharedLib, dyn_info: SharedLibDynInfo, name: &str) -> Option<u64> {
    // Simplified: would need to parse the library's symtab/strtab
    // For now, return 0 (not found)
    None
}

/// dlopen syscall implementation
pub fn sys_dlopen(path_ptr: u64, flags: u32) -> u64 {
    if path_ptr == 0 {
        return 0;
    }
    
    let path = unsafe {
        let mut len = 0usize;
        let ptr = path_ptr as *const u8;
        while *ptr.add(len) != 0 && len < 256 {
            len += 1;
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len))
    };
    
    match load_shared_lib(path) {
        Ok(addr) => addr,
        Err(_) => 0,
    }
}

/// dlsym syscall implementation
pub fn sys_dlsym(handle: u64, symbol_ptr: u64) -> u64 {
    if handle == 0 || symbol_ptr == 0 {
        return 0;
    }
    
    let symbol = unsafe {
        let mut len = 0usize;
        let ptr = symbol_ptr as *const u8;
        while *ptr.add(len) != 0 && len < 256 {
            len += 1;
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len))
    };
    
    // If handle is a library base, search that library
    // Otherwise search all loaded libraries
    if handle != u64::MAX {
        // TODO: Search specific library
    }
    
    lookup_symbol_in_libs(symbol)
}

/// dlclose syscall implementation
pub fn sys_dlclose(handle: u64) -> u64 {
    if handle == 0 {
        return u64::MAX;
    }
    
    // Find library by handle (base address)
    unsafe {
        for i in 0..SHARED_LIBS_COUNT {
            if SHARED_LIBS[i].as_ref().map(|l| l.base_vaddr == handle).unwrap_or(false) {
                let name = core::str::from_utf8(&SHARED_LIBS[i].as_ref().unwrap().name[..SHARED_LIBS[i].as_ref().unwrap().name_len]).unwrap_or("");
                let _ = unload_shared_lib(name);
                return 0;
            }
        }
    }
    
    u64::MAX
}