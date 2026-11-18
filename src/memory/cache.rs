//! Data/instruction cache maintenance for DMA buffers and freshly written code.
//!
//! With `SCTLR_EL1.C` set, CPU stores sit in the (non-coherent with devices)
//! data cache. A device that reads a buffer by DMA sees stale RAM unless the
//! lines were cleaned first; a device that writes a buffer leaves stale lines
//! in the cache unless they are invalidated afterwards. Code written through
//! the data side must likewise be cleaned to the point of unification and
//! the instruction cache invalidated before it is executed.
//!
//! The pure address maths ([`LineSpan`]) is host-tested; the cache
//! instructions themselves compile to no-ops outside bare-metal aarch64.

/// Smallest cache line the maths supports (CTR_EL0.DminLine = 0 means 4 words).
#[cfg_attr(
    not(all(target_arch = "aarch64", not(feature = "std"))),
    allow(dead_code)
)]
const MIN_LINE_BYTES: usize = 16;

/// Line size assumed off-target, where no CTR_EL0 exists (Cortex-A76 uses 64).
#[cfg(not(all(target_arch = "aarch64", not(feature = "std"))))]
const DEFAULT_LINE_BYTES: usize = 64;

/// Bytes per word, the unit CTR_EL0 line fields are expressed in.
#[cfg_attr(
    not(all(target_arch = "aarch64", not(feature = "std"))),
    allow(dead_code)
)]
const WORD_BYTES: usize = 4;

/// CTR_EL0.DminLine: log2(words) of the smallest data cache line.
#[cfg_attr(
    not(all(target_arch = "aarch64", not(feature = "std"))),
    allow(dead_code)
)]
const CTR_DMIN_LINE_SHIFT: u64 = 16;
/// CTR_EL0.IminLine: log2(words) of the smallest instruction cache line.
#[cfg_attr(
    not(all(target_arch = "aarch64", not(feature = "std"))),
    allow(dead_code)
)]
const CTR_IMIN_LINE_SHIFT: u64 = 0;
/// Width mask of the CTR_EL0 line-size fields.
#[cfg_attr(
    not(all(target_arch = "aarch64", not(feature = "std"))),
    allow(dead_code)
)]
const CTR_LINE_FIELD_MASK: u64 = 0xF;

/// A byte range widened to whole cache lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    /// First line-aligned address at or below the range start.
    pub start: usize,
    /// First line-aligned address at or above the range end.
    pub end: usize,
    /// True when the original start sits inside its line (partial head line).
    pub head_partial: bool,
    /// True when the original end sits inside its line (partial tail line).
    pub tail_partial: bool,
}

impl LineSpan {
    /// Widen `[addr, addr + len)` to `line`-byte boundaries. `line` must be a
    /// power of two; an empty range yields an empty span.
    pub fn new(addr: usize, len: usize, line: usize) -> Self {
        let mask = line - 1;
        let end_exact = addr.saturating_add(len);
        LineSpan {
            start: addr & !mask,
            end: end_exact.saturating_add(mask) & !mask,
            head_partial: addr & mask != 0,
            tail_partial: end_exact & mask != 0,
        }
    }

    /// Number of cache lines covered.
    pub fn line_count(&self, line: usize) -> usize {
        (self.end - self.start) / line
    }
}

/// How an invalidate treats a line. Lines shared with neighbouring data must be
/// cleaned before being dropped so the neighbour's dirty bytes survive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(all(target_arch = "aarch64", not(feature = "std"))),
    allow(dead_code)
)]
enum InvalidateKind {
    /// The whole line belongs to the buffer: drop it.
    Discard,
    /// The line also holds foreign bytes: write back, then drop.
    CleanThenDiscard,
}

/// Choose the invalidate flavour for the line starting at `line_addr`.
#[cfg_attr(
    not(all(target_arch = "aarch64", not(feature = "std"))),
    allow(dead_code)
)]
fn invalidate_kind(span: &LineSpan, line_addr: usize, line: usize) -> InvalidateKind {
    let is_head = line_addr == span.start && span.head_partial;
    let is_tail = line_addr + line == span.end && span.tail_partial;
    if is_head || is_tail {
        InvalidateKind::CleanThenDiscard
    } else {
        InvalidateKind::Discard
    }
}

