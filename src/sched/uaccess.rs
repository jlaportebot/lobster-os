//! User-pointer validation (audit Q4).
//!
//! Every syscall that takes a pointer argument must validate it before
//! dereferencing: the pointer must be non-null, fully inside the user VA
//! range (below the kernel's low-half boundary), and — on builds with
//! per-process address spaces — backed by a mapped page. Until Q4's
//! address-space split lands, the range check alone already prevents the
//! classic "pass a kernel address to a syscall" escalation on the shared
//! page table.

use crate::memory::process_vmem::{USER_ADDR_LIMIT, USER_CODE_START};

/// True when `[ptr, ptr + len)` lies entirely inside the EL0 window
/// `USER_CODE_START..USER_ADDR_LIMIT`. The kernel image (linked at 0x80000)
/// and its heap and stacks are identity-mapped low, below that window, so a
/// bare "below the top of the low half" check would let EL0 name kernel
/// memory. Overflow-safe; the empty range is accepted anywhere in the window.
#[must_use]
pub fn range_in_user_window(ptr: u64, len: u64) -> bool {
    match ptr.checked_add(len) {
        Some(end) => ptr >= USER_CODE_START && end <= USER_ADDR_LIMIT,
        None => false,
    }
}

/// Validate a user read buffer: `ptr` and `ptr + len` must both be in the
/// user range. Returns a slice or None (caller maps None to -EFAULT).
///
/// # Safety
/// The caller must be executing on behalf of the current EL0 task.
pub fn user_slice<'a>(ptr: u64, len: u64) -> Option<&'a [u8]> {
    if len == 0 {
        return Some(&[]);
    }
    if !range_in_user_window(ptr, len) {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) })
}

/// Validate a user write buffer (same rules as reads for now).
pub fn user_slice_mut<'a>(ptr: u64, len: u64) -> Option<&'a mut [u8]> {
    if len == 0 {
        return Some(&mut []);
    }
    if !range_in_user_window(ptr, len) {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize) })
}

/// Read a NUL-terminated string from user memory (bounded).
pub fn user_str<'a>(ptr: u64, max_len: u64) -> Option<&'a str> {
    let bytes = user_slice(ptr, max_len)?;
    let nul = bytes.iter().position(|&b| b == 0)?;
    core::str::from_utf8(&bytes[..nul]).ok()
}

/// True if `va` is a plausible user virtual address (fast check for
/// syscall paths that only need the range gate).
pub fn is_user_va(va: u64) -> bool {
    range_in_user_window(va, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    static SOME_KERNEL_STATIC: u64 = 0;

    #[test]
    fn kernel_text_address_is_rejected() {
        assert!(!range_in_user_window(0x8_0000, 64));
        assert!(user_slice_mut(0x8_0000, 64).is_none());
        assert!(!is_user_va(0x8_0000));
    }

    #[test]
    fn address_of_a_kernel_static_is_rejected() {
        let addr = core::ptr::addr_of!(SOME_KERNEL_STATIC) as u64;
        let inside = (USER_CODE_START..USER_ADDR_LIMIT).contains(&addr);
        assert_eq!(range_in_user_window(addr, 8), inside);
        assert!(!range_in_user_window(0x0100_0000, 8));
        assert!(
            !range_in_user_window(crate::memory::HEAP_START, 8),
            "kernel heap"
        );
    }

    #[test]
    fn null_and_low_pages_are_rejected() {
        assert!(!range_in_user_window(0, 16));
        assert!(!range_in_user_window(USER_CODE_START - 1, 16));
    }

    #[test]
    fn wrap_around_is_rejected() {
        assert!(!range_in_user_window(u64::MAX - 4, 16));
        assert!(!range_in_user_window(USER_CODE_START, u64::MAX));
    }

    #[test]
    fn upper_edge_is_exact() {
        assert!(range_in_user_window(USER_ADDR_LIMIT - 16, 16));
        assert!(!range_in_user_window(USER_ADDR_LIMIT - 15, 16));
        assert!(!range_in_user_window(USER_ADDR_LIMIT, 1));
    }

    #[test]
    fn valid_user_buffer_is_accepted() {
        assert!(range_in_user_window(USER_CODE_START, 4096));
        assert!(range_in_user_window(0x7F00_0000 - 4096, 4096));
        assert!(is_user_va(USER_CODE_START));
    }
}
