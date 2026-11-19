//! `/proc`, `/dev` and `/sys` as VFS directories. The generators in
//! `procfs`, `devfs` and `sysfs` produce content on demand; this module
//! projects them into the VFS as RAM nodes that are regenerated whenever a
//! path beneath them is resolved.

/// Create the three directories and initialise the generators.
pub fn mount() {}

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
    fn proc_files_are_regenerated_on_every_access() {
        let _guard = crate::test_support::serial_guard();
        boot();
        let first = names("/proc");
        vfs::remove("/proc/version").expect("remove stale node");
        assert!(!names("/proc").is_empty());
        assert!(names("/proc").iter().any(|n| n == "version"), "{first:?}");
    }
}
