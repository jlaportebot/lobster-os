//! ASID bookkeeping for per-process translation roots.
//!
//! TCR_EL1.AS is 0, so ASIDs are 8 bits; 0 belongs to the kernel root and
//! 1..=255 are handed to user roots. A root keeps its ASID for its whole
//! life, so switching to it needs no TLB maintenance. When all 255 are taken,
//! the whole table is recycled and the caller flushes every TLB once.

/// Number of ASIDs the hardware offers with 8-bit ASIDs.
pub const ASID_COUNT: usize = 256;

/// ASID of the kernel (boot) root; its descriptors are global anyway.
pub const KERNEL_ASID: u16 = 0;

/// Result of binding a root to an ASID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    /// ASID to put in TTBR0_EL1.
    pub asid: u16,
    /// Every previous binding was dropped: the caller must flush all TLBs.
    pub rolled_over: bool,
}

/// Which root owns which ASID (0 = unowned).
pub struct AsidTable {
    owner: [u64; ASID_COUNT],
    next: usize,
}

impl AsidTable {
    /// An empty table.
    pub const fn new() -> Self {
        Self {
            owner: [0; ASID_COUNT],
            next: 1,
        }
    }

    /// The ASID already bound to `root`, if any.
    fn find(&self, root: u64) -> Option<u16> {
        (1..ASID_COUNT)
            .find(|&asid| self.owner[asid] == root)
            .map(|asid| asid as u16)
    }

    /// First unowned ASID at or after the rotating cursor.
    fn find_free(&self) -> Option<usize> {
        (0..ASID_COUNT - 1)
            .map(|step| 1 + (self.next - 1 + step) % (ASID_COUNT - 1))
            .find(|&asid| self.owner[asid] == 0)
    }

    /// Return the ASID for `root`, binding a fresh one when needed.
    pub fn bind(&mut self, root: u64) -> Binding {
        if let Some(asid) = self.find(root) {
            return Binding {
                asid,
                rolled_over: false,
            };
        }
        let (asid, rolled_over) = match self.find_free() {
            Some(asid) => (asid, false),
            None => {
                self.owner = [0; ASID_COUNT];
                (1, true)
            }
        };
        self.owner[asid] = root;
        self.next = 1 + asid % (ASID_COUNT - 1);
        Binding {
            asid: asid as u16,
            rolled_over,
        }
    }

    /// Drop the binding of `root`; returns the ASID whose TLB entries the
    /// caller must invalidate before the number is reused.
    pub fn release(&mut self, root: u64) -> Option<u16> {
        let asid = self.find(root)?;
        self.owner[asid as usize] = 0;
        Some(asid)
    }
}

impl Default for AsidTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT_STRIDE: u64 = 0x1000;

    fn root(n: u64) -> u64 {
        0x100_0000 + n * ROOT_STRIDE
    }

    #[test]
    fn same_root_keeps_its_asid() {
        let mut t = AsidTable::new();
        let first = t.bind(root(1));
        assert_eq!(t.bind(root(1)), first);
        assert_eq!(first.asid, 1);
    }

    #[test]
    fn distinct_roots_get_distinct_nonzero_asids() {
        let mut t = AsidTable::new();
        let a = t.bind(root(1)).asid;
        let b = t.bind(root(2)).asid;
        assert_ne!(a, b);
        assert!(a != KERNEL_ASID && b != KERNEL_ASID);
    }

    #[test]
    fn release_returns_the_asid_and_frees_the_slot() {
        let mut t = AsidTable::new();
        let a = t.bind(root(1)).asid;
        assert_eq!(t.release(root(1)), Some(a));
        assert_eq!(t.release(root(1)), None);
    }

    #[test]
    fn rollover_after_255_roots_requests_a_flush_once() {
        let mut t = AsidTable::new();
        for n in 0..(ASID_COUNT as u64 - 1) {
            assert!(!t.bind(root(n)).rolled_over);
        }
        let wrapped = t.bind(root(1000));
        assert!(wrapped.rolled_over);
        assert_eq!(wrapped.asid, 1);
        assert!(!t.bind(root(1001)).rolled_over);
    }

    #[test]
    fn freed_asids_are_reused_before_rolling_over() {
        let mut t = AsidTable::new();
        for n in 0..(ASID_COUNT as u64 - 1) {
            t.bind(root(n));
        }
        let freed = t.release(root(7)).unwrap();
        let again = t.bind(root(2000));
        assert!(!again.rolled_over);
        assert_eq!(again.asid, freed);
    }

    #[test]
    fn a_root_that_lost_its_asid_in_rollover_gets_a_new_one() {
        let mut t = AsidTable::new();
        for n in 0..(ASID_COUNT as u64 - 1) {
            t.bind(root(n));
        }
        t.bind(root(1000));
        assert_eq!(t.release(root(3)), None);
        assert!(!t.bind(root(3)).rolled_over);
    }
}
