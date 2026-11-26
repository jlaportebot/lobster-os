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

/// `-EFAULT` as returned to EL0 (two's complement of 14).
pub const EFAULT: u64 = (-14i64) as u64;

/// PTE bit 6, AP[1]: the page is accessible from EL0.
const PTE_EL0_ACCESS: u64 = 1 << 6;
/// PTE bit 7, AP[2]: the page is read-only.
const PTE_READ_ONLY: u64 = 1 << 7;
/// Software bit marking a copy-on-write page.
const PTE_COW_BIT: u64 = crate::memory::process_vmem::PTE_COW;
/// Output-address mask of a descriptor.
const PTE_OUT_MASK: u64 = 0x0000_FFFF_FFFF_F000;
/// Descriptor type bits (valid + table/page).
const PTE_TYPE_MASK: u64 = 0b11;
/// Valid table (L0-L2) or page (L3) descriptor.
const PTE_TABLE_OR_PAGE: u64 = 0b11;
/// Bytes per page.
const PAGE: u64 = 4096;
/// Largest chunk read per step by the string helper.
const STR_CHUNK: usize = 64;

/// A virtual address supplied by EL0. It can only be turned into data by
/// the copy helpers below, which walk the page table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserPtr(pub u64);

impl UserPtr {
    /// The pointer `bytes` further on (wrapping is caught by the walk).
    pub fn offset(self, bytes: u64) -> UserPtr {
        UserPtr(self.0.wrapping_add(bytes))
    }
}

/// Why a user access failed. Every variant maps to `-EFAULT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Outside the EL0 window, or no valid translation.
    Unmapped,
    /// Mapped but not EL0-accessible, or read-only for a write.
    Denied,
    /// A string did not end inside the caller's buffer.
    TooLong,
}

/// Access to translation tables and page frames by physical address, so the
/// walk can run against the direct map in the kernel and a model on the host.
pub trait PhysMem {
    /// Entry `idx` of the 512-entry table at physical address `table`.
    fn entry(&self, table: u64, idx: usize) -> u64;
    /// Pointer to the byte at physical address `pa`.
    fn byte_ptr(&self, pa: u64) -> *mut u8;
}

/// The kernel's direct physical map.
pub struct DirectMap;

impl PhysMem for DirectMap {
    fn entry(&self, table: u64, idx: usize) -> u64 {
        let base = (crate::memory::PHYS_MAP_OFFSET + table) as *const u64;
        // SAFETY: `table` is the physical address of a live table page and
        // `idx < 512`, so the read stays inside that page of the direct map.
        unsafe { core::ptr::read_volatile(base.add(idx)) }
    }

    fn byte_ptr(&self, pa: u64) -> *mut u8 {
        (crate::memory::PHYS_MAP_OFFSET + pa) as *mut u8
    }
}

/// Translate one user byte address through the tree rooted at `root`.
///
/// Returns the physical address, or the reason it may not be used. Only a
/// 4 KiB page leaf with AP[1] set (EL0 accessible) is accepted, so the
/// kernel's own EL1-only block entries inside a process root are never
/// reachable even though their VAs lie in the numeric user window.
pub fn walk_user<M: PhysMem>(mem: &M, root: u64, va: u64, write: bool) -> Result<u64, Fault> {
    if !range_in_user_window(va, 1) {
        return Err(Fault::Unmapped);
    }
    let pte = leaf_entry(mem, root, va).ok_or(Fault::Unmapped)?;
    check_leaf(pte, write)?;
    Ok((pte & PTE_OUT_MASK) | (va & (PAGE - 1)))
}

/// The level-3 descriptor for `va`, if all upper levels are valid tables.
fn leaf_entry<M: PhysMem>(mem: &M, root: u64, va: u64) -> Option<u64> {
    let mut table = root;
    for level in 0..3u32 {
        let entry = mem.entry(table, ((va >> (39 - level * 9)) & 0x1FF) as usize);
        if entry & PTE_TYPE_MASK != PTE_TABLE_OR_PAGE {
            return None;
        }
        table = entry & PTE_OUT_MASK;
    }
    Some(mem.entry(table, ((va >> 12) & 0x1FF) as usize))
}

/// Validate a level-3 descriptor for an EL0 access.
fn check_leaf(pte: u64, write: bool) -> Result<(), Fault> {
    if pte & PTE_TYPE_MASK != PTE_TABLE_OR_PAGE {
        return Err(Fault::Unmapped);
    }
    if pte & PTE_EL0_ACCESS == 0 {
        return Err(Fault::Denied);
    }
    if write && pte & PTE_READ_ONLY != 0 {
        return Err(Fault::Denied);
    }
    Ok(())
}

