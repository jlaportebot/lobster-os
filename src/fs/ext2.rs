//! ext2 filesystem driver for block devices
//!
//! Implements the Second Extended Filesystem as described in the Linux kernel
//! documentation and the ext2 design book. Supports:
//! - Superblock reading and validation
//! - Block group descriptor table parsing
//! - Inode read/write via inode table
//! - Directory entry enumeration
//! - Block and inode bitmap allocation/deallocation
//! - Simple journal (write-ahead log) for crash consistency
//! - Integration with the VFS layer
//!
//! Limitations (embedded-friendly):
//! - Max 128 block groups (sufficient for volumes up to ~4 GB with 4K blocks)
//! - In-memory caches for superblock, group descriptors, bitmaps
//! - Indirect blocks: single, double, and triple indirect support
//! - Journal is a simple circular redo log, not full ext3 journaling

use super::vfs::*;
use crate::arch::timer::Timer;
#[allow(dead_code, unused_imports)]
use crate::println;

// ═══════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════

/// ext2 magic number
const EXT2_MAGIC: u16 = 0xEF53;

/// Default inode size (128 bytes)
const EXT2_INODE_SIZE: u16 = 128;

/// Root inode number
const EXT2_ROOT_INO: u32 = 2;

/// First non-reserved inode
const EXT2_FIRST_INO: u32 = 11;

/// Maximum block groups supported
pub const MAX_BLOCK_GROUPS: usize = 128;

/// Maximum in-memory inodes cached
const INODE_CACHE_SIZE: usize = 32;

/// Maximum journal entries
const JOURNAL_SIZE: usize = 64;

/// Maximum journal data per entry (one block = 4096 bytes)
const JOURNAL_BLOCK_SIZE: usize = 4096;

/// Journal superblock magic number
const JOURNAL_SUPERBLOCK_MAGIC: u32 = 0xC03B3998;

/// Journal commit record magic number
const JOURNAL_COMMIT_MAGIC: u32 = 0xC04B4A4C; // "COJL" (CoBall Journal Log)

/// Direct block count in inode
pub const EXT2_NDIR_BLOCKS: usize = 12;

/// Indirect block index in i_block array
pub const EXT2_IND_BLOCK: usize = 12;

/// Double-indirect block index in i_block array
pub const EXT2_DIND_BLOCK: usize = 13;

/// Triple-indirect block index in i_block array
pub const EXT2_TIND_BLOCK: usize = 14;

/// Maximum entries per indirect block (4K block / 4 bytes per entry)
const EXT2_ADDR_PER_BLOCK: u64 = 1024;

// ═══════════════════════════════════════════════════════════════
// On-disk structures (little-endian, packed)
// ═══════════════════════════════════════════════════════════════

/// ext2 superblock (1024 bytes, located at byte offset 1024)
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct Ext2Superblock {
    pub s_inodes_count: u32,
    pub s_blocks_count: u32,
    pub s_r_blocks_count: u32,
    pub s_free_blocks_count: u32,
    pub s_free_inodes_count: u32,
    pub s_first_data_block: u32,
    pub s_log_block_size: u32,
    pub s_log_frag_size: u32,
    pub s_blocks_per_group: u32,
    pub s_frags_per_group: u32,
    pub s_inodes_per_group: u32,
    pub s_mtime: u32,
    pub s_wtime: u32,
    pub s_mnt_count: u16,
    pub s_max_mnt_count: u16,
    pub s_magic: u16,
    pub s_state: u16,
    pub s_errors: u16,
    pub s_minor_rev_level: u16,
    pub s_lastcheck: u32,
    pub s_checkinterval: u32,
    pub s_creator_os: u32,
    pub s_rev_level: u32,
    pub s_def_resuid: u16,
    pub s_def_resgid: u16,
    // Revision 1 fields
    pub s_first_ino: u32,
    pub s_inode_size: u16,
    pub s_block_group_nr: u16,
    pub s_feature_compat: u32,
    pub s_feature_incompat: u32,
    pub s_feature_ro_compat: u32,
    pub s_uuid: [u8; 16],
    pub s_volume_name: [u8; 16],
    pub s_last_mounted: [u8; 64],
    pub s_algo_bitmap: u32,
    // Preallocation fields (revision 1)
    pub s_prealloc_blocks: u8,
    pub s_prealloc_dir_blocks: u8,
    pub s_reserved_gdt_blocks: u16,
    // Journaling fields (revision 1 + journal feature)
    pub s_journal_uuid: [u8; 16],
    pub s_journal_inum: u32,
    pub s_journal_dev: u32,
    pub s_last_orphan: u32,
    // Hash tree stuff
    pub s_hash_seed: [u32; 4],
    pub s_def_hash_version: u8,
    pub s_jnl_backup_type: u8,
    pub s_desc_size: u16,
    pub s_default_mount_opts: u32,
    pub s_first_meta_bg: u32,
    pub s_mkfs_time: u32,
    pub s_jnl_blocks: [u32; 17],
}

/// Block group descriptor (32 bytes)
#[repr(C, packed)]
#[derive(Copy, Clone, Default)]
pub struct Ext2GroupDesc {
    pub bg_block_bitmap: u32,
    pub bg_inode_bitmap: u32,
    pub bg_inode_table: u32,
    pub bg_free_blocks_count: u16,
    pub bg_free_inodes_count: u16,
    pub bg_used_dirs_count: u16,
    pub bg_pad: u16,
    pub bg_reserved: [u32; 3],
}

/// ext2 inode (128 bytes)
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct Ext2Inode {
    pub i_mode: u16,
    pub i_uid: u16,
    pub i_size: u32,
    pub i_atime: u32,
    pub i_ctime: u32,
    pub i_mtime: u32,
    pub i_dtime: u32,
    pub i_gid: u16,
    pub i_links_count: u16,
    pub i_blocks: u32,
    pub i_flags: u32,
    pub i_osd1: u32,
    pub i_block: [u32; 15], // 12 direct + 1 indirect + 1 double + 1 triple
    pub i_generation: u32,
    pub i_file_acl: u32,
    pub i_dir_acl: u32,
    pub i_faddr: u32,
    pub i_osd2: [u8; 12],
}

/// ext2 directory entry (variable length, minimum 12 bytes)
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct Ext2DirEntry {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
    // name follows (up to 255 bytes)
}

/// Directory entry file types
const EXT2_FT_UNKNOWN: u8 = 0;
const EXT2_FT_REG_FILE: u8 = 1;
const EXT2_FT_DIR: u8 = 2;
const EXT2_FT_CHRDEV: u8 = 3;
const EXT2_FT_BLKDEV: u8 = 4;
const EXT2_FT_FIFO: u8 = 5;
const EXT2_FT_SOCK: u8 = 6;
const EXT2_FT_SYMLINK: u8 = 7;

/// Inode mode bits
const EXT2_S_IFMT: u16 = 0xF000;
const EXT2_S_IFREG: u16 = 0x8000;
const EXT2_S_IFDIR: u16 = 0x4000;
const EXT2_S_IFLNK: u16 = 0xA000;
const EXT2_S_IRUSR: u16 = 0x0100;
const EXT2_S_IWUSR: u16 = 0x0080;
const EXT2_S_IXUSR: u16 = 0x0040;

// ═══════════════════════════════════════════════════════════════
// In-memory structures
// ═══════════════════════════════════════════════════════════════

/// Cached inode entry
#[derive(Copy, Clone)]
pub(crate) struct InodeCacheEntry {
    inode_num: u32,
    inode: Ext2Inode,
    dirty: bool,
    valid: bool,
}

/// Journal transaction states
#[derive(Copy, Clone, Debug, PartialEq)]
enum JournalTransactionState {
    Running,    // Transaction is in progress, entries being buffered
    Committing, // Currently flushing to disk
    Committed,  // Successfully committed to journal area
    Aborted,    // Transaction was aborted
}

/// Journal entry types
#[derive(Copy, Clone, Debug, PartialEq)]
enum JournalEntryType {
    BlockWrite,  // Write a full block
    InodeUpdate, // Update an inode
    None,        // Empty slot
}

/// A single journal entry
#[derive(Copy, Clone)]
struct JournalEntry {
    entry_type: JournalEntryType,
    block_num: u64,  // Block number being written
    inode_num: u32,  // Inode number (for inode updates)
    data_len: usize, // Length of data
    sequence: u32,   // Sequence number for ordering
}

/// Journal superblock structure
#[repr(C, packed)]
#[derive(Copy, Clone)]
struct JournalSuperblock {
    magic: u32,       // JOURNAL_SUPERBLOCK_MAGIC (0xC03B3998)
    sequence: u32,    // Last committed transaction sequence
    start_block: u32, // First block of journal area
    num_entries: u32, // Total journal entries
    pad: [u8; 4084],  // Pad to 4096 bytes
}

const JOURNAL_AREA_BLOCKS: u32 = 64; // Reserved journal area at end of FS

/// Buffered journal write entry
#[derive(Copy, Clone)]
struct BufferedJournalEntry {
    entry_type: JournalEntryType,
    block_num: u64,
    inode_num: u32,
    data: [u8; JOURNAL_BLOCK_SIZE],
    valid: bool,
}

/// ext2 filesystem instance
pub struct Ext2FileSystem {
    pub fs_id: u32,
    pub device_index: usize,
    pub partition_start: u64,
    pub block_size: u32,
    pub blocks_per_group: u32,
    pub inodes_per_group: u32,
    pub inode_size: u16,
    pub num_groups: u32,
    pub superblock: Ext2Superblock,
    pub group_descs: [Ext2GroupDesc; MAX_BLOCK_GROUPS],
    pub inode_cache: [InodeCacheEntry; INODE_CACHE_SIZE],
    pub inode_cache_idx: usize,
    /// Block bitmap cache (one per group, stored as bit arrays)
    pub block_bitmaps: [[u8; 512]; MAX_BLOCK_GROUPS], // 512 bytes = 4096 bits per group
    pub inode_bitmaps: [[u8; 512]; MAX_BLOCK_GROUPS],
    pub bitmap_loaded: [bool; MAX_BLOCK_GROUPS],
    /// Simple write-ahead journal
    journal_entries: [JournalEntry; JOURNAL_SIZE],
    journal_data: [[u8; JOURNAL_BLOCK_SIZE]; JOURNAL_SIZE],
    journal_head: usize,
    journal_tail: usize,
    journal_seq: u32,
    journal_enabled: bool,
    /// Transaction state for WAL
    journal_transaction_state: JournalTransactionState,
    /// Current transaction ID
    journal_transaction_id: u32,
    /// Buffered writes during transaction
    buffered_writes: [BufferedJournalEntry; JOURNAL_SIZE],
    num_buffered: usize,
}

static mut EXT2_INSTANCE: Option<Ext2FileSystem> = None;

/// Expand an ext2-backed directory: read its entries and materialize VFS
/// child nodes (once per directory per boot). Called lazily by the VFS on
/// first traversal into a backing-store directory.
pub fn expand_dir(ino: u32, parent_node_id: u64, fs_id: u32) -> Result<(), VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;
        let entries = fs.read_dir(ino)?;
        for entry in &entries {
            if entry.name_len == 0 {
                break;
            }
            let name = entry.name_str();
            if name == "." || name == ".." {
                continue;
            }
            // Skip if the child already exists in the VFS.
            if let Some(parent) = crate::fs::vfs::get_node(parent_node_id) {
                if parent.find_child(name).is_some() {
                    continue;
                }
            }
            if let Ok(node_id) = crate::fs::vfs::alloc_node(name, entry.file_type, parent_node_id) {
                crate::fs::vfs::set_node_backing(node_id, fs_id, entry.inode_id as u32);
            }
        }
    }
    Ok(())
}

/// Whether a VFS node's backing directory has been expanded already.
pub fn dir_expanded(node_id: u64) -> bool {
    crate::fs::vfs::node_dir_expanded(node_id)
}

