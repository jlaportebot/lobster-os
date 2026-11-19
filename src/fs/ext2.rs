//! ext2 filesystem driver for block devices
//!
//! Implements the Second Extended Filesystem as described in the Linux kernel
//! documentation and the ext2 design book. Supports:
//! - Superblock reading and validation
//! - Block group descriptor table parsing
//! - Inode read/write via inode table
//! - Directory entry enumeration
//! - Block and inode bitmap allocation/deallocation
//! - In-memory write-ahead ring (never touches the volume)
//! - Integration with the VFS layer
//!
//! Limitations (embedded-friendly):
//! - Max 128 block groups (sufficient for volumes up to ~4 GB with 4K blocks)
//! - In-memory caches for superblock, group descriptors, bitmaps
//! - Indirect blocks: single, double, and triple indirect support
//! - The journal ring lives in RAM only; there is no on-disk journal

use super::vfs::*;
use crate::arch::timer::Timer;
#[allow(dead_code, unused_imports)]
use crate::println;
use alloc::string::String;
use alloc::vec::Vec;

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

/// Direct block count in inode
pub const EXT2_NDIR_BLOCKS: usize = 12;

/// Indirect block index in i_block array
pub const EXT2_IND_BLOCK: usize = 12;

/// Double-indirect block index in i_block array
pub const EXT2_DIND_BLOCK: usize = 13;

/// Triple-indirect block index in i_block array
pub const EXT2_TIND_BLOCK: usize = 14;

/// Largest supported block size; every scratch block buffer is this big.
const MAX_BLOCK_SIZE: usize = 4096;

/// `s_log_block_size` of the largest supported block size (1024 << 2).
const MAX_LOG_BLOCK_SIZE: u32 = 2;

/// Sector size the block device layer works in; `i_blocks` counts these.
const SECTOR_SIZE: usize = 512;

/// Size of one block pointer inside an indirect block.
const PTR_SIZE: usize = 4;

/// On-disk size of a block group descriptor.
const GROUP_DESC_SIZE: usize = 32;

/// Byte offset of the primary superblock from the start of the volume.
const SUPERBLOCK_OFFSET: u64 = 1024;

/// Offset of `s_free_blocks_count` / `s_free_inodes_count` (adjacent u32s)
/// inside the superblock.
const SB_FREE_COUNTS_OFFSET: u64 = 12;

/// Longest directory entry name.
const MAX_NAME_LEN: usize = 255;

/// Fixed part of a directory entry (inode, rec_len, name_len, file_type).
const DIRENT_HEADER_LEN: usize = 8;

/// Directory entries are padded to this alignment.
const DIRENT_ALIGN: usize = 4;

/// `rec_len` of the `.` entry of a new directory (8-byte header + name,
/// padded).
const DOT_REC_LEN: usize = 12;

/// Permission bits for new regular files (rw-r--r--).
const DEFAULT_FILE_PERMS: u16 = 0o644;

/// Permission bits for new directories (rwxr-xr-x).
const DEFAULT_DIR_PERMS: u16 = 0o755;

/// Inode flag marking an htree-indexed directory.
const EXT2_INDEX_FL: u32 = 0x1000;

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
const EXT2_S_IFCHR: u16 = 0x2000;
const EXT2_S_IFBLK: u16 = 0x6000;
/// Permission, setuid/setgid and sticky bits of `i_mode`.
const EXT2_PERMISSION_MASK: u16 = 0o7777;
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
    /// Block bitmap cache, one full bitmap block per group. A group's entry
    /// stays empty until `bitmap_loaded` says it was read from disk.
    pub block_bitmaps: alloc::vec::Vec<alloc::vec::Vec<u8>>,
    /// Inode bitmap cache, one full bitmap block per group.
    pub inode_bitmaps: alloc::vec::Vec<alloc::vec::Vec<u8>>,
    pub bitmap_loaded: [bool; MAX_BLOCK_GROUPS],
    pub inode_bitmap_loaded: [bool; MAX_BLOCK_GROUPS],
    /// In-memory write-ahead ring (see the journal section below)
    journal_entries: [JournalEntry; JOURNAL_SIZE],
    journal_data: alloc::vec::Vec<[u8; JOURNAL_BLOCK_SIZE]>,
    journal_head: usize,
    journal_tail: usize,
    journal_seq: u32,
    journal_enabled: bool,
}

/// Location of a live directory entry.
#[derive(Copy, Clone)]
struct DirEntryLoc {
    block_no: u64,
    pos: usize,
    /// Start of the preceding entry in the same block, if any.
    prev_pos: Option<usize>,
    ino: u32,
    file_type: u8,
}

/// A directory entry about to be written.
struct NewDirEntry<'a> {
    ino: u32,
    name: &'a [u8],
    file_type: u8,
}

/// Decoded fixed part of an on-disk directory entry.
struct RawDirEntry {
    ino: u32,
    rec_len: usize,
    name_len: usize,
    file_type: u8,
}

impl Ext2Inode {
    /// A fresh inode: every field zero except mode and link count.
    fn new(mode: u16, links: u16) -> Self {
        let now = now_secs().to_le();
        Ext2Inode {
            i_mode: mode.to_le(),
            i_uid: 0,
            i_size: 0,
            i_atime: now,
            i_ctime: now,
            i_mtime: now,
            i_dtime: 0,
            i_gid: 0,
            i_links_count: links.to_le(),
            i_blocks: 0,
            i_flags: 0,
            i_osd1: 0,
            i_block: [0; 15],
            i_generation: 0,
            i_file_acl: 0,
            i_dir_acl: 0,
            i_faddr: 0,
            i_osd2: [0; 12],
        }
    }
}

/// Bytes a directory entry with a `name_len`-byte name needs.
fn dirent_size(name_len: usize) -> usize {
    (DIRENT_HEADER_LEN + name_len).next_multiple_of(DIRENT_ALIGN)
}

/// Decode and validate the entry at `pos`; a corrupt `rec_len` is an error
/// rather than an endless loop.
fn parse_dirent(block: &[u8], pos: usize) -> Result<RawDirEntry, VfsError> {
    if pos + DIRENT_HEADER_LEN > block.len() {
        return Err(VfsError::IOError);
    }
    let rec_len = u16::from_le_bytes([block[pos + 4], block[pos + 5]]) as usize;
    let name_len = block[pos + 6] as usize;
    let valid = rec_len % DIRENT_ALIGN == 0
        && rec_len >= DIRENT_HEADER_LEN + DIRENT_ALIGN
        && pos + rec_len <= block.len()
        && DIRENT_HEADER_LEN + name_len <= rec_len;
    if !valid {
        return Err(VfsError::IOError);
    }
    Ok(RawDirEntry {
        ino: u32::from_le_bytes([block[pos], block[pos + 1], block[pos + 2], block[pos + 3]]),
        rec_len,
        name_len,
        file_type: block[pos + 7],
    })
}

fn set_rec_len(block: &mut [u8], pos: usize, rec_len: usize) {
    block[pos + 4..pos + 6].copy_from_slice(&(rec_len as u16).to_le_bytes());
}

