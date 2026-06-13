//! Block device abstraction — trait for storage devices + MBR/GPT partition parsing
//!
//! Phase 13 improvements:
//! - PartitionBlockDevice wrapper for per-partition access
//! - GPT partition table support (in addition to MBR)
//! - Multi-block read/write trait methods
//! - Partition device registry
//! - SD card status/diagnostics API

use crate::println;

/// Block device error types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlockError {
    ReadError,
    WriteError,
    NoMedia,
    InvalidBlock,
    Timeout,
    Unsupported,
}

/// Block device trait — interface for all storage devices
pub trait BlockDevice {
    /// Read a single 512-byte block
    fn read_block(&self, block: u64, buf: &mut [u8; 512]) -> Result<(), BlockError>;
    /// Write a single 512-byte block
    fn write_block(&self, block: u64, buf: &[u8; 512]) -> Result<(), BlockError>;
    /// Read multiple contiguous blocks into a buffer
    fn read_blocks(
        &self,
        start_block: u64,
        buf: &mut [u8],
        count: usize,
    ) -> Result<(), BlockError> {
        if buf.len() < count * 512 {
            return Err(BlockError::InvalidBlock);
        }
        for i in 0..count {
            let mut block_buf = [0u8; 512];
            self.read_block(start_block + i as u64, &mut block_buf)?;
            buf[i * 512..(i + 1) * 512].copy_from_slice(&block_buf);
        }
        Ok(())
    }
    /// Write multiple contiguous blocks from a buffer
    fn write_blocks(&self, start_block: u64, buf: &[u8], count: usize) -> Result<(), BlockError> {
        if buf.len() < count * 512 {
            return Err(BlockError::InvalidBlock);
        }
        for i in 0..count {
            let mut block_buf = [0u8; 512];
            block_buf.copy_from_slice(&buf[i * 512..(i + 1) * 512]);
            self.write_block(start_block + i as u64, &block_buf)?;
        }
        Ok(())
    }
    /// Block size in bytes (typically 512)
    fn block_size(&self) -> usize;
    /// Total number of blocks
    fn total_blocks(&self) -> u64;
    /// Device name
    fn name(&self) -> &str;
    /// Check if media is present
    fn media_present(&self) -> bool {
        true
    }
    /// Device type string (for sysfs/diagnostics)
    fn device_type(&self) -> &str {
        "unknown"
    }
    /// Whether the device is read-only
    fn read_only(&self) -> bool {
        false
    }
    /// Flush/write-back cache (no-op by default)
    fn flush(&self) -> Result<(), BlockError> {
        Ok(())
    }
}

// ─── MBR Partition Table ────────────────────────────────────────────

/// MBR partition table entry (16 bytes)
#[repr(C, packed)]
pub struct MbrPartEntry {
    pub boot_indicator: u8,
    pub starting_chs: [u8; 3],
    pub partition_type: u8,
    pub ending_chs: [u8; 3],
    pub starting_lba: u32,
    pub size_in_blocks: u32,
}

/// Partition info
#[derive(Debug, Clone, Copy)]
pub struct PartitionInfo {
    pub index: usize,
    pub start_lba: u64,
    pub size_blocks: u64,
    pub partition_type: u8,
    pub bootable: bool,
    /// GUID type (for GPT partitions, zero for MBR)
    pub type_guid: [u8; 16],
    /// Unique partition GUID (for GPT, zero for MBR)
    pub unique_guid: [u8; 16],
    /// Partition name (UTF-16LE, up to 36 code units for GPT)
    pub name: [u16; 36],
}

impl PartitionInfo {
    pub const fn empty() -> Self {
        Self {
            index: 0,
            start_lba: 0,
            size_blocks: 0,
            partition_type: 0,
            bootable: false,
            type_guid: [0; 16],
            unique_guid: [0; 16],
            name: [0; 36],
        }
    }