/// Read a file's full contents from the mounted ext2 instance (backing store).
pub fn read_backing_file(ino: u32) -> Result<alloc::vec::Vec<u8>, VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;
        let inode = fs.read_inode(ino)?;
        let size = u32::from_le(inode.i_size) as usize;
        let mut buf = alloc::vec::Vec::with_capacity(size);
        unsafe {
            buf.set_len(size);
        }
        fs.read_file(ino, 0, &mut buf)?;
        Ok(buf)
    }
}

/// Create a new file in the ext2 backing store: allocate an inode, link it
/// into the parent directory, write `data`, and return the new inode number.
pub fn create_backing_file(parent_ino: u32, name: &str, data: &[u8]) -> Result<u32, VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;

        // 1. New inode
        let ino = fs.alloc_inode()?;
        {
            let mut inode = fs.read_inode(ino)?;
            let mode: u16 = 0o100644; // regular file, rw-r--r--
            inode.i_mode = mode.to_le();
            inode.i_size = 0u32.to_le();
            inode.i_blocks = 0u32.to_le();
            inode.i_links_count = 1u16.to_le();
            const EXT2_NBLOCKS: usize = 15;
            for i in 0..EXT2_NBLOCKS {
                inode.i_block[i] = 0u32.to_le();
            }
            fs.write_inode(ino, &inode)?;
        }

        // 2. Link into parent directory
        fs.add_dir_entry(parent_ino, ino, name, EXT2_FT_REG_FILE)?;

        // 3. Write the data
        fs.write_file(ino, 0, data)?;
        fs.journal_commit()?;
        Ok(ino)
    }
}

/// Remove a file (or empty dir) from the ext2 backing store: zero its dir
/// entry in the parent (keeping rec_len so the block stays traversable)
/// and mark the inode free in the bitmap.
pub fn remove_backing_file(parent_ino: u32, name: &str) -> Result<(), VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;
        let entries = fs.read_dir(parent_ino)?;
        // Find the entry's inode by name
        let mut target_ino = 0u32;
        for e in entries.iter() {
            if e.name_len == 0 {
                break;
            }
            if e.name_str() == name {
                target_ino = e.inode_id as u32;
                break;
            }
        }
        if target_ino == 0 {
            return Err(VfsError::NotFound);
        }

        // Rewrite the parent dir block(s), zeroing the matching entry's inode
        // and name while preserving rec_len.
        let parent = fs.read_inode(parent_ino)?;
        let dir_size = u32::from_le(parent.i_size) as usize;
        let total_blocks = (dir_size + fs.block_size as usize - 1) / fs.block_size as usize;
        for block_idx in 0..total_blocks {
            let bn = fs.resolve_block(&parent, block_idx as u64)?;
            if bn == 0 {
                continue;
            }
            let mut block_buf = [0u8; 4096];
            read_block(fs, bn, &mut block_buf[..fs.block_size as usize])?;
            let bs = fs.block_size as usize;
            let mut pos = 0usize;
            let mut changed = false;
            while pos + 12 <= bs {
                let rec_len = u16::from_le(unsafe { *(block_buf.as_ptr().add(pos + 4) as *const u16) }) as usize;
                if rec_len < 12 {
                    break;
                }
                let name_len = block_buf[pos + 6] as usize;
                let ino_le = u32::from_le(unsafe { *(block_buf.as_ptr().add(pos) as *const u32) });
                if ino_le == target_ino && name_len > 0 && name_len <= 255 && pos + 8 + name_len <= bs {
                    let existing = core::str::from_utf8(&block_buf[pos + 8..pos + 8 + name_len]).unwrap_or("");
                    if existing == name {
                        // Zero the entry (inode 0 = unused) but keep rec_len.
                        for b in block_buf[pos..pos + 8 + name_len].iter_mut() {
                            *b = 0;
                        }
                        // restore rec_len
                        block_buf[pos + 4] = (rec_len & 0xFF) as u8;
                        block_buf[pos + 5] = (rec_len >> 8) as u8;
                        changed = true;
                        break;
                    }
                }
                pos += rec_len;
            }
            if changed {
                write_block(fs, bn, &block_buf[..bs])?;
                break;
            }
        }

        // Free the inode in the bitmap
        fs.free_inode(target_ino)?;
        fs.journal_commit()?;
        Ok(())
    }
}

/// Rename in the backing store = remove old dir entry + add new one with
/// the same inode. Same-parent rename only (fm renames within the cwd).
pub fn rename_backing_file(parent_ino: u32, old_name: &str, new_name: &str) -> Result<(), VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;
        let entries = fs.read_dir(parent_ino)?;
        let mut target_ino = 0u32;
        for e in entries.iter() {
            if e.name_len == 0 {
                break;
            }
            if e.name_str() == old_name {
                target_ino = e.inode_id as u32;
                break;
            }
        }
        if target_ino == 0 {
            return Err(VfsError::NotFound);
        }

        let ft = {
            let inode = fs.read_inode(target_ino)?;
            let mode = u16::from_le(inode.i_mode);
            if mode & EXT2_S_IFDIR != 0 { EXT2_FT_DIR } else { EXT2_FT_REG_FILE }
        };

        // Remove the old entry, then link the same inode under the new name.
        remove_backing_file(parent_ino, old_name)?;
        fs.add_dir_entry(parent_ino, target_ino, new_name, ft)?;
        fs.journal_commit()?;
        Ok(())
    }
}

/// Create a directory in the ext2 backing store under `parent_ino`.
pub fn create_backing_dir(parent_ino: u32, name: &str) -> Result<u32, VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;

        let ino = fs.alloc_inode()?;
        {
            let mut inode = fs.read_inode(ino)?;
            inode.i_mode = (EXT2_S_IFDIR | 0o755).to_le();
            inode.i_size = 0u32.to_le();
            inode.i_blocks = 0u32.to_le();
            inode.i_links_count = 2u16.to_le(); // . + parent
            for bi in 0..15usize {
                inode.i_block[bi] = 0u32.to_le();
            }
            fs.write_inode(ino, &inode)?;
        }

        fs.add_dir_entry(parent_ino, ino, name, EXT2_FT_DIR)?;
        fs.journal_commit()?;
        Ok(ino)
    }
}

/// Write a file's full contents back through the mounted ext2 instance.
pub fn write_backing_file(ino: u32, data: &[u8]) -> Result<(), VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;
        fs.write_file(ino, 0, data)?;
        fs.journal_commit()?;
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// Helper: block device I/O
// ═══════════════════════════════════════════════════════════════

/// Read from the block device at a given byte offset + length.
/// Translates byte offsets to sector numbers and handles partial sector reads.
fn dev_read(
    device_index: usize,
    partition_start: u64,
    byte_offset: u64,
    buf: &mut [u8],
) -> Result<(), VfsError> {
    let dev = crate::fs::block_dev::get_device(device_index).ok_or(VfsError::IOError)?;
    let sector_size = 512u64;
    let first_sector = byte_offset / sector_size;
    let offset_in_sector = (byte_offset % sector_size) as usize;
    let mut bytes_copied = 0usize;
    let mut current_sector = first_sector;

    while bytes_copied < buf.len() {
        let abs_sector = partition_start + current_sector;
        let mut sector_buf = [0u8; 512];
        if dev.read_block(abs_sector, &mut sector_buf).is_err() {
            return Err(VfsError::IOError);
        }

        let src_start = if current_sector == first_sector {
            offset_in_sector
        } else {
            0
        };
        let to_copy = (512 - src_start).min(buf.len() - bytes_copied);
        buf[bytes_copied..bytes_copied + to_copy]
            .copy_from_slice(&sector_buf[src_start..src_start + to_copy]);
        bytes_copied += to_copy;
        current_sector += 1;

        if bytes_copied >= buf.len() {
            break;
        }
    }

    Ok(())
}

/// Write to the block device at a given byte offset + length.
pub fn dev_write(
    device_index: usize,
    partition_start: u64,
    byte_offset: u64,
    buf: &[u8],
) -> Result<(), VfsError> {
    let dev = crate::fs::block_dev::get_device(device_index).ok_or(VfsError::IOError)?;
    let sector_size = 512u64;
    let first_sector = byte_offset / sector_size;
    let offset_in_sector = (byte_offset % sector_size) as usize;
    let mut bytes_written = 0usize;
    let mut current_sector = first_sector;

    while bytes_written < buf.len() {
        let abs_sector = partition_start + current_sector;
        let mut sector_buf = [0u8; 512];

        // Read-modify-write for partial sectors
        if offset_in_sector != 0 || (buf.len() - bytes_written) < 512 {
            if dev.read_block(abs_sector, &mut sector_buf).is_err() {
                // If we can't read the sector, assume it's new and zeroed
                sector_buf.fill(0);
            }
        }

        let dst_start = if current_sector == first_sector {
            offset_in_sector
        } else {
            0
        };
        let to_copy = (512 - dst_start).min(buf.len() - bytes_written);
        sector_buf[dst_start..dst_start + to_copy]
            .copy_from_slice(&buf[bytes_written..bytes_written + to_copy]);

        if dev.write_block(abs_sector, &sector_buf).is_err() {
            return Err(VfsError::IOError);
        }

        bytes_written += to_copy;
        current_sector += 1;
    }

    Ok(())
}

/// Read a single block (block_size bytes) from the filesystem
fn read_block(fs: &Ext2FileSystem, block_num: u64, buf: &mut [u8]) -> Result<(), VfsError> {
    let byte_offset = if fs.superblock.s_first_data_block == 0 {
        block_num * fs.block_size as u64
    } else {
        block_num * fs.block_size as u64
    };
    if buf.len() < fs.block_size as usize {
        return Err(VfsError::IOError);
    }
    dev_read(
        fs.device_index,
        fs.partition_start,
        byte_offset,
        &mut buf[..fs.block_size as usize],
    )
}

/// Write a single block to the filesystem
fn write_block(fs: &Ext2FileSystem, block_num: u64, buf: &[u8]) -> Result<(), VfsError> {
    let byte_offset = block_num * fs.block_size as u64;
    let len = buf.len().min(fs.block_size as usize);
    dev_write(
        fs.device_index,
        fs.partition_start,
        byte_offset,
        &buf[..len],
    )
}

// ═══════════════════════════════════════════════════════════════
// Ext2FileSystem implementation
// ═══════════════════════════════════════════════════════════════

