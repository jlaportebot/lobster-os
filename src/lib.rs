//! LobsterOS — Bare-metal Rust OS for Raspberry Pi 5 (BCM2712)
//!
//! This crate provides the core kernel functionality as a library,
//! enabling unit testing and integration testing on host.

#![cfg_attr(not(feature = "std"), no_std)]
#![allow(
    dead_code,
    static_mut_refs,
    clippy::unusual_byte_groupings,
    private_interfaces,
    clippy::precedence,
    unused_results,
    clippy::multiple_crate_versions,
    clippy::wildcard_imports,
    renamed_and_removed_lints,
    clippy::identity_op,
    clippy::needless_range_loop,
    clippy::manual_is_multiple_of,
    clippy::unnecessary_cast,
    clippy::manual_div_ceil,
    clippy::never_loop,
    clippy::doc_lazy_continuation,
    clippy::manual_range_contains,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::manual_memcpy,
    clippy::manual_rotate,
    clippy::option_map_unit_fn,
    clippy::unnecessary_map_or,
    clippy::non_canonical_clone_impl,
    clippy::if_same_then_else,
    clippy::derivable_impls,
    clippy::fn_to_numeric_cast,
    clippy::too_many_arguments,
    clippy::manual_flatten,
    clippy::slow_vector_initialization,
    clippy::explicit_auto_deref,
    clippy::needless_borrow,
    clippy::clone_on_copy,
    clippy::manual_strip,
    clippy::char_lit_as_u8,
    clippy::single_match,
    clippy::repr_packed_without_abi,
    clippy::type_complexity,
    clippy::op_ref,
    clippy::let_and_return,
    clippy::manual_contains,
    clippy::missing_safety_doc,
    unused_must_use,
    dropping_references
)]

extern crate alloc;

// Core modules available on all targets (stubbed for x86_64)
pub mod arch;
pub mod console;
pub mod ipc;
pub mod memory;
pub mod net;
pub mod panic_handler;
pub mod qemu_test;
pub mod security;
pub mod sync;
pub mod terminal;

// AArch64-specific modules
#[cfg(target_arch = "aarch64")]
pub mod apps;
#[cfg(target_arch = "aarch64")]
pub mod display;
#[cfg(target_arch = "aarch64")]
pub mod drivers;
#[cfg(target_arch = "aarch64")]
pub mod dtb;
#[cfg(target_arch = "aarch64")]
pub mod fs;
#[cfg(target_arch = "aarch64")]
pub mod pkg;
#[cfg(target_arch = "aarch64")]
pub mod sched;
#[cfg(target_arch = "aarch64")]
pub mod shell;
#[cfg(target_arch = "aarch64")]
pub mod smp;
#[cfg(target_arch = "aarch64")]
pub mod usb;
#[cfg(target_arch = "aarch64")]
pub mod wm;

// Re-export commonly used types for testing
pub use crate::memory::{PhysAddr, VirtAddr, PAGE_SIZE};
pub use crate::sync::Spinlock;

/// Kernel initialization entry point — called by the binary
/// This is a stub for library usage; the binary provides the real implementation
#[cfg(target_arch = "aarch64")]
pub fn kernel_main(_dtb_ptr: *const u8) -> ! {
    // In the actual kernel, the binary's _start calls the real kernel_init
    // This stub is only used when the library is used as a library
    loop {
        crate::arch::cpu::wfe();
    }
}

// Unit tests for core functionality
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_physaddr_virtaddr_basic() {
        let pa = PhysAddr::new(0x1000);
        let va = VirtAddr::new(0x2000);
        assert_eq!(pa.as_u64(), 0x1000);
        assert_eq!(va.as_u64(), 0x2000);
    }

    #[test]
    fn test_page_size_constant() {
        assert_eq!(PAGE_SIZE, 4096);
    }

    #[test]
    fn test_spinlock_basic() {
        let lock = Spinlock::new(42);
        let mut guard = lock.lock();
        assert_eq!(*guard, 42);
        *guard = 100;
        drop(guard);
        assert_eq!(*lock.lock(), 100);
    }
}

/// SHA-1 and HMAC-SHA1 tests (from wifi driver)
#[cfg(all(test, feature = "std", target_arch = "aarch64"))]
mod crypto_tests {
    use crate::drivers::wifi::sha1::{hmac_sha1, sha1_hash};

    #[test]
    fn test_sha1_empty() {
        let hash = sha1_hash(b"");
        assert_eq!(
            hash,
            [
                0xda, 0x39, 0xa3, 0xee, 0x5e, 0x6b, 0x4b, 0x0d, 0x32, 0x55, 0xbf, 0xef, 0x95, 0x60,
                0x18, 0x90, 0xaf, 0xd8, 0x07, 0x09
            ]
        );
    }

    #[test]
    fn test_sha1_abc() {
        let hash = sha1_hash(b"abc");
        assert_eq!(
            hash,
            [
                0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50,
                0xc2, 0x6c, 0x9c, 0xd0, 0xd8, 0x9d
            ]
        );
    }

    #[test]
    fn test_hmac_sha1_basic() {
        let key = b"key";
        let data = b"data";
        let mac = hmac_sha1(key, data);
        assert_eq!(mac.len(), 20);
    }

    #[test]
    fn test_hmac_sha1_rfc2202() {
        let key = [0x0b; 20];
        let data = b"Hi There";
        let mac = hmac_sha1(&key, data);
        assert_eq!(
            mac,
            [
                0xb6, 0x17, 0x31, 0x86, 0x55, 0x05, 0x72, 0x64, 0xe2, 0x8b, 0xc0, 0xb6, 0xfb, 0x37,
                0x8c, 0x8e, 0xf1, 0x46, 0x4e, 0xdc
            ]
        );
    }
}

/// Memory allocator tests
#[cfg(all(test, feature = "std"))]
mod alloc_tests {
    use alloc::collections::BTreeMap;
    use alloc::string::String;
    use alloc::vec::Vec;

    #[test]
    fn test_vec_operations() {
        let mut v = Vec::new();
        v.push(1);
        v.push(2);
        v.push(3);
        assert_eq!(v.len(), 3);
        assert_eq!(v.pop(), Some(3));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn test_string_operations() {
        let mut s = String::from("hello");
        s.push_str(" world");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_btreemap_operations() {
        let mut m = BTreeMap::new();
        m.insert("a", 1);
        m.insert("b", 2);
        assert_eq!(m.get("a"), Some(&1));
        assert_eq!(m.len(), 2);
    }
}

/// Network stack tests (using core net types)
#[cfg(all(test, feature = "std"))]
mod net_tests {
    use core::net::Ipv4Addr;

    #[test]
    fn test_ipv4_addr_parsing() {
        let addr: Ipv4Addr = "192.168.1.1".parse().unwrap();
        assert_eq!(addr.octets(), [192, 168, 1, 1]);
    }
}

/// Integration test helpers for QEMU
#[cfg(all(test, feature = "qemu"))]
mod qemu_integration_tests {
    use crate::arch::cpu::delay_us;

    #[test]
    fn test_cpu_delay() {
        delay_us(100);
    }
}