    /// Human-readable partition type
    pub fn type_name(&self) -> &str {
        // Check GPT GUID first (if non-zero first 4 bytes)
        if self.type_guid[0] != 0 || self.type_guid[1] != 0 {
            return gpt_type_name(&self.type_guid);
        }
        match self.partition_type {
            0x00 => "Empty",
            0x01 => "FAT12",
            0x04 => "FAT16 (16-32MB)",
            0x06 => "FAT16 (32MB+)",
            0x07 => "NTFS/HPFS",
            0x0B => "FAT32",
            0x0C => "FAT32 (LBA)",
            0x0E => "FAT16 (LBA)",
            0x82 => "Linux Swap",
            0x83 => "Linux",
            0xEE => "GPT Protective",
            0xEF => "EFI System Partition",
            0xAF => "HFS/HFS+",
            _ => "Unknown",
        }
    }

    /// Size in MiB
    pub fn size_mib(&self) -> u64 {
        (self.size_blocks * 512) / (1024 * 1024)
    }

    /// Get partition name as a lossy UTF-8 string (GPT only)
    pub fn name_str(&self) -> &str {
        // Quick check if any name characters exist
        if self.name[0] == 0 {
            return "";
        }
        // Return a static empty string; actual name extraction requires alloc
        // which we avoid in no_std. Callers should use name() directly.
        ""
    }
}

// ─── GPT Partition Table ────────────────────────────────────────────

/// EFI GUID (16 bytes, mixed-endian as per UEFI spec)
#[repr(C)]
pub struct EfiGuid {
    pub time_low: u32,
    pub time_mid: u16,
    pub time_hi_and_version: u16,
    pub clock_seq_hi: u8,
    pub clock_seq_low: u8,
    pub node: [u8; 6],
}

/// GPT partition entry (128 bytes)
#[repr(C, packed)]
pub struct GptPartEntry {
    pub type_guid: [u8; 16],
    pub unique_guid: [u8; 16],
    pub starting_lba: u64,
    pub ending_lba: u64,
    pub attributes: u64,
    pub name: [u16; 36],
}

/// GPT header structure (LBA 1)
#[repr(C, packed)]
pub struct GptHeader {
    pub signature: [u8; 8], // "EFI PART"
    pub revision: u32,      // 0x00010000 = revision 1.0
    pub header_size: u32,
    pub header_crc32: u32,
    pub reserved: u32,
    pub my_lba: u64,
    pub alternate_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: [u8; 16],
    pub entry_start_lba: u64,
    pub entry_count: u32,
    pub entry_size: u32,
    pub entries_crc32: u32,
}

/// Well-known GPT partition type GUIDs
const GPT_TYPE_EFI_SYSTEM: [u8; 16] = [
    0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
];
const GPT_TYPE_LINUX_FS: [u8; 16] = [
    0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD7, 0x47, 0x64, 0x1D,
];
const GPT_TYPE_LINUX_SWAP: [u8; 16] = [
    0x46, 0x53, 0x9B, 0x06, 0x84, 0x1D, 0x80, 0x44, 0x9E, 0x09, 0xA9, 0x4E, 0xFB, 0x15, 0x71, 0x3D,
];
const GPT_TYPE_LINUX_LUKS: [u8; 16] = [
    0xA7, 0x5D, 0x53, 0xCA, 0x6F, 0x33, 0x41, 0x4E, 0x9A, 0x74, 0x1B, 0xC1, 0x30, 0xE4, 0xA1, 0x4C,
];
const GPT_TYPE_FAT32_LBA: [u8; 16] = [
    0x2D, 0x45, 0x4E, 0xEB, 0x06, 0x66, 0x46, 0x47, 0x98, 0xBD, 0x1C, 0x25, 0x3E, 0xD6, 0x54, 0x2E,
];

/// Look up a human-readable name for a GPT type GUID
fn gpt_type_name(guid: &[u8; 16]) -> &'static str {
    if guid == &GPT_TYPE_EFI_SYSTEM {
        return "EFI System Partition";
    }
    if guid == &GPT_TYPE_LINUX_FS {
        return "Linux filesystem";
    }
    if guid == &GPT_TYPE_LINUX_SWAP {
        return "Linux swap";
    }
    if guid == &GPT_TYPE_LINUX_LUKS {
        return "Linux LUKS";
    }
    if guid == &GPT_TYPE_FAT32_LBA {
        return "Microsoft Basic Data (FAT32)";
    }
    "GPT Unknown"
}

