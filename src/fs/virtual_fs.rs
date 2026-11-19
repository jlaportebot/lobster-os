//! `/proc`, `/dev` and `/sys` as VFS directories. The generators in
//! `procfs`, `devfs` and `sysfs` produce content on demand; this module
//! projects them into the VFS as RAM nodes that are regenerated whenever a
//! path beneath them is resolved.

use crate::fs::vfs;
use crate::fs::{devfs, procfs, sysfs};
use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

/// Size of the scratch buffer one generated file is rendered into.
const CONTENT_BUF_BYTES: usize = 2048;
/// Mount points of the three virtual filesystems.
const PROC_ROOT: &str = "/proc";
const DEV_ROOT: &str = "/dev";
const SYS_ROOT: &str = "/sys";

/// Set once [`mount`] has run; refreshing is a no-op before that.
static MOUNTED: AtomicBool = AtomicBool::new(false);
/// Set while a refresh writes nodes, so its own path lookups do not recurse.
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// Create the three directories, initialise the generators and fill the
/// nodes once.
pub fn mount() {
    for dir in [PROC_ROOT, DEV_ROOT, SYS_ROOT] {
        if let Err(e) = vfs::create_dir(dir) {
            crate::println!("[vfs] mkdir {} failed: {:?}", dir, e);
        }
    }
    devfs::init();
    procfs::init();
    sysfs::init();
    MOUNTED.store(true, Ordering::Relaxed);
    for dir in [PROC_ROOT, DEV_ROOT, SYS_ROOT] {
        refresh_for(dir);
    }
    crate::println!("[vfs] Virtual filesystems mounted: /proc, /dev, /sys");
}

/// Forget the mount (the VFS is being re-initialised).
pub fn reset() {
    MOUNTED.store(false, Ordering::Relaxed);
}

/// Whether `path` is `root` or lies beneath it (whole components).
fn is_under(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Regenerate the nodes of whichever virtual filesystem `path` belongs to.
/// Called by the VFS before it resolves any path.
pub fn refresh_for(path: &str) {
    if !MOUNTED.load(Ordering::Relaxed) || !path.starts_with('/') {
        return;
    }
    if REFRESHING.swap(true, Ordering::Acquire) {
        return;
    }
    if is_under(path, PROC_ROOT) {
        refresh_proc();
    } else if is_under(path, DEV_ROOT) {
        refresh_dev();
    } else if is_under(path, SYS_ROOT) {
        refresh_sys();
    }
    REFRESHING.store(false, Ordering::Release);
}

/// Replace /proc's files: stale per-process entries disappear and every
/// value is read fresh.
fn refresh_proc() {
    if let Ok(old) = vfs::list_dir(PROC_ROOT) {
        for entry in old {
            let _ = vfs::remove(&alloc::format!("{}/{}", PROC_ROOT, entry.name_str()));
        }
    }
    procfs::build_root_listing();
    let mut buf = [0u8; CONTENT_BUF_BYTES];
    for entry in procfs::root_entries() {
        let len = procfs::read_proc_file(entry.entry_type, entry.pid, &mut buf);
        let path = alloc::format!("{}/{}", PROC_ROOT, entry.name_str());
        let _ = vfs::write_file(&path, &buf[..len.min(buf.len())]);
    }
}

/// One empty node per registered device (reads go through the device ops,
/// not the node).
fn refresh_dev() {
    for index in 0..devfs::device_count() {
        if let Some(device) = devfs::get_device(index) {
            let path = alloc::format!("{}/{}", DEV_ROOT, device.name_str());
            let _ = vfs::write_file_if_missing(&path, b"");
        }
    }
}

/// Create the intermediate directories of `rel` (`cpu/core0/freq`) under /sys.
fn make_sys_parents(rel: &str) {
    let mut dir = String::from(SYS_ROOT);
    let mut parts = rel.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            break;
        }
        dir.push('/');
        dir.push_str(part);
        let _ = vfs::create_dir(&dir);
    }
}

/// Rewrite every sysfs value file with current content.
fn refresh_sys() {
    let (count, entries) = sysfs::get_entries();
    let mut buf = [0u8; CONTENT_BUF_BYTES];
    for entry in entries.iter().take(count) {
        make_sys_parents(entry.name_str());
        let len = sysfs::read_sysfs_entry(entry.entry_type, entry.param, &mut buf);
        let path = alloc::format!("{}/{}", SYS_ROOT, entry.name_str());
        let _ = vfs::write_file(&path, &buf[..len.min(buf.len())]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::vfs;

    fn names(path: &str) -> Vec<String> {
        vfs::list_dir(path)
            .map(|e| e.iter().map(|d| d.name_str().to_string()).collect())
            .unwrap_or_default()
    }

    fn boot() {
        vfs::init();
        mount();
    }

    #[test]
    fn dev_lists_the_registered_devices() {
        let _guard = crate::test_support::serial_guard();
        boot();
        let dev = names("/dev");
        for expected in ["null", "zero", "urandom"] {
            assert!(dev.iter().any(|n| n == expected), "{expected} in {dev:?}");
        }
    }

    #[test]
    fn proc_lists_and_reads_meminfo() {
        let _guard = crate::test_support::serial_guard();
        boot();
        assert!(names("/proc").iter().any(|n| n == "meminfo"));
        let text = vfs::read_file("/proc/meminfo").expect("read meminfo");
        assert!(!text.is_empty(), "meminfo has content");
    }

    #[test]
    fn sys_exposes_nested_entries() {
        let _guard = crate::test_support::serial_guard();
        boot();
        assert!(names("/sys").iter().any(|n| n == "kernel"));
        let version = vfs::read_file("/sys/kernel/version").expect("read version");
        assert!(!version.is_empty());
    }

    #[test]
    fn proc_is_regenerated_on_access() {
        let _guard = crate::test_support::serial_guard();
        boot();
        vfs::create_dir("/proc/stale").expect("stale entry");
        assert!(!names("/proc").iter().any(|n| n == "stale"));
        assert!(names("/proc").iter().any(|n| n == "uptime"));
    }

    #[test]
    fn paths_outside_the_virtual_roots_are_not_refreshed() {
        let _guard = crate::test_support::serial_guard();
        boot();
        vfs::create_dir("/procfs-lookalike").expect("mkdir");
        assert!(names("/procfs-lookalike").is_empty());
        assert!(names("/").iter().any(|n| n == "procfs-lookalike"));
    }
}
