//! Per-process translation roots (ADR 0002).
//!
//! A process root embeds the kernel's low-half descriptors (EL1-only blocks)
//! copied from the boot root, plus the process's own user tables. The kernel
//! therefore keeps running when a process root is loaded in TTBR0, and EL0 can
//! only reach the pages the kernel mapped for it.

use super::asid::{AsidTable, KERNEL_ASID};
use crate::sync::spinlock::Spinlock;
use core::sync::atomic::{AtomicBool, Ordering};

/// Entries in one translation table.
const ENTRIES: usize = 512;
/// Descriptor type bits.
const TYPE_MASK: u64 = 0b11;
/// A valid table descriptor.
const TABLE: u64 = 0b11;
/// A valid block descriptor.
const BLOCK: u64 = 0b01;
/// Output-address mask of a descriptor.
const OUT_MASK: u64 = 0x0000_FFFF_FFFF_F000;
/// L1 slots (1 GiB each) that hold user mappings: VA 0..2 GiB.
const USER_L1_SLOTS: usize = 2;
/// Bit position of the ASID inside TTBR0_EL1.
const ASID_SHIFT: u64 = 48;

/// ASID bindings of every live process root.
static ASIDS: Spinlock<AsidTable> = Spinlock::new(AsidTable::new());

/// Set once the first process root exists: the boot root's low half is then
/// frozen and `mmu` asserts nothing maps into it any more.
static LOW_HALF_FROZEN: AtomicBool = AtomicBool::new(false);

/// True after the first process root was built.
pub fn low_half_frozen() -> bool {
    LOW_HALF_FROZEN.load(Ordering::Relaxed)
}

/// Table memory the builder works on: the direct map in the kernel, a model
/// on the host.
pub trait TableMem {
    /// Entry `idx` of the table at physical address `table`.
    fn entry(&self, table: u64, idx: usize) -> u64;
    /// Store entry `idx` of the table at `table`.
    fn set_entry(&mut self, table: u64, idx: usize, value: u64);
    /// A zeroed table page, or None when out of memory.
    fn alloc_table(&mut self) -> Option<u64>;
    /// Return a table page.
    fn free_table(&mut self, table: u64);
}

/// The next-level table named by a table descriptor.
fn child(mem: &impl TableMem, table: u64, idx: usize) -> Option<u64> {
    let e = mem.entry(table, idx);
    (e & TYPE_MASK == TABLE).then_some(e & OUT_MASK)
}

/// Copy every entry of `src` into `dst`.
fn copy_table(mem: &mut impl TableMem, src: u64, dst: u64) {
    for i in 0..ENTRIES {
        let e = mem.entry(src, i);
        mem.set_entry(dst, i, e);
    }
}

/// Copy only the block descriptors (the kernel's EL1-only mappings) of `src`.
fn copy_blocks(mem: &mut impl TableMem, src: u64, dst: u64) {
    for i in 0..ENTRIES {
        let e = mem.entry(src, i);
        if e & TYPE_MASK == BLOCK {
            mem.set_entry(dst, i, e);
        }
    }
}

/// Build a process root from the boot root: L0 and L1 are copies, the two
/// user-facing L2 tables hold copies of the kernel's block descriptors only.
pub fn build_root<M: TableMem>(mem: &mut M, boot_root: u64) -> Option<u64> {
    let boot_l1 = child(mem, boot_root, 0)?;
    let mut owned = [0u64; 2 + USER_L1_SLOTS];
    let mut count = 0usize;
    let mut take = |mem: &mut M| -> Option<u64> {
        let t = mem.alloc_table()?;
        owned[count] = t;
        count += 1;
        Some(t)
    };
    let built = (|| {
        let l0 = take(mem)?;
        let l1 = take(mem)?;
        copy_table(mem, boot_root, l0);
        copy_table(mem, boot_l1, l1);
        for slot in 0..USER_L1_SLOTS {
            let l2 = take(mem)?;
            if let Some(boot_l2) = child(mem, boot_l1, slot) {
                copy_blocks(mem, boot_l2, l2);
            }
            mem.set_entry(l1, slot, l2 | TABLE);
        }
        mem.set_entry(l0, 0, l1 | TABLE);
        Some(l0)
    })();
    if built.is_none() {
        for &t in &owned[..count] {
            mem.free_table(t);
        }
    }
    built
}

/// Free a process root built by [`build_root`]: every user L3 table, the four
/// private tables, and (through `free_leaf`) every mapped user frame.
/// Shared kernel tables and copied block descriptors are never touched.
pub fn free_root<M: TableMem>(mem: &mut M, root: u64, free_leaf: &mut dyn FnMut(u64)) {
    let Some(l1) = child(mem, root, 0) else {
        mem.free_table(root);
        return;
    };
    for slot in 0..USER_L1_SLOTS {
        if let Some(l2) = child(mem, l1, slot) {
            free_l2(mem, l2, free_leaf);
        }
    }
    mem.free_table(l1);
    mem.free_table(root);
}

