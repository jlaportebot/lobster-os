//! ext2 filesystem checker (fsck) for LobsterOS
//!
//! Provides consistency checking and repair for ext2 filesystems:
//! - Validates superblock magic
//! - Checks block group descriptors
//! - Cross-checks block and inode bitmaps
//! - Detects unreachable and multiply-claimed blocks
//! - Verifies link counts against directory traversal
//! - Repairs simple inconsistencies

#![allow(dead_code, unused_imports)]

use super::ext2::{
    Ext2FileSystem, EXT2_DIND_BLOCK, EXT2_IND_BLOCK, EXT2_NDIR_BLOCKS, EXT2_TIND_BLOCK,
    MAX_BLOCK_GROUPS,
};
use crate::println;

/// Constants for ext2 filesystem
const EXT2_MAGIC: u16 = 0xEF53;
const EXT2_ROOT_INO: u32 = 2;
const EXT2_FIRST_INO: u32 = 11;

/// fsck result codes
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsckResult {
    Ok,
    ErrorsFound,
    Repaired,
    FatalError,
}

/// Check mode: dry-run or repair
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsckMode {
    CheckOnly,
    Repair,
}

/// Check the ext2 superblock
fn check_superblock(fs: &mut Ext2FileSystem) -> FsckResult {
    let magic = u16::from_le(fs.superblock.s_magic);
    if magic != EXT2_MAGIC {
        println!("[fsck] ERROR: Invalid superblock magic: {:#06X}", magic);
        return FsckResult::FatalError;
    }
    println!("[fsck] Superblock magic valid: {:#06X}", magic);

    // Check state: 1 = clean, 2 = has errors
    let state = u16::from_le(fs.superblock.s_state);
    if state != 1 {
        println!("[fsck] WARNING: Filesystem state = {} (not clean)", state);
    }

    // Check block size validity
    let log_block_size = u32::from_le(fs.superblock.s_log_block_size);
    let block_size = 1024u32 << log_block_size;
    if block_size != 1024 && block_size != 2048 && block_size != 4096 {
        println!("[fsck] ERROR: Invalid block size: {}", block_size);
        return FsckResult::FatalError;
    }

    // Check blocks count > 0
    let blocks_count = u32::from_le(fs.superblock.s_blocks_count);
    if blocks_count == 0 {
        println!("[fsck] ERROR: Blocks count is zero");
        return FsckResult::FatalError;
    }

    // Check inodes count > 0
    let inodes_count = u32::from_le(fs.superblock.s_inodes_count);
    if inodes_count == 0 {
        println!("[fsck] ERROR: Inodes count is zero");
        return FsckResult::FatalError;
    }

    // Check free counts don't exceed totals
    let free_blocks = u32::from_le(fs.superblock.s_free_blocks_count);
    let free_inodes = u32::from_le(fs.superblock.s_free_inodes_count);
    if free_blocks > blocks_count {
        println!(
            "[fsck] ERROR: Free blocks ({}) > total blocks ({})",
            free_blocks, blocks_count
        );
        return FsckResult::ErrorsFound;
    }
    if free_inodes > inodes_count {
        println!(
            "[fsck] ERROR: Free inodes ({}) > total inodes ({})",
            free_inodes, inodes_count
        );
        return FsckResult::ErrorsFound;
    }

    println!("[fsck] Superblock checks passed");
    FsckResult::Ok
}

