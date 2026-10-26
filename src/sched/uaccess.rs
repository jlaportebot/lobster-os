//! User-pointer validation (audit Q4).
//!
//! Every syscall that takes a pointer argument must validate it before
//! dereferencing: the pointer must be non-null, fully inside the user VA
//! range (below the kernel's low-half boundary), and — on builds with
//! per-process address spaces — backed by a mapped page. Until Q4's
//! address-space split lands, the range check alone already prevents the
//! classic "pass a kernel address to a syscall" escalation on the shared
//! page table.

/// Upper bound of user VA space (TTBR0 low half on this kernel is
/// [0, 0x0000_8000_0000_0000); user programs are linked far below).
const USER_VA_MAX: u64 = 0x0000_8000_0000_0000;

/// Validate a user read buffer: `ptr` and `ptr + len` must both be in the
/// user range. Returns a slice or None (caller maps None to -EFAULT).
///
/// # Safety
/// The caller must be executing on behalf of the current EL0 task.
pub fn user_slice<'a>(ptr: u64, len: u64) -> Option<&'a [u8]> {
    if len == 0 {
        return Some(&[]);
    }
    if ptr == 0 {
        return None;
    }
    let end = ptr.checked_add(len)?;
    if end > USER_VA_MAX {
        return None;
    }
    // A single page must not wrap the range check via overflow — checked above.
    Some(unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) })
}

/// Validate a user write buffer (same rules as reads for now).
pub fn user_slice_mut<'a>(ptr: u64, len: u64) -> Option<&'a mut [u8]> {
    if len == 0 {
        return Some(&mut []);
    }
    if ptr == 0 {
        return None;
    }
    let end = ptr.checked_add(len)?;
    if end > USER_VA_MAX {
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
    va != 0 && va < USER_VA_MAX
}