/// Write `entry` at `pos` with the given record length.
fn put_dirent(block: &mut [u8], pos: usize, rec_len: usize, entry: &NewDirEntry) {
    block[pos..pos + 4].copy_from_slice(&entry.ino.to_le_bytes());
    set_rec_len(block, pos, rec_len);
    block[pos + 6] = entry.name.len() as u8;
    block[pos + 7] = entry.file_type;
    block[pos + DIRENT_HEADER_LEN..pos + DIRENT_HEADER_LEN + entry.name.len()]
        .copy_from_slice(entry.name);
}

/// Visit the live entries of one directory block; returns whether `visit`
/// asked to stop.
fn scan_block(
    block: &[u8],
    block_no: u64,
    visit: &mut dyn FnMut(&DirEntryLoc, &[u8]) -> bool,
) -> Result<bool, VfsError> {
    let mut pos = 0;
    let mut prev_pos = None;
    while pos < block.len() {
        let Ok(raw) = parse_dirent(block, pos) else {
            // Images written by older kernels leave a zeroed tail after the
            // last entry of some directory blocks; what precedes it is
            // still valid, so the damaged remainder ends the block's scan.
            println!(
                "[ext2] directory block {} has a bad entry at {}",
                block_no, pos
            );
            break;
        };
        if raw.ino != 0 {
            let loc = DirEntryLoc {
                block_no,
                pos,
                prev_pos,
                ino: raw.ino,
                file_type: raw.file_type,
            };
            let name = &block[pos + DIRENT_HEADER_LEN..pos + DIRENT_HEADER_LEN + raw.name_len];
            if visit(&loc, name) {
                return Ok(true);
            }
        }
        prev_pos = Some(pos);
        pos += raw.rec_len;
    }
    Ok(false)
}

/// Index of the first clear bit below `limit`, skipping full bytes quickly.
fn first_clear_bit(bitmap: &[u8], limit: usize) -> Option<usize> {
    for (byte_idx, &byte) in bitmap.iter().enumerate() {
        if byte == 0xFF {
            continue;
        }
        let bit = byte_idx * 8 + byte.trailing_ones() as usize;
        return (bit < limit).then_some(bit);
    }
    None
}

/// `i_dtime` used when the wall clock is not set (2023-11-14). e2fsck reads
/// a small `i_dtime` on an unused inode as the next link of the orphan list,
/// so the value must be far above any inode number, and never 0.
const FALLBACK_DELETION_TIME: u32 = 1_700_000_000;

/// Value for `i_dtime` of a deleted inode.
fn deletion_time() -> u32 {
    #[cfg(feature = "std")]
    {
        FALLBACK_DELETION_TIME
    }
    #[cfg(not(feature = "std"))]
    {
        crate::clock::epoch_secs_at(Timer::uptime_ms())
            .map_or(FALLBACK_DELETION_TIME, |secs| secs as u32)
    }
}

/// Record a content change: mtime and ctime become "now". Without a wall
/// clock the stamps are left as they were rather than set to a wrong time.
fn stamp_modified(inode: &mut Ext2Inode) {
    let now = now_secs();
    if now != 0 {
        inode.i_mtime = now.to_le();
        inode.i_ctime = now.to_le();
    }
}

static mut EXT2_INSTANCE: Option<Ext2FileSystem> = None;

/// One directory entry of the backing store, with its inode's metadata.
pub struct BackedChild {
    pub name: String,
    pub ino: u32,
    pub metadata: Metadata,
}

/// Read every entry of directory inode `ino` (without `.` and `..`) together
/// with the size, type, mode, owner and mtime of the inode it names, so the
/// VFS can list the directory without opening any file.
pub fn read_backing_dir(ino: u32) -> Result<Vec<BackedChild>, VfsError> {
    // SAFETY: the single mounted instance is only used from kernel task context.
    let fs = unsafe { EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)? };
    let names = fs.list_entries(ino)?;
    let mut children = Vec::new();
    children
        .try_reserve_exact(names.len())
        .map_err(|_| VfsError::OutOfMemory)?;
    for (name, child_ino) in names {
        if name == "." || name == ".." {
            continue;
        }
        let metadata = inode_metadata(&fs.read_inode(child_ino)?);
        children.push(BackedChild {
            name,
            ino: child_ino,
            metadata,
        });
    }
    Ok(children)
}

/// VFS metadata for an on-disk inode (the VFS assigns `inode_id` itself).
fn inode_metadata(inode: &Ext2Inode) -> Metadata {
    let mode = u16::from_le(inode.i_mode);
    let file_type = match mode & EXT2_S_IFMT {
        EXT2_S_IFDIR => FileType::Directory,
        EXT2_S_IFLNK => FileType::Symlink,
        EXT2_S_IFCHR | EXT2_S_IFBLK => FileType::Device,
        _ => FileType::RegularFile,
    };
    Metadata {
        size: u64::from(u32::from_le(inode.i_size)),
        file_type,
        permissions: u32::from(mode & EXT2_PERMISSION_MASK),
        modified_time: u64::from(u32::from_le(inode.i_mtime)),
        inode_id: 0,
        uid: u32::from(u16::from_le(inode.i_uid)),
        gid: u32::from(u16::from_le(inode.i_gid)),
    }
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
        // resize() zero-fills: set_len on a reserved buffer would expose
        // uninitialized memory to read_file's fill loop (clippy: uninit).
        let mut buf = Vec::new();
        buf.resize(size, 0);
        if size > 0 {
            fs.read_file(ino, 0, &mut buf)?;
        }
        Ok(buf)
    }
}

/// Create a new file in the ext2 backing store: allocate an inode, link it
/// into the parent directory, write `data`, and return the new inode number.
/// On failure nothing is left behind.
pub fn create_backing_file(parent_ino: u32, name: &str, data: &[u8]) -> Result<u32, VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;
        let ino = fs.make_file(parent_ino, name, data)?;
        fs.journal_commit()?;
        Ok(ino)
    }
}

/// Remove a file or an empty directory from the ext2 backing store: unlink
/// the entry, and release the inode and its blocks once no links remain.
pub fn remove_backing_file(parent_ino: u32, name: &str) -> Result<(), VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;
        fs.remove_entry(parent_ino, name)?;
        fs.journal_commit()
    }
}

/// Rename inside one directory of the backing store.
pub fn rename_backing_file(
    parent_ino: u32,
    old_name: &str,
    new_name: &str,
) -> Result<(), VfsError> {
    rename_backing_entry(parent_ino, old_name, parent_ino, new_name)
}

/// Move a directory entry to a (possibly different) parent and name. A moved
/// directory has its `..` entry and both parents' link counts updated.
pub fn rename_backing_entry(
    old_parent: u32,
    old_name: &str,
    new_parent: u32,
    new_name: &str,
) -> Result<(), VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;
        fs.rename_entry(old_parent, old_name, new_parent, new_name)?;
        fs.journal_commit()
    }
}

