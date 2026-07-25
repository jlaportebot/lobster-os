//! Flattened Device Tree (FDT) parser for the BCM2712 DTB
//!
//! Parses the device tree to extract:
//! - Memory size (from /memory/reg)
//! - CPU count and frequencies
//! - Peripheral addresses
//! - Boot parameters

use crate::println;

/// Detected system info from DTB
pub static mut SYSTEM_MEMORY_SIZE: u64 = 0;
pub static mut CPU_COUNT: u32 = 0;
pub static mut SYSTEM_SERIAL: u64 = 0;
pub static mut BOARD_REVISION: u32 = 0;

/// Parse the DTB and extract system information
pub fn parse(dtb_ptr: u64) {
    let header = unsafe { &*(dtb_ptr as *const FdtHeader) };

    let magic = u32::from_be(header.magic);
    if magic != 0xD00DFEED {
        panic!("Invalid DTB magic number: {:#010x}", magic);
    }

    println!("[dtb] DTB header:");
    println!("  Magic: {:#010x}", magic);
    let totalsize = u32::from_be(header.totalsize) as u64;
    let off_dt_struct = u32::from_be(header.off_dt_struct) as u64;
    let off_dt_strings = u32::from_be(header.off_dt_strings) as u64;
    let off_mem_rsvmap = u32::from_be(header.off_mem_rsvmap) as u64;
    println!("  Total size: {} bytes", totalsize);
    println!("  Structure offset: {:#010x}", off_dt_struct);
    println!("  Strings offset: {:#010x}", off_dt_strings);
    println!(
        "  Memory reservation block offset: {:#010x}",
        off_mem_rsvmap
    );

    let struct_start = dtb_ptr + off_dt_struct;
    let strings_start = dtb_ptr + off_dt_strings;
    let struct_end_offset = totalsize;

    parse_structure_block(struct_start, strings_start, struct_end_offset);

    unsafe {
        if SYSTEM_MEMORY_SIZE == 0 {
            // Fallback: try to detect via memory reservation block
            let rsvmap = dtb_ptr + header.off_mem_rsvmap as u64;
            let mut off = 0u64;
            loop {
                let addr = u64::from_be(core::ptr::read_volatile((rsvmap + off) as *const u64));
                let size = u64::from_be(core::ptr::read_volatile((rsvmap + off + 8) as *const u64));
                if addr == 0 && size == 0 {
                    break;
                }
                if addr == 0 && size > SYSTEM_MEMORY_SIZE {
                    SYSTEM_MEMORY_SIZE = size;
                }
                off += 16;
            }
        }
        if SYSTEM_MEMORY_SIZE > 0 {
            println!(
                "[dtb] Detected memory: {} MB ({} GB)",
                SYSTEM_MEMORY_SIZE / (1024 * 1024),
                SYSTEM_MEMORY_SIZE / (1024 * 1024 * 1024)
            );
        } else {
            println!("[dtb] WARNING: Could not detect memory size from DTB");
        }
        if CPU_COUNT > 0 {
            println!("[dtb] Detected {} CPU cores", CPU_COUNT);
        }
    }
}