/// Decode a CTR_EL0 line-size field into bytes.
#[cfg_attr(
    not(all(target_arch = "aarch64", not(feature = "std"))),
    allow(dead_code)
)]
fn line_bytes_from_ctr(ctr: u64, shift: u64) -> usize {
    let log2_words = (ctr >> shift) & CTR_LINE_FIELD_MASK;
    (WORD_BYTES << log2_words).max(MIN_LINE_BYTES)
}

#[cfg(all(target_arch = "aarch64", not(feature = "std")))]
fn read_ctr() -> u64 {
    let ctr: u64;
    // SAFETY: CTR_EL0 is a read-only ID register readable at EL1 with no side effects.
    unsafe { core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack)) };
    ctr
}

/// Smallest data cache line in bytes (CTR_EL0.DminLine).
pub fn dcache_line_size() -> usize {
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    {
        line_bytes_from_ctr(read_ctr(), CTR_DMIN_LINE_SHIFT)
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "std"))))]
    {
        DEFAULT_LINE_BYTES
    }
}

/// Smallest instruction cache line in bytes (CTR_EL0.IminLine).
pub fn icache_line_size() -> usize {
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    {
        line_bytes_from_ctr(read_ctr(), CTR_IMIN_LINE_SHIFT)
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "std"))))]
    {
        DEFAULT_LINE_BYTES
    }
}

/// Write dirty lines in `[addr, addr + len)` back to RAM so a device can read them.
///
/// # Safety
/// `addr..addr + len` must be a mapped, cacheable kernel range.
pub unsafe fn clean_dcache_range(addr: usize, len: usize) {
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    {
        let line = dcache_line_size();
        let span = LineSpan::new(addr, len, line);
        let mut p = span.start;
        while p < span.end {
            // SAFETY: caller guarantees the range is mapped; `dc cvac` only writes back.
            core::arch::asm!("dc cvac, {}", in(reg) p, options(nostack));
            p += line;
        }
        core::arch::asm!("dsb sy", options(nostack));
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "std"))))]
    let _ = (addr, len);
}

/// Write back and drop every line of `[addr, addr + len)`.
///
/// Use before a device writes into a buffer the CPU has touched: it stops a
/// later eviction of a stale dirty line from overwriting the device's data.
///
/// # Safety
/// `addr..addr + len` must be a mapped, cacheable kernel range.
pub unsafe fn clean_invalidate_dcache_range(addr: usize, len: usize) {
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    {
        let line = dcache_line_size();
        let span = LineSpan::new(addr, len, line);
        let mut p = span.start;
        while p < span.end {
            // SAFETY: caller guarantees the range is mapped.
            core::arch::asm!("dc civac, {}", in(reg) p, options(nostack));
            p += line;
        }
        core::arch::asm!("dsb sy", options(nostack));
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "std"))))]
    let _ = (addr, len);
}

/// Drop cached copies of `[addr, addr + len)` after a device wrote it.
///
/// Partial head/tail lines are cleaned first so adjacent CPU-owned bytes in
/// the same line are not lost.
///
/// # Safety
/// `addr..addr + len` must be a mapped, cacheable kernel range.
pub unsafe fn invalidate_dcache_range(addr: usize, len: usize) {
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    {
        let line = dcache_line_size();
        let span = LineSpan::new(addr, len, line);
        let mut p = span.start;
        while p < span.end {
            // SAFETY: caller guarantees the range is mapped.
            match invalidate_kind(&span, p, line) {
                InvalidateKind::Discard => {
                    core::arch::asm!("dc ivac, {}", in(reg) p, options(nostack))
                }
                InvalidateKind::CleanThenDiscard => {
                    core::arch::asm!("dc civac, {}", in(reg) p, options(nostack))
                }
            }
            p += line;
        }
        core::arch::asm!("dsb sy", options(nostack));
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "std"))))]
    let _ = (addr, len);
}