/// Free the user L3 tables hanging off one private L2, then the L2 itself.
fn free_l2<M: TableMem>(mem: &mut M, l2: u64, free_leaf: &mut dyn FnMut(u64)) {
    for i in 0..ENTRIES {
        let Some(l3) = child(mem, l2, i) else {
            continue;
        };
        for j in 0..ENTRIES {
            let pte = mem.entry(l3, j);
            if pte & 1 != 0 {
                free_leaf(pte);
            }
        }
        mem.free_table(l3);
    }
    mem.free_table(l2);
}

/// True when every kernel block descriptor of the boot tables appears
/// unchanged in `root` (amendment A: identical descriptors everywhere).
pub fn kernel_blocks_match<M: TableMem>(mem: &M, boot_root: u64, root: u64) -> bool {
    let (Some(boot_l1), Some(l1)) = (child(mem, boot_root, 0), child(mem, root, 0)) else {
        return false;
    };
    (0..USER_L1_SLOTS).all(|slot| {
        let (Some(b), Some(p)) = (child(mem, boot_l1, slot), child(mem, l1, slot)) else {
            return true;
        };
        (0..ENTRIES)
            .filter(|&i| mem.entry(b, i) & TYPE_MASK == BLOCK)
            .all(|i| mem.entry(b, i) == mem.entry(p, i))
    })
}

/// [`TableMem`] over the kernel's direct map and frame allocator.
pub struct KernelTables;

impl TableMem for KernelTables {
    fn entry(&self, table: u64, idx: usize) -> u64 {
        // SAFETY: `table` is a live table page; `idx < 512` stays inside it.
        unsafe { core::ptr::read_volatile(table_ptr(table).add(idx)) }
    }

    fn set_entry(&mut self, table: u64, idx: usize, value: u64) {
        // SAFETY: as above; the kernel owns the page it writes.
        unsafe { core::ptr::write_volatile(table_ptr(table).add(idx), value) }
    }

    fn alloc_table(&mut self) -> Option<u64> {
        let frame = super::frame_alloc::allocate_frame()?;
        for i in 0..ENTRIES {
            self.set_entry(frame.0, i, 0);
        }
        Some(frame.0)
    }

    fn free_table(&mut self, table: u64) {
        super::frame_alloc::deallocate_frame(super::PhysAddr::new(table));
    }
}

/// Direct-map pointer to the table at physical address `table`.
fn table_ptr(table: u64) -> *mut u64 {
    (super::PHYS_MAP_OFFSET + table) as *mut u64
}

/// Allocate a process root and publish the table writes.
pub fn new_user_root() -> Option<u64> {
    let boot = super::mmu::boot_root_phys();
    let root = build_root(&mut KernelTables, boot)?;
    LOW_HALF_FROZEN.store(true, Ordering::Relaxed);
    crate::arch::cpu::dsb();
    Some(root)
}

/// Free a process root and every user frame it maps.
pub fn free_user_root(root: u64) {
    if loaded_root() == root {
        activate_kernel();
    }
    let released = ASIDS.lock().release(root);
    if let Some(asid) = released {
        flush_asid(asid);
    }
    free_root(&mut KernelTables, root, &mut |pte| {
        super::process_vmem::release_leaf(pte)
    });
}

/// Invalidate every cached translation tagged with `asid`.
fn flush_asid(asid: u16) {
    let operand = (asid as u64) << ASID_SHIFT;
    // SAFETY: TLB maintenance by ASID only discards cached translations.
    unsafe {
        core::arch::asm!("tlbi aside1is, {v}", "dsb ish", "isb", v = in(reg) operand);
    }
}

/// Invalidate every cached translation (ASID rollover).
fn flush_all() {
    // SAFETY: TLB maintenance only discards cached translations.
    unsafe {
        core::arch::asm!("tlbi vmalle1is", "dsb ish", "isb");
    }
}

/// Switch TTBR0_EL1 to a user root under its own ASID. User descriptors
/// are non-global, so no TLB flush is needed unless the ASIDs rolled over.
pub fn activate_user(root: u64) {
    let binding = ASIDS.lock().bind(root);
    if binding.rolled_over {
        flush_all();
    }
    install(root, binding.asid);
}

/// Write TTBR0_EL1 after the table stores are visible.
fn install(root: u64, asid: u16) {
    let value = root | ((asid as u64) << ASID_SHIFT);
    crate::arch::cpu::dsb();
    // SAFETY: `root` is a live root containing the kernel's low-half blocks,
    // so instruction fetch continues after the write; the barriers order the
    // table stores before the switch and the switch before later accesses.
    unsafe {
        core::arch::asm!("msr TTBR0_EL1, {v}", "isb", v = in(reg) value);
    }
}

/// ASID currently installed in TTBR0_EL1.
pub fn loaded_asid() -> u16 {
    let value: u64;
    // SAFETY: reading TTBR0_EL1 has no side effects.
    unsafe { core::arch::asm!("mrs {}, TTBR0_EL1", out(reg) value) };
    (value >> ASID_SHIFT) as u16
}

/// Table base currently installed in TTBR0_EL1 (ASID and CnP stripped).
pub fn loaded_root() -> u64 {
    let value: u64;
    // SAFETY: reading TTBR0_EL1 has no side effects.
    unsafe { core::arch::asm!("mrs {}, TTBR0_EL1", out(reg) value) };
    value & OUT_MASK
}

