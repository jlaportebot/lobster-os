//! Initial RAM Disk (initrd) loader — cpio newc format

use crate::println;
use super::vfs::*;

/// CPIO newc format magic
const CPIO_MAGIC: [u8; 6] = *b"070701";
const CPIO_MAGIC_CRC: [u8; 6] = *b"070702";
const CPIO_TRAILER: &str = "TRAILER!!!";

/// CPIO newc header (110 bytes, all ASCII hex)
#[repr(C)]
pub struct CpioHeader {
    pub magic: [u8; 6],
    pub inode: [u8; 8],
    pub mode: [u8; 8],
    pub uid: [u8; 8],
    pub gid: [u8; 8],
    pub nlink: [u8; 8],
    pub mtime: [u8; 8],
    pub filesize: [u8; 8],
    pub devmajor: [u8; 8],
    pub devminor: [u8; 8],
    pub rdevmajor: [u8; 8],
    pub rdevminor: [u8; 8],
    pub namesize: [u8; 8],
    pub check: [u8; 8],
}

/// Parse a hex ASCII string to u64
fn parse_hex_ascii(buf: &[u8]) -> u64 {
    let mut result = 0u64;
    for &b in buf {
        result <<= 4;
        if b >= b'0' && b <= b'9' {
            result |= (b - b'0') as u64;
        } else if b >= b'a' && b <= b'f' {
            result |= (b - b'a' + 10) as u64;
        } else if b >= b'A' && b <= b'F' {
            result |= (b - b'A' + 10) as u64;
        }
    }
    result
}

/// Align up to 4-byte boundary
fn align4(val: usize) -> usize {
    (val + 3) & !3
}

/// Load an initrd from a memory region and populate the VFS under /initrd/
pub fn load_initrd(base: *const u8, size: usize) -> Result<(), VfsError> {
    if size < 110 {
        println!("[initrd] Too small to be a valid cpio archive");
        return Err(VfsError::IOError);
    }

    // Create /initrd/ directory in VFS
    let initrd_inode = alloc_node("initrd", FileType::Directory, 0)?;

    let mut offset = 0usize;
    let mut file_count = 0usize;

    while offset + 110 <= size {
        let ptr = unsafe { base.add(offset) };
        let header = unsafe { &*(ptr as *const CpioHeader) };

        // Verify magic
        if header.magic != CPIO_MAGIC && header.magic != CPIO_MAGIC_CRC {
            println!("[initrd] Invalid magic at offset {}, stopping", offset);
            break;
        }

        let mode = parse_hex_ascii(&header.mode) as u32;
        let filesize = parse_hex_ascii(&header.filesize) as usize;
        let namesize = parse_hex_ascii(&header.namesize) as usize;

        // Parse filename
        let name_offset = offset + 110;
        if name_offset + namesize > size {
            println!("[initrd] Name extends beyond archive");
            break;
        }

        let name_ptr = unsafe { base.add(name_offset) };
        let name_bytes = unsafe { core::slice::from_raw_parts(name_ptr, namesize.saturating_sub(1)) };
        let name = core::str::from_utf8(name_bytes).unwrap_or("<invalid>");

        // Check for trailer
        if name == CPIO_TRAILER {
            println!("[initrd] Reached trailer, done");
            break;
        }

        // Data starts after header + name, aligned to 4 bytes
        let data_offset = align4(name_offset + namesize);
        let data_end = data_offset + filesize;

        // Determine file type from mode
        let file_type = if mode & 0o170000 == 0o040000 {
            FileType::Directory
        } else if mode & 0o170000 == 0o120000 {
            FileType::Symlink
        } else {
            FileType::RegularFile
        };

        // Strip leading / from name
        let clean_name = name.trim_start_matches('/');

        // Handle path components (create intermediate directories)
        // no_std: split path and take last component without Vec
        let mut entry_name = clean_name;
        let mut last_slash = 0usize;
        for (i, b) in clean_name.as_bytes().iter().enumerate() {
            if *b == b'/' { last_slash = i; }
        }
        if last_slash > 0 {
            entry_name = &clean_name[last_slash + 1..];
        };

        if entry_name.is_empty() {
            // Skip entries with empty names
            offset = align4(data_end);
            continue;
        }

        // Create VFS node
        let node_id = alloc_node(entry_name, file_type, initrd_inode)?;
        if file_type == FileType::RegularFile && filesize > 0 {
            if let Some(node) = get_node_mut(node_id) {
                node.data_offset = data_offset as u64;
                node.data_len = filesize as u64;
                node.metadata.size = filesize as u64;
            }
        }

        file_count += 1;
        if file_count <= 20 {
            let type_str = if file_type == FileType::Directory { "dir " } else { "file" };
            println!("[initrd] {} {} ({} bytes)", type_str, clean_name, filesize);
        }

        // Advance to next entry
        offset = align4(data_end);
    }

    if file_count > 20 {
        println!("[initrd] ... and {} more entries", file_count - 20);
    }
    println!("[initrd] Loaded {} files from initrd", file_count);

    // Mount in VFS
    mount("/initrd", 0, initrd_inode)?;

    Ok(())
}
