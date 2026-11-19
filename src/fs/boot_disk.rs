//! Boot-time mount of the persistent disk (QEMU `lobster-vd0`) and the
//! persistent prefix map that sits on top of it.

use crate::fs::{block_dev, ext2, prefix_map};
use crate::println;

/// Bytes per block-device sector, for the size line in the boot log.
const SECTOR_BYTES: u64 = 512;
/// Bytes per mebibyte, for the size line in the boot log.
const MIB: u64 = 1024 * 1024;

/// Detect the paravirtual disk, mount its whole-disk ext2 volume at
/// `/sd` and turn on the persistent prefix map. Must run right after
/// `vfs::init()` and before anything reads or writes `/etc` or `/var`.
/// Returns whether the disk is mounted; without one everything stays RAM.
pub fn mount() -> bool {
    println!("[vd] Probing lobster-vd0 (paravirtual disk)...");
    let Some(blocks) = crate::drivers::lobster_storage::detect() else {
        println!("[vd] no paravirtual disk (LBST_DISKIMG unset)");
        return false;
    };
    crate::drivers::lobster_storage::TOTAL_BLOCKS
        .store(blocks, core::sync::atomic::Ordering::Relaxed);
    static DISK: crate::drivers::lobster_storage::LobsterDisk =
        crate::drivers::lobster_storage::LobsterDisk::new();
    let index = match block_dev::register_device(&DISK) {
        Ok(index) => index,
        Err(e) => {
            println!("[vd] register failed: {:?}", e);
            return false;
        }
    };
    println!(
        "[vd] lobster-vd0: {} blocks ({} MB) registered as device {}",
        blocks,
        blocks * SECTOR_BYTES / MIB,
        index
    );
    if let Err(e) = ext2::mount(index, 0, prefix_map::BACKING_ROOT) {
        println!("[ext2] /sd mount failed: {:?}", e);
        return false;
    }
    let active = prefix_map::enable();
    println!("[ext2] persistent /sd mounted from lobster-vd0 ({} prefixes on disk)", active);
    true
}