fn parse_structure_block(struct_start: u64, strings_start: u64, struct_end: u64) {
    let mut offset = 0u64;
    let mut depth: u32 = 0;
    let mut current_node_name: [&str; 16] = [""; 16];
    let mut in_memory_node = false;

    while offset < struct_end {
        let token_ptr = struct_start + offset;
        let token = u32::from_be(unsafe { core::ptr::read_volatile(token_ptr as *const u32) });

        match token {
            FDT_BEGIN_NODE => {
                offset += 4;
                let name = read_string_at(struct_start + offset);
                let name_len = name.len() + 1;
                offset += ((name_len + 3) & !3) as u64;

                current_node_name[depth as usize] = name;
                depth += 1;

                in_memory_node =
                    name.starts_with("memory@") || name == "memory" || name.starts_with("memory");
            }
            FDT_END_NODE => {
                depth = depth.saturating_sub(1);
                in_memory_node = false;
                offset += 4;
            }
            FDT_PROP => {
                offset += 4;

                let prop_len = u32::from_be(unsafe {
                    core::ptr::read_volatile((struct_start + offset) as *const u32)
                }) as u64;
                offset += 4;

                let name_off = u32::from_be(unsafe {
                    core::ptr::read_volatile((struct_start + offset) as *const u32)
                }) as u64;
                offset += 4;

                let prop_name = read_string_at(strings_start + name_off);
                let prop_data_ptr = struct_start + offset;
                let prop_data_end = prop_data_ptr + prop_len;

                if depth > 0 {
                    let parent = current_node_name[(depth - 1) as usize];

                    if in_memory_node && prop_name == "reg" {
                        let (addr, size) = parse_reg_property(prop_data_ptr, prop_len);
                        unsafe {
                            if size > SYSTEM_MEMORY_SIZE {
                                SYSTEM_MEMORY_SIZE = size;
                            }
                        }
                        println!(
                            "[dtb] memory@reg: addr={:#010x} size={} MB",
                            addr,
                            size / (1024 * 1024)
                        );
                    }

                    if parent.starts_with("cpu@") && prop_name == "reg" {
                        unsafe {
                            CPU_COUNT += 1;
                        }
                    }

                    if parent == "cpus" && prop_name.starts_with("#address-cells") {
                        // Parse #address-cells
                    }

                    if prop_name == "serial-number" && prop_len == 8 {
                        let serial = u64::from_be(unsafe {
                            core::ptr::read_volatile(prop_data_ptr as *const u64)
                        });
                        unsafe {
                            SYSTEM_SERIAL = serial;
                        }
                        println!("[dtb] Serial: {:#018x}", serial);
                    }

                    if prop_name == "revision" && prop_len == 4 {
                        let rev = u32::from_be(unsafe {
                            core::ptr::read_volatile(prop_data_ptr as *const u32)
                        });
                        unsafe {
                            BOARD_REVISION = rev;
                        }
                        println!("[dtb] Board revision: {:#010x}", rev);
                    }
                }

                let _ = prop_data_end;
                offset += (prop_len as u64 + 3) & !3;
            }
            FDT_NOP => {
                offset += 4;
            }
            FDT_END => {
                break;
            }
            _ => {
                offset += 4;
            }
        }
    }
}

fn parse_mem_property(ptr: u64, len: u32) -> u64 {
    if len >= 16 {
        let addr = unsafe { core::ptr::read_volatile(ptr as *const u64) };
        let size = unsafe { core::ptr::read_volatile((ptr + 8) as *const u64) };
        // Convert from big-endian if needed
        // On aarch64, device tree values are big-endian
        let size_le = u64::from_be(size);
        let _ = u64::from_be(addr);
        size_le
    } else if len >= 8 {
        let size = unsafe { core::ptr::read_volatile(ptr as *const u64) };
        u64::from_be(size)
    } else {
        0
    }
}

fn parse_reg_property(ptr: u64, _len: u64) -> (u64, u64) {
    let addr = unsafe { core::ptr::read_volatile(ptr as *const u64) };
    let size = unsafe { core::ptr::read_volatile((ptr + 8) as *const u64) };
    (u64::from_be(addr), u64::from_be(size))
}

/// Read a null-terminated string from memory
fn read_string_at(ptr: u64) -> &'static str {
    if ptr == 0 {
        return "";
    }

    let mut len = 0usize;
    while len < 256 {
        let byte = unsafe { core::ptr::read_volatile((ptr + len as u64) as *const u8) };
        if byte == 0 {
            break;
        }
        len += 1;
    }

    if len == 0 {
        return "";
    }

    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };

    core::str::from_utf8(bytes).unwrap_or_default()
}

/// Get the detected memory size (0 if not yet parsed)
pub fn get_memory_size() -> u64 {
    unsafe { SYSTEM_MEMORY_SIZE }
}

/// Get the detected CPU count (0 if not yet parsed)
pub fn get_cpu_count() -> u32 {
    unsafe { CPU_COUNT }
}

/// Get the board serial number
pub fn get_serial() -> u64 {
    unsafe { SYSTEM_SERIAL }
}

/// Get the board revision
pub fn get_board_revision() -> u32 {
    unsafe { BOARD_REVISION }
}

/// FDT header — all fields are big-endian per the FDT spec.
/// Use u32::from_be() when reading values.
#[repr(C)]
struct FdtHeader {
    magic: u32,
    totalsize: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    off_mem_rsvmap: u32,
    version: u32,
    last_comp_version: u32,
    boot_cpuid_phys: u32,
    size_dt_strings: u32,
    size_dt_struct: u32,
}

const FDT_BEGIN_NODE: u32 = 0x00000001;
const FDT_END_NODE: u32 = 0x00000002;
const FDT_PROP: u32 = 0x00000003;
const FDT_NOP: u32 = 0x00000004;
const FDT_END: u32 = 0x00000009;