/// Check block group descriptors for consistency
fn check_group_descs(fs: &mut Ext2FileSystem) -> FsckResult {
    let mut errors = false;
    let blocks_per_group = u32::from_le(fs.superblock.s_blocks_per_group);
    let inodes_per_group = fs.inodes_per_group;
    let blocks_count = u32::from_le(fs.superblock.s_blocks_count);
    let _inodes_count = u32::from_le(fs.superblock.s_inodes_count);

    for g in 0..fs.num_groups as usize {
        let gdesc = &fs.group_descs[g];

        // Check block bitmap location
        let bg_block_bitmap = u32::from_le(gdesc.bg_block_bitmap);
        if bg_block_bitmap == 0 {
            println!("[fsck] WARNING: Block bitmap is zero in group {}", g);
            errors = true;
        } else if bg_block_bitmap > blocks_count {
            println!(
                "[fsck] ERROR: Block bitmap {} beyond blocks count in group {}",
                bg_block_bitmap, g
            );
            errors = true;
        }

        // Check inode bitmap location
        let bg_inode_bitmap = u32::from_le(gdesc.bg_inode_bitmap);
        if bg_inode_bitmap == 0 {
            println!("[fsck] WARNING: Inode bitmap is zero in group {}", g);
            errors = true;
        } else if bg_inode_bitmap > blocks_count {
            println!(
                "[fsck] ERROR: Inode bitmap {} beyond blocks count in group {}",
                bg_inode_bitmap, g
            );
            errors = true;
        }

        // Check inode table location
        let bg_inode_table = u32::from_le(gdesc.bg_inode_table);
        if bg_inode_table == 0 {
            println!("[fsck] WARNING: Inode table is zero in group {}", g);
            errors = true;
        } else if bg_inode_table > blocks_count {
            println!(
                "[fsck] ERROR: Inode table {} beyond blocks count in group {}",
                bg_inode_table, g
            );
            errors = true;
        }

        // Check free blocks count doesn't exceed group total
        let bg_free_blocks = u16::from_le(gdesc.bg_free_blocks_count);
        if bg_free_blocks as u32 > blocks_per_group {
            println!(
                "[fsck] ERROR: Group {} free blocks ({}) exceeds per-group limit ({})",
                g, bg_free_blocks, blocks_per_group
            );
            errors = true;
        }

        // Check free inodes count doesn't exceed per-group inodes
        let bg_free_inodes = u16::from_le(gdesc.bg_free_inodes_count);
        if bg_free_inodes as u32 > inodes_per_group {
            println!(
                "[fsck] ERROR: Group {} free inodes ({}) exceeds per-group limit ({})",
                g, bg_free_inodes, inodes_per_group
            );
            errors = true;
        }

        // Cross-check: sum of group free counts should match superblock
        // (done after the loop)
    }

    // Cross-check superblock free counts against sum of group descriptors
    let mut sum_free_blocks: u32 = 0;
    let mut sum_free_inodes: u32 = 0;
    let mut _sum_used_dirs: u32 = 0;
    for g in 0..fs.num_groups as usize {
        sum_free_blocks += u16::from_le(fs.group_descs[g].bg_free_blocks_count) as u32;
        sum_free_inodes += u16::from_le(fs.group_descs[g].bg_free_inodes_count) as u32;
        _sum_used_dirs += u16::from_le(fs.group_descs[g].bg_used_dirs_count) as u32;
    }
    let sb_free_blocks = u32::from_le(fs.superblock.s_free_blocks_count);
    let sb_free_inodes = u32::from_le(fs.superblock.s_free_inodes_count);
    if sum_free_blocks != sb_free_blocks {
        println!(
            "[fsck] WARNING: Sum of group free blocks ({}) != superblock free blocks ({})",
            sum_free_blocks, sb_free_blocks
        );
        errors = true;
    }
    if sum_free_inodes != sb_free_inodes {
        println!(
            "[fsck] WARNING: Sum of group free inodes ({}) != superblock free inodes ({})",
            sum_free_inodes, sb_free_inodes
        );
        errors = true;
    }

    if errors {
        FsckResult::ErrorsFound
    } else {
        println!("[fsck] Block group descriptors OK");
        FsckResult::Ok
    }
}

/// Check if inode is marked as used in the inode bitmap
fn is_inode_in_bitmap(fs: &mut Ext2FileSystem, group: usize, inode_num: u32) -> bool {
    let _ = fs.load_inode_bitmap(group as u32);

    let inode_offset = (inode_num - 1) % fs.inodes_per_group;
    let byte_idx = (inode_offset as usize) / 8;
    let bit_idx = (inode_offset as usize) % 8;

    if byte_idx >= 512 {
        return false;
    }

    (fs.inode_bitmaps[group][byte_idx] & (1 << bit_idx)) != 0
}

