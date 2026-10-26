//! LobsterOS — Bare-metal Rust OS for Raspberry Pi 5 (BCM2712)
//!
//! Binary entry point that delegates to the library.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(feature = "std"), no_main)]
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

use core::arch::global_asm;
use lobster_os::kernel_main;

#[cfg(target_arch = "aarch64")]
global_asm!(include_str!("boot.S"));

#[cfg(all(target_arch = "x86_64", not(test)))]
global_asm!(include_str!("boot_x86_64.S"));

// Linker symbols for kernel boundaries
extern "C" {
    static __kernel_end: u64;
    static __bss_start: u64;
    static __bss_end: u64;
}

/// Kernel entry point for AArch64 — called from boot.S with x0 = DTB pointer.
/// Not built for host `std` test binaries (their C runtime owns _start).
#[cfg(all(target_arch = "aarch64", not(feature = "std")))]
#[no_mangle]
pub extern "C" fn _start(dtb_ptr: *const u8) -> ! {
    kernel_main(dtb_ptr)
}

/// Panic handler for x86_64 test builds
#[cfg(all(target_arch = "x86_64", test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // In test mode, we can't really halt
    loop {
        core::hint::spin_loop();
    }
}

/// Kernel initialization — called from boot.S with x0 = DTB pointer
#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub unsafe extern "C" fn kernel_init(dtb_ptr: u64) -> ! {
    // Delegate to library's kernel_main
    lobster_os::kernel_main(dtb_ptr as *const u8)
}

/// Secondary core initialization — called from boot.S for cores 1-3
#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub unsafe extern "C" fn secondary_core_init(_core_id: u64) -> ! {
    // For now, just halt
    loop {
        core::arch::asm!("wfe");
    }
}
