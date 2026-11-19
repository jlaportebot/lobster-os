//! Persistent prefix map: while the disk is mounted, selected top-level paths
//! (`/etc`, `/var`, ...) are rewritten to the same path under `/sd` before the
//! VFS looks them up, so system state survives a reboot with one mechanism
//! instead of per-module `/sd/...` paths. See docs/adr/0001-persistent-root.md.

use crate::fs::vfs;
use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

/// Where the persistent disk is mounted.
pub const BACKING_ROOT: &str = "/sd";

/// Top-level directories that live on the disk while it is mounted. The
/// position in this table is the bit in `ACTIVE_MASK`.
pub const PERSISTENT_PREFIXES: [&str; 8] = [
    "/bin", "/etc", "/var", "/usr", "/apps", "/opt", "/root", "/home",
];

/// Bit set per prefix whose backing directory exists on disk.
static ACTIVE_MASK: AtomicU32 = AtomicU32::new(0);

/// Bit of `prefix` in the mask, or `None` for a path that is not in the table.
fn prefix_bit(prefix: &str) -> Option<u32> {
    PERSISTENT_PREFIXES
        .iter()
        .position(|p| *p == prefix)
        .map(|index| 1 << index)
}

/// Turn the map on for one prefix of [`PERSISTENT_PREFIXES`].
pub fn activate(prefix: &str) {
    if let Some(bit) = prefix_bit(prefix) {
        ACTIVE_MASK.fetch_or(bit, Ordering::Relaxed);
    }
}

/// Turn the whole map off (no disk: everything is RAM).
pub fn deactivate_all() {
    ACTIVE_MASK.store(0, Ordering::Relaxed);
}

/// Whether `prefix` currently resolves onto the disk.
pub fn is_active(prefix: &str) -> bool {
    prefix_bit(prefix).is_some_and(|bit| ACTIVE_MASK.load(Ordering::Relaxed) & bit != 0)
}

/// Mount-time setup: make sure each prefix's backing directory exists on the
/// disk and turn the map on for it. A prefix whose directory cannot be
/// created stays RAM-only. Returns how many prefixes are active.
pub fn enable() -> usize {
    let mut active = 0;
    for prefix in PERSISTENT_PREFIXES {
        let backing = alloc::format!("{BACKING_ROOT}{prefix}");
        if vfs::resolve_path(&backing).is_err() {
            if let Err(e) = vfs::create_dir(&backing) {
                crate::println!(
                    "[vfs] {} stays in RAM: mkdir {} failed: {:?}",
                    prefix,
                    backing,
                    e
                );
                continue;
            }
        }
        activate(prefix);
        active += 1;
    }
    active
}

/// Resolve `.`, `..` and empty components lexically; borrowed when the path
/// is already clean. `..` at the root stays at the root.
fn normalize(path: &str) -> Cow<'_, str> {
    let clean = path
        .split('/')
        .skip(1)
        .all(|part| !part.is_empty() && part != "." && part != "..");
    if clean || path == "/" {
        return Cow::Borrowed(path);
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    Cow::Owned(alloc::format!("/{}", parts.join("/")))
}

/// The active prefix that `path` (already normalised) starts with, as a
/// whole path component.
fn matching_prefix(path: &str) -> Option<&'static str> {
    PERSISTENT_PREFIXES.iter().copied().find(|prefix| {
        is_active(prefix)
            && path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// Rewrite `path` onto the disk when it falls under an active prefix. With
/// the map off the path is returned untouched.
pub fn rewrite(path: &str) -> Cow<'_, str> {
    if ACTIVE_MASK.load(Ordering::Relaxed) == 0 || !path.starts_with('/') {
        return Cow::Borrowed(path);
    }
    let path = normalize(path);
    if matching_prefix(&path).is_none() {
        return path;
    }
    Cow::Owned(alloc::format!("{BACKING_ROOT}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enable_all() {
        deactivate_all();
        for prefix in PERSISTENT_PREFIXES {
            activate(prefix);
        }
    }

    #[test]
    fn every_prefix_and_nested_path_maps_onto_the_disk() {
        let _guard = crate::test_support::serial_guard();
        enable_all();
        for prefix in PERSISTENT_PREFIXES {
            assert_eq!(rewrite(prefix), format!("/sd{prefix}"));
            let nested = format!("{prefix}/a/b.txt");
            assert_eq!(rewrite(&nested), format!("/sd{nested}"));
        }
        deactivate_all();
    }

    #[test]
    fn volatile_and_virtual_paths_are_never_mapped() {
        let _guard = crate::test_support::serial_guard();
        enable_all();
        for path in [
            "/",
            "/tmp",
            "/tmp/x",
            "/dev/null",
            "/proc/uptime",
            "/sys/kernel",
            "/sd/etc/x",
            "/share/a",
        ] {
            assert_eq!(rewrite(path), path);
        }
        deactivate_all();
    }

    #[test]
    fn a_prefix_that_is_not_a_whole_component_is_not_mapped() {
        let _guard = crate::test_support::serial_guard();
        enable_all();
        for path in ["/etcetera", "/etc2/x", "/variable", "/homework/a", "/binx"] {
            assert_eq!(rewrite(path), path);
        }
        deactivate_all();
    }

    #[test]
    fn dot_dot_is_resolved_before_the_prefix_is_matched() {
        let _guard = crate::test_support::serial_guard();
        enable_all();
        assert_eq!(rewrite("/etc/../tmp/x"), "/tmp/x");
        assert_eq!(rewrite("/tmp/../etc/passwd"), "/sd/etc/passwd");
        assert_eq!(rewrite("/etc/./pkg//repos.conf"), "/sd/etc/pkg/repos.conf");
        assert_eq!(rewrite("/etc/.."), "/");
        assert_eq!(rewrite("/../../etc"), "/sd/etc");
        deactivate_all();
    }

    #[test]
    fn the_map_is_off_without_a_disk() {
        let _guard = crate::test_support::serial_guard();
        deactivate_all();
        for prefix in PERSISTENT_PREFIXES {
            assert!(!is_active(prefix));
            assert_eq!(rewrite(prefix), prefix);
        }
        assert_eq!(rewrite("/etc/../tmp"), "/etc/../tmp");
    }

    #[test]
    fn only_activated_prefixes_are_mapped() {
        let _guard = crate::test_support::serial_guard();
        deactivate_all();
        activate("/etc");
        assert!(is_active("/etc"));
        assert!(!is_active("/var"));
        assert_eq!(rewrite("/etc/x"), "/sd/etc/x");
        assert_eq!(rewrite("/var/x"), "/var/x");
        deactivate_all();
    }
}