/// Cross-check block bitmaps against actual inode usage
fn check_block_allocation(fs: &mut Ext2FileSystem) -> FsckResult {
    let mut used_blocks = [false; 65536]; // Track up to 64K blocks
    let mut errors = false;
    let mut multiply_claimed = 0u32;
    let blocks_count = u32::from_le(fs.superblock.s_blocks_count);
    let inodes_count = u32::from_le(fs.superblock.s_inodes_count);

    // Walk all inodes and build used-block set
    for inode_num in EXT2_FIRST_INO..inodes_count {
        let inode = match fs.read_inode(inode_num) {
            Ok(i) => i,
            Err(_) => continue,
        };

        // Skip unallocated inodes
        let group = ((inode_num - 1) / fs.inodes_per_group) as usize;
        if group >= MAX_BLOCK_GROUPS {
            continue;
        }

        if !is_inode_in_bitmap(fs, group, inode_num) {
            continue; // Inode not allocated
        }

        // Check direct blocks
        for i in 0..EXT2_NDIR_BLOCKS {
            let block = u32::from_le(inode.i_block[i]);
            if block != 0 {
                if block >= blocks_count {
                    println!(
                        "[fsck] ERROR: Inode {} references block {} beyond filesystem",
                        inode_num, block
                    );
                    errors = true;
                    continue;
                }
                if block as usize >= used_blocks.len() {
                    continue; // Out of tracking range
                }
                if used_blocks[block as usize] {
                    multiply_claimed += 1;
                    println!(
                        "[fsck] WARNING: Multiply claimed block {} (inode {})",
                        block, inode_num
                    );
                }
                used_blocks[block as usize] = true;
            }
        }

        // Check single indirect block pointer
        let ind_block = u32::from_le(inode.i_block[EXT2_IND_BLOCK]);
        if ind_block != 0 {
            if ind_block >= blocks_count {
                println!(
                    "[fsck] ERROR: Inode {} indirect block {} beyond filesystem",
                    inode_num, ind_block
                );
                errors = true;
            } else if (ind_block as usize) < used_blocks.len() {
                if used_blocks[ind_block as usize] {
                    multiply_claimed += 1;
                }
                used_blocks[ind_block as usize] = true;
            }
        }

        // Check double indirect block pointer
        let dind_block = u32::from_le(inode.i_block[EXT2_DIND_BLOCK]);
        if dind_block != 0 {
            if dind_block >= blocks_count {
                println!(
                    "[fsck] ERROR: Inode {} double-indirect block {} beyond filesystem",
                    inode_num, dind_block
                );
                errors = true;
            } else if (dind_block as usize) < used_blocks.len() {
                if used_blocks[dind_block as usize] {
                    multiply_claimed += 1;
                }
                used_blocks[dind_block as usize] = true;
            }
        }

        // Check triple indirect block pointer
        let tind_block = u32::from_le(inode.i_block[EXT2_TIND_BLOCK]);
        if tind_block != 0 {
            if tind_block >= blocks_count {
                println!(
                    "[fsck] ERROR: Inode {} triple-indirect block {} beyond filesystem",
                    inode_num, tind_block
                );
                errors = true;
            } else if (tind_block as usize) < used_blocks.len() {
                if used_blocks[tind_block as usize] {
                    multiply_claimed += 1;
                }
                used_blocks[tind_block as usize] = true;
            }
        }
    }

    // Compare with bitmap
    let mut bitmap_errors = 0u32;
    let blocks_per_group = u32::from_le(fs.superblock.s_blocks_per_group);
    for g in 0..fs.num_groups as usize {
        let _ = fs.load_block_bitmap(g as u32);

        let bitmap = &fs.block_bitmaps[g];
        for byte_idx in 0..512 {
            for bit in 0..8 {
                let block_offset = (byte_idx * 8 + bit) as u64;
                let global_block = g as u64 * blocks_per_group as u64 + block_offset;

                if global_block >= blocks_count as u64 {
                    continue;
                }
                if global_block as usize >= used_blocks.len() {
                    continue;
                }

                let in_bitmap = (bitmap[byte_idx] & (1 << bit)) != 0;
                let actually_used = used_blocks[global_block as usize];

                if in_bitmap && !actually_used {
                    // Bitmap says used but no inode claims it
                    bitmap_errors += 1;
                } else if !in_bitmap && actually_used {
                    // Used but not in bitmap - serious error
                    errors = true;
                    bitmap_errors += 1;
                    println!(
                        "[fsck] ERROR: Block {} used but not in bitmap",
                        global_block
                    );
                }
            }
        }
    }

    if multiply_claimed > 0 {
        println!("[fsck] Found {} multiply claimed blocks", multiply_claimed);
        errors = true;
    }

    if bitmap_errors > 0 {
        println!("[fsck] {} block bitmap inconsistencies", bitmap_errors);
        errors = true;
    }

    if errors {
        FsckResult::ErrorsFound
    } else {
        println!("[fsck] Block allocation check OK");
        FsckResult::Ok
    }
}

