//! Filesystem subsystem — VFS, FAT32, initrd, block devices

pub mod vfs;
pub mod fat32;
pub mod initrd;
pub mod block_dev;

pub use vfs::*;