/// Detect partitions from MBR of a block device
pub fn detect_partitions(dev: &dyn BlockDevice) -> [Option<PartitionInfo>; 4] {
    let mut result: [Option<PartitionInfo>; 4] = [None; 4];
    let mut mbr = [0u8; 512];

    if dev.read_block(0, &mut mbr).is_err() {
        println!("[block_dev] Failed to read MBR from {}", dev.name());
        return result;
    }

    // Check MBR signature
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        println!(
            "[block_dev] Invalid MBR signature: {:02X} {:02X}",
            mbr[510], mbr[511]
        );
        return result;
    }

    // Check if this is a GPT protective MBR (partition type 0xEE)
    // If so, we should parse GPT instead
    for i in 0..4usize {
        let entry_offset = 446 + i * 16;
        let ptype = mbr[entry_offset + 4];
        if ptype == 0xEE {
            println!("[block_dev] GPT protective MBR detected, parsing GPT...");
            return detect_gpt_partitions(dev);
        }
    }

    // Parse 4 MBR partition entries at offset 446
    for i in 0..4usize {
        let entry_offset = 446 + i * 16;
        let entry = unsafe { &*(mbr[entry_offset..].as_ptr() as *const MbrPartEntry) };

        if entry.partition_type == 0x00 {
            continue; // Empty partition
        }

        let start_lba = u32::from_le_bytes([
            mbr[entry_offset + 8],
            mbr[entry_offset + 9],
            mbr[entry_offset + 10],
            mbr[entry_offset + 11],
        ]);

        let size_blocks = u32::from_le_bytes([
            mbr[entry_offset + 12],
            mbr[entry_offset + 13],
            mbr[entry_offset + 14],
            mbr[entry_offset + 15],
        ]);

        result[i] = Some(PartitionInfo {
            index: i,
            start_lba: start_lba as u64,
            size_blocks: size_blocks as u64,
            partition_type: entry.partition_type,
            bootable: entry.boot_indicator == 0x80,
            type_guid: [0; 16],
            unique_guid: [0; 16],
            name: [0; 36],
        });

        println!(
            "[block_dev] MBR Partition {}: {} LBA={}, {} blocks ({} MiB) {}",
            i,
            result[i].unwrap().type_name(),
            start_lba,
            size_blocks,
            result[i].unwrap().size_mib(),
            if entry.boot_indicator == 0x80 {
                "[Boot]"
            } else {
                ""
            }
        );
    }

    result
}

/// Detect partitions from GPT (GUID Partition Table) of a block device
fn detect_gpt_partitions(dev: &dyn BlockDevice) -> [Option<PartitionInfo>; 4] {
    let mut result: [Option<PartitionInfo>; 4] = [None; 4];
    let mut header_buf = [0u8; 512];

    // Read GPT header from LBA 1
    if dev.read_block(1, &mut header_buf).is_err() {
        println!("[block_dev] Failed to read GPT header from {}", dev.name());
        return result;
    }

    // Verify GPT signature "EFI PART"
    let header = unsafe { &*(header_buf.as_ptr() as *const GptHeader) };
    if &header.signature != b"EFI PART" {
        println!("[block_dev] Invalid GPT signature");
        return result;
    }

    // Read fields from packed struct with copies to avoid unaligned reference errors
    let revision = header.revision;
    let entry_count = header.entry_count;
    let entry_size = header.entry_size;
    let entry_start_lba = header.entry_start_lba;
    println!(
        "[block_dev] GPT revision: {:#010x}, entries: {}, entry size: {}",
        revision, entry_count, entry_size
    );
    // Read up to 4 entries — each is 128 bytes, so 4 entries = 512 bytes = 1 block
    let entry_lba = entry_start_lba;
    let entry_count_val = entry_count.min(128) as usize; // Cap at 128 entries
    let entry_size_val = entry_size as usize;

    // Read enough blocks for the entries we want
    let entries_per_block = 512 / entry_size_val.max(1);
    let blocks_needed =
        ((4.min(entry_count_val) + entries_per_block - 1) / entries_per_block).max(1);

    // We read up to 4 partition entries (enough for most SD cards)
    let max_entries = 4.min(entry_count_val);
    let mut entries_buf = [0u8; 2048]; // Up to 4 blocks

    for b in 0..blocks_needed.min(4) {
        let mut block_buf = [0u8; 512];
        if dev
            .read_block(entry_lba + b as u64, &mut block_buf)
            .is_err()
        {
            println!(
                "[block_dev] Failed to read GPT entries at LBA {}",
                entry_lba + b as u64
            );
            return result;
        }
        let start = b * 512;
        let end = (start + 512).min(2048);
        entries_buf[start..end].copy_from_slice(&block_buf);
    }

    // Parse up to 4 entries
    for i in 0..max_entries {
        let offset = i * entry_size_val;
        if offset + 128 > entries_buf.len() {
            break;
        }

        let entry = unsafe { &*(entries_buf[offset..].as_ptr() as *const GptPartEntry) };

        // Skip empty entries (all-zero type GUID)
        if entry.type_guid == [0u8; 16] {
            continue;
        }

        let start_lba = entry.starting_lba;
        let ending_lba = entry.ending_lba;
        let size_blocks = if ending_lba > start_lba {
            ending_lba - start_lba + 1
        } else {
            0
        };

        result[i] = Some(PartitionInfo {
            index: i,
            start_lba,
            size_blocks,
            partition_type: 0, // GPT uses GUIDs, not MBR type codes
            bootable: (entry.attributes & 0x0000000000000001) != 0, // RequiredPartition flag
            type_guid: entry.type_guid,
            unique_guid: entry.unique_guid,
            name: entry.name,
        });

        let info = result[i].unwrap();
        println!(
            "[block_dev] GPT Partition {}: {} LBA={}, {} blocks ({} MiB)",
            i,
            info.type_name(),
            start_lba,
            size_blocks,
            info.size_mib(),
        );
    }

    result
}

