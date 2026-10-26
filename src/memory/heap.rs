//! Kernel heap allocator
//!
//! Provides a `#[global_allocator]` implementation using a simple
//! linked-list (first-fit) allocator backed by physical frames.
//!
//! The heap region is: HEAP_START .. HEAP_START + HEAP_SIZE
//! We map frames on demand as allocations grow.

use crate::memory::{frame_alloc, HEAP_SIZE, HEAP_START, PAGE_SIZE};
use crate::println;

use core::alloc::{GlobalAlloc, Layout};

/// Heap region start (virtual)
const HEAP_VIRT_START: usize = HEAP_START as usize;
const HEAP_VIRT_END: usize = (HEAP_START + HEAP_SIZE) as usize;

/// Linked-list heap block header
#[repr(C)]
struct HeapHeader {
    /// Size of this block (including header), in bytes
    size: usize,
    /// Whether this block is free
    is_free: bool,
    /// Pointer to next block in the list
    next: *mut HeapHeader,
}

const HEADER_SIZE: usize = core::mem::size_of::<HeapHeader>();

/// The heap state
static mut HEAP_INITIALIZED: bool = false;
static mut HEAP_HEAD: *mut HeapHeader = core::ptr::null_mut();
static mut HEAP_CURRENT_TOP: usize = 0; // Current top of mapped heap

/// Initialize the kernel heap
pub unsafe fn init() {
    if HEAP_INITIALIZED {
        return;
    }

    println!(
        "[heap] Initializing kernel heap at {:#018x} ({} MB)",
        HEAP_START,
        HEAP_SIZE / (1024 * 1024)
    );

    // Map the first page of the heap
    let first_frame = frame_alloc::allocate_frame().expect("[heap] No frames available for heap!");
    crate::memory::vmem::map_kernel_page(
        HEAP_START,
        first_frame.0,
        crate::memory::vmem::PTE_KERNEL_DATA,
    );

    HEAP_CURRENT_TOP = HEAP_VIRT_START + PAGE_SIZE as usize;

    // Set up the first block as one big free block
    let head = HEAP_VIRT_START as *mut HeapHeader;
    (*head) = HeapHeader {
        size: PAGE_SIZE as usize,
        is_free: true,
        next: core::ptr::null_mut(),
    };
    HEAP_HEAD = head;
    HEAP_INITIALIZED = true;

    println!(
        "[heap] Heap initialized ({} KB first page)",
        PAGE_SIZE / 1024
    );
}