impl Ext2FileSystem {
    /// Create a new ext2 filesystem instance by reading the superblock
    /// and group descriptor table from the block device.
    pub fn new(device_index: usize, partition_start: u64) -> Result<Self, VfsError> {
        // Read superblock (at byte offset 1024)
        let mut sb_buf = [0u8; 1024];
        dev_read(device_index, partition_start, 1024, &mut sb_buf)?;

        let superblock = unsafe { *(sb_buf.as_ptr() as *const Ext2Superblock) };

        // Validate magic number
        let magic = u16::from_le(superblock.s_magic);
        if magic != EXT2_MAGIC {
            println!(
                "[ext2] Invalid magic: {:#06X}, expected {:#06X}",
                magic, EXT2_MAGIC
            );
            return Err(VfsError::IOError);
        }

        let block_size = 1024u32 << superblock.s_log_block_size;
        let inode_size = if superblock.s_rev_level == 0 {
            EXT2_INODE_SIZE
        } else {
            u16::from_le(superblock.s_inode_size)
        };

        let blocks_per_group = u32::from_le(superblock.s_blocks_per_group);
        let inodes_per_group = u32::from_le(superblock.s_inodes_per_group);
        let blocks_count = u32::from_le(superblock.s_blocks_count);
        let num_groups = (blocks_count + blocks_per_group - 1) / blocks_per_group;

        println!("[ext2] Superblock valid:");
        println!("[ext2]   Block size: {} bytes", block_size);
        println!("[ext2]   Inode size: {} bytes", inode_size);
        println!(
            "[ext2]   Blocks: {}, Free: {}",
            blocks_count,
            u32::from_le(superblock.s_free_blocks_count)
        );
        println!(
            "[ext2]   Inodes: {}, Free: {}",
            u32::from_le(superblock.s_inodes_count),
            u32::from_le(superblock.s_free_inodes_count)
        );
        println!(
            "[ext2]   Blocks/group: {}, Inodes/group: {}",
            blocks_per_group, inodes_per_group
        );
        println!("[ext2]   Block groups: {}", num_groups);
        println!(
            "[ext2]   Volume: {}",
            core::str::from_utf8(&superblock.s_volume_name).unwrap_or("?")
        );

        // Read block group descriptor table
        // Located in the block after the superblock
        let gdt_block = if block_size == 1024 { 2 } else { 1 };
        let num_groups_usize = num_groups as usize;
        let gdt_size = num_groups_usize * core::mem::size_of::<Ext2GroupDesc>();
        let mut gdt_buf = [0u8; 4096]; // Enough for 128 group descriptors
        if gdt_size > gdt_buf.len() {
            println!("[ext2] Too many block groups ({})", num_groups);
            return Err(VfsError::OutOfMemory);
        }

        let gdt_byte_offset = gdt_block as u64 * block_size as u64;
        dev_read(
            device_index,
            partition_start,
            gdt_byte_offset,
            &mut gdt_buf[..gdt_size],
        )?;

        let mut group_descs: [Ext2GroupDesc; MAX_BLOCK_GROUPS] = [Ext2GroupDesc {
            bg_block_bitmap: 0,
            bg_inode_bitmap: 0,
            bg_inode_table: 0,
            bg_free_blocks_count: 0,
            bg_free_inodes_count: 0,
            bg_used_dirs_count: 0,
            bg_pad: 0,
            bg_reserved: [0; 3],
        }; MAX_BLOCK_GROUPS];

        for i in 0..num_groups_usize {
            let src = unsafe { &*(gdt_buf[i * 32..].as_ptr() as *const Ext2GroupDesc) };
            group_descs[i] = *src;
        }

        let fs_id = register_fs();

        // Initialize the filesystem instance
        let fs = Ext2FileSystem {
            fs_id,
            device_index,
            partition_start,
            block_size,
            blocks_per_group,
            inodes_per_group,
            inode_size,
            num_groups,
            superblock,
            group_descs,
            inode_cache: [InodeCacheEntry {
                inode_num: 0,
                inode: unsafe { core::mem::zeroed() },
                dirty: false,
                valid: false,
            }; INODE_CACHE_SIZE],
            inode_cache_idx: 0,
            block_bitmaps: [[0u8; 512]; MAX_BLOCK_GROUPS],
            inode_bitmaps: [[0u8; 512]; MAX_BLOCK_GROUPS],
            bitmap_loaded: [false; MAX_BLOCK_GROUPS],
            journal_entries: [JournalEntry {
                entry_type: JournalEntryType::None,
                block_num: 0,
                inode_num: 0,
                data_len: 0,
                sequence: 0,
            }; JOURNAL_SIZE],
            journal_data: [[0u8; JOURNAL_BLOCK_SIZE]; JOURNAL_SIZE],
            journal_head: 0,
            journal_tail: 0,
            journal_seq: 0,
            journal_enabled: true,
            journal_transaction_state: JournalTransactionState::Aborted,
            journal_transaction_id: 0,
            buffered_writes: [BufferedJournalEntry {
                entry_type: JournalEntryType::None,
                block_num: 0,
                inode_num: 0,
                data: [0u8; JOURNAL_BLOCK_SIZE],
                valid: false,
            }; JOURNAL_SIZE],
            num_buffered: 0,
        };

        Ok(fs)
    }

    /// Get the filesystem ID
    pub fn id(&self) -> u32 {
        self.fs_id
    }

    // ─── Inode operations ──────────────────────────────────────────

    /// Read an inode from disk by number (1-based)
    pub fn read_inode(&mut self, inode_num: u32) -> Result<Ext2Inode, VfsError> {
        if inode_num == 0 {
            return Err(VfsError::NotFound);
        }

        // Check cache first
        for entry in &self.inode_cache {
            if entry.valid && entry.inode_num == inode_num {
                return Ok(entry.inode);
            }
        }

        // Calculate location: which block group, which table index
        let group = (inode_num - 1) / self.inodes_per_group;
        let index = (inode_num - 1) % self.inodes_per_group;

        if group as usize >= MAX_BLOCK_GROUPS {
            return Err(VfsError::IOError);
        }

        let gdesc = &self.group_descs[group as usize];
        let inode_table_block = u32::from_le(gdesc.bg_inode_table);
        let inode_offset = inode_table_block as u64 * self.block_size as u64
            + index as u64 * self.inode_size as u64;

        let mut inode_buf = [0u8; 128];
        dev_read(
            self.device_index,
            self.partition_start,
            inode_offset,
            &mut inode_buf,
        )?;

        let inode = unsafe { *(inode_buf.as_ptr() as *const Ext2Inode) };

        // Cache the inode
        self.inode_cache[self.inode_cache_idx] = InodeCacheEntry {
            inode_num,
            inode,
            dirty: false,
            valid: true,
        };
        self.inode_cache_idx = (self.inode_cache_idx + 1) % INODE_CACHE_SIZE;

        Ok(inode)
    }

    /// Write an inode back to disk
    pub fn write_inode(&mut self, inode_num: u32, inode: &Ext2Inode) -> Result<(), VfsError> {
        let group = (inode_num - 1) / self.inodes_per_group;
        let index = (inode_num - 1) % self.inodes_per_group;

        if group as usize >= MAX_BLOCK_GROUPS {
            return Err(VfsError::IOError);
        }

        // Journal the inode update before writing
        if self.journal_enabled {
            self.journal_inode(inode_num, inode)?;
        }

        let gdesc = &self.group_descs[group as usize];
        let inode_table_block = u32::from_le(gdesc.bg_inode_table);
        let inode_offset = inode_table_block as u64 * self.block_size as u64
            + index as u64 * self.inode_size as u64;

        let inode_bytes =
            unsafe { core::slice::from_raw_parts(inode as *const Ext2Inode as *const u8, 128) };
        dev_write(
            self.device_index,
            self.partition_start,
            inode_offset,
            inode_bytes,
        )?;

        // Update cache
        for entry in &mut self.inode_cache {
            if entry.valid && entry.inode_num == inode_num {
                entry.inode = *inode;
                entry.dirty = false;
                break;
            }
        }

        Ok(())
    }

    // ─── Block bitmap operations ───────────────────────────────────

    /// Load block bitmap for a given block group from disk
    pub fn load_block_bitmap(&mut self, group: u32) -> Result<(), VfsError> {
        if group as usize >= MAX_BLOCK_GROUPS || self.bitmap_loaded[group as usize] {
            return Ok(());
        }

        let gdesc = &self.group_descs[group as usize];
        let bitmap_block = u32::from_le(gdesc.bg_block_bitmap);
        let bitmap_offset = bitmap_block as u64 * self.block_size as u64;

        let bitmap_size = self.block_size as usize;
        if bitmap_size > 512 {
            // Read first 512 bytes of bitmap (enough for 4096 blocks)
            dev_read(
                self.device_index,
                self.partition_start,
                bitmap_offset,
                &mut self.block_bitmaps[group as usize],
            )?;
        } else {
            dev_read(
                self.device_index,
                self.partition_start,
                bitmap_offset,
                &mut self.block_bitmaps[group as usize][..bitmap_size],
            )?;
        }

        self.bitmap_loaded[group as usize] = true;
        Ok(())
    }

    /// Load inode bitmap for a given block group from disk
    pub fn load_inode_bitmap(&mut self, group: u32) -> Result<(), VfsError> {
        if group as usize >= MAX_BLOCK_GROUPS {
            return Ok(());
        }
        // Check if already loaded (use a separate tracking approach)
        let gdesc = &self.group_descs[group as usize];
        let bitmap_block = u32::from_le(gdesc.bg_inode_bitmap);
        if bitmap_block == 0 {
            return Ok(()); // No bitmap
        }
        let bitmap_offset = bitmap_block as u64 * self.block_size as u64;
        let bitmap_size = self.block_size as usize;

        // Bitmap windows are 512 bytes in the cache regardless of the on-disk
        // block size; for larger blocks only the first 512 bytes are needed
        // (4096 inodes per group maximum).
        if bitmap_size >= 512 {
            dev_read(
                self.device_index,
                self.partition_start,
                bitmap_offset,
                &mut self.inode_bitmaps[group as usize],
            )?;
        } else {
            dev_read(
                self.device_index,
                self.partition_start,
                bitmap_offset,
                &mut self.inode_bitmaps[group as usize][..bitmap_size],
            )?;
        }

        Ok(())
    }

    /// Allocate a block from the filesystem. Returns the block number.
    pub fn alloc_block(&mut self) -> Result<u64, VfsError> {
        for group in 0..self.num_groups as usize {
            if !self.bitmap_loaded[group] {
                self.load_block_bitmap(group as u32)?;
            }

            let bitmap = &mut self.block_bitmaps[group];
            for byte_idx in 0..512 {
                if bitmap[byte_idx] != 0xFF {
                    // Find first free bit
                    for bit in 0..8 {
                        if bitmap[byte_idx] & (1 << bit) == 0 {
                            bitmap[byte_idx] |= 1 << bit;
                            let block_offset = (byte_idx * 8 + bit) as u64;
                            let global_block =
                                group as u64 * self.blocks_per_group as u64 + block_offset;

                            // Write bitmap back
                            let gdesc = self.group_descs[group];
                            let bitmap_block = u32::from_le(gdesc.bg_block_bitmap);
                            let bitmap_offset = bitmap_block as u64 * self.block_size as u64;
                            // Bitmap cache holds 512 bytes regardless of the
                            // on-disk block size (4096 inodes/group max).
                            let bitmap_len = self.block_size as usize;
                            let bitmap_bytes: &mut [u8] = if bitmap_len >= 512 {
                                &mut bitmap[..512]
                            } else {
                                &mut bitmap[..bitmap_len]
                            };
                            dev_write(
                                self.device_index,
                                self.partition_start,
                                bitmap_offset,
                                bitmap_bytes,
                            )?;

                            // Update free block count
                            self.superblock.s_free_blocks_count =
                                u32::from_le(self.superblock.s_free_blocks_count)
                                    .saturating_sub(1)
                                    .to_le();

                            println!("[ext2] Allocated block {} (group {})", global_block, group);
                            return Ok(global_block);
                        }
                    }
                }
            }
        }

        println!("[ext2] No free blocks available");
        Err(VfsError::OutOfMemory)
    }

    /// Free a block in the filesystem
    pub fn free_block(&mut self, block_num: u64) -> Result<(), VfsError> {
        let group = (block_num / self.blocks_per_group as u64) as usize;
        let offset = (block_num % self.blocks_per_group as u64) as usize;

        if group >= MAX_BLOCK_GROUPS {
            return Err(VfsError::IOError);
        }

        if !self.bitmap_loaded[group] {
            self.load_block_bitmap(group as u32)?;
        }

        let byte_idx = offset / 8;
        let bit_idx = offset % 8;
        if byte_idx >= 512 {
            return Err(VfsError::IOError);
        }

        self.block_bitmaps[group][byte_idx] &= !(1 << bit_idx);

        // Write bitmap back
        let gdesc = self.group_descs[group];
        let bitmap_block = u32::from_le(gdesc.bg_block_bitmap);
        let bitmap_offset = bitmap_block as u64 * self.block_size as u64;
        dev_write(
            self.device_index,
            self.partition_start,
            bitmap_offset,
            &self.block_bitmaps[group][..self.block_size as usize],
        )?;

        // Update free block count
        self.superblock.s_free_blocks_count =
            (u32::from_le(self.superblock.s_free_blocks_count) + 1).to_le();

        println!("[ext2] Freed block {}", block_num);
        Ok(())
    }