/// Check inode bitmap consistency
fn check_inode_allocation(fs: &mut Ext2FileSystem) -> FsckResult {
    let mut errors = false;
    let mut unreachable = 0u32;
    let inodes_count = u32::from_le(fs.superblock.s_inodes_count);

    for g in 0..fs.num_groups as usize {
        let _ = fs.load_inode_bitmap(g as u32);

        // Copy bitmap to avoid holding an immutable borrow while calling fs.read_inode()
        let bitmap = fs.inode_bitmaps[g];

        for i in 0..fs.inodes_per_group {
            let inode_num = g as u32 * fs.inodes_per_group + i + 1;

            if inode_num < EXT2_FIRST_INO {
                continue; // Reserved inodes
            }
            if inode_num > inodes_count {
                break; // Beyond filesystem inodes
            }

            let byte_idx = (i as usize) / 8;
            let bit_idx = (i as usize) % 8;

            if byte_idx >= 512 {
                continue;
            }

            let in_bitmap = (bitmap[byte_idx] & (1 << bit_idx)) != 0;

            if in_bitmap {
                // Read the inode and check if it's actually in use
                let inode = match fs.read_inode(inode_num) {
                    Ok(i) => i,
                    Err(_) => continue,
                };

                // If inode appears to be zero/empty but is marked allocated
                if inode.i_mode == 0 && inode.i_size == 0 && inode.i_blocks == 0 {
                    unreachable += 1;
                    // Don't flag as error yet — could be recently deleted
                    // and not yet reclaimed
                }

                // Check for invalid mode
                let mode = u16::from_le(inode.i_mode);
                if mode != 0 {
                    let type_bits = mode & 0xF000;
                    if type_bits != 0x4000  // directory
						&& type_bits != 0x8000  // regular file
						&& type_bits != 0xA000  // symlink
						&& type_bits != 0x2000  // block device
						&& type_bits != 0x6000  // char device
						&& type_bits != 0x1000  // fifo
						&& type_bits != 0xC000
                    // socket
                    {
                        println!(
                            "[fsck] ERROR: Inode {} has invalid mode type bits: {:#06X}",
                            inode_num, type_bits
                        );
                        errors = true;
                    }
                }
            } else {
                // Inode not in bitmap — check if it has nonzero content (should be allocated)
                let inode = match fs.read_inode(inode_num) {
                    Ok(i) => i,
                    Err(_) => continue,
                };
                if inode.i_mode != 0 || u32::from_le(inode.i_size) != 0 {
                    // Inode has content but isn't marked in bitmap
                    println!(
                        "[fsck] ERROR: Inode {} has content but not in bitmap",
                        inode_num
                    );
                    errors = true;
                }
            }
        }
    }

    if unreachable > 0 {
        println!(
            "[fsck] {} potentially unreachable inodes (allocated but empty)",
            unreachable
        );
    }

    if errors {
        FsckResult::ErrorsFound
    } else {
        println!("[fsck] Inode allocation check OK");
        FsckResult::Ok
    }
}