// ─── Partition Block Device ─────────────────────────────────────────

/// A block device that wraps a partition of a parent block device.
/// Translates block addresses by adding the partition's start LBA offset.
pub struct PartitionBlockDevice {
    /// Index of the parent device in the BLOCK_DEVICES registry
    pub parent_index: usize,
    /// Start LBA of this partition (offset into parent device)
    pub start_lba: u64,
    /// Number of blocks in this partition
    pub num_blocks: u64,
    /// Name of this partition device
    pub device_name: [u8; 32],
    /// Name length
    pub name_len: usize,
}

impl PartitionBlockDevice {
    pub const fn new(parent_index: usize, start_lba: u64, num_blocks: u64, name: &str) -> Self {
        let mut device_name = [0u8; 32];
        let mut name_len = 0;
        let name_bytes = name.as_bytes();
        while name_len < name_bytes.len() && name_len < 31 {
            device_name[name_len] = name_bytes[name_len];
            name_len += 1;
        }
        Self {
            parent_index,
            start_lba,
            num_blocks,
            device_name,
            name_len,
        }
    }
}

impl BlockDevice for PartitionBlockDevice {
    fn read_block(&self, block: u64, buf: &mut [u8; 512]) -> Result<(), BlockError> {
        if block >= self.num_blocks {
            return Err(BlockError::InvalidBlock);
        }
        let parent = get_device(self.parent_index).ok_or(BlockError::NoMedia)?;
        parent.read_block(self.start_lba + block, buf)
    }

    fn write_block(&self, block: u64, buf: &[u8; 512]) -> Result<(), BlockError> {
        if block >= self.num_blocks {
            return Err(BlockError::InvalidBlock);
        }
        let parent = get_device(self.parent_index).ok_or(BlockError::NoMedia)?;
        parent.write_block(self.start_lba + block, buf)
    }

    fn read_blocks(
        &self,
        start_block: u64,
        buf: &mut [u8],
        count: usize,
    ) -> Result<(), BlockError> {
        if start_block + count as u64 > self.num_blocks {
            return Err(BlockError::InvalidBlock);
        }
        let parent = get_device(self.parent_index).ok_or(BlockError::NoMedia)?;
        parent.read_blocks(self.start_lba + start_block, buf, count)
    }

