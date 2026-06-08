//! Flattened Device Tree (FDT) parser for the BCM2712 DTB

use crate::println;

/// Parse the DTB and extract system information
pub fn parse(dtb_ptr: u64) {
    // Parse the DTB header
    let header = unsafe { &*(dtb_ptr as *const FdtHeader) };

    // Verify magic number
    if header.magic != 0xD00DFEED {
        panic!("Invalid DTB magic number: {:#010x}", header.magic);
    }

    println!("[dtb] DTB header:");
    println!(" Magic: {:#010x}", header.magic);
    println!(" Total size: {} bytes", header.totalsize);
    println!(" Structure offset: {:#010x}", header.off_dt_struct);
    println!(" Strings offset: {:#010x}", header.off_dt_strings);
    println!(
        " Memory reservation block offset: {:#010x}",
        header.off_mem_rsvmap
    );

    // Parse the structure block
    let struct_start = dtb_ptr + header.off_dt_struct as u64;
    let strings_start = dtb_ptr + header.off_dt_strings as u64;

    // Walk the structure block
    parse_structure_block(struct_start, strings_start, header.totalsize as u64);
}

/// Parse the structure block and extract information
fn parse_structure_block(struct_start: u64, strings_start: u64, struct_end: u64) {
    let mut offset = 0u64;

    while offset < struct_end {
        let token_ptr = struct_start + offset;
        let token = unsafe { core::ptr::read_volatile(token_ptr as *const u32) };

        match token {
            FDT_BEGIN_NODE => {
                offset += 4; // Skip token
                let node_name = read_string_at(struct_start + offset);
                println!("[dtb] Begin node: {}", node_name);

                // Skip node name (null-terminated string, aligned to 4 bytes)
                let name_len = node_name.len() + 1; // +1 for null terminator
                offset += ((name_len + 3) & !3) as u64; // Align to 4 bytes
            }
            FDT_END_NODE => {
                println!("[dtb] End node");
                offset += 4; // Skip token
            }
            FDT_PROP => {
                offset += 4; // Skip token

                // Read property length and name offset
                let prop_len =
                    unsafe { core::ptr::read_volatile((struct_start + offset) as *const u32) };
                offset += 4;

                let name_offset =
                    unsafe { core::ptr::read_volatile((struct_start + offset) as *const u32) };
                offset += 4;

                // Read property name
                let prop_name = read_string_at(strings_start + name_offset as u64);
                println!("[dtb] Property: {} ({} bytes)", prop_name, prop_len);

                // Skip property data (aligned to 4 bytes)
                offset += (prop_len as u64 + 3) & !3;
            }
            FDT_NOP => {
                println!("[dtb] NOP token");
                offset += 4; // Skip token
            }
            FDT_END => {
                println!("[dtb] End of structure block");
                break;
            }
            _ => {
                println!("[dtb] Unknown token: {:#010x}", token);
                offset += 4;
            }
        }
    }
}

/// Read a null-terminated string from memory
fn read_string_at(_ptr: u64) -> &'static str {
    // This is a simplified implementation
    // In a real implementation, we would need to properly parse the string
    "unknown"
}

/// FDT header structure
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

/// FDT token constants
const FDT_BEGIN_NODE: u32 = 0x00000001;
const FDT_END_NODE: u32 = 0x00000002;
const FDT_PROP: u32 = 0x00000003;
const FDT_NOP: u32 = 0x00000004;
const FDT_END: u32 = 0x00000009;