    /// Allocate an inode from the filesystem. Returns the inode number (1-based).
    pub fn alloc_inode(&mut self) -> Result<u32, VfsError> {
        for group in 0..self.num_groups as usize {
            self.load_inode_bitmap(group as u32)?;

            let bitmap = &mut self.inode_bitmaps[group];
            for byte_idx in 0..512 {
                if bitmap[byte_idx] != 0xFF {
                    for bit in 0..8 {
                        if bitmap[byte_idx] & (1 << bit) == 0 {
                            bitmap[byte_idx] |= 1 << bit;
                            let inode_offset = (byte_idx * 8 + bit) as u32;
                            let global_inode =
                                group as u32 * self.inodes_per_group + inode_offset + 1;

                            // Write bitmap back
                            let gdesc = self.group_descs[group];
                            let bitmap_block = u32::from_le(gdesc.bg_inode_bitmap);
                            let bitmap_offset = bitmap_block as u64 * self.block_size as u64;
                            // Bitmap cache is 512 bytes max (see load_inode_bitmap)
                            let bitmap_len = self.block_size as usize;
                            let bitmap_bytes: &mut [u8] = if bitmap_len >= 512 {
                                &mut bitmap[..512]
                            } else {
                                &mut bitmap[..bitmap_len]
                            };
                            dev_write(
                                self.device_index,
                                self.partition_start,
                                bitmap_offset,
                                bitmap_bytes,
                            )?;

                            self.superblock.s_free_inodes_count =
                                u32::from_le(self.superblock.s_free_inodes_count)
                                    .saturating_sub(1)
                                    .to_le();

                            println!("[ext2] Allocated inode {}", global_inode);
                            return Ok(global_inode);
                        }
                    }
                }
            }
        }

        println!("[ext2] No free inodes available");
        Err(VfsError::OutOfMemory)
    }

    /// Free an inode in the filesystem
    pub fn free_inode(&mut self, inode_num: u32) -> Result<(), VfsError> {
        if inode_num == 0 {
            return Err(VfsError::NotFound);
        }
        let group = ((inode_num - 1) / self.inodes_per_group) as usize;
        let offset = ((inode_num - 1) % self.inodes_per_group) as usize;

        if group >= MAX_BLOCK_GROUPS {
            return Err(VfsError::IOError);
        }

        self.load_inode_bitmap(group as u32)?;

        let byte_idx = offset / 8;
        let bit_idx = offset % 8;
        if byte_idx >= 512 {
            return Err(VfsError::IOError);
        }

        self.inode_bitmaps[group][byte_idx] &= !(1 << bit_idx);

        let gdesc = self.group_descs[group];
        let bitmap_block = u32::from_le(gdesc.bg_inode_bitmap);
        let bitmap_offset = bitmap_block as u64 * self.block_size as u64;
        // Bitmap caches are 512 bytes max (see load_inode_bitmap).
        let bitmap_len = self.block_size as usize;
        let bitmap_bytes: &[u8] = if bitmap_len >= 512 {
            &self.inode_bitmaps[group][..512]
        } else {
            &self.inode_bitmaps[group][..bitmap_len]
        };
        dev_write(
            self.device_index,
            self.partition_start,
            bitmap_offset,
            bitmap_bytes,
        )?;

        self.superblock.s_free_inodes_count =
            (u32::from_le(self.superblock.s_free_inodes_count) + 1).to_le();

        println!("[ext2] Freed inode {}", inode_num);
        Ok(())
    }

    // ─── Directory operations ───────────────────────────────────────

    /// Read directory entries from an inode. Returns up to MAX_DIR_ENTRIES entries.
    pub fn read_dir(
        &mut self,
        dir_inode_num: u32,
    ) -> Result<[DirEntry; MAX_DIR_ENTRIES], VfsError> {
        let inode = self.read_inode(dir_inode_num)?;

        // Verify it's a directory
        let mode = u16::from_le(inode.i_mode);
        if mode & EXT2_S_IFMT != EXT2_S_IFDIR {
            return Err(VfsError::NotADirectory);
        }

        let mut entries = [DirEntry::empty(); MAX_DIR_ENTRIES];
        let dir_size = u32::from_le(inode.i_size) as usize;
        let mut entry_count = 0usize;
        let mut offset = 0usize;

        // Read directory data blocks (supports indirect blocks)
        let total_blocks = (dir_size + self.block_size as usize - 1) / self.block_size as usize;
        for block_idx in 0..total_blocks {
            if entry_count >= MAX_DIR_ENTRIES {
                break;
            }
            let block_num = match self.resolve_block(&inode, block_idx as u64) {
                Ok(bn) => bn,
                Err(_) => break,
            };
            if block_num == 0 {
                // Sparse block — skip
                offset += self.block_size as usize;
                continue;
            }

            let mut block_buf = [0u8; 4096];
            let block_buf_slice = &mut block_buf[..self.block_size as usize];
            read_block(self, block_num, block_buf_slice)?;

            let mut pos = 0usize;
            while pos < self.block_size as usize - 12
                && offset + pos < dir_size
                && entry_count < MAX_DIR_ENTRIES
            {
                let raw = &block_buf_slice[pos..];
                if raw.len() < 12 {
                    break;
                }

                let dentry = unsafe { &*(raw.as_ptr() as *const Ext2DirEntry) };
                let d_inode = u32::from_le(dentry.inode);
                let d_rec_len = u16::from_le(dentry.rec_len) as usize;
                let d_name_len = dentry.name_len as usize;
                let d_file_type = dentry.file_type;

                if d_rec_len == 0 {
                    break; // Prevent infinite loop
                }

                if d_inode != 0 && d_name_len > 0 && d_rec_len >= 12 {
                    // ext2 dir entry: 8-byte header (inode:4 + rec_len:2 + name_len:1 + file_type:1)
                    let name_start_actual = pos + 8;
                    let name_end = name_start_actual + d_name_len;

                    if name_end <= block_buf_slice.len() {
                        let name_bytes = &block_buf_slice[name_start_actual..name_end];
                        let name_str = core::str::from_utf8(name_bytes).unwrap_or("?");

                        let ft = match d_file_type {
                            EXT2_FT_REG_FILE => FileType::RegularFile,
                            EXT2_FT_DIR => FileType::Directory,
                            EXT2_FT_SYMLINK => FileType::Symlink,
                            EXT2_FT_CHRDEV | EXT2_FT_BLKDEV => FileType::Device,
                            _ => FileType::RegularFile,
                        };

                        entries[entry_count].set_name(name_str);
                        entries[entry_count].file_type = ft;
                        entries[entry_count].inode_id = d_inode as u64;

                        // Try to get size from the child inode
                        if let Ok(child_inode) = self.read_inode(d_inode) {
                            entries[entry_count].size = u32::from_le(child_inode.i_size) as u64;
                            entries[entry_count].uid = u16::from_le(child_inode.i_uid) as u32;
                            entries[entry_count].gid = u16::from_le(child_inode.i_gid) as u32;
                        }
                        entries[entry_count].is_dir = ft == FileType::Directory;

                        entry_count += 1;
                    }
                }

                pos += d_rec_len;
                if d_rec_len < 12 {
                    break; // Corrupted entry
                }
            }

            offset += self.block_size as usize;
        }

        Ok(entries)
    }

    /// Lookup a name in a directory inode. Returns the child inode number.
    pub fn lookup(&mut self, dir_inode_num: u32, name: &str) -> Result<u32, VfsError> {
        let entries = self.read_dir(dir_inode_num)?;
        for entry in &entries {
            if entry.name_len == 0 {
                break;
            }
            if entry.name_str() == name {
                return Ok(entry.inode_id as u32);
            }
        }
        Err(VfsError::NotFound)
    }

    /// Add a directory entry to a parent directory inode
    pub fn add_dir_entry(
        &mut self,
        parent_inode_num: u32,
        child_inode_num: u32,
        name: &str,
        file_type: u8,
    ) -> Result<(), VfsError> {
        let mut parent = self.read_inode(parent_inode_num)?;

        // Find the last data block of the parent directory
        let dir_size = u32::from_le(parent.i_size) as usize;
        let last_block_idx = dir_size / self.block_size as usize;

        if last_block_idx >= EXT2_NDIR_BLOCKS + (self.block_size as usize / 4) * 3 {
            println!("[ext2] Directory too large");
            return Err(VfsError::OutOfMemory);
        }

        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(255);
        // Entry size: 8-byte header + name, rounded up to 4-byte alignment
        let entry_len = ((8 + name_len + 3) & !3) as u16;

        // If the directory has no blocks yet, allocate one (supports indirect blocks)
        let block_num = {
            let existing = self.resolve_block(&parent, last_block_idx as u64)?;
            if existing != 0 {
                existing
            } else {
                self.alloc_logical_block(&mut parent, last_block_idx as u64)?
            }
        };

        // Create the directory entry
        let mut block_buf = [0u8; 4096];
        read_block(self, block_num, &mut block_buf[..self.block_size as usize])?;

        // Find free space in the block (simplified: append at the end)
        // Scan existing entries to find the last one and use remaining space
        let mut pos = 0usize;
        let bs = self.block_size as usize;
        while pos < bs - 12 {
            let raw = &block_buf[pos..];
            let rec_len = u16::from_le(unsafe { *(raw.as_ptr() as *const u16).add(2) }) as usize;
            if rec_len == 0 {
                break;
            }
            let actual_len = {
                let n_len = raw[6] as usize;
                (8 + n_len + 3) & !3
            };
            if pos + rec_len >= bs {
                // This is the last entry; shrink it to actual_len and add our entry after
                // Write updated rec_len for the old entry
                let old_rec = actual_len as u16;
                block_buf[pos + 4] = old_rec as u8;
                block_buf[pos + 5] = (old_rec >> 8) as u8;

                pos += actual_len;
                let remaining = bs - pos;
                if remaining < entry_len as usize {
                    // Need a new block
                    println!("[ext2] No space in directory block, need expansion");
                    return Err(VfsError::OutOfMemory);
                }
                break;
            }
            pos += rec_len;
        }

        if pos + entry_len as usize > bs {
            println!("[ext2] Directory block full");
            return Err(VfsError::OutOfMemory);
        }

        // Write the new entry
        let child_le = child_inode_num.to_le();
        block_buf[pos..pos + 4].copy_from_slice(&child_le.to_ne_bytes());
        block_buf[pos + 4..pos + 6].copy_from_slice(&entry_len.to_le_bytes());
        block_buf[pos + 6] = name_len as u8;
        block_buf[pos + 7] = file_type;
        block_buf[pos + 8..pos + 8 + name_len].copy_from_slice(&name_bytes[..name_len]);

        write_block(self, block_num, &block_buf[..bs])?;

        // Update parent inode size: cover all blocks through the one we
        // appended into. `pos` is block-relative, so a write into block N
        // must grow i_size to (N+1)*block_size at minimum, or boot-time
        // read_dir (which sizes by i_size) would never see later blocks.
        let min_size = ((last_block_idx + 1) * bs) as u32;
        let new_size = (u32::from_le(parent.i_size))
            .max(min_size)
            .max((last_block_idx * bs + pos + entry_len as usize) as u32);
        parent.i_size = new_size.to_le();
        self.write_inode(parent_inode_num, &parent)?;

        println!(
            "[ext2] Added dir entry '{}' (inode={}) to parent {}",
            name, child_inode_num, parent_inode_num
        );
        Ok(())
    }

    // ─── Indirect block resolution ─────────────────────────────────

