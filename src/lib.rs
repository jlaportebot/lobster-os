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

// Linker symbols for kernel boundaries (defined in linker script)
extern "C" {
    static __kernel_end: u64;
    static __bss_start: u64;
    static __bss_end: u64;
}

/// Kernel initialization entry point — called by the binary
/// This is the real kernel initialization for AArch64
#[cfg(target_arch = "aarch64")]
pub fn kernel_main(dtb_ptr: *const u8) -> ! {
    // 1. UART + Console first
    unsafe {
        crate::drivers::uart::Uart10::init();
    }
    crate::console::init();

    crate::println!();
    crate::println!("╔══════════════════════════════════════╗");
    crate::println!("║  LobsterOS v0.1.0                   ║");
    crate::println!("║  Bare-metal Rust on BCM2712         ║");
    crate::println!("║  Quad-core Cortex-A76 @ 2.4 GHz    ║");
    crate::println!("╚══════════════════════════════════════╝");
    crate::println!();

    let el = crate::arch::cpu::current_el();
    crate::println!("[boot] Current EL: EL{}", el);
    crate::println!("[boot] DTB pointer: {:#018x}", dtb_ptr as u64);
    crate::println!(
        "[boot] Stack pointer: {:#018x}",
        crate::arch::cpu::read_sp()
    );

    // 2. Zero BSS
    let bss_start = unsafe { &__bss_start as *const u64 as u64 };
    let bss_end = unsafe { &__bss_end as *const u64 as u64 };
    let bss_size = bss_end - bss_start;
    unsafe {
        core::ptr::write_bytes(bss_start as *mut u8, 0, bss_size as usize);
    }
    crate::println!(
        "[boot] BSS zeroed: {:#010x}..{:#010x} ({} bytes)",
        bss_start,
        bss_end,
        bss_size
    );

    // 3. GIC
    crate::println!("[gic] Initializing GIC-400...");
    unsafe {
        crate::arch::gic::Gic::init();
    }
    crate::println!("[gic] GIC-400 distributor + CPU interface enabled");

    // 4. Timer
    crate::println!("[timer] Initializing system timer...");
    unsafe {
        crate::arch::timer::Timer::init();
    }

    // 5. Enable IRQs
    crate::println!("[boot] Enabling IRQs...");
    crate::arch::cpu::enable_irqs();
    crate::println!("[boot] IRQs enabled");

    // 6. Memory Management (Phase 2)
    let kernel_phys_start = 0x80000u64; // kernel8.img load address
    let kernel_end = unsafe { &__kernel_end as *const u64 as u64 };
    let kernel_size = kernel_end - kernel_phys_start;
    // Assume 8GB RAM for RPi 5 (will parse DTB later for actual size)
    let ram_size = 8 * 1024 * 1024 * 1024u64;

    crate::println!(
        "[boot] Kernel: {:#010x}..{:#010x} ({} KB)",
        kernel_phys_start,
        kernel_end,
        kernel_size / 1024
    );

    // Initialize MMU (this will set up page tables and enable virtual memory)
    unsafe {
        crate::memory::mmu::init(kernel_phys_start, kernel_size, ram_size);

        // Now running with virtual memory enabled!
        // Switch to high VA kernel addresses (the identity map is still there
        // for transition, but we'll use the direct phys map from now on)

        // Initialize frame allocator
        crate::memory::frame_alloc::init(ram_size, kernel_end);

        // Initialize heap
        crate::memory::heap::init();

        // Register memory regions for the memory map
        crate::memory::vmem::register_region(crate::memory::vmem::MemoryRegion {
            virt_start: crate::memory::VirtAddr::new(crate::memory::KERNEL_IMAGE_START),
            phys_start: crate::memory::PhysAddr::new(kernel_phys_start),
            size: kernel_size,
            flags: crate::memory::vmem::PTE_KERNEL_CODE,
            name: "kernel",
        });
        crate::memory::vmem::register_region(crate::memory::vmem::MemoryRegion {
            virt_start: crate::memory::VirtAddr::new(crate::memory::PHYS_MAP_OFFSET),
            phys_start: crate::memory::PhysAddr::new(0),
            size: ram_size,
            flags: crate::memory::vmem::PTE_KERNEL_DATA,
            name: "phys-map",
        });
        crate::memory::vmem::register_region(crate::memory::vmem::MemoryRegion {
            virt_start: crate::memory::VirtAddr::new(crate::memory::HEAP_START),
            phys_start: crate::memory::PhysAddr::new(0), // dynamically mapped
            size: crate::memory::HEAP_SIZE,
            flags: crate::memory::vmem::PTE_KERNEL_DATA,
            name: "heap",
        });
        crate::memory::vmem::register_region(crate::memory::vmem::MemoryRegion {
            virt_start: crate::memory::VirtAddr::new(crate::memory::DEVICE_MAP_START),
            phys_start: crate::memory::PhysAddr::new(0x10_0000_0000),
            size: 0x1_0000_0000,
            flags: crate::memory::vmem::PTE_DEVICE,
            name: "devices",
        });
    }

    // Print memory map
    crate::memory::vmem::print_memory_map();

    // Test heap allocation
    crate::println!("[heap] Testing allocation...");
    unsafe {
        use core::alloc::{GlobalAlloc, Layout};
        let layout = Layout::from_size_align(256, 8).unwrap();
        let ptr = crate::memory::heap::LobsterAllocator.alloc(layout);
        if !ptr.is_null() {
            crate::println!("[heap] 256-byte allocation at {:#018x}: OK!", ptr as u64);
            // Write test pattern
            for i in 0u8..=255u8 {
                core::ptr::write_volatile(ptr.add(i as usize), i);
            }
            // Verify
            let mut ok = true;
            for i in 0u8..=255u8 {
                if core::ptr::read_volatile(ptr.add(i as usize)) != i {
                    ok = false;
                    break;
                }
            }
            if ok {
                crate::println!("[heap] Read/write test: PASSED!");
            } else {
                crate::println!("[heap] Read/write test: FAILED!");
            }
            crate::memory::heap::LobsterAllocator.dealloc(ptr, layout);
            crate::println!("[heap] Deallocation: OK!");
        } else {
            crate::println!("[heap] 256-byte allocation: FAILED!");
        }
    }

    crate::memory::heap::print_stats();

    // Frame allocator test
    crate::println!("[frame-alloc] Testing frame allocation...");
    if let Some(frame) = crate::memory::frame_alloc::allocate_frame() {
        crate::println!("[frame-alloc] Allocated frame at {:#010x}: OK!", frame.0);
        crate::memory::frame_alloc::deallocate_frame(frame);
        crate::println!("[frame-alloc] Deallocated: OK!");
    }
    let (total, allocated, first_free) = crate::memory::frame_alloc::stats();
    crate::println!(
        "[frame-alloc] Stats: {}/{} frames used, first free at frame {}",
        allocated,
        total,
        first_free
    );

    // 7. Initialize framebuffer display
    crate::println!("[display] Initializing framebuffer...");
    let info = match crate::display::fb::init(1920, 1080, 32) {
        Ok(info) => info,
        Err(_) => match crate::display::fb::init(640, 480, 32) {
            Ok(info) => info,
            Err(_) => crate::display::fb::get_info(),
        },
    };
    if info.width > 0 {
        crate::display::console::init(info);
        crate::display::logo::draw_splash(info);
        crate::display::logo::draw_ascii_logo();
        crate::println!("[display] Framebuffer + console ready");
    } else {
        crate::println!("[display] No framebuffer available (UART only)");
        crate::display::logo::draw_ascii_logo();
    }

    // 8. Initialize filesystem
    crate::println!("[fs] Initializing VFS...");
    crate::fs::vfs::init();
    crate::println!("[fs] VFS initialized");

    // 9. Parse the DTB
    crate::println!("[dtb] Parsing device tree...");
    crate::dtb::parse(dtb_ptr as u64);

    // 10. Start scheduler and services
    crate::println!("[sched] Starting scheduler...");
    crate::sched::init();

    // 11. Initialize SMP (bring up secondary cores)
    crate::println!("[smp] Starting secondary cores...");
    crate::smp::init_core0();
    crate::smp::boot_secondary_cores();

    // 12. Initialize network stack
    crate::println!("[net] Initializing network stack...");
    unsafe { crate::net::init() };

    // 13. Initialize USB
    crate::println!("[usb] Initializing USB stack...");
    crate::usb::init();

    // 14. Initialize window manager
    crate::println!("[wm] Initializing window manager...");
    crate::wm::init();

    // 15. Initialize core apps
    crate::println!("[apps] Initializing core applications...");
    crate::apps::init();

    // 16. Run kernel main loop
    crate::println!("[boot] LobsterOS initialization complete!");
    crate::println!("[boot] Entering main loop...");
    loop {
        crate::arch::cpu::wfe();
    }
}

// Unit tests for core functionality
#[cfg(all(test, feature = "std"))]
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