/// Make code just written to `[addr, addr + len)` visible to instruction fetch.
///
/// Cleans the data cache to the point of unification, invalidates the
/// instruction cache for the range, and synchronises the pipeline.
///
/// # Safety
/// `addr..addr + len` must be a mapped kernel or user range accessible at EL1.
pub unsafe fn sync_icache_range(addr: usize, len: usize) {
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    {
        let dline = dcache_line_size();
        let dspan = LineSpan::new(addr, len, dline);
        let mut p = dspan.start;
        while p < dspan.end {
            // SAFETY: caller guarantees the range is mapped.
            core::arch::asm!("dc cvau, {}", in(reg) p, options(nostack));
            p += dline;
        }
        core::arch::asm!("dsb ish", options(nostack));
        let iline = icache_line_size();
        let ispan = LineSpan::new(addr, len, iline);
        let mut p = ispan.start;
        while p < ispan.end {
            // SAFETY: caller guarantees the range is mapped.
            core::arch::asm!("ic ivau, {}", in(reg) p, options(nostack));
            p += iline;
        }
        core::arch::asm!("dsb ish", "isb", options(nostack));
    }
    #[cfg(not(all(target_arch = "aarch64", not(feature = "std"))))]
    let _ = (addr, len);
}

/// Invalidate the whole instruction cache (used when first enabling it).
pub fn invalidate_icache_all() {
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    // SAFETY: `ic iallu` only discards instruction cache contents; the barriers order it.
    unsafe {
        core::arch::asm!("ic iallu", "dsb nsh", "isb", options(nostack));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: usize = 64;

    #[test]
    fn aligned_range_is_unchanged_and_not_partial() {
        let s = LineSpan::new(0x1000, 128, LINE);
        assert_eq!((s.start, s.end), (0x1000, 0x1080));
        assert!(!s.head_partial && !s.tail_partial);
        assert_eq!(s.line_count(LINE), 2);
    }

    #[test]
    fn unaligned_range_widens_to_whole_lines() {
        let s = LineSpan::new(0x1010, 100, LINE);
        assert_eq!((s.start, s.end), (0x1000, 0x1080));
        assert!(s.head_partial && s.tail_partial);
    }

    #[test]
    fn empty_range_covers_no_lines() {
        let s = LineSpan::new(0x1000, 0, LINE);
        assert_eq!(s.line_count(LINE), 0);
    }

    #[test]
    fn single_byte_covers_one_line() {
        let s = LineSpan::new(0x103F, 1, LINE);
        assert_eq!((s.start, s.end), (0x1000, 0x1040));
        assert!(s.head_partial && !s.tail_partial);
    }

    #[test]
    fn range_straddling_a_boundary_covers_two_lines() {
        let s = LineSpan::new(0x103F, 2, LINE);
        assert_eq!(s.line_count(LINE), 2);
    }

    #[test]
    fn only_edge_lines_of_unaligned_invalidate_are_cleaned_first() {
        let s = LineSpan::new(0x1010, 200, LINE);
        assert_eq!(
            invalidate_kind(&s, 0x1000, LINE),
            InvalidateKind::CleanThenDiscard
        );
        assert_eq!(invalidate_kind(&s, 0x1040, LINE), InvalidateKind::Discard);
        assert_eq!(
            invalidate_kind(&s, 0x10C0, LINE),
            InvalidateKind::CleanThenDiscard
        );
    }

    #[test]
    fn aligned_invalidate_discards_every_line() {
        let s = LineSpan::new(0x2000, 192, LINE);
        for p in [0x2000, 0x2040, 0x2080] {
            assert_eq!(invalidate_kind(&s, p, LINE), InvalidateKind::Discard);
        }
    }

    #[test]
    fn ctr_line_field_decodes_cortex_a76_64_byte_lines() {
        let ctr = 0x8444_C004u64;
        assert_eq!(line_bytes_from_ctr(ctr, CTR_DMIN_LINE_SHIFT), 64);
        assert_eq!(line_bytes_from_ctr(ctr, CTR_IMIN_LINE_SHIFT), 64);
    }

    #[test]
    fn huge_range_does_not_overflow() {
        let s = LineSpan::new(usize::MAX - 10, 100, LINE);
        assert!(s.end >= s.start);
    }
}