/// Resolves a copy-on-write fault for the page at the given address in the
/// tree with the given root; true when the page became writable.
type CowResolver<'a> = &'a dyn Fn(u64, u64) -> bool;

/// Bytes from `va` to the end of its page, capped at `remaining`.
fn chunk_len(va: u64, remaining: usize) -> usize {
    ((PAGE - (va & (PAGE - 1))) as usize).min(remaining)
}

/// Walk for a write, resolving a copy-on-write page once if needed.
fn walk_for_write<M: PhysMem>(
    mem: &M,
    root: u64,
    cow: CowResolver<'_>,
    va: u64,
) -> Result<u64, Fault> {
    match walk_user(mem, root, va, true) {
        Err(Fault::Denied) => {
            let is_cow = leaf_entry(mem, root, va).is_some_and(|p| p & PTE_COW_BIT != 0);
            if !is_cow || !cow(root, va & !(PAGE - 1)) {
                return Err(Fault::Denied);
            }
            walk_user(mem, root, va, true)
        }
        other => other,
    }
}

/// Core of every copy: walk page by page, then hand each frame chunk to
/// `each(frame_ptr, offset_in_buffer, chunk_len)`.
fn copy_pages<M: PhysMem>(
    mem: &M,
    root: u64,
    cow: Option<CowResolver<'_>>,
    user: u64,
    len: usize,
    mut each: impl FnMut(*mut u8, usize, usize),
) -> Result<(), Fault> {
    if len != 0 && !range_in_user_window(user, len as u64) {
        return Err(Fault::Unmapped);
    }
    let mut done = 0usize;
    while done < len {
        let va = user + done as u64;
        let pa = match cow {
            Some(resolver) => walk_for_write(mem, root, resolver, va)?,
            None => walk_user(mem, root, va, false)?,
        };
        let n = chunk_len(va, len - done);
        each(mem.byte_ptr(pa), done, n);
        done += n;
    }
    Ok(())
}

/// Copy user memory at `src` (in the tree `root`) into `dst`.
pub fn copy_from_user_with<M: PhysMem>(
    mem: &M,
    root: u64,
    src: UserPtr,
    dst: &mut [u8],
) -> Result<(), Fault> {
    let out = dst.as_mut_ptr();
    copy_pages(mem, root, None, src.0, dst.len(), |frame, off, n| {
        // SAFETY: `frame` points at `n` bytes inside one mapped user frame and
        // `off + n <= dst.len()`; kernel buffer and user frame are disjoint.
        unsafe { core::ptr::copy_nonoverlapping(frame, out.add(off), n) }
    })
}

/// Copy `src` into user memory at `dst` (in the tree `root`).
pub fn copy_to_user_with<M: PhysMem>(
    mem: &M,
    root: u64,
    cow: CowResolver<'_>,
    dst: UserPtr,
    src: &[u8],
) -> Result<(), Fault> {
    let input = src.as_ptr();
    copy_pages(mem, root, Some(cow), dst.0, src.len(), |frame, off, n| {
        // SAFETY: `frame` points at `n` writable bytes inside one mapped user
        // frame and `off + n <= src.len()`; kernel buffer and frame are disjoint.
        unsafe { core::ptr::copy_nonoverlapping(input.add(off), frame, n) }
    })
}

/// Copy a NUL-terminated string into `out`; returns its length (no NUL).
pub fn user_cstr_with<M: PhysMem>(
    mem: &M,
    root: u64,
    src: UserPtr,
    out: &mut [u8],
) -> Result<usize, Fault> {
    let mut len = 0usize;
    while len < out.len() {
        let at = src.offset(len as u64);
        let room = chunk_len(at.0, STR_CHUNK.min(out.len() - len));
        let chunk = &mut out[len..len + room];
        copy_from_user_with(mem, root, at, chunk)?;
        if let Some(nul) = chunk.iter().position(|&b| b == 0) {
            return Ok(len + nul);
        }
        len += room;
    }
    Err(Fault::TooLong)
}

/// Page-table root of the running EL0 task, if it has one.
pub fn current_root() -> Option<u64> {
    let pid = crate::sched::current_pid();
    let idx = crate::sched::find_task_index(pid)?;
    // SAFETY: the entry is read once by value while the task is running.
    let root = unsafe { crate::sched::TASK_TABLE[idx].as_ref().map(|t| t.ttbr0_phys) }?;
    (root != 0).then_some(root)
}

/// Resolve a copy-on-write fault in the real address space.
fn resolve_cow(root: u64, page: u64) -> bool {
    crate::memory::process_vmem::handle_cow_fault(root, page)
}