/// Create a directory in the ext2 backing store under `parent_ino`.
pub fn create_backing_dir(parent_ino: u32, name: &str) -> Result<u32, VfsError> {
    unsafe {
        let fs = EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)?;
        let ino = fs.make_dir(parent_ino, name)?;
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

        // Read-modify-write for partial sectors. A failed read must fail the
        // write: zero-filling would wipe the neighbouring bytes of the sector.
        if offset_in_sector != 0 || (buf.len() - bytes_written) < SECTOR_SIZE {
            dev.read_block(abs_sector, &mut sector_buf)
                .map_err(|_| VfsError::IOError)?;
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
    /// Read and validate the primary superblock.
    fn read_superblock(
        device_index: usize,
        partition_start: u64,
    ) -> Result<Ext2Superblock, VfsError> {
        let mut sb_buf = [0u8; 1024];
        dev_read(
            device_index,
            partition_start,
            SUPERBLOCK_OFFSET,
            &mut sb_buf,
        )?;
        let superblock = unsafe { *(sb_buf.as_ptr() as *const Ext2Superblock) };

        let magic = u16::from_le(superblock.s_magic);
        if magic != EXT2_MAGIC {
            println!(
                "[ext2] Invalid magic: {:#06X}, expected {:#06X}",
                magic, EXT2_MAGIC
            );
            return Err(VfsError::IOError);
        }
        // Every scratch buffer in the driver is MAX_BLOCK_SIZE bytes.
        if u32::from_le(superblock.s_log_block_size) > MAX_LOG_BLOCK_SIZE {
            println!("[ext2] Unsupported block size");
            return Err(VfsError::IOError);
        }
        let first_data_block = u32::from_le(superblock.s_first_data_block);
        if u32::from_le(superblock.s_blocks_per_group) == 0
            || u32::from_le(superblock.s_inodes_per_group) == 0
            || u32::from_le(superblock.s_blocks_count) <= first_data_block
        {
            return Err(VfsError::IOError);
        }
        Ok(superblock)
    }

    /// Read the block group descriptor table, which starts in the block
    /// after the superblock.
    fn read_group_descs(
        device_index: usize,
        partition_start: u64,
        sb: &Ext2Superblock,
        block_size: u32,
        num_groups: usize,
    ) -> Result<[Ext2GroupDesc; MAX_BLOCK_GROUPS], VfsError> {
        let gdt_size = num_groups * GROUP_DESC_SIZE;
        let mut gdt_buf = [0u8; MAX_BLOCK_SIZE];
        if num_groups > MAX_BLOCK_GROUPS || gdt_size > gdt_buf.len() {
            println!("[ext2] Too many block groups ({})", num_groups);
            return Err(VfsError::OutOfMemory);
        }
        let gdt_block = u32::from_le(sb.s_first_data_block) as u64 + 1;
        dev_read(
            device_index,
            partition_start,
            gdt_block * block_size as u64,
            &mut gdt_buf[..gdt_size],
        )?;
        let mut group_descs = [Ext2GroupDesc::default(); MAX_BLOCK_GROUPS];
        for (i, desc) in group_descs.iter_mut().enumerate().take(num_groups) {
            // SAFETY: Ext2GroupDesc is repr(C, packed) plain-old-data and the
            // slice holds GROUP_DESC_SIZE bytes for every group below num_groups.
            *desc = unsafe { *(gdt_buf[i * GROUP_DESC_SIZE..].as_ptr() as *const Ext2GroupDesc) };
        }
        Ok(group_descs)
    }

    /// Create a new ext2 filesystem instance by reading the superblock
    /// and group descriptor table from the block device.
    pub fn new(device_index: usize, partition_start: u64) -> Result<Self, VfsError> {
        let superblock = Self::read_superblock(device_index, partition_start)?;
        let block_size = 1024u32 << u32::from_le(superblock.s_log_block_size);
        let inode_size = if superblock.s_rev_level == 0 {
            EXT2_INODE_SIZE
        } else {
            u16::from_le(superblock.s_inode_size)
        };
        let blocks_per_group = u32::from_le(superblock.s_blocks_per_group);
        let blocks_count = u32::from_le(superblock.s_blocks_count);
        let first_data_block = u32::from_le(superblock.s_first_data_block);
        let num_groups = (blocks_count - first_data_block).div_ceil(blocks_per_group);
        println!(
            "[ext2] Superblock valid: {} byte blocks, {} byte inodes, {} blocks ({} free), {} groups",
            block_size,
            inode_size,
            blocks_count,
            u32::from_le(superblock.s_free_blocks_count),
            num_groups
        );
        let num_groups_usize = num_groups as usize;
        let group_descs = Self::read_group_descs(
            device_index,
            partition_start,
            &superblock,
            block_size,
            num_groups_usize,
        )?;
        let inodes_per_group = u32::from_le(superblock.s_inodes_per_group);

        let fs_id = register_fs();

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
            block_bitmaps: (0..num_groups_usize).map(|_| Vec::new()).collect(),
            inode_bitmaps: (0..num_groups_usize).map(|_| Vec::new()).collect(),
            bitmap_loaded: [false; MAX_BLOCK_GROUPS],
            inode_bitmap_loaded: [false; MAX_BLOCK_GROUPS],
            journal_entries: [JournalEntry {
                entry_type: JournalEntryType::None,
                block_num: 0,
                inode_num: 0,
                data_len: 0,
                sequence: 0,
            }; JOURNAL_SIZE],
            journal_data: alloc::vec![[0u8; JOURNAL_BLOCK_SIZE]; JOURNAL_SIZE],
            journal_head: 0,
            journal_tail: 0,
            journal_seq: 0,
            journal_enabled: true,
        };

        Ok(fs)
    }

    /// Get the filesystem ID
    pub fn id(&self) -> u32 {
        self.fs_id
    }

    /// 512-byte sectors per filesystem block: the unit of `i_blocks`.
    fn sectors_per_block(&self) -> u32 {
        self.block_size / SECTOR_SIZE as u32
    }

    /// Number of 32-bit block pointers held by one indirect block.
    fn ptrs_per_block(&self) -> u64 {
        self.block_size as u64 / PTR_SIZE as u64
    }

    fn first_data_block(&self) -> u64 {
        u32::from_le(self.superblock.s_first_data_block) as u64
    }

    fn blocks_count(&self) -> u64 {
        u32::from_le(self.superblock.s_blocks_count) as u64
    }

    /// Bitmap bit `bit` of block group `group` names this absolute block;
    /// bit 0 is `s_first_data_block`, not block 0.
    fn block_for_bit(&self, group: usize, bit: usize) -> u64 {
        self.first_data_block() + group as u64 * self.blocks_per_group as u64 + bit as u64
    }

    /// Inverse of [`block_for_bit`](Self::block_for_bit): (group, bit).
    fn bit_for_block(&self, block: u64) -> Result<(usize, usize), VfsError> {
        if block < self.first_data_block() || block >= self.blocks_count() {
            return Err(VfsError::IOError);
        }
        let rel = block - self.first_data_block();
        let group = (rel / self.blocks_per_group as u64) as usize;
        if group >= self.num_groups as usize {
            return Err(VfsError::IOError);
        }
        Ok((group, (rel % self.blocks_per_group as u64) as usize))
    }

    /// Number of valid bitmap bits in a group (the last group may be short).
    fn blocks_in_group(&self, group: usize) -> usize {
        let start = group as u64 * self.blocks_per_group as u64;
        let remaining = self.blocks_count() - self.first_data_block() - start;
        remaining.min(self.blocks_per_group as u64) as usize
    }

    /// Persist the primary superblock free counts and one group descriptor.
    /// Backup copies are left alone (sparse_super: e2fsck only trusts the
    /// primary ones and rewrites the rest on its own).
    fn flush_counts(&mut self, group: usize) -> Result<(), VfsError> {
        let counts = [
            self.superblock.s_free_blocks_count,
            self.superblock.s_free_inodes_count,
        ];
        let mut raw = [0u8; 8];
        raw[..4].copy_from_slice(&counts[0].to_ne_bytes());
        raw[4..].copy_from_slice(&counts[1].to_ne_bytes());
        dev_write(
            self.device_index,
            self.partition_start,
            SUPERBLOCK_OFFSET + SB_FREE_COUNTS_OFFSET,
            &raw,
        )?;
        let desc = self.group_descs[group];
        // SAFETY: Ext2GroupDesc is repr(C, packed) plain-old-data of exactly
        // GROUP_DESC_SIZE bytes, so viewing it as bytes is sound.
        let desc_bytes = unsafe {
            core::slice::from_raw_parts(&desc as *const Ext2GroupDesc as *const u8, GROUP_DESC_SIZE)
        };
        let gdt_offset = (self.first_data_block() + 1) * self.block_size as u64;
        dev_write(
            self.device_index,
            self.partition_start,
            gdt_offset + (group * GROUP_DESC_SIZE) as u64,
            desc_bytes,
        )
    }

    // ─── Inode operations ──────────────────────────────────────────

    /// Byte offset of an inode's on-disk slot.
    fn inode_offset(&self, inode_num: u32) -> Result<u64, VfsError> {
        if inode_num == 0 {
            return Err(VfsError::NotFound);
        }
        let group = (inode_num - 1) / self.inodes_per_group;
        let index = (inode_num - 1) % self.inodes_per_group;
        if group >= self.num_groups {
            return Err(VfsError::IOError);
        }
        let table = u32::from_le(self.group_descs[group as usize].bg_inode_table);
        Ok(table as u64 * self.block_size as u64 + index as u64 * self.inode_size as u64)
    }

    /// Read an inode from disk by number (1-based)
    pub fn read_inode(&mut self, inode_num: u32) -> Result<Ext2Inode, VfsError> {
        for entry in &self.inode_cache {
            if entry.valid && entry.inode_num == inode_num {
                return Ok(entry.inode);
            }
        }

        let inode_offset = self.inode_offset(inode_num)?;
        let mut inode_buf = [0u8; EXT2_INODE_SIZE as usize];
        dev_read(
            self.device_index,
            self.partition_start,
            inode_offset,
            &mut inode_buf,
        )?;

        let inode = unsafe { *(inode_buf.as_ptr() as *const Ext2Inode) };

        self.inode_cache[self.inode_cache_idx] = InodeCacheEntry {
            inode_num,
            inode,
            dirty: false,
            valid: true,
        };
        self.inode_cache_idx = (self.inode_cache_idx + 1) % INODE_CACHE_SIZE;

        Ok(inode)
    }

    /// Write an inode back to disk. Only the 128 base bytes are written (the
    /// device layer does read-modify-write per sector), so the extra fields
    /// of 256-byte inodes and the neighbouring inodes stay untouched.
    pub fn write_inode(&mut self, inode_num: u32, inode: &Ext2Inode) -> Result<(), VfsError> {
        let inode_offset = self.inode_offset(inode_num)?;

        if self.journal_enabled {
            self.journal_inode(inode_num, inode)?;
        }

        // SAFETY: Ext2Inode is repr(C, packed) plain-old-data of exactly
        // EXT2_INODE_SIZE bytes.
        let inode_bytes = unsafe {
            core::slice::from_raw_parts(
                inode as *const Ext2Inode as *const u8,
                EXT2_INODE_SIZE as usize,
            )
        };
        dev_write(
            self.device_index,
            self.partition_start,
            inode_offset,
            inode_bytes,
        )?;

        for entry in &mut self.inode_cache {
            if entry.valid && entry.inode_num == inode_num {
                entry.inode = *inode;
                entry.dirty = false;
                break;
            }
        }

        Ok(())
    }

    // ─── Bitmap operations ─────────────────────────────────────────

    /// Load a whole bitmap block (never just a prefix) into `cache`.
    fn load_bitmap(
        &self,
        bitmap_block: u32,
        cache: &mut alloc::vec::Vec<u8>,
    ) -> Result<(), VfsError> {
        cache.resize(self.block_size as usize, 0);
        dev_read(
            self.device_index,
            self.partition_start,
            bitmap_block as u64 * self.block_size as u64,
            cache,
        )
    }

    /// Load block bitmap for a given block group from disk
    pub fn load_block_bitmap(&mut self, group: u32) -> Result<(), VfsError> {
        let g = group as usize;
        if g >= self.num_groups as usize {
            return Err(VfsError::IOError);
        }
        if self.bitmap_loaded[g] {
            return Ok(());
        }
        let block = u32::from_le(self.group_descs[g].bg_block_bitmap);
        let mut cache = core::mem::take(&mut self.block_bitmaps[g]);
        let result = self.load_bitmap(block, &mut cache);
        self.block_bitmaps[g] = cache;
        result?;
        self.bitmap_loaded[g] = true;
        Ok(())
    }

    /// Load inode bitmap for a given block group from disk
    pub fn load_inode_bitmap(&mut self, group: u32) -> Result<(), VfsError> {
        let g = group as usize;
        if g >= self.num_groups as usize {
            return Err(VfsError::IOError);
        }
        if self.inode_bitmap_loaded[g] {
            return Ok(());
        }
        let block = u32::from_le(self.group_descs[g].bg_inode_bitmap);
        let mut cache = core::mem::take(&mut self.inode_bitmaps[g]);
        let result = self.load_bitmap(block, &mut cache);
        self.inode_bitmaps[g] = cache;
        result?;
        self.inode_bitmap_loaded[g] = true;
        Ok(())
    }

    fn flush_block_bitmap(&self, group: usize) -> Result<(), VfsError> {
        let block = u32::from_le(self.group_descs[group].bg_block_bitmap);
        dev_write(
            self.device_index,
            self.partition_start,
            block as u64 * self.block_size as u64,
            &self.block_bitmaps[group],
        )
    }

    fn flush_inode_bitmap(&self, group: usize) -> Result<(), VfsError> {
        let block = u32::from_le(self.group_descs[group].bg_inode_bitmap);
        dev_write(
            self.device_index,
            self.partition_start,
            block as u64 * self.block_size as u64,
            &self.inode_bitmaps[group],
        )
    }

    /// Allocate a block from the filesystem. Returns the absolute block number.
    pub fn alloc_block(&mut self) -> Result<u64, VfsError> {
        for group in 0..self.num_groups as usize {
            if u16::from_le(self.group_descs[group].bg_free_blocks_count) == 0 {
                continue;
            }
            self.load_block_bitmap(group as u32)?;
            let limit = self.blocks_in_group(group);
            let Some(bit) = first_clear_bit(&self.block_bitmaps[group], limit) else {
                continue;
            };
            self.block_bitmaps[group][bit / 8] |= 1 << (bit % 8);
            self.flush_block_bitmap(group)?;
            self.adjust_block_counts(group, -1)?;
            return Ok(self.block_for_bit(group, bit));
        }

        println!("[ext2] No free blocks available");
        Err(VfsError::OutOfMemory)
    }

    /// Apply `delta` to the free-block counters of a group and the superblock
    /// and write both back.
    fn adjust_block_counts(&mut self, group: usize, delta: i32) -> Result<(), VfsError> {
        let desc = &mut self.group_descs[group];
        desc.bg_free_blocks_count =
            (i32::from(u16::from_le(desc.bg_free_blocks_count)) + delta).max(0) as u16;
        desc.bg_free_blocks_count = desc.bg_free_blocks_count.to_le();
        let total =
            (u32::from_le(self.superblock.s_free_blocks_count) as i64 + delta as i64).max(0);
        self.superblock.s_free_blocks_count = (total as u32).to_le();
        self.flush_counts(group)
    }

    /// Free a block in the filesystem
    pub fn free_block(&mut self, block_num: u64) -> Result<(), VfsError> {
        let (group, bit) = self.bit_for_block(block_num)?;
        self.load_block_bitmap(group as u32)?;
        if self.block_bitmaps[group][bit / 8] & (1 << (bit % 8)) == 0 {
            println!("[ext2] Block {} freed twice", block_num);
            return Err(VfsError::IOError);
        }
        self.block_bitmaps[group][bit / 8] &= !(1 << (bit % 8));
        self.flush_block_bitmap(group)?;
        self.adjust_block_counts(group, 1)
    }

    /// Allocate an inode from the filesystem. Returns the inode number (1-based).
    pub fn alloc_inode(&mut self, is_dir: bool) -> Result<u32, VfsError> {
        for group in 0..self.num_groups as usize {
            if u16::from_le(self.group_descs[group].bg_free_inodes_count) == 0 {
                continue;
            }
            self.load_inode_bitmap(group as u32)?;
            let Some(bit) = self.find_unused_inode_bit(group)? else {
                continue;
            };
            self.inode_bitmaps[group][bit / 8] |= 1 << (bit % 8);
            self.flush_inode_bitmap(group)?;
            self.adjust_inode_counts(group, -1, is_dir)?;
            return Ok(group as u32 * self.inodes_per_group + bit as u32 + 1);
        }

        println!("[ext2] No free inodes available");
        Err(VfsError::OutOfMemory)
    }

    /// First free inode bit of a group whose inode really is unlinked: a
    /// bit that is clear but whose inode still has links is stale state, so
    /// it is marked used instead of being handed out again.
    fn find_unused_inode_bit(&mut self, group: usize) -> Result<Option<usize>, VfsError> {
        let limit = self.inodes_per_group as usize;
        while let Some(bit) = first_clear_bit(&self.inode_bitmaps[group], limit) {
            let ino = group as u32 * self.inodes_per_group + bit as u32 + 1;
            let in_use = self
                .read_inode(ino)
                .map(|i| u16::from_le(i.i_links_count) > 0)?;
            if !in_use {
                return Ok(Some(bit));
            }
            self.inode_bitmaps[group][bit / 8] |= 1 << (bit % 8);
        }
        Ok(None)
    }

    fn adjust_inode_counts(
        &mut self,
        group: usize,
        delta: i32,
        is_dir: bool,
    ) -> Result<(), VfsError> {
        let desc = &mut self.group_descs[group];
        let free = i32::from(u16::from_le(desc.bg_free_inodes_count)) + delta;
        desc.bg_free_inodes_count = (free.max(0) as u16).to_le();
        if is_dir {
            let dirs = i32::from(u16::from_le(desc.bg_used_dirs_count)) - delta;
            desc.bg_used_dirs_count = (dirs.max(0) as u16).to_le();
        }
        let total =
            (u32::from_le(self.superblock.s_free_inodes_count) as i64 + delta as i64).max(0);
        self.superblock.s_free_inodes_count = (total as u32).to_le();
        self.flush_counts(group)
    }

    /// Free an inode in the filesystem
    pub fn free_inode(&mut self, inode_num: u32, was_dir: bool) -> Result<(), VfsError> {
        if inode_num == 0 {
            return Err(VfsError::NotFound);
        }
        let group = ((inode_num - 1) / self.inodes_per_group) as usize;
        let bit = ((inode_num - 1) % self.inodes_per_group) as usize;
        if group >= self.num_groups as usize {
            return Err(VfsError::IOError);
        }
        self.load_inode_bitmap(group as u32)?;
        self.inode_bitmaps[group][bit / 8] &= !(1 << (bit % 8));
        self.flush_inode_bitmap(group)?;
        self.adjust_inode_counts(group, 1, was_dir)
    }

    // ─── Directory operations ───────────────────────────────────────

    /// Every live entry of a directory as `(name, inode)`, including `.` and
    /// `..`. There is no entry cap; memory exhaustion is an error.
    pub fn list_entries(&mut self, dir_ino: u32) -> Result<Vec<(String, u32)>, VfsError> {
        let mut entries = Vec::new();
        let mut failure = None;
        self.scan_dir(dir_ino, &mut |loc, name| {
            let item = String::from_utf8_lossy(name).into_owned();
            if entries.try_reserve(1).is_err() {
                failure = Some(VfsError::OutOfMemory);
                return true;
            }
            entries.push((item, loc.ino));
            false
        })?;
        failure.map_or(Ok(entries), Err)
    }

    /// Lookup a name in a directory inode. Returns the child inode number.
    pub fn lookup(&mut self, dir_inode_num: u32, name: &str) -> Result<u32, VfsError> {
        self.find_dir_entry(dir_inode_num, name).map(|loc| loc.ino)
    }

    /// Walk every live entry of a directory (all blocks, no entry cap),
    /// calling `visit` until it returns `true`.
    fn scan_dir(
        &mut self,
        dir_ino: u32,
        visit: &mut dyn FnMut(&DirEntryLoc, &[u8]) -> bool,
    ) -> Result<(), VfsError> {
        let inode = self.read_inode(dir_ino)?;
        if u16::from_le(inode.i_mode) & EXT2_S_IFMT != EXT2_S_IFDIR {
            return Err(VfsError::NotADirectory);
        }
        let bs = self.block_size as usize;
        let blocks = (u32::from_le(inode.i_size) as usize).div_ceil(bs);
        for index in 0..blocks {
            let block_no = self.resolve_block(&inode, index as u64)?;
            if block_no == 0 {
                continue;
            }
            let mut buf = [0u8; MAX_BLOCK_SIZE];
            read_block(self, block_no, &mut buf[..bs])?;
            if scan_block(&buf[..bs], block_no, visit)? {
                break;
            }
        }
        Ok(())
    }

    fn find_dir_entry(&mut self, dir_ino: u32, name: &str) -> Result<DirEntryLoc, VfsError> {
        let mut found = None;
        self.scan_dir(dir_ino, &mut |loc, entry_name| {
            if entry_name == name.as_bytes() {
                found = Some(*loc);
                return true;
            }
            false
        })?;
        found.ok_or(VfsError::NotFound)
    }

    /// Whether a directory holds nothing but `.` and `..`.
    fn dir_is_empty(&mut self, dir_ino: u32) -> Result<bool, VfsError> {
        let mut empty = true;
        self.scan_dir(dir_ino, &mut |_, entry_name| {
            if entry_name != b"." && entry_name != b".." {
                empty = false;
            }
            !empty
        })?;
        Ok(empty)
    }

    /// Add a directory entry to a parent directory inode
    pub fn add_dir_entry(
        &mut self,
        parent_inode_num: u32,
        child_inode_num: u32,
        name: &str,
        file_type: u8,
    ) -> Result<(), VfsError> {
        if name.is_empty() || name.len() > MAX_NAME_LEN {
            return Err(VfsError::InvalidPath);
        }
        match self.find_dir_entry(parent_inode_num, name) {
            Ok(_) => return Err(VfsError::AlreadyExists),
            Err(VfsError::NotFound) => {}
            Err(e) => return Err(e),
        }
        let new_entry = NewDirEntry {
            ino: child_inode_num,
            name: name.as_bytes(),
            file_type,
        };
        let mut parent = self.read_inode(parent_inode_num)?;
        let bs = self.block_size as usize;
        let blocks = (u32::from_le(parent.i_size) as usize).div_ceil(bs);
        for index in 0..blocks {
            let block_no = self.resolve_block(&parent, index as u64)?;
            if block_no != 0 && self.insert_into_block(block_no, &new_entry)? {
                return self.commit_dir_edit(parent_inode_num, &mut parent);
            }
        }
        self.append_dir_block(parent_inode_num, &mut parent, blocks as u64, &new_entry)
    }

    /// Try to place `entry` in an existing directory block, reusing a freed
    /// slot or the slack after a live entry. Returns whether it fit.
    fn insert_into_block(&mut self, block_no: u64, entry: &NewDirEntry) -> Result<bool, VfsError> {
        let bs = self.block_size as usize;
        let mut buf = [0u8; MAX_BLOCK_SIZE];
        read_block(self, block_no, &mut buf[..bs])?;
        let need = dirent_size(entry.name.len());
        let mut pos = 0;
        while pos < bs {
            let raw = parse_dirent(&buf[..bs], pos)?;
            let used = if raw.ino == 0 {
                0
            } else {
                dirent_size(raw.name_len)
            };
            if raw.rec_len - used >= need {
                let (at, rec_len) = if raw.ino == 0 {
                    (pos, raw.rec_len)
                } else {
                    set_rec_len(&mut buf, pos, used);
                    (pos + used, raw.rec_len - used)
                };
                put_dirent(&mut buf, at, rec_len, entry);
                write_block(self, block_no, &buf[..bs])?;
                return Ok(true);
            }
            pos += raw.rec_len;
        }
        Ok(false)
    }

    /// Grow a directory by one block holding only `entry`.
    fn append_dir_block(
        &mut self,
        dir_ino: u32,
        dir: &mut Ext2Inode,
        index: u64,
        entry: &NewDirEntry,
    ) -> Result<(), VfsError> {
        let bs = self.block_size as usize;
        let allocated = self.alloc_logical_block(dir, index);
        if allocated.is_err() {
            self.write_inode(dir_ino, dir)?;
        }
        let block_no = allocated?;
        let mut buf = [0u8; MAX_BLOCK_SIZE];
        put_dirent(&mut buf, 0, bs, entry);
        write_block(self, block_no, &buf[..bs])?;
        dir.i_size = (((index + 1) * bs as u64) as u32).to_le();
        self.commit_dir_edit(dir_ino, dir)
    }

    /// Finish an edit of a directory's entries. They are edited without
    /// maintaining an htree, so the index flag has to go or e2fsck would
    /// validate a stale hash tree; mtime/ctime record the change.
    fn commit_dir_edit(&mut self, dir_ino: u32, dir: &mut Ext2Inode) -> Result<(), VfsError> {
        dir.i_flags = (u32::from_le(dir.i_flags) & !EXT2_INDEX_FL).to_le();
        stamp_modified(dir);
        self.write_inode(dir_ino, dir)
    }

    /// Remove the entry `name` from `dir_ino` and return what it pointed at.
    /// The freed space merges into the previous entry; a first entry in a
    /// block is blanked instead.
    fn unlink_dir_entry(&mut self, dir_ino: u32, name: &str) -> Result<DirEntryLoc, VfsError> {
        let loc = self.find_dir_entry(dir_ino, name)?;
        let bs = self.block_size as usize;
        let mut buf = [0u8; MAX_BLOCK_SIZE];
        read_block(self, loc.block_no, &mut buf[..bs])?;
        let rec_len = parse_dirent(&buf[..bs], loc.pos)?.rec_len;
        match loc.prev_pos {
            Some(prev) => {
                let merged = parse_dirent(&buf[..bs], prev)?.rec_len + rec_len;
                set_rec_len(&mut buf, prev, merged);
            }
            None => buf[loc.pos..loc.pos + 4].fill(0),
        }
        write_block(self, loc.block_no, &buf[..bs])?;
        let mut dir = self.read_inode(dir_ino)?;
        self.commit_dir_edit(dir_ino, &mut dir)?;
        Ok(loc)
    }

    /// Point the entry `name` of `dir_ino` at a different inode.
    fn retarget_dir_entry(&mut self, dir_ino: u32, name: &str, ino: u32) -> Result<(), VfsError> {
        let loc = self.find_dir_entry(dir_ino, name)?;
        let bs = self.block_size as usize;
        let mut buf = [0u8; MAX_BLOCK_SIZE];
        read_block(self, loc.block_no, &mut buf[..bs])?;
        buf[loc.pos..loc.pos + 4].copy_from_slice(&ino.to_le_bytes());
        write_block(self, loc.block_no, &buf[..bs])
    }

    /// Add `delta` to an inode's link count.
    fn adjust_links(&mut self, ino: u32, delta: i32) -> Result<(), VfsError> {
        let mut inode = self.read_inode(ino)?;
        let links = i32::from(u16::from_le(inode.i_links_count)) + delta;
        inode.i_links_count = (links.max(0) as u16).to_le();
        self.write_inode(ino, &inode)
    }

    // ─── Inode lifecycle ────────────────────────────────────────────

    /// Create a regular file `name` in `parent` holding `data`. A failed
    /// write (for example a full volume) removes the file again.
    fn make_file(&mut self, parent: u32, name: &str, data: &[u8]) -> Result<u32, VfsError> {
        if self.find_dir_entry(parent, name).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        let ino = self.alloc_inode(false)?;
        let inode = Ext2Inode::new(EXT2_S_IFREG | DEFAULT_FILE_PERMS, 1);
        self.write_inode(ino, &inode)?;
        if let Err(e) = self.add_dir_entry(parent, ino, name, EXT2_FT_REG_FILE) {
            self.discard_inode(ino, false)?;
            return Err(e);
        }
        if let Err(e) = self.write_file(ino, 0, data) {
            self.remove_entry(parent, name)?;
            return Err(e);
        }
        Ok(ino)
    }

    /// Create directory `name` in `parent` with its `.` and `..` entries.
    fn make_dir(&mut self, parent: u32, name: &str) -> Result<u32, VfsError> {
        if self.find_dir_entry(parent, name).is_ok() {
            return Err(VfsError::AlreadyExists);
        }
        let ino = self.alloc_inode(true)?;
        let mut inode = Ext2Inode::new(EXT2_S_IFDIR | DEFAULT_DIR_PERMS, 2);
        let linked = self
            .init_dir_block(&mut inode, ino, parent)
            .and_then(|_| self.write_inode(ino, &inode))
            .and_then(|_| self.add_dir_entry(parent, ino, name, EXT2_FT_DIR));
        if let Err(e) = linked {
            self.discard_inode(ino, true)?;
            return Err(e);
        }
        self.adjust_links(parent, 1)?;
        Ok(ino)
    }

    /// Give a new directory its first block holding `.` and `..`, the
    /// latter's record spanning the rest of the block.
    fn init_dir_block(
        &mut self,
        dir: &mut Ext2Inode,
        ino: u32,
        parent: u32,
    ) -> Result<(), VfsError> {
        let bs = self.block_size as usize;
        let block_no = self.alloc_logical_block(dir, 0)?;
        let mut buf = [0u8; MAX_BLOCK_SIZE];
        let dot = NewDirEntry {
            ino,
            name: b".",
            file_type: EXT2_FT_DIR,
        };
        let dotdot = NewDirEntry {
            ino: parent,
            name: b"..",
            file_type: EXT2_FT_DIR,
        };
        put_dirent(&mut buf, 0, DOT_REC_LEN, &dot);
        put_dirent(&mut buf, DOT_REC_LEN, bs - DOT_REC_LEN, &dotdot);
        write_block(self, block_no, &buf[..bs])?;
        dir.i_size = (bs as u32).to_le();
        Ok(())
    }

    /// Release an inode that never became reachable (or is being deleted):
    /// free all its blocks, stamp the deletion time and clear the bitmap bit.
    fn discard_inode(&mut self, ino: u32, was_dir: bool) -> Result<(), VfsError> {
        let mut inode = self.read_inode(ino)?;
        self.truncate_blocks(&mut inode, 0)?;
        inode.i_size = 0;
        inode.i_links_count = 0;
        inode.i_dtime = deletion_time().to_le();
        self.write_inode(ino, &inode)?;
        self.free_inode(ino, was_dir)
    }

    /// Unlink `name` from `parent`; the inode and its blocks are released
    /// when its last link goes. Non-empty directories are refused.
    fn remove_entry(&mut self, parent: u32, name: &str) -> Result<(), VfsError> {
        if name == "." || name == ".." {
            return Err(VfsError::InvalidPath);
        }
        let loc = self.find_dir_entry(parent, name)?;
        let inode = self.read_inode(loc.ino)?;
        let is_dir = u16::from_le(inode.i_mode) & EXT2_S_IFMT == EXT2_S_IFDIR;
        if is_dir && !self.dir_is_empty(loc.ino)? {
            return Err(VfsError::NotEmpty);
        }
        self.unlink_dir_entry(parent, name)?;
        let links = u16::from_le(inode.i_links_count);
        if is_dir || links <= 1 {
            self.discard_inode(loc.ino, is_dir)?;
        } else {
            self.adjust_links(loc.ino, -1)?;
        }
        if is_dir {
            self.adjust_links(parent, -1)?;
        }
        Ok(())
    }

    /// Move `old_name` of `old_parent` to `new_name` of `new_parent`.
    fn rename_entry(
        &mut self,
        old_parent: u32,
        old_name: &str,
        new_parent: u32,
        new_name: &str,
    ) -> Result<(), VfsError> {
        let src = self.find_dir_entry(old_parent, old_name)?;
        if old_parent == new_parent && old_name == new_name {
            return Ok(());
        }
        match self.find_dir_entry(new_parent, new_name) {
            Ok(_) => return Err(VfsError::AlreadyExists),
            Err(VfsError::NotFound) => {}
            Err(e) => return Err(e),
        }
        self.add_dir_entry(new_parent, src.ino, new_name, src.file_type)?;
        self.unlink_dir_entry(old_parent, old_name)?;
        if src.file_type == EXT2_FT_DIR && old_parent != new_parent {
            self.retarget_dir_entry(src.ino, "..", new_parent)?;
            self.adjust_links(old_parent, -1)?;
            self.adjust_links(new_parent, 1)?;
        }
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

    /// Where logical block `logical` lives: the `i_block` slot, how many
    /// indirection levels lie below it, and the pointer index at each level.
    fn block_path(&self, logical: u64) -> Result<(usize, usize, [u64; 3]), VfsError> {
        let p = self.ptrs_per_block();
        let direct = EXT2_NDIR_BLOCKS as u64;
        if logical < direct {
            return Ok((logical as usize, 0, [0; 3]));
        }
        let rel = logical - direct;
        if rel < p {
            return Ok((EXT2_IND_BLOCK, 1, [rel, 0, 0]));
        }
        let rel = rel - p;
        if rel < p * p {
            return Ok((EXT2_DIND_BLOCK, 2, [rel / p, rel % p, 0]));
        }
        let rel = rel - p * p;
        if rel < p * p * p {
            return Ok((EXT2_TIND_BLOCK, 3, [rel / (p * p), (rel / p) % p, rel % p]));
        }
        Err(VfsError::InvalidOffset)
    }

    /// Allocate a zeroed block for `inode` and account for it in `i_blocks`
    /// (512-byte sectors, indirect blocks included).
    fn alloc_inode_block(&mut self, inode: &mut Ext2Inode) -> Result<u64, VfsError> {
        let block = self.alloc_block()?;
        let zeroes = [0u8; MAX_BLOCK_SIZE];
        write_block(self, block, &zeroes[..self.block_size as usize])?;
        let sectors = u32::from_le(inode.i_blocks) + self.sectors_per_block();
        inode.i_blocks = sectors.to_le();
        Ok(block)
    }

    /// Free a block owned by `inode` and take it out of `i_blocks`.
    fn release_inode_block(&mut self, inode: &mut Ext2Inode, block: u64) -> Result<(), VfsError> {
        self.free_block(block)?;
        let sectors = u32::from_le(inode.i_blocks).saturating_sub(self.sectors_per_block());
        inode.i_blocks = sectors.to_le();
        Ok(())
    }

    /// Ensure the block at `logical_block` is allocated, creating indirect
    /// blocks as needed. Returns the physical block number and mutates the
    /// inode's i_block array and i_blocks for anything newly allocated.
    fn alloc_logical_block(
        &mut self,
        inode: &mut Ext2Inode,
        logical_block: u64,
    ) -> Result<u64, VfsError> {
        let (slot, depth, idx) = self.block_path(logical_block)?;
        let mut current = u32::from_le(inode.i_block[slot]) as u64;
        if current == 0 {
            current = self.alloc_inode_block(inode)?;
            inode.i_block[slot] = (current as u32).to_le();
        }
        for &index in &idx[..depth] {
            let existing = self.read_indirect_ptr(current, index)?;
            current = if existing != 0 {
                existing as u64
            } else {
                let fresh = self.alloc_inode_block(inode)?;
                self.write_indirect_ptr(current, index, fresh as u32)?;
                fresh
            };
        }
        Ok(current)
    }

    /// Free every logical block at or beyond `keep_blocks`, together with
    /// indirect blocks that end up empty.
    fn truncate_blocks(&mut self, inode: &mut Ext2Inode, keep_blocks: u64) -> Result<(), VfsError> {
        for slot in (keep_blocks as usize).min(EXT2_NDIR_BLOCKS)..EXT2_NDIR_BLOCKS {
            let block = u32::from_le(inode.i_block[slot]);
            if block != 0 {
                self.release_inode_block(inode, block as u64)?;
                inode.i_block[slot] = 0;
            }
        }
        let p = self.ptrs_per_block();
        let (mut base, mut span) = (EXT2_NDIR_BLOCKS as u64, p);
        for depth in 1..=3usize {
            let slot = EXT2_IND_BLOCK + depth - 1;
            let root = u32::from_le(inode.i_block[slot]) as u64;
            if root != 0 && keep_blocks < base + span {
                let emptied =
                    self.free_subtree(inode, root, depth, keep_blocks.saturating_sub(base))?;
                if emptied {
                    self.release_inode_block(inode, root)?;
                    inode.i_block[slot] = 0;
                }
            }
            base += span;
            span *= p;
        }
        Ok(())
    }

    /// Free the data below the indirect block `table` (`depth` levels above
    /// the data) from relative block `keep_from` on. Returns whether the
    /// table is now empty; the caller then frees the table itself.
    fn free_subtree(
        &mut self,
        inode: &mut Ext2Inode,
        table: u64,
        depth: usize,
        keep_from: u64,
    ) -> Result<bool, VfsError> {
        let bs = self.block_size as usize;
        let mut buf = [0u8; MAX_BLOCK_SIZE];
        read_block(self, table, &mut buf[..bs])?;
        let child_span = self.ptrs_per_block().pow(depth as u32 - 1);
        let mut changed = false;
        for i in 0..bs / PTR_SIZE {
            let child =
                u32::from_le_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]]);
            let start = i as u64 * child_span;
            if child == 0 || start + child_span <= keep_from {
                continue;
            }
            let emptied = depth == 1
                || self.free_subtree(
                    inode,
                    child as u64,
                    depth - 1,
                    keep_from.saturating_sub(start),
                )?;
            if emptied {
                self.release_inode_block(inode, child as u64)?;
                buf[i * 4..i * 4 + 4].fill(0);
                changed = true;
            }
        }
        if changed {
            write_block(self, table, &buf[..bs])?;
        }
        Ok(buf[..bs].iter().all(|&b| b == 0))
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
    /// A full rewrite from offset 0 that is shorter than the old content
    /// truncates the file. If the volume fills up, the blocks written so far
    /// stay consistently accounted and the error is returned.
    pub fn write_file(
        &mut self,
        inode_num: u32,
        offset: u64,
        buf: &[u8],
    ) -> Result<usize, VfsError> {
        let mut inode = self.read_inode(inode_num)?;
        let old_size = u32::from_le(inode.i_size) as u64;
        let (written, failure) = self.write_data(&mut inode, offset, buf);
        let end = offset + written as u64;

        let shrinks = offset == 0 && failure.is_none() && end < old_size;
        let new_size = if shrinks { end } else { end.max(old_size) };
        if shrinks {
            let keep = new_size.div_ceil(self.block_size as u64);
            self.truncate_blocks(&mut inode, keep)?;
        }
        inode.i_size = (new_size as u32).to_le();
        stamp_modified(&mut inode);
        self.write_inode(inode_num, &inode)?;

        match failure {
            Some(e) => Err(e),
            None => Ok(written),
        }
    }

    /// Copy `buf` into the file's blocks. Returns the bytes stored and the
    /// error that stopped the copy, if any.
    fn write_data(
        &mut self,
        inode: &mut Ext2Inode,
        offset: u64,
        buf: &[u8],
    ) -> (usize, Option<VfsError>) {
        let bs = self.block_size as u64;
        let mut written = 0usize;
        while written < buf.len() {
            let pos = offset + written as u64;
            let in_block = (pos % bs) as usize;
            let chunk = (bs as usize - in_block).min(buf.len() - written);
            let stored = self.alloc_logical_block(inode, pos / bs).and_then(|block| {
                self.store_chunk(block, in_block, &buf[written..written + chunk])
            });
            if let Err(e) = stored {
                return (written, Some(e));
            }
            written += chunk;
        }
        (written, None)
    }

    /// Read-modify-write part of one data block.
    fn store_chunk(&mut self, block: u64, in_block: usize, data: &[u8]) -> Result<(), VfsError> {
        let bs = self.block_size as usize;
        let mut block_buf = [0u8; MAX_BLOCK_SIZE];
        read_block(self, block, &mut block_buf[..bs])?;
        block_buf[in_block..in_block + data.len()].copy_from_slice(data);
        if self.journal_enabled {
            self.journal_block(block, &block_buf[..bs])?;
        }
        write_block(self, block, &block_buf[..bs])
    }

    // ─── Journal (in-memory write-ahead ring) ───────────────────────
    //
    // The ring only lives in RAM: nothing is ever written to the volume, so
    // it cannot touch blocks the ext2 bitmaps do not own, and replay at mount
    // starts from an empty ring.

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

    /// Tag the mount node as the ext2 root; its children are materialized on
    /// first use by the VFS (`ensure_expanded`), not eagerly here.
    fn populate_vfs(&mut self, parent_id: u64, fs_id: u32) -> Result<(), VfsError> {
        crate::fs::vfs::set_node_backing(parent_id, fs_id, EXT2_ROOT_INO);
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
        // SAFETY: the single mounted instance is only used from kernel task context.
        let fs = unsafe { EXT2_INSTANCE.as_mut().ok_or(VfsError::IOError)? };
        let metadata = inode_metadata(&fs.read_inode(inode_id as u32)?);
        Ok(Metadata {
            inode_id,
            ..metadata
        })
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

#[cfg(all(test, feature = "std"))]
#[path = "ext2_tests.rs"]
mod tests;

/// Block and inode usage of the mounted ext2 volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ext2Usage {
    pub block_size: u32,
    pub blocks_total: u64,
    pub blocks_free: u64,
    pub inodes_total: u32,
    pub inodes_free: u32,
}

/// Usage counts from the in-memory superblock, or `None` when no ext2
/// volume is mounted.
pub fn usage() -> Option<Ext2Usage> {
    // SAFETY: the filesystem instance is only touched from the single VFS thread.
    let fs = unsafe { (*core::ptr::addr_of!(EXT2_INSTANCE)).as_ref()? };
    Some(Ext2Usage {
        block_size: fs.block_size,
        blocks_total: u64::from(u32::from_le(fs.superblock.s_blocks_count)),
        blocks_free: u64::from(u32::from_le(fs.superblock.s_free_blocks_count)),
        inodes_total: u32::from_le(fs.superblock.s_inodes_count),
        inodes_free: u32::from_le(fs.superblock.s_free_inodes_count),
    })
}
