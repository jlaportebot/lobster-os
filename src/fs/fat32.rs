//! FAT32 filesystem driver for block devices

use crate::println;
use super::vfs::*;
use super::block_dev::BlockDevice;

/// FAT32 BPB (BIOS Parameter Block)
#[derive(Copy, Clone)]
#[repr(C, packed)]
pub struct Fat32Bpb {
    pub jmp_boot: [u8; 3],
    pub oem_name: [u8; 8],
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub num_fats: u8,
    pub root_entry_count: u16,
    pub total_sectors_16: u16,
    pub media_type: u8,
    pub fat_size_16: u16,
    pub sectors_per_track: u16,
    pub num_heads: u16,
    pub hidden_sectors: u32,
    pub total_sectors_32: u32,
    // Extended BPB for FAT32
    pub fat_size_32: u32,
    pub ext_flags: u16,
    pub fs_version: u16,
    pub root_cluster: u32,
    pub fs_info_sector: u16,
    pub backup_boot_sector: u16,
    pub reserved: [u8; 12],
    pub drive_number: u8,
    pub reserved1: u8,
    pub boot_sig: u8,
    pub volume_id: u32,
    pub volume_label: [u8; 11],
    pub fs_type: [u8; 8],
}

/// FAT32 directory entry (32 bytes)
#[repr(C, packed)]
pub struct Fat32DirEntry {
    pub name: [u8; 11],
    pub attributes: u8,
    pub nt_reserved: u8,
    pub creation_time_tenth: u8,
    pub creation_time: u16,
    pub creation_date: u16,
    pub last_access_date: u16,
    pub first_cluster_high: u16,
    pub last_write_time: u16,
    pub last_write_date: u16,
    pub first_cluster_low: u16,
    pub file_size: u32,
}

/// LFN directory entry (32 bytes)
#[repr(C, packed)]
pub struct LfnDirEntry {
    pub sequence: u8,
    pub name_part1: [u16; 5],
    pub attributes: u8,
    pub type_field: u8,
    pub checksum: u8,
    pub name_part2: [u16; 6],
    pub cluster_zero: u16,
    pub name_part3: [u16; 2],
}

/// Attribute flags
const ATTR_READ_ONLY: u8 = 0x01;
const ATTR_HIDDEN: u8 = 0x02;
const ATTR_SYSTEM: u8 = 0x04;
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LFN: u8 = ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID;

/// FAT32 end-of-chain markers
const FAT32_EOC: u32 = 0x0FFFFFF8;
const FAT32_EOC_MAX: u32 = 0x0FFFFFFF;
const FAT32_FREE: u32 = 0x00000000;
const FAT32_BAD: u32 = 0x0FFFFFF7;

/// FAT32 cluster cache (4 entries)
const CLUSTER_CACHE_SIZE: usize = 4;

#[derive(Copy, Clone)]
struct ClusterCacheEntry {
    cluster: u32,
    next_cluster: u32,
    valid: bool,
}

/// FAT32 filesystem instance
pub struct Fat32FileSystem {
    fs_id: u32,
    bpb: Fat32Bpb,
    fat_start_sector: u64,
    data_start_sector: u64,
    total_clusters: u32,
    cluster_cache: [ClusterCacheEntry; CLUSTER_CACHE_SIZE],
    cache_idx: usize,
    device_index: usize, // Which block device
    partition_start: u64, // LBA offset for partition
}

static mut FAT32_INSTANCE: Option<Fat32FileSystem> = None;

impl Fat32FileSystem {
    /// Create a new FAT32 filesystem from a block device and partition offset
    pub fn new(device_index: usize, partition_start: u64) -> Result<Self, VfsError> {
        let mut bpb_bytes = [0u8; 512];
        
        // Read BPB from first sector of partition
        read_block_device(device_index, partition_start, &mut bpb_bytes)?;
        
        // Parse BPB
        let bpb = unsafe { &*(bpb_bytes.as_ptr() as *const Fat32Bpb) };
        
        // Validate FAT32 signature
        if bpb.boot_sig != 0x29 && bpb.fs_type[0] != b'F' {
            println!("[fat32] Warning: non-standard BPB signature");
        }

        let bytes_per_sector = bpb.bytes_per_sector as u64;
        let fat_start = partition_start + bpb.reserved_sectors as u64;
        let fat_size = bpb.fat_size_32 as u64;
        let data_start = fat_start + (bpb.num_fats as u64 * fat_size);
        let root_dir_sectors = ((bpb.root_entry_count as u64 * 32) + bytes_per_sector - 1) / bytes_per_sector;
        let total_data_sectors = if bpb.total_sectors_32 != 0 {
            bpb.total_sectors_32 as u64 - (bpb.reserved_sectors as u64 + bpb.num_fats as u64 * fat_size + root_dir_sectors)
        } else {
            0
        };
        let sectors_per_cluster = bpb.sectors_per_cluster as u64;
        let total_clusters = (total_data_sectors / sectors_per_cluster) as u32;

        println!("[fat32] Bytes/sector: {}", bytes_per_sector);
        println!("[fat32] Sectors/cluster: {}", sectors_per_cluster);
        println!("[fat32] Reserved sectors: {}", unsafe { core::ptr::addr_of!((*bpb).reserved_sectors).read_unaligned() });
        println!("[fat32] Number of FATs: {}", bpb.num_fats);
        println!("[fat32] FAT size: {} sectors", unsafe { core::ptr::addr_of!((*bpb).fat_size_32).read_unaligned() });
        println!("[fat32] Root cluster: {}", unsafe { core::ptr::addr_of!((*bpb).root_cluster).read_unaligned() });
        println!("[fat32] Total clusters: {}", total_clusters);
        println!("[fat32] Data start sector: {}", data_start);

        let fs_id = register_fs();

        Ok(Self {
            fs_id,
            bpb: *bpb,
            fat_start_sector: fat_start,
            data_start_sector: data_start,
            total_clusters,
            cluster_cache: [ClusterCacheEntry {
                cluster: 0,
                next_cluster: 0,
                valid: false,
            }; CLUSTER_CACHE_SIZE],
            cache_idx: 0,
            device_index,
            partition_start,
        })
    }

