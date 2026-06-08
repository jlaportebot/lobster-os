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
unsafe fn expand_heap(min_size: usize) -> bool {
    let pages_needed = (min_size + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;

    for _ in 0..pages_needed {
        if HEAP_CURRENT_TOP >= HEAP_VIRT_END {
            println!("[heap] Out of heap virtual address space!");
            return false;
        }

        let frame = match frame_alloc::allocate_frame() {
            Some(f) => f,
            None => {
                println!("[heap] Out of physical frames for heap expansion!");
                return false;
            }
        };

        crate::memory::vmem::map_kernel_page(
            HEAP_CURRENT_TOP as u64,
            frame.0,
            crate::memory::vmem::PTE_KERNEL_DATA,
        );

        // Create a new free block at the expanded region
        let new_block = HEAP_CURRENT_TOP as *mut HeapHeader;
        (*new_block) = HeapHeader {
            size: PAGE_SIZE as usize,
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

        HEAP_CURRENT_TOP += PAGE_SIZE as usize;
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
            // Check alignment
            let data_start = current as usize + HEADER_SIZE;
            let aligned = (data_start + align - 1) & !(align - 1);
            let padding = aligned - data_start;
            let total_needed = size + padding;

            if (*current).size >= total_needed + HEADER_SIZE {
                // Split the block
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
            } else if (*current).size >= size {
                // Use the whole block (even with some waste)
                (*current).is_free = false;
                let data_start = current as usize + HEADER_SIZE;
                let aligned = (data_start + align - 1) & !(align - 1);
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

    // Find the block header (it's right before the data)
    let header = (ptr as usize - HEADER_SIZE) as *mut HeapHeader;

    // Validate this is actually a heap pointer
    let addr = header as usize;
    if addr < HEAP_VIRT_START || addr >= HEAP_CURRENT_TOP {
        return;
    }

    (*header).is_free = true;

    // Coalesce adjacent free blocks
    let mut current = HEAP_HEAD;
    while !current.is_null() {
        if (*current).is_free {
            let next = (*current).next;
            while !next.is_null() && (*next).is_free {
                // Check adjacency
                if current as usize + (*current).size == next as usize {
                    (*current).size += (*next).size;
                    (*current).next = (*next).next;
                } else {
                    break;
                }
                let next = (*current).next;
                if next.is_null() {
                    break;
                }
                // Safe: we just set current.next above
                break;
            }
        }
        current = (*current).next;
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

/// Register the global allocator
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