/// Verify inode link counts against directory entries
fn check_link_counts(fs: &mut Ext2FileSystem) -> FsckResult {
    let mut errors = false;
    let inodes_count = u32::from_le(fs.superblock.s_inodes_count);

    // Allocate link count array
    let mut link_counts = [0u16; 4096]; // Track links for up to 4096 inodes

    // Walk all directories and count references
    for inode_num in EXT2_FIRST_INO..inodes_count.min(4096) {
        let group = ((inode_num - 1) / fs.inodes_per_group) as usize;
        if group >= MAX_BLOCK_GROUPS {
            continue;
        }
        if !is_inode_in_bitmap(fs, group, inode_num) {
            continue;
        }

        let inode = match fs.read_inode(inode_num) {
            Ok(i) => i,
            Err(_) => continue,
        };

        let mode = u16::from_le(inode.i_mode);
        if mode & 0xF000 != 0x4000 {
            continue; // Not a directory
        }

        // Read directory entries
        let dir_entries = match fs.read_dir(inode_num) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in &dir_entries {
            if entry.inode_id == 0 {
                continue;
            }
            let target = entry.inode_id as usize;
            if target < link_counts.len() {
                link_counts[target] = link_counts[target].saturating_add(1);
            }
        }
    }

    // Compare link counts with inodes
    for inode_num in EXT2_FIRST_INO..inodes_count.min(4096) {
        let group = ((inode_num - 1) / fs.inodes_per_group) as usize;
        if group >= MAX_BLOCK_GROUPS {
            continue;
        }
        if !is_inode_in_bitmap(fs, group, inode_num) {
            continue;
        }

        let inode = match fs.read_inode(inode_num) {
            Ok(i) => i,
            Err(_) => continue,
        };

        let stored_links = u16::from_le(inode.i_links_count);
        let counted_links = link_counts[inode_num as usize];

        if stored_links != counted_links && counted_links > 0 {
            println!(
                "[fsck] WARNING: Inode {} link count {} != counted {}",
                inode_num, stored_links, counted_links
            );
            errors = true;
        }
    }

    // Check root inode specifically
    let root_inode = match fs.read_inode(EXT2_ROOT_INO) {
        Ok(i) => i,
        Err(_) => {
            println!("[fsck] ERROR: Cannot read root inode");
            return FsckResult::FatalError;
        }
    };
    let root_links = u16::from_le(root_inode.i_links_count);
    if root_links < 2 {
        println!("[fsck] WARNING: Root inode link count {} < 2", root_links);
        errors = true;
    }

    if errors {
        FsckResult::ErrorsFound
    } else {
        println!("[fsck] Link counts OK");
        FsckResult::Ok
    }
}

/// Check directory structure consistency
fn check_directory_structure(fs: &mut Ext2FileSystem) -> FsckResult {
    let mut errors = false;
    let inodes_count = u32::from_le(fs.superblock.s_inodes_count);

    for inode_num in EXT2_FIRST_INO..inodes_count.min(4096) {
        let group = ((inode_num - 1) / fs.inodes_per_group) as usize;
        if group >= MAX_BLOCK_GROUPS {
            continue;
        }
        if !is_inode_in_bitmap(fs, group, inode_num) {
            continue;
        }

        let inode = match fs.read_inode(inode_num) {
            Ok(i) => i,
            Err(_) => continue,
        };

        let mode = u16::from_le(inode.i_mode);
        if mode & 0xF000 != 0x4000 {
            continue; // Not a directory
        }

        let dir_entries = match fs.read_dir(inode_num) {
            Ok(e) => e,
            Err(_) => {
                println!("[fsck] ERROR: Cannot read directory inode {}", inode_num);
                errors = true;
                continue;
            }
        };

        // Check that directory has . and .. entries
        let mut has_dot = false;
        let mut has_dotdot = false;
        for entry in &dir_entries {
            if entry.inode_id == 0 {
                continue;
            }
            if entry.name_len == 1 && entry.name[0] == b'.' {
                has_dot = true;
                // . should point to self
                if entry.inode_id != inode_num as u64 {
                    println!(
                        "[fsck] ERROR: Directory inode {} '.' points to {} instead of self",
                        inode_num, entry.inode_id
                    );
                    errors = true;
                }
            }
            if entry.name_len == 2 && entry.name[0] == b'.' && entry.name[1] == b'.' {
                has_dotdot = true;
            }
        }

        if !has_dot {
            println!(
                "[fsck] WARNING: Directory inode {} missing '.' entry",
                inode_num
            );
            errors = true;
        }
        if !has_dotdot {
            println!(
                "[fsck] WARNING: Directory inode {} missing '..' entry",
                inode_num
            );
            errors = true;
        }
    }

    if errors {
        FsckResult::ErrorsFound
    } else {
        println!("[fsck] Directory structure check OK");
        FsckResult::Ok
    }
}