    /// Resolve a logical block index to a physical block number.
    /// Supports direct (0..11), single indirect (12..1035),
    /// double indirect (1036..1,048,611), and triple indirect
    /// (1,048,612..1,073,741,823) with 4K blocks / 1024 ptrs per block.
    fn resolve_block(&mut self, inode: &Ext2Inode, logical_block: u64) -> Result<u64, VfsError> {
        let ptrs_per_block = self.block_size as u64 / 4; // 1024 for 4K blocks
        let direct_limit = EXT2_NDIR_BLOCKS as u64; // 12
        let ind_limit = direct_limit + ptrs_per_block; // 1036
        let dind_limit = ind_limit + ptrs_per_block * ptrs_per_block; // 1,048,612

        if logical_block < direct_limit {
            // Direct block
            let bn = u32::from_le(inode.i_block[logical_block as usize]);
            return Ok(bn as u64);
        }

        if logical_block < ind_limit {
            // Single indirect: i_block[12] → block of 1024 pointers
            let ind_block = u32::from_le(inode.i_block[EXT2_IND_BLOCK]);
            if ind_block == 0 {
                return Ok(0); // sparse
            }
            let index = logical_block - direct_limit;
            let ptr = self.read_indirect_ptr(ind_block as u64, index)?;
            return Ok(ptr as u64);
        }

        if logical_block < dind_limit {
            // Double indirect: i_block[13] → block of ptrs → block of ptrs
            let dind_block = u32::from_le(inode.i_block[EXT2_DIND_BLOCK]);
            if dind_block == 0 {
                return Ok(0); // sparse
            }
            let offset = logical_block - ind_limit;
            let outer_idx = offset / ptrs_per_block;
            let inner_idx = offset % ptrs_per_block;
            let ind_block = self.read_indirect_ptr(dind_block as u64, outer_idx)?;
            if ind_block == 0 {
                return Ok(0); // sparse
            }
            let ptr = self.read_indirect_ptr(ind_block as u64, inner_idx)?;
            return Ok(ptr as u64);
        }

        // Triple indirect: i_block[14] → 3 levels of indirection
        let tind_block = u32::from_le(inode.i_block[EXT2_TIND_BLOCK]);
        if tind_block == 0 {
            return Ok(0); // sparse
        }
        let offset = logical_block - dind_limit;
        let l0_idx = offset / (ptrs_per_block * ptrs_per_block);
        let l1_idx = (offset / ptrs_per_block) % ptrs_per_block;
        let l2_idx = offset % ptrs_per_block;
        let dind_block = self.read_indirect_ptr(tind_block as u64, l0_idx)?;
        if dind_block == 0 {
            return Ok(0);
        }
        let ind_block = self.read_indirect_ptr(dind_block as u64, l1_idx)?;
        if ind_block == 0 {
            return Ok(0);
        }
        let ptr = self.read_indirect_ptr(ind_block as u64, l2_idx)?;
        Ok(ptr as u64)
    }

    /// Read a 32-bit pointer from an indirect block at the given index.
    fn read_indirect_ptr(&self, block_num: u64, index: u64) -> Result<u32, VfsError> {
        let mut buf = [0u8; 4096];
        read_block(self, block_num, &mut buf[..self.block_size as usize])?;
        let ptrs_per_block = self.block_size as usize / 4;
        let idx = index as usize;
        if idx >= ptrs_per_block {
            return Err(VfsError::IOError);
        }
        let ptr = u32::from_le_bytes([
            buf[idx * 4],
            buf[idx * 4 + 1],
            buf[idx * 4 + 2],
            buf[idx * 4 + 3],
        ]);
        Ok(ptr)
    }

    /// Write a 32-bit pointer into an indirect block at the given index.
    fn write_indirect_ptr(&self, block_num: u64, index: u64, ptr: u32) -> Result<(), VfsError> {
        let mut buf = [0u8; 4096];
        read_block(self, block_num, &mut buf[..self.block_size as usize])?;
        let idx = index as usize;
        let ptrs_per_block = self.block_size as usize / 4;
        if idx >= ptrs_per_block {
            return Err(VfsError::IOError);
        }
        let bytes = ptr.to_le_bytes();
        buf[idx * 4..idx * 4 + 4].copy_from_slice(&bytes);
        write_block(self, block_num, &buf[..self.block_size as usize])?;
        Ok(())
    }

    /// Allocate or return an indirect block, writing it into the parent pointer.
    /// `parent_block_num` is the block containing the pointer slot.
    /// `slot_index` is the index within that block.
    /// Returns the block number of the (possibly newly allocated) indirect block.
    fn ensure_indirect_block(
        &mut self,
        parent_block_num: u64,
        slot_index: u64,
    ) -> Result<u64, VfsError> {
        let existing = self.read_indirect_ptr(parent_block_num, slot_index)?;
        if existing != 0 {
            return Ok(existing as u64);
        }
        // Allocate a new block and zero-fill it
        let new_block = self.alloc_block()?;
        let zeroes = [0u8; 4096];
        write_block(self, new_block, &zeroes[..self.block_size as usize])?;
        self.write_indirect_ptr(parent_block_num, slot_index, new_block as u32)?;
        Ok(new_block)
    }

    /// Ensure the block at `logical_block` is allocated, creating indirect
    /// blocks as needed. Returns the physical block number and mutates the
    /// inode's i_block array for any newly allocated indirect pointer blocks.
    fn alloc_logical_block(
        &mut self,
        inode: &mut Ext2Inode,
        logical_block: u64,
    ) -> Result<u64, VfsError> {
        let ptrs_per_block = self.block_size as u64 / 4;
        let direct_limit = EXT2_NDIR_BLOCKS as u64;
        let ind_limit = direct_limit + ptrs_per_block;
        let dind_limit = ind_limit + ptrs_per_block * ptrs_per_block;

        // Direct blocks
        if logical_block < direct_limit {
            let bn = u32::from_le(inode.i_block[logical_block as usize]);
            if bn != 0 {
                return Ok(bn as u64);
            }
            let new_block = self.alloc_block()?;
            // Zero the fresh block so directory scans see valid (empty) entries
            let zeroes = [0u8; 4096];
            write_block(self, new_block, &zeroes[..self.block_size as usize])?;
            inode.i_block[logical_block as usize] = (new_block as u32).to_le();
            return Ok(new_block);
        }

        // Single indirect
        if logical_block < ind_limit {
            let ind_bn = u32::from_le(inode.i_block[EXT2_IND_BLOCK]);
            let ind_block = if ind_bn == 0 {
                let new_block = self.alloc_block()?;
                let zeroes = [0u8; 4096];
                write_block(self, new_block, &zeroes[..self.block_size as usize])?;
                inode.i_block[EXT2_IND_BLOCK] = (new_block as u32).to_le();
                new_block
            } else {
                ind_bn as u64
            };
            let index = logical_block - direct_limit;
            let data_bn = self.read_indirect_ptr(ind_block, index)?;
            if data_bn != 0 {
                return Ok(data_bn as u64);
            }
            let new_block = self.alloc_block()?;
            self.write_indirect_ptr(ind_block, index, new_block as u32)?;
            return Ok(new_block);
        }

        // Double indirect
        if logical_block < dind_limit {
            let dind_bn = u32::from_le(inode.i_block[EXT2_DIND_BLOCK]);
            let dind_block = if dind_bn == 0 {
                let new_block = self.alloc_block()?;
                let zeroes = [0u8; 4096];
                write_block(self, new_block, &zeroes[..self.block_size as usize])?;
                inode.i_block[EXT2_DIND_BLOCK] = (new_block as u32).to_le();
                new_block
            } else {
                dind_bn as u64
            };
            let offset = logical_block - ind_limit;
            let outer_idx = offset / ptrs_per_block;
            let inner_idx = offset % ptrs_per_block;
            let ind_block = self.ensure_indirect_block(dind_block, outer_idx)?;
            let data_bn = self.read_indirect_ptr(ind_block, inner_idx)?;
            if data_bn != 0 {
                return Ok(data_bn as u64);
            }
            let new_block = self.alloc_block()?;
            self.write_indirect_ptr(ind_block, inner_idx, new_block as u32)?;
            return Ok(new_block);
        }

        // Triple indirect
        let tind_bn = u32::from_le(inode.i_block[EXT2_TIND_BLOCK]);
        let tind_block = if tind_bn == 0 {
            let new_block = self.alloc_block()?;
            let zeroes = [0u8; 4096];
            write_block(self, new_block, &zeroes[..self.block_size as usize])?;
            inode.i_block[EXT2_TIND_BLOCK] = (new_block as u32).to_le();
            new_block
        } else {
            tind_bn as u64
        };
        let offset = logical_block - dind_limit;
        let l0_idx = offset / (ptrs_per_block * ptrs_per_block);
        let l1_idx = (offset / ptrs_per_block) % ptrs_per_block;
        let l2_idx = offset % ptrs_per_block;
        let dind_block = self.ensure_indirect_block(tind_block, l0_idx)?;
        let ind_block = self.ensure_indirect_block(dind_block, l1_idx)?;
        let data_bn = self.read_indirect_ptr(ind_block, l2_idx)?;
        if data_bn != 0 {
            return Ok(data_bn as u64);
        }
        let new_block = self.alloc_block()?;
        self.write_indirect_ptr(ind_block, l2_idx, new_block as u32)?;
        Ok(new_block)
    }

    // ─── File read/write ────────────────────────────────────────────