    /// Read the FAT entry for a cluster
    pub fn get_fat_entry(&mut self, cluster: u32) -> u32 {
        // Check cache first
        for entry in &self.cluster_cache {
            if entry.valid && entry.cluster == cluster {
                return entry.next_cluster;
            }
        }

        // Calculate which sector contains this FAT entry
        let bytes_per_sector = self.bpb.bytes_per_sector as u64;
        let fat_offset = cluster as u64 * 4;
        let fat_sector = self.fat_start_sector + (fat_offset / bytes_per_sector);
        let entry_offset = (fat_offset % bytes_per_sector) as usize;

        let mut sector_buf = [0u8; 512];
        if read_block_device(self.device_index, fat_sector, &mut sector_buf).is_err() {
            return FAT32_BAD;
        }

        let next_cluster = u32::from_le_bytes([
            sector_buf[entry_offset],
            sector_buf[entry_offset + 1],
            sector_buf[entry_offset + 2],
            sector_buf[entry_offset + 3],
        ]) & 0x0FFFFFFF;

        // Update cache
        self.cluster_cache[self.cache_idx] = ClusterCacheEntry {
            cluster,
            next_cluster,
            valid: true,
        };
        self.cache_idx = (self.cache_idx + 1) % CLUSTER_CACHE_SIZE;

        next_cluster
    }

    /// Check if a cluster is end-of-chain
    pub fn is_eoc(cluster: u32) -> bool {
        cluster >= FAT32_EOC && cluster <= FAT32_EOC_MAX
    }

    /// Convert cluster number to absolute sector number
    pub fn cluster_to_sector(&self, cluster: u32) -> u64 {
        self.data_start_sector + ((cluster as u64 - 2) * self.bpb.sectors_per_cluster as u64)
    }

    /// Read data from a cluster chain starting at a given cluster
    pub fn read_cluster_chain(
        &self,
        start_cluster: u32,
        offset: &mut u64,
        buf: &mut [u8],
    ) -> Result<usize, VfsError> {
        let cluster_size = self.bpb.sectors_per_cluster as u64 * self.bpb.bytes_per_sector as u64;
        let mut current_cluster = start_cluster;
        let mut bytes_read = 0usize;
        let mut skip_bytes = *offset;

        loop {
            if Self::is_eoc(current_cluster) || current_cluster == 0 {
                break;
            }

            if skip_bytes >= cluster_size {
                skip_bytes -= cluster_size;
                // Follow chain (need mutable access for cache)
                unsafe {
                    if let Some(ref mut fs) = FAT32_INSTANCE {
                        current_cluster = fs.get_fat_entry(current_cluster);
                    } else {
                        break;
                    }
                }
                continue;
            }

            let sector = self.cluster_to_sector(current_cluster);
            let sector_count = self.bpb.sectors_per_cluster as usize;
            let mut sector_buf = [0u8; 512];

            for s in 0..sector_count {
                let abs_sector = sector + s as u64;
                read_block_device(self.device_index, abs_sector, &mut sector_buf)?;

                let buf_start = (s * 512) as u64;
                if buf_start < skip_bytes {
                    continue; // Skip this sector
                }
                
                let src_start = if skip_bytes > buf_start {
                    (skip_bytes - buf_start) as usize
                } else {
                    0
                };
                
                let to_copy = (512 - src_start).min(buf.len() - bytes_read);
                if to_copy == 0 {
                    break;
                }
                
                buf[bytes_read..bytes_read + to_copy]
                    .copy_from_slice(&sector_buf[src_start..src_start + to_copy]);
                bytes_read += to_copy;
            }

            if bytes_read >= buf.len() {
                break;
            }

            unsafe {
                if let Some(ref mut fs) = FAT32_INSTANCE {
                    current_cluster = fs.get_fat_entry(current_cluster);
                } else {
                    break;
                }
            }
        }

        *offset += bytes_read as u64;
        Ok(bytes_read)
    }