/// Expand the heap by mapping additional frames
/// Optimized to allocate contiguous chunks for large allocations
unsafe fn expand_heap(min_size: usize) -> bool {
    let pages_needed = (min_size + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;

    // For large allocations, try to allocate contiguous frames
    // This avoids the overhead of mapping pages one by one
    let chunk_size = pages_needed.min(64); // Allocate up to 64 pages (256KB) at a time
    let mut pages_done = 0;

    while pages_done < pages_needed {
        if HEAP_CURRENT_TOP >= HEAP_VIRT_END {
            println!("[heap] Out of heap virtual address space!");
            return false;
        }

        let remaining = pages_needed - pages_done;
        let this_chunk = chunk_size.min(remaining);

        // Try to allocate multiple contiguous frames
        let frames_allocated = if this_chunk > 1 {
            frame_alloc::allocate_contiguous_frames(this_chunk)
        } else {
            frame_alloc::allocate_frame().map(|f| alloc::vec::Vec::from([f]))
        };

        let frames = match frames_allocated {
            Some(f) => f,
            None => {
                println!("[heap] Out of physical frames for heap expansion!");
                return false;
            }
        };

        // Map all frames contiguously
        for (i, frame) in frames.iter().enumerate() {
            crate::memory::vmem::map_kernel_page(
                HEAP_CURRENT_TOP as u64 + (i * PAGE_SIZE as usize) as u64,
                frame.0,
                crate::memory::vmem::PTE_KERNEL_DATA,
            );
        }

        // Create a single large free block for the contiguous region
        let total_size = frames.len() * PAGE_SIZE as usize;
        let new_block = HEAP_CURRENT_TOP as *mut HeapHeader;
        (*new_block) = HeapHeader {
            size: total_size,
            is_free: true,
            next: core::ptr::null_mut(),
        };

        // Append to the free list
        let mut current = HEAP_HEAD;
        while !(*current).next.is_null() {
            current = (*current).next;
        }
        (*current).next = new_block;

        // Try to coalesce with the previous block if it was free and adjacent
        if current as usize + (*current).size == new_block as usize && (*current).is_free {
            (*current).size += (*new_block).size;
            (*current).next = (*new_block).next;
        }

        HEAP_CURRENT_TOP += total_size;
        pages_done += frames.len();
    }

    true
}

/// First-fit allocation from the linked list
pub unsafe fn heap_alloc(layout: Layout) -> *mut u8 {
    let size = layout.size().max(HEADER_SIZE);
    let align = layout.align().max(8);

    // Walk the free list looking for a fit
    let mut current = HEAP_HEAD;

    while !current.is_null() {
        if (*current).is_free {
            // data_start = header + HEADER_SIZE, then align up
            let data_start = current as usize + HEADER_SIZE;
            let aligned = (data_start + align - 1) & !(align - 1);
            let padding = aligned - data_start;
            // total_needed = HEADER + padding + data — must include the header
            // so the split block starts AFTER the data, not overlapping it
            let total_needed = HEADER_SIZE + padding + size;

            if (*current).size >= total_needed + HEADER_SIZE {
                // Split the block: new free block starts right after our data
                let remaining = (*current).size - total_needed;
                if remaining > HEADER_SIZE {
                    let next_block = (current as usize + total_needed) as *mut HeapHeader;
                    (*next_block) = HeapHeader {
                        size: remaining,
                        is_free: true,
                        next: (*current).next,
                    };
                    (*current).size = total_needed;
                    (*current).next = next_block;
                }

                (*current).is_free = false;
                return aligned as *mut u8;
            } else if (*current).size >= total_needed {
                // Use the whole block (no split, padding absorbed)
                (*current).is_free = false;
                return aligned as *mut u8;
            }
        }

        current = (*current).next;
    }

    // No suitable block found — expand the heap
    if expand_heap(size + HEADER_SIZE * 2) {
        // Retry allocation after expansion
        return heap_alloc(layout);
    }

    core::ptr::null_mut()
}

/// Free a heap allocation
pub unsafe fn heap_free(ptr: *mut u8, _layout: Layout) {
    if ptr.is_null() {
        return;
    }

    // Find the block header by scanning the free list (not by assuming
    // ptr - HEADER_SIZE, because alignment padding may offset the data).
    // We search for the block whose data range contains `ptr`.
    let mut header_ptr: *mut HeapHeader = core::ptr::null_mut();
    let mut current = HEAP_HEAD;
    while !current.is_null() {
        let data_start = current as usize + HEADER_SIZE;
        let data_end = current as usize + (*current).size;
        if (ptr as usize) >= data_start && (ptr as usize) < data_end {
            header_ptr = current;
            break;
        }
        current = (*current).next;
    }

    if header_ptr.is_null() {
        return;
    }

    (*header_ptr).is_free = true;

    // Coalesce adjacent free blocks — walk the list and merge each
    // free block with its free, immediately-adjacent successor.
    let mut iter = HEAP_HEAD;
    while !iter.is_null() {
        if (*iter).is_free {
            // Re-fetch successor after each merge so we keep merging
            // a run of free blocks in one pass.
            while !(*iter).next.is_null() {
                let next = (*iter).next;
                if (*next).is_free && iter as usize + (*iter).size == next as usize {
                    (*iter).size += (*next).size;
                    (*iter).next = (*next).next;
                } else {
                    break;
                }
            }
        }
        iter = (*iter).next;
    }
}

/// The global allocator struct
pub struct LobsterAllocator;

unsafe impl GlobalAlloc for LobsterAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        heap_alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        heap_free(ptr, layout);
    }
}

/// Register the global allocator (bare-metal builds only — host `std` test
/// builds use the system allocator, since the custom heap needs the MMU
/// page tables that only exist on target).
#[cfg(not(feature = "std"))]
#[global_allocator]
static ALLOCATOR: LobsterAllocator = LobsterAllocator;

/// Print heap statistics
pub fn print_stats() {
    unsafe {
        let mut total_blocks = 0usize;
        let mut free_blocks = 0usize;
        let mut used_blocks = 0usize;
        let mut free_bytes = 0usize;
        let mut used_bytes = 0usize;

        let mut current = HEAP_HEAD;
        while !current.is_null() {
            total_blocks += 1;
            if (*current).is_free {
                free_blocks += 1;
                free_bytes += (*current).size;
            } else {
                used_blocks += 1;
                used_bytes += (*current).size;
            }
            current = (*current).next;
        }

        let mapped_kb = (HEAP_CURRENT_TOP - HEAP_VIRT_START) / 1024;
        println!(
            "[heap] Blocks: {} total, {} free, {} used",
            total_blocks, free_blocks, used_blocks
        );
        println!(
            "[heap] Memory: {} KB free, {} KB used, {} KB mapped",
            free_bytes / 1024,
            used_bytes / 1024,
            mapped_kb
        );
    }
}