/// Copy from user memory of the current task into `dst`.
pub fn copy_from_user(src: UserPtr, dst: &mut [u8]) -> Result<(), Fault> {
    let root = current_root().ok_or(Fault::Unmapped)?;
    copy_from_user_with(&DirectMap, root, src, dst)
}

/// Copy `src` to user memory of the current task.
pub fn copy_to_user(dst: UserPtr, src: &[u8]) -> Result<(), Fault> {
    let root = current_root().ok_or(Fault::Unmapped)?;
    copy_to_user_with(&DirectMap, root, &resolve_cow, dst, src)
}

/// Copy `src` into the user memory of the task whose root is `root`
/// (loader, argv and relocation writes before that task runs).
pub fn copy_to_user_in(root: u64, dst: UserPtr, src: &[u8]) -> Result<(), Fault> {
    copy_to_user_with(&DirectMap, root, &resolve_cow, dst, src)
}

/// Read a plain-old-data value from the current task's memory.
pub fn read_user<T: Copy>(src: UserPtr) -> Result<T, Fault> {
    let mut value = core::mem::MaybeUninit::<T>::uninit();
    // SAFETY: the slice covers exactly the storage of `value`.
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr() as *mut u8, core::mem::size_of::<T>())
    };
    copy_from_user(src, bytes)?;
    // SAFETY: every byte was written by the successful copy above.
    Ok(unsafe { value.assume_init() })
}

/// Write a plain-old-data value to the current task's memory.
pub fn write_user<T: Copy>(dst: UserPtr, value: &T) -> Result<(), Fault> {
    // SAFETY: reads the bytes of a live `Copy` value.
    let bytes = unsafe {
        core::slice::from_raw_parts(value as *const T as *const u8, core::mem::size_of::<T>())
    };
    copy_to_user(dst, bytes)
}

/// Read a NUL-terminated UTF-8 string of at most `max` bytes.
pub fn user_string(src: UserPtr, max: usize) -> Result<alloc::string::String, Fault> {
    let root = current_root().ok_or(Fault::Unmapped)?;
    let mut buf = alloc::vec![0u8; max];
    let len = user_cstr_with(&DirectMap, root, src, &mut buf)?;
    buf.truncate(len);
    alloc::string::String::from_utf8(buf).map_err(|_| Fault::Denied)
}

