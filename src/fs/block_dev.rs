//! Block device abstraction — trait for storage devices + MBR partition parsing

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
}

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
}

impl PartitionInfo {
    pub const fn empty() -> Self {
        Self {
            index: 0,
            start_lba: 0,
            size_blocks: 0,
            partition_type: 0,
            bootable: false,
        }
    }

    /// Human-readable partition type
    pub fn type_name(&self) -> &str {
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
            0xAF => "HFS/HFS+",
            _ => "Unknown",
        }
    }

    /// Size in MiB
    pub fn size_mib(&self) -> u64 {
        (self.size_blocks * 512) / (1024 * 1024)
    }
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
        println!("[block_dev] Invalid MBR signature: {:02X} {:02X}", mbr[510], mbr[511]);
        return result;
    }

    // Parse 4 partition entries at offset 446
    for i in 0..4usize {
        let entry_offset = 446 + i * 16;
        let entry = unsafe {
            &*(mbr[entry_offset..].as_ptr() as *const MbrPartEntry)
        };

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
        });

        println!(
            "[block_dev] Partition {}: {} LBA={}, {} blocks ({} MiB) {}",
            i,
            result[i].unwrap().type_name(),
            start_lba,
            size_blocks,
            result[i].unwrap().size_mib(),
            if entry.boot_indicator == 0x80 { "[Boot]" } else { "" }
        );
    }

    result
}

/// Maximum registered block devices
const MAX_BLOCK_DEVICES: usize = 4;

/// Global block device registry
static mut BLOCK_DEVICES: [Option<&'static dyn BlockDevice>; MAX_BLOCK_DEVICES] = [None; MAX_BLOCK_DEVICES];
static mut BLOCK_DEVICE_COUNT: usize = 0;

/// Register a block device
pub fn register_device(dev: &'static dyn BlockDevice) -> Result<usize, BlockError> {
    unsafe {
        if BLOCK_DEVICE_COUNT >= MAX_BLOCK_DEVICES {
            return Err(BlockError::Unsupported);
        }
        let idx = BLOCK_DEVICE_COUNT;
        BLOCK_DEVICES[idx] = Some(dev);
        BLOCK_DEVICE_COUNT += 1;
        println!("[block_dev] Registered device '{}' at index {}", dev.name(), idx);
        Ok(idx)
    }
}

/// Get a registered block device by index
pub fn get_device(index: usize) -> Option<&'static dyn BlockDevice> {
    unsafe {
        if index < BLOCK_DEVICE_COUNT {
            BLOCK_DEVICES[index]
        } else {
            None
        }
    }
}

/// SD Card device stub — will be connected to SDHCI driver later
pub struct SdCardDevice {
    pub index: usize,
}

impl SdCardDevice {
    pub const fn new(index: usize) -> Self {
        Self { index }
    }
}

impl BlockDevice for SdCardDevice {
    fn read_block(&self, block: u64, buf: &mut [u8; 512]) -> Result<(), BlockError> {
        // TODO: Connect to BCM2712 SDHCI driver (emmc2)
        // For now, return zeros
        buf.fill(0);
        let _ = (self.index, block);
        Ok(())
    }

    fn write_block(&self, block: u64, _buf: &[u8; 512]) -> Result<(), BlockError> {
        // TODO: Connect to BCM2712 SDHCI driver
        let _ = (self.index, block);
        Err(BlockError::Unsupported)
    }

    fn block_size(&self) -> usize {
        512
    }

    fn total_blocks(&self) -> u64 {
        // TODO: Read actual card size from CSD register
        // Default: 32GB card ≈ 61056000 blocks
        61056000
    }

    fn name(&self) -> &str {
        "sdcard"
    }

    fn media_present(&self) -> bool {
        // TODO: Check card detect
        true
    }
}

/// Initialize the block device subsystem
pub fn init() {
    unsafe {
        BLOCK_DEVICE_COUNT = 0;
    }
    println!("[block_dev] Block device subsystem initialized");
}