/// Repair mode: fix bitmap inconsistencies
fn repair_bitmaps(fs: &mut Ext2FileSystem) -> FsckResult {
    println!("[fsck] Running repair mode...");
    let mut repaired = 0u32;
    let blocks_count = u32::from_le(fs.superblock.s_blocks_count);
    let inodes_count = u32::from_le(fs.superblock.s_inodes_count);
    let blocks_per_group = u32::from_le(fs.superblock.s_blocks_per_group);

    // Rebuild block bitmaps from inode block references
    for g in 0..fs.num_groups as usize {
        let _ = fs.load_block_bitmap(g as u32);
    }

    // Collect which blocks are actually used by inodes
    let mut used_blocks = [false; 65536];
    for inode_num in EXT2_FIRST_INO..inodes_count {
        let group = ((inode_num - 1) / fs.inodes_per_group) as usize;
        if group >= MAX_BLOCK_GROUPS {
            continue;
        }
        if !is_inode_in_bitmap(fs, group, inode_num) {
            continue;
        }
        let inode = match fs.read_inode(inode_num) {
            Ok(i) => i,
            Err(_) => continue,
        };

        for i in 0..EXT2_NDIR_BLOCKS {
            let block = u32::from_le(inode.i_block[i]);
            if block != 0 && (block as usize) < used_blocks.len() {
                used_blocks[block as usize] = true;
            }
        }
        let ind = u32::from_le(inode.i_block[EXT2_IND_BLOCK]);
        if ind != 0 && (ind as usize) < used_blocks.len() {
            used_blocks[ind as usize] = true;
        }
        let dind = u32::from_le(inode.i_block[EXT2_DIND_BLOCK]);
        if dind != 0 && (dind as usize) < used_blocks.len() {
            used_blocks[dind as usize] = true;
        }
        let tind = u32::from_le(inode.i_block[EXT2_TIND_BLOCK]);
        if tind != 0 && (tind as usize) < used_blocks.len() {
            used_blocks[tind as usize] = true;
        }
    }

    // Fix block bitmaps: mark used blocks, clear unused
    for g in 0..fs.num_groups as usize {
        let _ = fs.load_block_bitmap(g as u32);
        let mut bitmap = fs.block_bitmaps[g];

        for byte_idx in 0..512 {
            for bit in 0..8 {
                let block_offset = (byte_idx * 8 + bit) as u64;
                let global_block = g as u64 * blocks_per_group as u64 + block_offset;
                if global_block >= blocks_count as u64 || global_block as usize >= used_blocks.len()
                {
                    continue;
                }
                let should_be_set = used_blocks[global_block as usize];
                let is_set = (bitmap[byte_idx] & (1 << bit)) != 0;
                if should_be_set != is_set {
                    if should_be_set {
                        bitmap[byte_idx] |= 1 << bit;
                    } else {
                        bitmap[byte_idx] &= !(1 << bit);
                    }
                    repaired += 1;
                }
            }
        }

        // Write back corrected bitmap
        fs.block_bitmaps[g] = bitmap;
        // Write the bitmap to disk
        let gdesc = &fs.group_descs[g];
        let bitmap_block = u32::from_le(gdesc.bg_block_bitmap);
        let bitmap_offset = bitmap_block as u64 * fs.block_size as u64;
        let bitmap_size = fs.block_size as usize;
        let write_len = bitmap_size.min(512);
        // Use dev_write through the fs methods
        // We need to reconstruct the full block for writing
        let mut block_buf = [0u8; 4096];
        block_buf[..write_len].copy_from_slice(&bitmap[..write_len]);
        // Direct device write
        let _ = crate::fs::ext2::dev_write(
            fs.device_index,
            fs.partition_start,
            bitmap_offset,
            &block_buf[..bitmap_size],
        );
    }

    // Recalculate free block count
    let mut total_free: u32 = 0;
    for g in 0..fs.num_groups as usize {
        let bitmap = &fs.block_bitmaps[g];
        let group_blocks = if g as u32 == fs.num_groups - 1 {
            blocks_count - (g as u32 * blocks_per_group)
        } else {
            blocks_per_group
        };
        for b in 0..group_blocks {
            let byte_idx = (b / 8) as usize;
            let bit_idx = (b % 8) as usize;
            if byte_idx < 512 && (bitmap[byte_idx] & (1 << bit_idx)) == 0 {
                total_free += 1;
            }
        }
    }
    let old_free = u32::from_le(fs.superblock.s_free_blocks_count);
    if old_free != total_free {
        println!(
            "[fsck] Repaired free block count: {} -> {}",
            old_free, total_free
        );
        fs.superblock.s_free_blocks_count = total_free.to_le();
        repaired += 1;
    }

    println!("[fsck] Repair completed: {} fixes applied", repaired);

    if repaired > 0 {
        FsckResult::Repaired
    } else {
        FsckResult::Ok
    }
}

