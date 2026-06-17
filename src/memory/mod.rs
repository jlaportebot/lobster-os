//! Memory management module

pub mod frame_alloc;
pub mod heap;
pub mod mmu;
pub mod process_vmem;
pub mod vmem;

/// Physical address
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(pub u64);

/// Virtual address
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtAddr(pub u64);

impl PhysAddr {
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
    pub const fn as_usize(&self) -> usize {
        self.0 as usize
    }
    pub const fn align_up(&self, align: u64) -> Self {
        Self((self.0 + align - 1) & !(align - 1))
    }
    pub const fn align_down(&self, align: u64) -> Self {
        Self(self.0 & !(align - 1))
    }
    pub const fn is_aligned(&self, align: u64) -> bool {
        self.0 & (align - 1) == 0
    }
}

impl VirtAddr {
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
    pub const fn as_usize(&self) -> usize {
        self.0 as usize
    }
    pub const fn as_ptr<T>(&self) -> *const T {
        self.0 as *const T
    }
    pub const fn as_mut_ptr<T>(&self) -> *mut T {
        self.0 as *mut T
    }
    pub const fn align_up(&self, align: u64) -> Self {
        Self((self.0 + align - 1) & !(align - 1))
    }
    pub const fn align_down(&self, align: u64) -> Self {
        Self(self.0 & !(align - 1))
    }
}

/// Page size (4KB)
pub const PAGE_SIZE: u64 = 4096;
pub const PAGE_SHIFT: u64 = 12;

/// Kernel address space layout
/// BCM2712 has up to 8GB RAM mapped starting at physical 0x0.
/// We use a direct physical map at a high virtual address.
/// Start of kernel virtual address space (TTBR1, upper half)
pub const KERNEL_VIRT_START: u64 = 0xFFFF_0000_0000_0000;

/// Direct physical memory map offset — maps phys 0x0 → this VA
pub const PHYS_MAP_OFFSET: u64 = 0xFFFF_8000_0000_0000;

/// Convert physical address to direct-mapped virtual address
#[inline]
pub const fn phys_to_virt(paddr: PhysAddr) -> VirtAddr {
    VirtAddr::new(paddr.0 + PHYS_MAP_OFFSET)
}

/// Convert direct-mapped virtual address back to physical
#[inline]
pub const fn virt_to_phys(vaddr: VirtAddr) -> PhysAddr {
    PhysAddr::new(vaddr.0 - PHYS_MAP_OFFSET)
}

/// Kernel image virtual addresses (identity-mapped at first, then switched)
pub const KERNEL_IMAGE_START: u64 = 0xFFFF_8000_0008_0000;

/// Heap area: 256MB starting after kernel image area
pub const HEAP_START: u64 = 0xFFFF_9000_0000_0000;
pub const HEAP_SIZE: u64 = 256 * 1024 * 1024; // 256 MB

/// Device memory (MMIO) is mapped at a fixed high address
pub const DEVICE_MAP_START: u64 = 0xFFFF_C000_0000_0000;