/// Read exactly `len` bytes (a path or buffer given as pointer + length).
pub fn user_bytes(src: UserPtr, len: usize) -> Result<alloc::vec::Vec<u8>, Fault> {
    let mut buf = alloc::vec![0u8; len];
    copy_from_user(src, &mut buf)?;
    Ok(buf)
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
    use core::cell::UnsafeCell;
    use std::collections::HashMap;

    /// Host model of translation tables and frames.
    #[derive(Default)]
    struct Model {
        tables: HashMap<u64, Vec<u64>>,
        frames: HashMap<u64, Box<UnsafeCell<[u8; 4096]>>>,
        next: u64,
    }

    impl PhysMem for Model {
        fn entry(&self, table: u64, idx: usize) -> u64 {
            self.tables.get(&table).map_or(0, |t| t[idx])
        }
        fn byte_ptr(&self, pa: u64) -> *mut u8 {
            let base = pa & !(PAGE - 1);
            let f = self.frames.get(&base).expect("frame");
            // SAFETY: offset stays inside the 4 KiB frame.
            unsafe { (f.get() as *mut u8).add((pa - base) as usize) }
        }
    }

    const ROOT: u64 = 0x1000;
    const USER_RW: u64 = 0b11 | (1 << 6) | (1 << 10);
    const USER_RO: u64 = USER_RW | (1 << 7);
    const KERNEL_ONLY: u64 = 0b11 | (1 << 10);

    impl Model {
        fn new() -> Self {
            let mut m = Model {
                next: 0x10_0000,
                ..Default::default()
            };
            m.tables.insert(ROOT, vec![0; 512]);
            m
        }
        fn alloc_table(&mut self) -> u64 {
            self.next += PAGE;
            self.tables.insert(self.next, vec![0; 512]);
            self.next
        }
        fn map(&mut self, va: u64, flags: u64) -> u64 {
            let mut table = ROOT;
            for level in 0..3u32 {
                let idx = ((va >> (39 - level * 9)) & 0x1FF) as usize;
                let cur = self.tables[&table][idx];
                let next = if cur & 1 == 0 {
                    let t = self.alloc_table();
                    self.tables.get_mut(&table).unwrap()[idx] = t | 0b11;
                    t
                } else {
                    cur & PTE_OUT_MASK
                };
                table = next;
            }
            self.next += PAGE;
            let pa = self.next;
            self.frames.insert(pa, Box::new(UnsafeCell::new([0; 4096])));
            self.tables.get_mut(&table).unwrap()[((va >> 12) & 0x1FF) as usize] = pa | flags;
            pa
        }
    }

    const BASE: u64 = USER_CODE_START;

    #[test]
    fn roundtrip_across_a_page_boundary() {
        let mut m = Model::new();
        m.map(BASE, USER_RW);
        m.map(BASE + PAGE, USER_RW);
        let data: Vec<u8> = (0..100u8).collect();
        let at = UserPtr(BASE + PAGE - 40);
        copy_to_user_with(&m, ROOT, &|_, _| false, at, &data).unwrap();
        let mut back = vec![0u8; 100];
        copy_from_user_with(&m, ROOT, at, &mut back).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn unmapped_page_faults_and_crossing_into_it_faults() {
        let mut m = Model::new();
        m.map(BASE, USER_RW);
        let mut b = [0u8; 8];
        assert_eq!(
            copy_from_user_with(&m, ROOT, UserPtr(0x5000_0000), &mut b),
            Err(Fault::Unmapped)
        );
        let mut long = vec![0u8; 64];
        assert_eq!(
            copy_from_user_with(&m, ROOT, UserPtr(BASE + PAGE - 8), &mut long),
            Err(Fault::Unmapped)
        );
    }

    #[test]
    fn kernel_only_and_out_of_window_pointers_fault() {
        let mut m = Model::new();
        m.map(BASE, KERNEL_ONLY);
        let mut b = [0u8; 8];
        assert_eq!(
            copy_from_user_with(&m, ROOT, UserPtr(BASE), &mut b),
            Err(Fault::Denied)
        );
        assert_eq!(
            copy_from_user_with(&m, ROOT, UserPtr(0x8_0000), &mut b),
            Err(Fault::Unmapped)
        );
        assert_eq!(
            copy_from_user_with(&m, ROOT, UserPtr(0), &mut b),
            Err(Fault::Unmapped)
        );
    }

    #[test]
    fn read_only_page_refuses_writes_but_allows_reads() {
        let mut m = Model::new();
        m.map(BASE, USER_RO);
        let nocow = |_: u64, _: u64| false;
        assert_eq!(
            copy_to_user_with(&m, ROOT, &nocow, UserPtr(BASE), &[1, 2]),
            Err(Fault::Denied)
        );
        let mut b = [9u8; 2];
        assert!(copy_from_user_with(&m, ROOT, UserPtr(BASE), &mut b).is_ok());
    }

    #[test]
    fn cow_resolver_runs_only_for_cow_pages_with_the_page_address() {
        let mut m = Model::new();
        m.map(BASE, USER_RO | PTE_COW_BIT);
        m.map(BASE + PAGE, USER_RO);
        let calls = core::cell::Cell::new(0u32);
        let spy = |_root: u64, page: u64| -> bool {
            assert_eq!(page, BASE);
            calls.set(calls.get() + 1);
            false
        };
        let at = UserPtr(BASE + 12);
        assert_eq!(
            copy_to_user_with(&m, ROOT, &spy, at, &[1]),
            Err(Fault::Denied)
        );
        assert_eq!(calls.get(), 1);
        let plain = UserPtr(BASE + PAGE);
        assert_eq!(
            copy_to_user_with(&m, ROOT, &spy, plain, &[1]),
            Err(Fault::Denied)
        );
        assert_eq!(
            calls.get(),
            1,
            "plain read-only page must not reach the resolver"
        );
    }

    #[test]
    fn string_stops_at_nul_and_rejects_unterminated() {
        let mut m = Model::new();
        let pa = m.map(BASE, USER_RW);
        // SAFETY: test frame, in bounds.
        unsafe { core::ptr::copy_nonoverlapping(b"hi\0".as_ptr(), m.byte_ptr(pa), 3) };
        let mut out = [0u8; 16];
        assert_eq!(user_cstr_with(&m, ROOT, UserPtr(BASE), &mut out), Ok(2));
        let mut tiny = [0u8; 2];
        assert_eq!(
            user_cstr_with(&m, ROOT, UserPtr(BASE), &mut tiny),
            Err(Fault::TooLong)
        );
    }

    #[test]
    fn zero_length_copy_needs_no_mapping() {
        let m = Model::new();
        assert!(copy_from_user_with(&m, ROOT, UserPtr(0x5000_0000), &mut []).is_ok());
    }
}
