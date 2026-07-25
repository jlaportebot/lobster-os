//! Physical frame allocator
//!
//! Manages allocation and deallocation of 4KB physical frames.
//! Uses a simple bitmap allocator for early boot, will be upgraded
//! to a buddy allocator in Phase 10.

use crate::memory::{PhysAddr, PAGE_SIZE};
use crate::println;

/// Maximum RAM we support (16 GB for RPi 5 16GB model)
const MAX_RAM: u64 = 16 * 1024 * 1024 * 1024;
const MAX_FRAMES: usize = (MAX_RAM / PAGE_SIZE) as usize; // 4M frames

/// Bitmap: 1 bit per frame, 1 = allocated, 0 = free
/// 2M bits = 256KB of bitmap storage
static mut FRAME_BITMAP: [u64; (MAX_FRAMES + 63) / 64] = [0u64; (MAX_FRAMES + 63) / 64];

/// Total number of frames available
static mut TOTAL_FRAMES: usize = 0;

/// Number of frames currently allocated
static mut ALLOCATED_FRAMES: usize = 0;

/// First frame that's available for allocation (after kernel)
static mut FIRST_FREE_FRAME: usize = 0;

/// Spinlock for frame allocator (using LRXR/STXR for lock-free would be better,
/// but a simple irq-disable lock works for now)
static mut FRAME_LOCK: bool = false;

fn lock() {
    unsafe {
        crate::arch::cpu::disable_irqs();
        while core::sync::atomic::AtomicBool::compare_exchange(
            &core::ptr::read_volatile(
                &FRAME_LOCK as *const bool as *const core::sync::atomic::AtomicBool,
            ),
            false,
            true,
            core::sync::atomic::Ordering::Acquire,
            core::sync::atomic::Ordering::Relaxed,
        )
        .is_err()
        {
            core::hint::spin_loop();
        }
    }
}

fn unlock() {
    unsafe {
        FRAME_LOCK = false;
        crate::arch::cpu::enable_irqs();
    }
}

/// A physical frame handle — automatically deallocates on Drop
#[derive(Debug)]
pub struct Frame {
    pub paddr: PhysAddr,
    pub order: usize, // 0 = 4KB, 1 = 8KB, etc. (for buddy future)
}

impl Frame {
    /// Create a frame from a physical address
    pub const fn new(paddr: PhysAddr) -> Self {
        Self { paddr, order: 0 }
    }

    /// Get the kernel virtual address for this frame
    pub fn kernel_vaddr(&self) -> u64 {
        crate::memory::phys_to_virt(self.paddr).0
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        deallocate_frame(self.paddr);
    }
}

/// Initialize the frame allocator
/// `ram_size`: total physical RAM in bytes
/// `kernel_end_phys`: first free physical address after the kernel
pub unsafe fn init(ram_size: u64, kernel_end_phys: u64) {
    TOTAL_FRAMES = (ram_size / PAGE_SIZE as u64) as usize;
    FIRST_FREE_FRAME = ((kernel_end_phys + PAGE_SIZE - 1) / PAGE_SIZE as u64) as usize;

    // Mark all frames as allocated initially
    for i in 0..(TOTAL_FRAMES + 63) / 64 {
        FRAME_BITMAP[i] = !0u64; // All 1s = all allocated
    }

    // Mark free frames (after kernel) as available
    for frame in FIRST_FREE_FRAME..TOTAL_FRAMES {
        let word = frame / 64;
        let bit = frame % 64;
        FRAME_BITMAP[word] &= !(1u64 << bit);
    }

    ALLOCATED_FRAMES = FIRST_FREE_FRAME;

    let free_frames = TOTAL_FRAMES - ALLOCATED_FRAMES;
    let free_kb = free_frames as u64 * PAGE_SIZE / 1024;
    println!(
        "[frame-alloc] {} total frames ({} MB RAM)",
        TOTAL_FRAMES,
        ram_size / (1024 * 1024)
    );
    println!(
        "[frame-alloc] Kernel uses {} frames ({} KB)",
        ALLOCATED_FRAMES,
        ALLOCATED_FRAMES as u64 * PAGE_SIZE / 1024
    );
    println!(
        "[frame-alloc] {} free frames ({} KB / {} MB available)",
        free_frames,
        free_kb,
        free_kb / 1024
    );
}