    /// Read data from an inode at a given offset.
    /// Supports direct, single/double/triple indirect blocks.
    pub fn read_file(
        &mut self,
        inode_num: u32,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, VfsError> {
        let inode = self.read_inode(inode_num)?;
        let file_size = u32::from_le(inode.i_size) as u64;

        if offset >= file_size {
            return Err(VfsError::EndOfFile);
        }

        let remaining = (file_size - offset) as usize;
        let to_read = buf.len().min(remaining);
        let mut bytes_read = 0usize;
        let mut current_offset = offset;

        while bytes_read < to_read {
            let block_idx = current_offset / self.block_size as u64;
            let offset_in_block = (current_offset % self.block_size as u64) as usize;

            let block_num = self.resolve_block(&inode, block_idx)?;
            if block_num == 0 {
                // Sparse file: return zeros
                let chunk = (self.block_size as usize - offset_in_block).min(to_read - bytes_read);
                buf[bytes_read..bytes_read + chunk].fill(0);
                bytes_read += chunk;
                current_offset += chunk as u64;
                continue;
            }

            let mut block_buf = [0u8; 4096];
            read_block(self, block_num, &mut block_buf[..self.block_size as usize])?;

            let chunk = (self.block_size as usize - offset_in_block).min(to_read - bytes_read);
            buf[bytes_read..bytes_read + chunk]
                .copy_from_slice(&block_buf[offset_in_block..offset_in_block + chunk]);
            bytes_read += chunk;
            current_offset += chunk as u64;
        }

        Ok(bytes_read)
    }

    /// Write data to an inode at a given offset, allocating blocks as needed.
    /// Supports direct, single/double/triple indirect blocks.
    pub fn write_file(
        &mut self,
        inode_num: u32,
        offset: u64,
        buf: &[u8],
    ) -> Result<usize, VfsError> {
        let mut inode = self.read_inode(inode_num)?;

        let mut bytes_written = 0usize;
        let mut current_offset = offset;

        while bytes_written < buf.len() {
            let block_idx = current_offset / self.block_size as u64;
            let offset_in_block = (current_offset % self.block_size as u64) as usize;

            let block_num = self.alloc_logical_block(&mut inode, block_idx)?;

            // Read-modify-write the block
            let mut block_buf = [0u8; 4096];
            read_block(self, block_num, &mut block_buf[..self.block_size as usize])?;

            let chunk = (self.block_size as usize - offset_in_block).min(buf.len() - bytes_written);
            block_buf[offset_in_block..offset_in_block + chunk]
                .copy_from_slice(&buf[bytes_written..bytes_written + chunk]);

            // Journal the block write before committing
            if self.journal_enabled {
                self.journal_block(block_num, &block_buf[..self.block_size as usize])?;
            }

            write_block(self, block_num, &block_buf[..self.block_size as usize])?;

            bytes_written += chunk;
            current_offset += chunk as u64;
        }

        // Update inode size
        let new_size = current_offset.max(u32::from_le(inode.i_size) as u64);
        inode.i_size = (new_size as u32).to_le();
        // Update block count (in 512-byte sectors)
        let sectors = (new_size + 511) / 512;
        inode.i_blocks = (sectors as u32).to_le();

        self.write_inode(inode_num, &inode)?;

        Ok(bytes_written)
    }

    // ─── Journal (simple write-ahead log) ───────────────────────────

    /// Journal a block write before committing
    fn journal_block(&mut self, block_num: u64, data: &[u8]) -> Result<(), VfsError> {
        let idx = self.journal_head;
        let len = data.len().min(JOURNAL_BLOCK_SIZE);

        self.journal_entries[idx] = JournalEntry {
            entry_type: JournalEntryType::BlockWrite,
            block_num,
            inode_num: 0,
            data_len: len,
            sequence: self.journal_seq,
        };
        self.journal_data[idx][..len].copy_from_slice(&data[..len]);

        self.journal_seq += 1;
        self.journal_head = (self.journal_head + 1) % JOURNAL_SIZE;
        if self.journal_head == self.journal_tail {
            // Journal full — commit (replay all entries)
            self.journal_commit()?;
        }

        Ok(())
    }

    /// Journal an inode update
    fn journal_inode(&mut self, inode_num: u32, inode: &Ext2Inode) -> Result<(), VfsError> {
        let idx = self.journal_head;

        self.journal_entries[idx] = JournalEntry {
            entry_type: JournalEntryType::InodeUpdate,
            block_num: 0,
            inode_num,
            data_len: core::mem::size_of::<Ext2Inode>(),
            sequence: self.journal_seq,
        };
        let inode_bytes =
            unsafe { core::slice::from_raw_parts(inode as *const Ext2Inode as *const u8, 128) };
        self.journal_data[idx][..128].copy_from_slice(inode_bytes);

        self.journal_seq += 1;
        self.journal_head = (self.journal_head + 1) % JOURNAL_SIZE;
        if self.journal_head == self.journal_tail {
            self.journal_commit()?;
        }

        Ok(())
    }

    /// Commit the journal — replay all pending entries to ensure consistency.
    /// In a real implementation, this would write a commit record first,
    /// then replay, then clear. For our simple journal, we just clear it
    /// since writes already happened.
    pub fn journal_commit(&mut self) -> Result<(), VfsError> {
        if self.journal_tail == self.journal_head {
            return Ok(()); // Nothing to commit
        }

        println!(
            "[ext2] Journal commit: seq={}, entries pending",
            self.journal_seq
        );

        // In this simplified journal, writes already went to disk.
        // A full journal would: (1) write all data to journal area,
        // (2) write commit record, (3) write to final locations,
        // (4) clear journal. We just mark entries as committed.
        self.journal_tail = self.journal_head;
        Ok(())
    }

    /// Replay the journal after a crash — re-apply any pending writes.
    /// This is the recovery path called at mount time.
    pub fn journal_replay(&mut self) -> Result<(), VfsError> {
        let mut replayed = 0u32;
        let mut idx = self.journal_tail;

        while idx != self.journal_head {
            let entry = &self.journal_entries[idx];
            match entry.entry_type {
                JournalEntryType::BlockWrite => {
                    // Re-write the block from journal data
                    write_block(
                        self,
                        entry.block_num,
                        &self.journal_data[idx][..entry.data_len],
                    )?;
                    replayed += 1;
                }
                JournalEntryType::InodeUpdate => {
                    // Re-write the inode
                    let inode = unsafe { *(self.journal_data[idx].as_ptr() as *const Ext2Inode) };
                    self.write_inode(entry.inode_num, &inode)?;
                    replayed += 1;
                }
                JournalEntryType::None => {}
            }

            idx = (idx + 1) % JOURNAL_SIZE;
        }

        if replayed > 0 {
            println!("[ext2] Journal replay: {} entries recovered", replayed);
        }

        // Clear journal after replay
        self.journal_tail = 0;
        self.journal_head = 0;

        Ok(())
    }

    // ─── Path resolution ────────────────────────────────────────────

    /// Resolve an absolute path to an inode number.
    /// Walks directory tree from root inode.
    pub fn resolve_path(&mut self, path: &str) -> Result<u32, VfsError> {
        let mut current_inode = EXT2_ROOT_INO;

        for component in path.split('/') {
            if component.is_empty() {
                continue;
            }
            current_inode = self.lookup(current_inode, component)?;
        }

        Ok(current_inode)
    }

    // ─── VFS integration ────────────────────────────────────────────

    /// Populate the VFS with entries from the ext2 root directory.
    fn populate_vfs(&mut self, parent_id: u64, fs_id: u32) -> Result<(), VfsError> {
        let root_entries = self.read_dir(EXT2_ROOT_INO)?;

        for entry in &root_entries {
            if entry.name_len == 0 {
                break;
            }
            let name = entry.name_str();
            if name == "." || name == ".." {
                continue; // Skip self/parent references
            }
            if let Ok(node_id) = alloc_node(name, entry.file_type, parent_id) {
                // Tag the node with the ext2 filesystem id + record the ext2
                // inode so reads/writes can route through the backing store.
                let ext2_ino = entry.inode_id as u32;
                crate::fs::vfs::set_node_backing(node_id, fs_id, ext2_ino);
            }
        }

        Ok(())
    }

    // ─── Diagnostics ────────────────────────────────────────────────

    /// Print filesystem summary
    pub fn print_summary(&self) {
        println!("[ext2] === Filesystem Summary ===");
        println!("[ext2]   Block size: {} bytes", self.block_size);
        println!("[ext2]   Inode size: {} bytes", self.inode_size);
        println!(
            "[ext2]   Total blocks: {}",
            u32::from_le(self.superblock.s_blocks_count)
        );
        println!(
            "[ext2]   Free blocks: {}",
            u32::from_le(self.superblock.s_free_blocks_count)
        );
        println!(
            "[ext2]   Total inodes: {}",
            u32::from_le(self.superblock.s_inodes_count)
        );
        println!(
            "[ext2]   Free inodes: {}",
            u32::from_le(self.superblock.s_free_inodes_count)
        );
        println!("[ext2]   Block groups: {}", self.num_groups);
        println!(
            "[ext2]   Journal: {}",
            if self.journal_enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        println!("[ext2] =============================");
    }
}

// ═══════════════════════════════════════════════════════════════
// FileSystem trait implementation
// ═══════════════════════════════════════════════════════════════

impl FileSystem for Ext2FileSystem {
    fn name(&self) -> &str {
        "ext2"
    }

    fn fs_id(&self) -> u32 {
        self.fs_id
    }

    fn root_inode_id(&self) -> u64 {
        EXT2_ROOT_INO as u64
    }

    fn read_node(&self, inode_id: u64, buf: &mut [u8], offset: u64) -> Result<usize, VfsError> {
        // Need mutable access for cache — use the global instance
        unsafe {
            if let Some(ref mut fs) = EXT2_INSTANCE {
                fs.read_file(inode_id as u32, offset, buf)
            } else {
                Err(VfsError::IOError)
            }
        }
    }

    fn write_node(&self, inode_id: u64, buf: &[u8], offset: u64) -> Result<usize, VfsError> {
        unsafe {
            if let Some(ref mut fs) = EXT2_INSTANCE {
                fs.write_file(inode_id as u32, offset, buf)
            } else {
                Err(VfsError::IOError)
            }
        }
    }

    fn get_metadata(&self, inode_id: u64) -> Result<Metadata, VfsError> {
        unsafe {
            if let Some(ref mut fs) = EXT2_INSTANCE {
                let inode = fs.read_inode(inode_id as u32)?;
                let mode = u16::from_le(inode.i_mode);
                let ft = if mode & EXT2_S_IFDIR != 0 {
                    FileType::Directory
                } else if mode & EXT2_S_IFLNK != 0 {
                    FileType::Symlink
                } else {
                    FileType::RegularFile
                };
                Ok(Metadata {
                    size: u32::from_le(inode.i_size) as u64,
                    file_type: ft,
                    permissions: (mode & 0o7777) as u32,
                    modified_time: u32::from_le(inode.i_mtime) as u64,
                    inode_id,
                    uid: u16::from_le(inode.i_uid) as u32,
                    gid: u16::from_le(inode.i_gid) as u32,
                })
            } else {
                Err(VfsError::IOError)
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Public API — mount and initialize
// ═══════════════════════════════════════════════════════════════

/// Mount an ext2 filesystem from a block device at a given partition offset
pub fn mount(device_index: usize, partition_start: u64, mount_path: &str) -> Result<(), VfsError> {
    let mut fs = Ext2FileSystem::new(device_index, partition_start)?;
    let fs_id = fs.id();

    // Replay journal for crash recovery
    fs.journal_replay()?;

    // Create the mountpoint directory in the VFS (idempotent) so path
    // resolution reaches it, then populate the ext2 root's entries beneath.
    let mount_node = match crate::fs::vfs::mkdir(mount_path) {
        Ok(id) => Some(id),
        Err(crate::fs::vfs::VfsError::AlreadyExists) => {
            crate::fs::vfs::resolve_path(mount_path).ok()
        }
        Err(e) => return Err(e),
    };
    let parent_id = mount_node.unwrap_or(0);

    // Populate VFS with root directory entries under the mountpoint
    fs.populate_vfs(parent_id, fs_id)?;

    // Mount in VFS
    let root_inode = EXT2_ROOT_INO as u64;
    crate::fs::vfs::mount(mount_path, fs_id, root_inode)?;

    fs.print_summary();

    unsafe {
        EXT2_INSTANCE = Some(fs);
    }

    println!("[ext2] Mounted at {}", mount_path);
    Ok(())
}

/// Initialize the ext2 subsystem
pub fn init() {
    println!("[ext2] ext2 filesystem driver initialized");
}

/// Probe a block device partition for an ext2 filesystem by checking the superblock magic.
/// Returns Ok(()) if the superblock is valid, Err otherwise.
pub fn probe_ext2(device_index: usize, partition_start: u64) -> bool {
    let mut sb_buf = [0u8; 1024];
    if dev_read(device_index, partition_start, 1024, &mut sb_buf).is_err() {
        return false;
    }
    let superblock = unsafe { *(sb_buf.as_ptr() as *const Ext2Superblock) };
    let magic = u16::from_le(superblock.s_magic);
    magic == EXT2_MAGIC
}

/// Probe all registered partition devices for ext2 and mount the first one found as root.
/// Called from kernel_main after block devices and VFS are initialized.
pub fn probe_and_mount_rootfs() {
    let part_count = crate::fs::block_dev::partition_count();
    if part_count == 0 {
        println!("[ext2] No partitions found, skipping rootfs mount");
        return;
    }

    for i in 0..part_count {
        if let Some(part) = crate::fs::block_dev::get_partition_device(i) {
            let start_lba_bytes = part.start_lba * 512;
            let name_len = part.name_len;
            let name_str = core::str::from_utf8(&part.device_name[..name_len]).unwrap_or("?");

            println!(
                "[ext2] Probing partition '{}' (LBA {}) for ext2...",
                name_str, part.start_lba
            );

            if probe_ext2(part.parent_index, start_lba_bytes) {
                println!("[ext2] Found ext2 filesystem on partition '{}'", name_str);
                match mount(part.parent_index, start_lba_bytes, "/") {
                    Ok(()) => {
                        println!("[ext2] Root filesystem mounted successfully");
                        return;
                    }
                    Err(e) => {
                        println!("[ext2] Failed to mount rootfs: {:?}", e);
                        continue;
                    }
                }
            }
        }
    }

    // No ext2 partition found — try FAT32 as fallback for boot partition
    println!("[ext2] No ext2 partition found, trying FAT32...");
    for i in 0..part_count {
        if let Some(part) = crate::fs::block_dev::get_partition_device(i) {
            let start_lba_bytes = part.start_lba * 512;
            if crate::fs::fat32::mount(part.parent_index, start_lba_bytes, "/boot").is_ok() {
                println!("[fs] FAT32 boot partition mounted at /boot");
                return;
            }
        }
    }

    println!("[fs] No mountable root filesystem found");
}

/// Probe all registered partition devices for ext2 and mount the first one found as /etc.
/// Called from kernel_main after block devices and VFS are initialized.
/// This provides a persistent /etc for configuration files (WiFi, users, packages, etc.)
pub fn probe_and_mount_etc() {
    let part_count = crate::fs::block_dev::partition_count();
    if part_count == 0 {
        println!("[ext2] No partitions found, skipping /etc mount");
        return;
    }

    for i in 0..part_count {
        if let Some(part) = crate::fs::block_dev::get_partition_device(i) {
            let start_lba_bytes = part.start_lba * 512;
            let name_len = part.name_len;
            let name_str = core::str::from_utf8(&part.device_name[..name_len]).unwrap_or("?");

            println!(
                "[ext2] Probing partition '{}' (LBA {}) for ext2 (/etc mount)...",
                name_str, part.start_lba
            );

            if probe_ext2(part.parent_index, start_lba_bytes) {
                println!("[ext2] Found ext2 filesystem on partition '{}'", name_str);
                match mount(part.parent_index, start_lba_bytes, "/etc") {
                    Ok(()) => {
                        println!("[ext2] /etc filesystem mounted successfully");
                        // Ensure /etc/wifi directory exists for WiFi persistence
                        let _ = crate::fs::vfs::create_dir("/etc/wifi");
                        println!("[ext2] Created /etc/wifi directory");
                        return;
                    }
                    Err(e) => {
                        println!("[ext2] Failed to mount /etc: {:?}", e);
                        continue;
                    }
                }
            }
        }
    }

    // No ext2 partition found for /etc — try FAT32 as fallback
    println!("[ext2] No ext2 partition for /etc, trying FAT32...");
    for i in 0..part_count {
        if let Some(part) = crate::fs::block_dev::get_partition_device(i) {
            let start_lba_bytes = part.start_lba * 512;
            if crate::fs::fat32::mount(part.parent_index, start_lba_bytes, "/etc").is_ok() {
                println!("[fs] FAT32 partition mounted at /etc");
                let _ = crate::fs::vfs::create_dir("/etc/wifi");
                return;
            }
        }
    }

    println!("[fs] No mountable /etc filesystem found, /etc will be on initrd (read-only)");
}

// ═══════════════════════════════════════════════════════════════
// mkfs — create an ext2 filesystem on a block device
// ═══════════════════════════════════════════════════════════════

/// Parameters for creating a new ext2 filesystem
pub struct MkfsOptions {
    /// Block size in bytes (must be 1024, 2048, or 4096)
    pub block_size: u32,
    /// Number of inodes (0 = auto-calculate based on disk size)
    pub num_inodes: u32,
    /// Volume label (up to 16 bytes)
    pub volume_name: [u8; 16],
    /// Reserved blocks percentage (typically 5%)
    pub reserved_blocks_percent: u8,
}

impl Default for MkfsOptions {
    fn default() -> Self {
        Self {
            block_size: 4096,
            num_inodes: 0,
            volume_name: [0u8; 16],
            reserved_blocks_percent: 5,
        }
    }
}

/// Create an ext2 filesystem on a block device partition.
///
/// This writes the superblock, block group descriptors, bitmaps, inode table,
/// and the root directory. After calling this, the partition can be mounted
/// with `mount()`.
///
/// # Arguments
/// * `device_index` - Index of the block device in the registry
/// * `partition_start` - Byte offset of the partition start on the device
/// * `partition_size_bytes` - Size of the partition in bytes
/// * `opts` - Filesystem creation options
pub fn mkfs(
    device_index: usize,
    partition_start: u64,
    partition_size_bytes: u64,
    opts: &MkfsOptions,
) -> Result<(), VfsError> {
    let block_size = opts.block_size;
    if block_size != 1024 && block_size != 2048 && block_size != 4096 {
        println!(
            "[ext2 mkfs] Invalid block size {}, must be 1024/2048/4096",
            block_size
        );
        return Err(VfsError::IOError);
    }

    let blocks_count = (partition_size_bytes / block_size as u64) as u32;
    if blocks_count < 8 {
        println!(
            "[ext2 mkfs] Partition too small ({} blocks minimum required)",
            8
        );
        return Err(VfsError::IOError);
    }

    let log_block_size = match block_size {
        1024 => 0u32,
        2048 => 1,
        4096 => 2,
        _ => return Err(VfsError::IOError),
    };

    let blocks_per_group = block_size * 8; // One bitmap block covers this many blocks
    let num_groups = (blocks_count + blocks_per_group - 1) / blocks_per_group;
    if num_groups as usize > MAX_BLOCK_GROUPS {
        println!(
            "[ext2 mkfs] Too many block groups ({}, max {})",
            num_groups, MAX_BLOCK_GROUPS
        );
        return Err(VfsError::IOError);
    }

    // Calculate inode count
    let inodes_per_group = if opts.num_inodes > 0 {
        let ipg = (opts.num_inodes + num_groups - 1) / num_groups;
        // Round up to 8 (bitmap granularity)
        ((ipg + 7) / 8) * 8
    } else {
        // Auto: 1 inode per 8KB of disk, per group
        let ipg = (blocks_per_group * block_size) / 8192;
        ((ipg.max(8) + 7) / 8) * 8 // Round up to 8, min 8
    };
    let inodes_count = inodes_per_group * num_groups;

    // Inode table size per group (in blocks)
    let inode_size: u32 = EXT2_INODE_SIZE as u32;
    let inodes_per_block = block_size / inode_size;
    let inode_table_blocks = (inodes_per_group + inodes_per_block - 1) / inodes_per_block;

    // Reserved blocks
    let reserved_blocks = blocks_count * opts.reserved_blocks_percent as u32 / 100;

    // First data block (superblock lives in block 1 for 1024-byte blocks, block 0 otherwise)
    let first_data_block: u32 = if block_size == 1024 { 1 } else { 0 };

    // Free blocks: total - superblock/GDT/bitmap/inode_table blocks per group
    let gdt_blocks = (num_groups as u64 * 32 + block_size as u64 - 1) / block_size as u64;
    let overhead_per_group = 2u32 + inode_table_blocks; // block bitmap + inode bitmap + inode table
    let mut free_blocks = 0u32;
    for g in 0..num_groups {
        let group_start = g * blocks_per_group + first_data_block;
        let group_blocks = if g == num_groups - 1 {
            blocks_count - group_start
        } else {
            blocks_per_group
        };
        // Group 0 also has superblock + GDT blocks
        let overhead = if g == 0 {
            overhead_per_group + 1 + gdt_blocks as u32 // superblock block + GDT blocks
        } else {
            overhead_per_group
        };
        free_blocks += group_blocks.saturating_sub(overhead);
    }
    free_blocks = free_blocks.saturating_sub(reserved_blocks);

    // Get current timestamp for filesystem creation time
    let now_secs = (Timer::uptime_ms() / 1000) as u32;

    // ── Write superblock ──────────────────────────────────────────
    let mut sb = Ext2Superblock {
        s_inodes_count: inodes_count.to_le(),
        s_blocks_count: blocks_count.to_le(),
        s_r_blocks_count: reserved_blocks.to_le(),
        s_free_blocks_count: free_blocks.to_le(),
        s_free_inodes_count: (inodes_count - EXT2_FIRST_INO + 1).to_le(),
        s_first_data_block: first_data_block.to_le(),
        s_log_block_size: log_block_size.to_le(),
        s_log_frag_size: log_block_size.to_le(), // Fragment size = block size
        s_blocks_per_group: blocks_per_group.to_le(),
        s_frags_per_group: blocks_per_group.to_le(),
        s_inodes_per_group: inodes_per_group.to_le(),
        s_mtime: now_secs.to_le(),
        s_wtime: now_secs.to_le(),
        s_mnt_count: 0,
        s_max_mnt_count: 0xFFFF,
        s_magic: EXT2_MAGIC.to_le(),
        s_state: 1u16.to_le(),  // EXT2_VALID_FS
        s_errors: 1u16.to_le(), // Continue on errors
        s_minor_rev_level: 0,
        s_lastcheck: now_secs.to_le(),
        s_checkinterval: 0,
        s_creator_os: 0u32.to_le(), // Linux
        s_rev_level: 1u32.to_le(),  // Revision 1 (dynamic inode sizes)
        s_def_resuid: 0,
        s_def_resgid: 0,
        s_first_ino: EXT2_FIRST_INO.to_le(),
        s_inode_size: EXT2_INODE_SIZE.to_le(),
        s_block_group_nr: 0,
        s_feature_compat: 0,
        s_feature_incompat: 1u32.to_le(), // EXT2_FEATURE_INCOMPAT_FILETYPE
        s_feature_ro_compat: 0,
        s_uuid: [0; 16], // Generated below
        s_volume_name: opts.volume_name,
        s_last_mounted: [0; 64],
        s_algo_bitmap: 0,
        s_prealloc_blocks: 0,
        s_prealloc_dir_blocks: 0,
        s_reserved_gdt_blocks: 0,
        s_journal_uuid: [0; 16],
        s_journal_inum: 0,
        s_journal_dev: 0,
        s_last_orphan: 0,
        s_hash_seed: [0; 4],
        s_def_hash_version: 0,
        s_jnl_backup_type: 0,
        s_desc_size: 0,
        s_default_mount_opts: 0,
        s_first_meta_bg: 0,
        s_mkfs_time: now_secs.to_le(),
        s_jnl_blocks: [0; 17],
    };

    // Copy a simple UUID (using device/partition as entropy — not crypto quality)
    let mut uuid_idx = 0usize;
    let dev_entropy = [
        device_index as u8,
        partition_start as u8,
        (partition_start >> 8) as u8,
        (partition_start >> 16) as u8,
        (partition_start >> 24) as u8,
        (partition_size_bytes) as u8,
        (partition_size_bytes >> 8) as u8,
        (partition_size_bytes >> 16) as u8,
        (partition_size_bytes >> 24) as u8,
        (partition_size_bytes >> 32) as u8,
        (partition_size_bytes >> 40) as u8,
        (partition_size_bytes >> 48) as u8,
        (partition_size_bytes >> 56) as u8,
        blocks_count as u8,
        (blocks_count >> 8) as u8,
        inodes_count as u8,
    ];
    while uuid_idx < 16 {
        sb.s_uuid[uuid_idx] = dev_entropy[uuid_idx].wrapping_mul(0x6B).wrapping_add(0xA3);
        uuid_idx += 1;
    }
    // Set UUID version 4 indicator
    sb.s_uuid[6] = (sb.s_uuid[6] & 0x0F) | 0x40;
    sb.s_uuid[8] = (sb.s_uuid[8] & 0x3F) | 0x80;

    // Write superblock at byte offset 1024
    let sb_bytes = unsafe {
        core::slice::from_raw_parts(
            &sb as *const Ext2Superblock as *const u8,
            core::mem::size_of::<Ext2Superblock>(),
        )
    };
    // Zero-fill a full block for the superblock area
    let mut sb_block = [0u8; 4096];
    let copy_len = sb_bytes.len().min(sb_block.len());
    sb_block[..copy_len].copy_from_slice(&sb_bytes[..copy_len]);

    // Superblock is at offset 1024 from the partition start
    let _sb_offset = if block_size == 1024 { 1024u64 } else { 0u64 }; // For 1K blocks, sb is at block 1
                                                                      // Actually: superblock is ALWAYS at byte 1024
    let sb_byte_offset = 1024u64;
    // Write the superblock data (we write 1024 bytes starting at byte 1024)
    dev_write(
        device_index,
        partition_start,
        sb_byte_offset,
        &sb_block[..1024],
    )?;

    println!(
        "[ext2 mkfs] Superblock written: {} blocks, {} inodes, {} groups",
        blocks_count, inodes_count, num_groups
    );

    // ── Write block group descriptors ─────────────────────────────
    // GDT is in the block immediately following the superblock
    let gdt_block_num = if block_size == 1024 { 2u64 } else { 1u64 };
    let gdt_byte_offset = gdt_block_num * block_size as u64;

    // Calculate block layout per group
    // Per group: block_bitmap(1) + inode_bitmap(1) + inode_table(inode_table_blocks) + data
    let mut gdt_buf = [0u8; 4096]; // Support up to 128 groups * 32 bytes = 4096
    for g in 0..num_groups as usize {
        let group_base = (g as u32 * blocks_per_group) + first_data_block;

        // Block bitmap is always the first block of the group's data area
        let bg_block_bitmap = group_base;
        // Inode bitmap is the next block
        let bg_inode_bitmap = group_base + 1;
        // Inode table follows
        let bg_inode_table = group_base + 2;

        // Calculate free blocks in this group
        let group_blocks = if g as u32 == num_groups - 1 {
            blocks_count - (g as u32 * blocks_per_group + first_data_block)
        } else {
            blocks_per_group
        };
        let overhead = if g == 0 {
            1u16 + gdt_blocks as u16 + 2 + inode_table_blocks as u16 // superblock + gdt + bitmaps + inode_table
        } else {
            2u16 + inode_table_blocks as u16 // bitmaps + inode_table
        };
        let bg_free_blocks = group_blocks.saturating_sub(overhead as u32) as u16;
        let bg_free_inodes = inodes_per_group as u16; // All inodes are free initially
        let bg_used_dirs = if g == 0 { 1u16 } else { 0u16 }; // Root dir in group 0

        let gdesc = Ext2GroupDesc {
            bg_block_bitmap: bg_block_bitmap.to_le(),
            bg_inode_bitmap: bg_inode_bitmap.to_le(),
            bg_inode_table: bg_inode_table.to_le(),
            bg_free_blocks_count: bg_free_blocks.to_le(),
            bg_free_inodes_count: bg_free_inodes.to_le(),
            bg_used_dirs_count: bg_used_dirs.to_le(),
            bg_pad: 0,
            bg_reserved: [0; 3],
        };

        let gdesc_bytes =
            unsafe { core::slice::from_raw_parts(&gdesc as *const Ext2GroupDesc as *const u8, 32) };
        let offset = g * 32;
        if offset + 32 <= gdt_buf.len() {
            gdt_buf[offset..offset + 32].copy_from_slice(gdesc_bytes);
        }
    }

    let gdt_total_bytes = num_groups as usize * 32;
    dev_write(
        device_index,
        partition_start,
        gdt_byte_offset,
        &gdt_buf[..gdt_total_bytes],
    )?;

    println!("[ext2 mkfs] GDT written: {} group descriptors", num_groups);

    // ── Write block bitmaps ────────────────────────────────────────
    // For each group, write the block bitmap: mark overhead blocks as used
    for g in 0..num_groups as usize {
        let group_base = (g as u32 * blocks_per_group) + first_data_block;
        let bitmap_block = group_base; // First block of group = block bitmap
        let bitmap_byte_offset = bitmap_block as u64 * block_size as u64;

        let group_blocks = if g as u32 == num_groups - 1 {
            blocks_count - (g as u32 * blocks_per_group + first_data_block)
        } else {
            blocks_per_group
        };

        let mut bitmap = [0u8; 512]; // 4096 bits = 512 bytes, covers 4096 blocks
        let overhead_count = if g == 0 {
            1u32 + gdt_blocks as u32 + 2 + inode_table_blocks // sb + gdt + bitmaps + inode_table
        } else {
            2u32 + inode_table_blocks // bitmaps + inode_table
        };

        // Mark overhead blocks as used
        for b in 0..overhead_count {
            let byte_idx = (b / 8) as usize;
            let bit_idx = (b % 8) as usize;
            if byte_idx < bitmap.len() {
                bitmap[byte_idx] |= 1 << bit_idx;
            }
        }

        // Mark blocks beyond group_blocks as used (padding bits)
        for b in group_blocks..blocks_per_group {
            let byte_idx = (b / 8) as usize;
            let bit_idx = (b % 8) as usize;
            if byte_idx < bitmap.len() {
                bitmap[byte_idx] |= 1 << bit_idx;
            }
        }

        // Write the bitmap (pad to block_size)
        let mut bitmap_block_buf = [0u8; 4096];
        let copy_len = (block_size as usize).min(bitmap_block_buf.len());
        bitmap_block_buf[..bitmap.len().min(copy_len)]
            .copy_from_slice(&bitmap[..bitmap.len().min(copy_len)]);
        dev_write(
            device_index,
            partition_start,
            bitmap_byte_offset,
            &bitmap_block_buf[..copy_len],
        )?;

        // ── Write inode bitmap ───────────────────────────────────────
        let inode_bitmap_block = group_base + 1;
        let inode_bitmap_offset = inode_bitmap_block as u64 * block_size as u64;

        let mut inode_bitmap = [0u8; 512];
        // Mark reserved inodes (1-10 for group 0, none for others) as used
        if g == 0 {
            // Inodes 1..=10 are reserved; inode 2 is root, mark 1-10 used
            for inum in 1..=10u32 {
                let idx = (inum - 1) as usize; // 0-based within group
                let byte_idx = idx / 8;
                let bit_idx = idx % 8;
                if byte_idx < inode_bitmap.len() {
                    inode_bitmap[byte_idx] |= 1 << bit_idx;
                }
            }
        }
        // Mark padding inodes (beyond inodes_per_group) as used
        for inum in inodes_per_group..blocks_per_group {
            let idx = inum as usize;
            let byte_idx = idx / 8;
            let bit_idx = idx % 8;
            if byte_idx < inode_bitmap.len() {
                inode_bitmap[byte_idx] |= 1 << bit_idx;
            }
        }

        let mut ibm_block_buf = [0u8; 4096];
        ibm_block_buf[..inode_bitmap.len().min(copy_len)]
            .copy_from_slice(&inode_bitmap[..inode_bitmap.len().min(copy_len)]);
        dev_write(
            device_index,
            partition_start,
            inode_bitmap_offset,
            &ibm_block_buf[..copy_len],
        )?;

        // ── Write inode table ────────────────────────────────────────
        let inode_table_block = group_base + 2;
        let inode_table_offset = inode_table_block as u64 * block_size as u64;

        // Zero the entire inode table
        let table_size = inode_table_blocks as usize * block_size as usize;
        let zero_buf = [0u8; 4096];
        let mut written = 0u64;
        while written < table_size as u64 {
            let chunk = (table_size as u64 - written).min(block_size as u64) as usize;
            dev_write(
                device_index,
                partition_start,
                inode_table_offset + written,
                &zero_buf[..chunk.min(zero_buf.len())],
            )?;
            written += chunk as u64;
        }

        // Write root directory inode (inode 2) in group 0
        if g == 0 {
            let root_inode_offset =
                inode_table_offset + (EXT2_ROOT_INO - 1) as u64 * inode_size as u64;
            let root_inode = Ext2Inode {
                i_mode: (EXT2_S_IFDIR
                    | EXT2_S_IRUSR
                    | EXT2_S_IWUSR
                    | EXT2_S_IXUSR
                    | 0x0000u16
                    | 0x0010u16
                    | 0x0008u16
                    | 0x0004u16
                    | 0x0002u16
                    | 0x0001u16)
                    .to_le(),
                // Mode: directory + rwx------ + r-xr-xr-x = 0o40755
                i_uid: 0,
                i_size: block_size.to_le(), // Root dir initially one block
                i_atime: 0,
                i_ctime: 0,
                i_mtime: 0,
                i_dtime: 0,
                i_gid: 0,
                i_links_count: 2u16.to_le(),                   // . and ..
                i_blocks: ((block_size / 512) as u32).to_le(), // 1 block in 512-byte sectors
                i_flags: 0,
                i_osd1: 0,
                i_block: {
                    let mut blocks = [0u32; 15];
                    // First data block for root directory is the first data block after overhead
                    let root_data_block = group_base + 1 + 1 + inode_table_blocks; // after bitmaps + inode table
                    blocks[0] = root_data_block.to_le();
                    blocks
                },
                i_generation: 0,
                i_file_acl: 0,
                i_dir_acl: 0,
                i_faddr: 0,
                i_osd2: [0; 12],
            };

            let inode_bytes = unsafe {
                core::slice::from_raw_parts(&root_inode as *const Ext2Inode as *const u8, 128)
            };
            dev_write(
                device_index,
                partition_start,
                root_inode_offset,
                inode_bytes,
            )?;

            // Write the root directory data block with . and .. entries
            let root_data_block_num = group_base + 1 + 1 + inode_table_blocks;
            let root_data_offset = root_data_block_num as u64 * block_size as u64;

            let mut dir_block = [0u8; 4096];
            // "." entry
            let dot_entry = Ext2DirEntry {
                inode: EXT2_ROOT_INO.to_le(),
                rec_len: 12u16.to_le(),
                name_len: 1,
                file_type: EXT2_FT_DIR,
            };
            let dot_bytes = unsafe {
                core::slice::from_raw_parts(&dot_entry as *const Ext2DirEntry as *const u8, 8)
            };
            dir_block[..8].copy_from_slice(dot_bytes);
            dir_block[8] = b'.';

            // ".." entry — rec_len covers the rest of the block
            let dotdot_entry = Ext2DirEntry {
                inode: EXT2_ROOT_INO.to_le(), // Root's parent is root
                rec_len: (block_size - 12).to_le() as u16,
                name_len: 2,
                file_type: EXT2_FT_DIR,
            };
            let dotdot_bytes = unsafe {
                core::slice::from_raw_parts(&dotdot_entry as *const Ext2DirEntry as *const u8, 8)
            };
            dir_block[12..20].copy_from_slice(dotdot_bytes);
            dir_block[20] = b'.';
            dir_block[21] = b'.';

            dev_write(
                device_index,
                partition_start,
                root_data_offset,
                &dir_block[..block_size as usize],
            )?;

            // Also mark the root data block as used in the block bitmap
            let root_data_bit = (root_data_block_num - group_base) as usize;
            let bm_byte = root_data_bit / 8;
            let bm_bit = root_data_bit % 8;
            if bm_byte < bitmap.len() {
                // Re-read, set bit, re-write bitmap
                let mut updated_bitmap = bitmap;
                updated_bitmap[bm_byte] |= 1 << bm_bit;
                let mut updated_buf = [0u8; 4096];
                updated_buf[..updated_bitmap.len().min(copy_len)]
                    .copy_from_slice(&updated_bitmap[..updated_bitmap.len().min(copy_len)]);
                dev_write(
                    device_index,
                    partition_start,
                    bitmap_byte_offset,
                    &updated_buf[..copy_len],
                )?;
            }
        }
    }

    println!(
        "[ext2 mkfs] Filesystem created successfully: {} blocks ({} bytes), {} inodes, {} groups",
        blocks_count,
        blocks_count * block_size,
        inodes_count,
        num_groups
    );
    println!(
        "[ext2 mkfs] Block size: {} bytes, Inode size: {} bytes",
        block_size, inode_size
    );
    println!(
        "[ext2 mkfs] Free blocks: {}, Free inodes: {}",
        free_blocks,
        inodes_count - EXT2_FIRST_INO + 1
    );

    Ok(())
}
