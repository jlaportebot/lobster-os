//! Persistent prefix map: while the disk is mounted, selected top-level paths
//! (`/etc`, `/var`, ...) are rewritten to the same path under `/sd` before the
//! VFS looks them up, so system state survives a reboot with one mechanism
//! instead of per-module `/sd/...` paths. See docs/adr/0001-persistent-root.md.

use alloc::borrow::Cow;
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

/// Turn the map on for one prefix of [`PERSISTENT_PREFIXES`].
pub fn activate(prefix: &str) {
    let _ = prefix;
}

/// Turn the whole map off (no disk: everything is RAM).
pub fn deactivate_all() {
    ACTIVE_MASK.store(0, Ordering::Relaxed);
}

/// Whether `prefix` currently resolves onto the disk.
pub fn is_active(prefix: &str) -> bool {
    let _ = prefix;
    false
}

/// Mount-time setup: make sure each prefix's backing directory exists on the
/// disk and turn the map on for it. Returns how many prefixes are active.
pub fn enable() -> usize {
    0
}

/// Rewrite `path` onto the disk when it falls under an active prefix.
pub fn rewrite(path: &str) -> Cow<'_, str> {
    Cow::Borrowed(path)
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
        for path in ["/", "/tmp", "/tmp/x", "/dev/null", "/proc/uptime", "/sys/kernel", "/sd/etc/x", "/share/a"] {
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