    fn write_blocks(&self, start_block: u64, buf: &[u8], count: usize) -> Result<(), BlockError> {
        if start_block + count as u64 > self.num_blocks {
            return Err(BlockError::InvalidBlock);
        }
        let parent = get_device(self.parent_index).ok_or(BlockError::NoMedia)?;
        parent.write_blocks(self.start_lba + start_block, buf, count)
    }

    fn block_size(&self) -> usize {
        512
    }

    fn total_blocks(&self) -> u64 {
        self.num_blocks
    }

    fn name(&self) -> &str {
        core::str::from_utf8(&self.device_name[..self.name_len]).unwrap_or("part?")
    }

    fn media_present(&self) -> bool {
        get_device(self.parent_index).map_or(false, |d| d.media_present())
    }

    fn device_type(&self) -> &str {
        "partition"
    }
}

// ─── Device Registry ────────────────────────────────────────────────

/// Maximum registered block devices (whole-disk + partitions)
const MAX_BLOCK_DEVICES: usize = 8;
/// Maximum registered partition devices
const MAX_PARTITION_DEVICES: usize = 8;

/// Global block device registry (whole-disk devices)
static mut BLOCK_DEVICES: [Option<&'static dyn BlockDevice>; MAX_BLOCK_DEVICES] =
    [None; MAX_BLOCK_DEVICES];
static mut BLOCK_DEVICE_COUNT: usize = 0;

/// Global partition device registry
static mut PARTITION_DEVICES: [Option<PartitionBlockDevice>; MAX_PARTITION_DEVICES] =
    [None, None, None, None, None, None, None, None];
static mut PARTITION_DEVICE_COUNT: usize = 0;

/// Register a block device (whole-disk)
pub fn register_device(dev: &'static dyn BlockDevice) -> Result<usize, BlockError> {
    unsafe {
        if BLOCK_DEVICE_COUNT >= MAX_BLOCK_DEVICES {
            return Err(BlockError::Unsupported);
        }
        let idx = BLOCK_DEVICE_COUNT;
        BLOCK_DEVICES[idx] = Some(dev);
        BLOCK_DEVICE_COUNT += 1;
        println!(
            "[block_dev] Registered device '{}' at index {}",
            dev.name(),
            idx
        );
        Ok(idx)
    }
}

/// Register a partition block device
pub fn register_partition_device(dev: PartitionBlockDevice) -> Result<usize, BlockError> {
    unsafe {
        if PARTITION_DEVICE_COUNT >= MAX_PARTITION_DEVICES {
            return Err(BlockError::Unsupported);
        }
        let idx = PARTITION_DEVICE_COUNT;
        let parent_index = dev.parent_index;
        let start_lba = dev.start_lba;
        let num_blocks = dev.num_blocks;
        let name_len = dev.name_len;
        PARTITION_DEVICES[idx] = Some(dev);
        PARTITION_DEVICE_COUNT += 1;
        let dev_ref = PARTITION_DEVICES[idx].as_ref().unwrap();
        let name_str = core::str::from_utf8(&dev_ref.device_name[..name_len]).unwrap_or("?");
        println!(
            "[block_dev] Registered partition '{}' (parent={}, LBA={}, {} blocks)",
            name_str, parent_index, start_lba, num_blocks
        );
        Ok(idx)
    }
}

/// Get a registered block device by index (whole-disk)
pub fn get_device(index: usize) -> Option<&'static dyn BlockDevice> {
    unsafe {
        if index < BLOCK_DEVICE_COUNT {
            BLOCK_DEVICES[index]
        } else {
            None
        }
    }
}

/// Get a partition device by index
pub fn get_partition_device(index: usize) -> Option<&'static PartitionBlockDevice> {
    unsafe {
        if index < PARTITION_DEVICE_COUNT {
            PARTITION_DEVICES[index].as_ref()
        } else {
            None
        }
    }
}

/// Get total number of registered whole-disk devices
pub fn device_count() -> usize {
    unsafe { BLOCK_DEVICE_COUNT }
}

/// Get total number of registered partition devices
pub fn partition_count() -> usize {
    unsafe { PARTITION_DEVICE_COUNT }
}

// ─── SD Card Status ─────────────────────────────────────────────────