/// Return to the kernel's own table (kernel threads, teardown).
pub fn activate_kernel() {
    install(super::mmu::boot_root_phys(), KERNEL_ASID);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Model {
        tables: HashMap<u64, Vec<u64>>,
        next: u64,
        live: usize,
    }

    impl TableMem for Model {
        fn entry(&self, table: u64, idx: usize) -> u64 {
            self.tables[&table][idx]
        }
        fn set_entry(&mut self, table: u64, idx: usize, v: u64) {
            self.tables.get_mut(&table).unwrap()[idx] = v;
        }
        fn alloc_table(&mut self) -> Option<u64> {
            self.next += 0x1000;
            self.live += 1;
            self.tables.insert(self.next, vec![0; ENTRIES]);
            Some(self.next)
        }
        fn free_table(&mut self, table: u64) {
            assert!(self.tables.remove(&table).is_some(), "double free");
            self.live -= 1;
        }
    }

    const IDENTITY_BLOCK: u64 = 0x0000_0000 | BLOCK | (1 << 10) | (1 << 54);
    const GIC_SLOT: usize = 0x41;

    /// A boot root with an identity L2, a peripheral L2 and a GIC L1 slot.
    fn boot(m: &mut Model) -> u64 {
        let l0 = m.alloc_table().unwrap();
        let l1 = m.alloc_table().unwrap();
        let l2a = m.alloc_table().unwrap();
        let l2b = m.alloc_table().unwrap();
        let gic = m.alloc_table().unwrap();
        m.set_entry(l0, 0, l1 | TABLE);
        m.set_entry(l1, 0, l2a | TABLE);
        m.set_entry(l1, 1, l2b | TABLE);
        m.set_entry(l1, GIC_SLOT, gic | TABLE);
        for i in 0..64 {
            m.set_entry(l2a, i, ((i as u64) << 21) | IDENTITY_BLOCK);
        }
        m.set_entry(l2a, 128, 0xAAAA_0000 | TABLE);
        m.set_entry(l2b, 0x1E0, 0x7C00_0000 | BLOCK | (1 << 54) | (1 << 53));
        m.set_entry(gic, 0, 0x10_7FE0_0000 | BLOCK);
        l0
    }

    #[test]
    fn copied_kernel_descriptors_equal_boot_descriptors() {
        let mut m = Model::default();
        let b = boot(&mut m);
        let root = build_root(&mut m, b).unwrap();
        assert!(kernel_blocks_match(&m, b, root));
    }

    #[test]
    fn stale_boot_user_tables_are_not_copied() {
        let mut m = Model::default();
        let b = boot(&mut m);
        let root = build_root(&mut m, b).unwrap();
        let l1 = child(&m, root, 0).unwrap();
        let l2a = child(&m, l1, 0).unwrap();
        assert_eq!(m.entry(l2a, 128), 0);
    }

    #[test]
    fn kernel_gic_table_is_shared_not_copied() {
        let mut m = Model::default();
        let b = boot(&mut m);
        let boot_l1 = child(&m, b, 0).unwrap();
        let root = build_root(&mut m, b).unwrap();
        let l1 = child(&m, root, 0).unwrap();
        assert_eq!(m.entry(l1, GIC_SLOT), m.entry(boot_l1, GIC_SLOT));
        assert_ne!(m.entry(l1, 0), m.entry(boot_l1, 0));
    }

    #[test]
    fn free_returns_exactly_what_build_took_and_spares_the_boot_tables() {
        let mut m = Model::default();
        let b = boot(&mut m);
        let before = m.live;
        let root = build_root(&mut m, b).unwrap();
        assert_eq!(m.live, before + 4);
        let l1 = child(&m, root, 0).unwrap();
        let l2a = child(&m, l1, 0).unwrap();
        let l3 = m.alloc_table().unwrap();
        m.set_entry(l3, 5, 0x9000 | 0b11);
        m.set_entry(l2a, 130, l3 | TABLE);
        let mut leaves = Vec::new();
        free_root(&mut m, root, &mut |pte| leaves.push(pte));
        assert_eq!(leaves, vec![0x9000 | 0b11]);
        assert_eq!(m.live, before);
    }

    #[test]
    fn out_of_tables_rolls_back_every_allocation() {
        struct Limited(Model, usize);
        impl TableMem for Limited {
            fn entry(&self, t: u64, i: usize) -> u64 {
                self.0.entry(t, i)
            }
            fn set_entry(&mut self, t: u64, i: usize, v: u64) {
                self.0.set_entry(t, i, v)
            }
            fn alloc_table(&mut self) -> Option<u64> {
                if self.1 == 0 {
                    return None;
                }
                self.1 -= 1;
                self.0.alloc_table()
            }
            fn free_table(&mut self, t: u64) {
                self.0.free_table(t)
            }
        }
        let mut m = Model::default();
        let b = boot(&mut m);
        let live = m.live;
        let mut lim = Limited(m, 3);
        assert!(build_root(&mut lim, b).is_none());
        assert_eq!(lim.0.live, live);
    }
}