    /// Read directory entries from a cluster
    pub fn read_dir(&self, cluster: u32) -> Result<[DirEntry; MAX_DIR_ENTRIES], VfsError> {
        let mut entries = [DirEntry::empty(); MAX_DIR_ENTRIES];
        let mut entry_count = 0usize;
        let mut current_cluster = cluster;
        let cluster_size = self.bpb.sectors_per_cluster as usize * self.bpb.bytes_per_sector as usize;
        let mut lfn_name = [0u16; 256];
        let mut lfn_len = 0usize;

        loop {
            if Self::is_eoc(current_cluster) || current_cluster == 0 || entry_count >= MAX_DIR_ENTRIES {
                break;
            }

            let sector = self.cluster_to_sector(current_cluster);
            let mut sector_buf = [0u8; 512];

            for s in 0..self.bpb.sectors_per_cluster as usize {
                read_block_device(self.device_index, sector + s as u64, &mut sector_buf)?;

                for entry_idx in 0..(512 / 32) {
                    if entry_count >= MAX_DIR_ENTRIES {
                        break;
                    }
                    
                    let offset = entry_idx * 32;
                    let raw = &sector_buf[offset..offset + 32];
                    
                    // Check for end of directory
                    if raw[0] == 0x00 {
                        return Ok(entries);
                    }
                    if raw[0] == 0xE5 {
                        continue; // Deleted entry
                    }

                    let first_byte = raw[0];
                    
                    // Check if LFN entry
                    if raw[11] == ATTR_LFN {
                        let lfn = unsafe { &*(raw.as_ptr() as *const LfnDirEntry) };
                        let seq = lfn.sequence & 0x1F;
                        if seq == 0 || seq > 20 {
                            continue;
                        }
                        // Extract name parts (simplified)
                        lfn_len = 0;
                        // In real implementation, combine LFN parts
                        continue;
                    }

                    // Regular 8.3 entry
                    let dir_entry = unsafe { &*(raw.as_ptr() as *const Fat32DirEntry) };
                    
                    if dir_entry.attributes & ATTR_VOLUME_ID != 0 {
                        continue; // Skip volume label
                    }

                    let is_dir = dir_entry.attributes & ATTR_DIRECTORY != 0;
                    
                    // Convert 8.3 name to string
                    let mut name_buf = [0u8; 13];
                    let mut name_len = 0usize;
                    
                    // Copy name part (8 chars)
                    for i in 0..8 {
                        if dir_entry.name[i] == b' ' {
                            break;
                        }
                        name_buf[name_len] = dir_entry.name[i];
                        name_len += 1;
                    }
                    
                    // Copy extension (3 chars)
                    if dir_entry.name[8] != b' ' {
                        name_buf[name_len] = b'.';
                        name_len += 1;
                        for i in 8..11 {
                            if dir_entry.name[i] == b' ' {
                                break;
                            }
                            name_buf[name_len] = dir_entry.name[i];
                            name_len += 1;
                        }
                    }

                    let first_cluster = ((dir_entry.first_cluster_high as u32) << 16) 
                        | dir_entry.first_cluster_low as u32;

                    entries[entry_count].name[..name_len].copy_from_slice(&name_buf[..name_len]);
                    entries[entry_count].name_len = name_len;
                    entries[entry_count].file_type = if is_dir {
                        FileType::Directory
                    } else {
                        FileType::RegularFile
                    };
                    entries[entry_count].inode_id = first_cluster as u64;
                    entry_count += 1;
                }
            }

            // Follow chain
            unsafe {
                if let Some(ref mut fs) = FAT32_INSTANCE {
                    current_cluster = fs.get_fat_entry(current_cluster);
                } else {
                    break;
                }
            }
        }

        Ok(entries)
    }

    /// Get the filesystem ID
    pub fn id(&self) -> u32 {
        self.fs_id
    }
}

/// Helper: read from block device (simplified)
fn read_block_device(device_index: usize, sector: u64, buf: &mut [u8; 512]) -> Result<(), VfsError> {
    // In real implementation, call block_dev module
    // For now, return zeros
    buf.fill(0);
    let _ = (device_index, sector);
    Ok(())
}

/// Initialize FAT32 filesystem on a block device
pub fn mount(device_index: usize, partition_start: u64, mount_path: &str) -> Result<(), VfsError> {
    let fs = Fat32FileSystem::new(device_index, partition_start)?;
    let root_cluster = fs.bpb.root_cluster;
    let fs_id = fs.id();
    
    unsafe {
        FAT32_INSTANCE = Some(fs);
    }
    
    // Read root directory and populate VFS
    let root_entries = if let Some(ref fs) = unsafe { FAT32_INSTANCE.as_ref() } {
        fs.read_dir(root_cluster)?
    } else {
        return Err(VfsError::IOError);
    };
    
    // Create mount point in VFS
    let mount_inode = alloc_node(mount_path.trim_start_matches('/'), FileType::Directory, 0)?;
    
    for entry in &root_entries[..] {
        if entry.name_len == 0 {
            break;
        }
        let _ = alloc_node(entry.name_str(), entry.file_type, mount_inode);
    }
    
    crate::fs::vfs::mount(mount_path, fs_id, root_cluster as u64)?;
    
    println!("[fat32] Mounted at {}", mount_path);
    Ok(())
}