/// SD card status information (for sysfs/diagnostics)
#[derive(Debug, Clone, Copy)]
pub struct SdCardStatus {
    pub initialized: bool,
    pub card_type: &'static str,
    pub capacity_mib: u64,
    pub total_blocks: u64,
    pub bus_width: u32,
    pub clock_mhz: u32,
    pub read_only: bool,
    pub write_protected: bool,
    pub high_capacity: bool,
}

impl SdCardStatus {
    pub const fn empty() -> Self {
        Self {
            initialized: false,
            card_type: "None",
            capacity_mib: 0,
            total_blocks: 0,
            bus_width: 0,
            clock_mhz: 0,
            read_only: false,
            write_protected: false,
            high_capacity: false,
        }
    }
}

/// Get SD card status from the eMMC driver
#[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
pub fn sd_card_status() -> SdCardStatus {
    let info = crate::drivers::emmc::card_info();
    SdCardStatus {
        initialized: crate::drivers::emmc::is_initialized(),
        card_type: match info.card_type {
            crate::drivers::emmc::SdCardType::SdV1 => "SDv1",
            crate::drivers::emmc::SdCardType::SdV2 => "SDv2",
            crate::drivers::emmc::SdCardType::SdHc => "SDHC",
            crate::drivers::emmc::SdCardType::SdXc => "SDXC",
        },
        capacity_mib: info.capacity_bytes / (1024 * 1024),
        total_blocks: info.total_blocks,
        bus_width: info.bus_width,
        clock_mhz: info.clock_freq / 1_000_000,
        read_only: false,
        write_protected: false,
        high_capacity: info.high_capacity,
    }
}

#[cfg(any(feature = "qemu", target_arch = "x86_64"))]
pub fn sd_card_status() -> SdCardStatus {
    SdCardStatus::empty()
}

/// Print a summary of all registered block devices and partitions
pub fn print_disk_summary() {
    unsafe {
        println!("[block_dev] === Disk Summary ===");
        for i in 0..BLOCK_DEVICE_COUNT {
            if let Some(dev) = BLOCK_DEVICES[i] {
                println!(
                    "[block_dev]   {}: {} ({} blocks, {} bytes/block, type={})",
                    i,
                    dev.name(),
                    dev.total_blocks(),
                    dev.block_size(),
                    dev.device_type()
                );
            }
        }
        for i in 0..PARTITION_DEVICE_COUNT {
            if let Some(ref dev) = PARTITION_DEVICES[i] {
                println!(
                    "[block_dev]   p{}: {} (parent={}, LBA={}, {} blocks)",
                    i,
                    core::str::from_utf8(&dev.device_name[..dev.name_len]).unwrap_or("?"),
                    dev.parent_index,
                    dev.start_lba,
                    dev.num_blocks
                );
            }
        }
        println!("[block_dev] =====================");
    }
}

/// Initialize the block device subsystem
pub fn init() {
    unsafe {
        BLOCK_DEVICE_COUNT = 0;
        PARTITION_DEVICE_COUNT = 0;
    }
    println!("[block_dev] Block device subsystem initialized");
}

/// Register all partitions from a block device by parsing its partition table
/// Call this after registering a block device (e.g., after emmc::init())
pub fn register_partitions(device_index: usize) -> Result<usize, BlockError> {
    let dev = get_device(device_index).ok_or(BlockError::NoMedia)?;

    let partitions = detect_partitions(dev);
    let mut registered = 0;

    for (i, part_info) in partitions.iter().enumerate() {
        if let Some(info) = part_info {
            if info.size_blocks == 0 {
                continue;
            }

            let name = match info.type_name() {
                "Linux filesystem" | "Linux native" => "ext2",
                "Microsoft Basic Data (FAT32)" | "FAT32" | "FAT16" | "FAT12" => "fat32",
                "Linux swap" => "swap",
                "Extended" => "extended",
                _ => "part",
            };

            let partition_name = alloc::format!("{}{}", name, i);
            let pdev = PartitionBlockDevice::new(
                device_index,
                info.start_lba,
                info.size_blocks,
                &partition_name,
            );

            if register_partition_device(pdev).is_ok() {
                registered += 1;
            }
        }
    }

    println!(
        "[block_dev] Registered {} partitions from device {}",
        registered, device_index
    );
    Ok(registered)
}