/// Allocate a single 4KB physical frame
pub fn allocate_frame() -> Option<PhysAddr> {
    lock();
    let result = unsafe { allocate_frame_inner() };
    unlock();

    // If allocation failed, try OOM handler
    if result.is_none() {
        // Try soft OOM first (free caches)
        if crate::memory::oom::handle(crate::memory::oom::OomSeverity::Soft, 1) {
            // Retry allocation after cache cleanup
            lock();
            let retry = unsafe { allocate_frame_inner() };
            unlock();
            if retry.is_some() {
                return retry;
            }
        }

        // Hard OOM — kill a process and retry
        if crate::memory::oom::handle(crate::memory::oom::OomSeverity::Hard, 1) {
            lock();
            let retry = unsafe { allocate_frame_inner() };
            unlock();
            return retry;
        }
    }

    result
}

unsafe fn allocate_frame_inner() -> Option<PhysAddr> {
    // Linear scan from FIRST_FREE_FRAME (fast enough for early boot)
    for word_idx in (FIRST_FREE_FRAME / 64)..(TOTAL_FRAMES + 63) / 64 {
        let word = FRAME_BITMAP[word_idx];
        if word != !0u64 {
            // Find first zero bit (free frame)
            let bit = word.trailing_ones() as usize;
            if bit < 64 {
                let frame = word_idx * 64 + bit;
                if frame < TOTAL_FRAMES {
                    FRAME_BITMAP[word_idx] |= 1u64 << bit;
                    ALLOCATED_FRAMES += 1;
                    return Some(PhysAddr::new((frame as u64) * PAGE_SIZE));
                }
            }
        }
    }
    None
}

/// Allocate contiguous frames (for larger allocations)
pub fn allocate_contiguous(count: usize) -> Option<PhysAddr> {
    lock();
    let result = unsafe { allocate_contiguous_inner(count) };
    unlock();
    result
}

unsafe fn allocate_contiguous_inner(count: usize) -> Option<PhysAddr> {
    if count == 0 {
        return None;
    }
    if count == 1 {
        return allocate_frame_inner();
    }

    let mut run_start: usize = 0;
    let mut run_length: usize = 0;

    for frame in FIRST_FREE_FRAME..TOTAL_FRAMES {
        let word = frame / 64;
        let bit = frame % 64;
        let is_free = (FRAME_BITMAP[word] >> bit) & 1 == 0;

        if is_free {
            if run_length == 0 {
                run_start = frame;
            }
            run_length += 1;
            if run_length == count {
                // Mark all frames as allocated
                for f in run_start..(run_start + count) {
                    let w = f / 64;
                    let b = f % 64;
                    FRAME_BITMAP[w] |= 1u64 << b;
                }
                ALLOCATED_FRAMES += count;
                return Some(PhysAddr::new((run_start as u64) * PAGE_SIZE));
            }
        } else {
            run_length = 0;
        }
    }
    None
}

/// Deallocate a physical frame
pub fn deallocate_frame(paddr: PhysAddr) {
    lock();
    unsafe { deallocate_frame_inner(paddr) };
    unlock();
}

unsafe fn deallocate_frame_inner(paddr: PhysAddr) {
    let frame = (paddr.0 / PAGE_SIZE as u64) as usize;
    if frame < FIRST_FREE_FRAME || frame >= TOTAL_FRAMES {
        return; // Can't free kernel or out-of-range frames
    }
    let word = frame / 64;
    let bit = frame % 64;
    if (FRAME_BITMAP[word] >> bit) & 1 == 0 {
        return; // Already free — double-free guard
    }
    FRAME_BITMAP[word] &= !(1u64 << bit);
    ALLOCATED_FRAMES -= 1;
}

/// Get frame allocator statistics
pub fn stats() -> (usize, usize, usize) {
    lock();
    let (total, allocated, first_free) =
        unsafe { (TOTAL_FRAMES, ALLOCATED_FRAMES, FIRST_FREE_FRAME) };
    unlock();
    (total, allocated, first_free)
}