/// Main fsck entry point
pub fn fsck(device_index: usize, partition_start: u64, mode: FsckMode) -> FsckResult {
    println!("[fsck] Starting ext2 filesystem check...");
    println!(
        "[fsck] Device {}, partition at {:#018x}",
        device_index, partition_start
    );

    let mut fs = match Ext2FileSystem::new(device_index, partition_start) {
        Ok(fs) => fs,
        Err(_) => {
            println!("[fsck] ERROR: Cannot open filesystem");
            return FsckResult::FatalError;
        }
    };

    let mut overall = FsckResult::Ok;

    // Run checks
    let result = check_superblock(&mut fs);
    if result == FsckResult::FatalError {
        return result;
    }
    if result != FsckResult::Ok {
        overall = result;
    }

    let result = check_group_descs(&mut fs);
    if result != FsckResult::Ok {
        overall = result;
    }

    let result = check_block_allocation(&mut fs);
    if result != FsckResult::Ok {
        overall = result;
    }

    let result = check_inode_allocation(&mut fs);
    if result != FsckResult::Ok && overall == FsckResult::Ok {
        overall = result;
    }

    let result = check_link_counts(&mut fs);
    if result != FsckResult::Ok && overall == FsckResult::Ok {
        overall = result;
    }

    let result = check_directory_structure(&mut fs);
    if result != FsckResult::Ok && overall == FsckResult::Ok {
        overall = result;
    }

    // Repair mode
    if mode == FsckMode::Repair && overall != FsckResult::Ok {
        overall = repair_bitmaps(&mut fs);
    }

    match overall {
        FsckResult::Ok => println!("[fsck] No errors found"),
        FsckResult::ErrorsFound => println!("[fsck] Errors found but not repaired"),
        FsckResult::Repaired => println!("[fsck] Errors repaired"),
        FsckResult::FatalError => println!("[fsck] Fatal error during check"),
    }

    overall
}

/// Initialize the fsck module
pub fn init() {
    println!("[fsck] ext2 filesystem checker initialized");
}
