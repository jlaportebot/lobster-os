//! Host tests for the ext2 driver: the real driver code runs against an
//! in-memory disk built from a sparse `mke2fs` fixture. After every scenario
//! the image is checked twice: by a small independent checker in this file
//! (so the tests have teeth without e2fsprogs) and by the real `e2fsck -fn`
//! when it is installed.

use super::*;
use crate::fs::block_dev::{BlockDevice, BlockError};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

const FIXTURE_1K: &[u8] = include_bytes!("../../tests/fixtures/ext2_1k.blocks");
const FIXTURE_4K: &[u8] = include_bytes!("../../tests/fixtures/ext2_4k.blocks");
const FIXTURE_MAGIC: &[u8; 8] = b"EXT2FX01";
const FIXTURE_HEADER_LEN: usize = 8 + 8 + 4 + 4;
const SECTOR: usize = 512;
const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;
/// Stack for the thread running one scenario: the driver keeps large
/// fixed-size caches inline in `Ext2FileSystem`.
const TEST_STACK_BYTES: usize = 64 * MIB;
const SPARSE_SUPER_POWERS: [u32; 3] = [3, 5, 7];
const FIRST_NON_RESERVED_INO: u32 = 11;
const RESIZE_INO: u32 = 7;
const EXT2_INDEX_FL: u32 = 0x1000;

// ─── In-memory disk registered through the real block_dev registry ───

struct RamDisk {
    data: Mutex<Vec<u8>>,
}

impl BlockDevice for RamDisk {
    fn read_block(&self, block: u64, buf: &mut [u8; SECTOR]) -> Result<(), BlockError> {
        let data = self.data.lock().expect("ram disk lock");
        let start = block as usize * SECTOR;
        let src = data
            .get(start..start + SECTOR)
            .ok_or(BlockError::InvalidBlock)?;
        buf.copy_from_slice(src);
        Ok(())
    }

    fn write_block(&self, block: u64, buf: &[u8; SECTOR]) -> Result<(), BlockError> {
        let mut data = self.data.lock().expect("ram disk lock");
        let start = block as usize * SECTOR;
        let dst = data
            .get_mut(start..start + SECTOR)
            .ok_or(BlockError::InvalidBlock)?;
        dst.copy_from_slice(buf);
        Ok(())
    }

    fn block_size(&self) -> usize {
        SECTOR
    }

    fn total_blocks(&self) -> u64 {
        (self.data.lock().expect("ram disk lock").len() / SECTOR) as u64
    }

    fn name(&self) -> &str {
        "ramdisk-test"
    }
}

static DISK: RamDisk = RamDisk {
    data: Mutex::new(Vec::new()),
};
static DEVICE_INDEX: OnceLock<usize> = OnceLock::new();

fn device_index() -> usize {
    *DEVICE_INDEX
        .get_or_init(|| crate::fs::block_dev::register_device(&DISK).expect("register ram disk"))
}

fn expand_fixture(fixture: &[u8]) -> Vec<u8> {
    assert_eq!(&fixture[..8], FIXTURE_MAGIC, "fixture magic");
    let total = u64::from_le_bytes(fixture[8..16].try_into().expect("len")) as usize;
    let chunk = u32::from_le_bytes(fixture[16..20].try_into().expect("chunk")) as usize;
    let count = u32::from_le_bytes(fixture[20..24].try_into().expect("count")) as usize;
    let mut image = vec![0u8; total];
    let mut pos = FIXTURE_HEADER_LEN;
    for _ in 0..count {
        let index = u32::from_le_bytes(fixture[pos..pos + 4].try_into().expect("idx")) as usize;
        image[index * chunk..(index + 1) * chunk]
            .copy_from_slice(&fixture[pos + 4..pos + 4 + chunk]);
        pos += 4 + chunk;
    }
    image
}

/// A mounted fixture volume; holds the global test lock for its lifetime.
struct Volume {
    _guard: MutexGuard<'static, ()>,
}

fn fs() -> &'static mut Ext2FileSystem {
    // SAFETY: tests holding `Volume` are serialized by the crate-wide serial_guard.
    unsafe { EXT2_INSTANCE.as_mut().expect("volume mounted") }
}

impl Volume {
    fn mount(fixture: &[u8]) -> Volume {
        let guard = crate::test_support::serial_guard();
        device_index();
        *DISK.data.lock().expect("ram disk lock") = expand_fixture(fixture);
        let vol = Volume { _guard: guard };
        vol.remount();
        vol
    }

    /// Drop the in-memory driver state and mount the same image again.
    fn remount(&self) {
        let mut fresh = Ext2FileSystem::new(device_index(), 0).expect("mount fixture");
        fresh.journal_replay().expect("journal replay");
        // SAFETY: serialized by the crate-wide serial_guard.
        unsafe {
            EXT2_INSTANCE = Some(fresh);
        }
    }

    fn image(&self) -> Vec<u8> {
        DISK.data.lock().expect("ram disk lock").clone()
    }
}

/// Run `body` against a freshly mounted fixture on a big-stack thread.
fn with_volume(fixture: &'static [u8], body: impl FnOnce(&Volume) + Send + 'static) {
    let handle = std::thread::Builder::new()
        .stack_size(TEST_STACK_BYTES)
        .spawn(move || body(&Volume::mount(fixture)))
        .expect("spawn test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}

// ─── Independent image checker (mini e2fsck) ─────────────────────────

fn le16(d: &[u8], off: usize) -> u32 {
    u16::from_le_bytes([d[off], d[off + 1]]) as u32
}

fn le32(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
}

struct RawInode {
    mode: u32,
    size: u32,
    links: u32,
    blocks: u32,
    dtime: u32,
    flags: u32,
    iblock: [u32; 15],
}

struct Img<'a> {
    d: &'a [u8],
    bs: usize,
    first_data: u32,
    bpg: u32,
    ipg: u32,
    isz: usize,
    groups: usize,
    blocks: u32,
}

struct GroupDesc {
    block_bitmap: u32,
    inode_bitmap: u32,
    inode_table: u32,
    free_blocks: u32,
    free_inodes: u32,
    used_dirs: u32,
}

impl<'a> Img<'a> {
    fn new(d: &'a [u8]) -> Img<'a> {
        let sb = 1024;
        let bpg = le32(d, sb + 32);
        let blocks = le32(d, sb + 4);
        let first_data = le32(d, sb + 20);
        Img {
            d,
            bs: 1024 << le32(d, sb + 24),
            first_data,
            bpg,
            ipg: le32(d, sb + 40),
            isz: le16(d, sb + 88) as usize,
            groups: ((blocks - first_data + bpg - 1) / bpg) as usize,
            blocks,
        }
    }

    fn sb(&self, off: usize) -> u32 {
        le32(self.d, 1024 + off)
    }

    fn gd(&self, g: usize) -> GroupDesc {
        let off = (self.first_data as usize + 1) * self.bs + g * 32;
        GroupDesc {
            block_bitmap: le32(self.d, off),
            inode_bitmap: le32(self.d, off + 4),
            inode_table: le32(self.d, off + 8),
            free_blocks: le16(self.d, off + 12),
            free_inodes: le16(self.d, off + 14),
            used_dirs: le16(self.d, off + 16),
        }
    }

    fn bitmap_bit(&self, bitmap_block: u32, n: usize) -> bool {
        self.d[bitmap_block as usize * self.bs + n / 8] & (1 << (n % 8)) != 0
    }

    fn inode_in_use(&self, ino: u32) -> bool {
        let g = ((ino - 1) / self.ipg) as usize;
        self.bitmap_bit(self.gd(g).inode_bitmap, ((ino - 1) % self.ipg) as usize)
    }

    fn inode(&self, ino: u32) -> RawInode {
        let g = ((ino - 1) / self.ipg) as usize;
        let idx = ((ino - 1) % self.ipg) as usize;
        let off = self.gd(g).inode_table as usize * self.bs + idx * self.isz;
        let mut iblock = [0u32; 15];
        for (i, slot) in iblock.iter_mut().enumerate() {
            *slot = le32(self.d, off + 40 + i * 4);
        }
        RawInode {
            mode: le16(self.d, off),
            size: le32(self.d, off + 4),
            dtime: le32(self.d, off + 20),
            links: le16(self.d, off + 26),
            blocks: le32(self.d, off + 28),
            flags: le32(self.d, off + 32),
            iblock,
        }
    }

    fn block(&self, n: u32) -> &[u8] {
        &self.d[n as usize * self.bs..(n as usize + 1) * self.bs]
    }

    fn has_super(&self, g: usize) -> bool {
        let g = g as u32;
        g <= 1
            || SPARSE_SUPER_POWERS.iter().any(|&p| {
                (1..)
                    .map(|e| p.pow(e))
                    .take_while(|&v| v <= g)
                    .any(|v| v == g)
            })
    }

    fn group_start(&self, g: usize) -> u32 {
        self.first_data + g as u32 * self.bpg
    }

    /// Every block (data and indirect) an inode references.
    fn inode_blocks(&self, ino: &RawInode) -> Result<Vec<u32>, String> {
        let mut out = Vec::new();
        for (slot, &b) in ino.iblock.iter().enumerate() {
            if b == 0 {
                continue;
            }
            let depth = slot.saturating_sub(11);
            self.collect(b, depth, &mut out)?;
        }
        Ok(out)
    }

    /// The data blocks of an inode in logical order (indirect blocks left out).
    fn data_blocks(&self, ino: &RawInode) -> Result<Vec<u32>, String> {
        let mut out = Vec::new();
        for (slot, &b) in ino.iblock.iter().enumerate() {
            if b != 0 {
                self.collect_data(b, slot.saturating_sub(11), &mut out)?;
            }
        }
        Ok(out)
    }

    fn collect_data(&self, b: u32, depth: usize, out: &mut Vec<u32>) -> Result<(), String> {
        if b >= self.blocks {
            return Err(format!("block {} beyond volume", b));
        }
        if depth == 0 {
            out.push(b);
            return Ok(());
        }
        for i in 0..self.bs / 4 {
            let child = le32(self.block(b), i * 4);
            if child != 0 {
                self.collect_data(child, depth - 1, out)?;
            }
        }
        Ok(())
    }

    fn collect(&self, b: u32, depth: usize, out: &mut Vec<u32>) -> Result<(), String> {
        if b >= self.blocks {
            return Err(format!("block {} beyond volume", b));
        }
        out.push(b);
        if depth == 0 {
            return Ok(());
        }
        for i in 0..self.bs / 4 {
            let child = le32(self.block(b), i * 4);
            if child != 0 {
                self.collect(child, depth - 1, out)?;
            }
        }
        Ok(())
    }
}

fn check_blocks(img: &Img) -> Result<Vec<u32>, String> {
    let mut claimed = vec![0u32; img.blocks as usize];
    let total_inodes = img.ipg * img.groups as u32;
    // The resize inode owns one double-indirect block; its leaves are the
    // reserved GDT blocks, which are counted as group metadata instead.
    claimed[img.inode(RESIZE_INO).iblock[EXT2_DIND_BLOCK] as usize] += 1;
    for ino in 1..=total_inodes {
        if !img.inode_in_use(ino) || (ino != 2 && ino < FIRST_NON_RESERVED_INO) {
            continue;
        }
        let raw = img.inode(ino);
        let blocks = img.inode_blocks(&raw)?;
        let expect = blocks.len() as u32 * (img.bs / SECTOR) as u32;
        if raw.blocks != expect {
            return Err(format!(
                "inode {}: i_blocks is {}, should be {}",
                ino, raw.blocks, expect
            ));
        }
        for b in blocks {
            claimed[b as usize] += 1;
            if claimed[b as usize] > 1 {
                return Err(format!("block {} multiply claimed (inode {})", b, ino));
            }
        }
    }
    Ok(claimed)
}

fn check_bitmaps_and_counts(img: &Img, claimed: &[u32]) -> Result<(), String> {
    let gdt_blocks = (img.groups * 32 + img.bs - 1) / img.bs;
    let reserved = le16(img.d, 1024 + 206) as usize;
    let (mut free_total, mut free_inodes_total) = (0u32, 0u32);
    for g in 0..img.groups {
        let gd = img.gd(g);
        let start = img.group_start(g);
        let end = (start + img.bpg).min(img.blocks);
        let mut meta = vec![gd.block_bitmap, gd.inode_bitmap];
        let table_blocks = (img.ipg as usize * img.isz + img.bs - 1) / img.bs;
        meta.extend((0..table_blocks as u32).map(|i| gd.inode_table + i));
        if img.has_super(g) {
            meta.extend((0..(1 + gdt_blocks + reserved) as u32).map(|i| start + i));
        }
        let mut free = 0u32;
        for b in start..end {
            let expect = claimed[b as usize] > 0 || meta.contains(&b);
            let set = img.bitmap_bit(gd.block_bitmap, (b - start) as usize);
            if expect != set {
                return Err(format!(
                    "group {} block {}: bitmap says {}, expected {}",
                    g, b, set, expect
                ));
            }
            free += u32::from(!set);
        }
        if free != gd.free_blocks {
            return Err(format!(
                "group {} free blocks {} should be {}",
                g, gd.free_blocks, free
            ));
        }
        free_total += free;
        let used_ino = (0..img.ipg as usize)
            .filter(|&i| img.bitmap_bit(gd.inode_bitmap, i))
            .count() as u32;
        if img.ipg - used_ino != gd.free_inodes {
            return Err(format!(
                "group {} free inodes {} should be {}",
                g,
                gd.free_inodes,
                img.ipg - used_ino
            ));
        }
        free_inodes_total += gd.free_inodes;
    }
    if free_total != img.sb(12) {
        return Err(format!(
            "sb free blocks {} should be {}",
            img.sb(12),
            free_total
        ));
    }
    if free_inodes_total != img.sb(16) {
        return Err(format!(
            "sb free inodes {} should be {}",
            img.sb(16),
            free_inodes_total
        ));
    }
    Ok(())
}

/// Parse one directory: returns (name, ino, file_type) of all live entries.
fn parse_dir(img: &Img, dir: u32) -> Result<Vec<(String, u32, u32)>, String> {
    let raw = img.inode(dir);
    if raw.size as usize % img.bs != 0 {
        return Err(format!("dir {} size {} not block multiple", dir, raw.size));
    }
    if raw.flags & EXT2_INDEX_FL != 0 {
        return Err(format!("dir {} still flagged as indexed", dir));
    }
    let mut entries = Vec::new();
    for (n, &b) in img
        .data_blocks(&raw)?
        .iter()
        .take(raw.size as usize / img.bs)
        .enumerate()
    {
        let blk = img.block(b);
        let mut pos = 0;
        while pos < img.bs {
            let (ino, rec) = (le32(blk, pos), le16(blk, pos + 4) as usize);
            let name_len = blk[pos + 6] as usize;
            if rec % 4 != 0 || rec < 8 + name_len || pos + rec > img.bs {
                return Err(format!(
                    "dir {} block {} bad rec_len {} at {}",
                    dir, n, rec, pos
                ));
            }
            if ino != 0 {
                let name = String::from_utf8_lossy(&blk[pos + 8..pos + 8 + name_len]).to_string();
                entries.push((name, ino, blk[pos + 7] as u32));
            } else if n == 0 && pos == 0 {
                return Err(format!("dir {} first entry empty", dir));
            }
            pos += rec;
        }
    }
    let head: Vec<&str> = entries.iter().take(2).map(|e| e.0.as_str()).collect();
    if head != [".", ".."] || entries[0].1 != dir {
        return Err(format!(
            "dir {} must start with '.' and '..': {:?}",
            dir, head
        ));
    }
    Ok(entries)
}

fn file_type_for(mode: u32) -> u32 {
    match mode & 0xF000 {
        0x4000 => EXT2_FT_DIR as u32,
        0x8000 => EXT2_FT_REG_FILE as u32,
        _ => EXT2_FT_UNKNOWN as u32,
    }
}

fn check_tree(img: &Img) -> Result<(), String> {
    let mut refs: BTreeMap<u32, u32> = BTreeMap::new();
    let mut dirs_per_group = vec![0u32; img.groups];
    let mut stack = vec![2u32];
    refs.insert(2, 0);
    while let Some(dir) = stack.pop() {
        dirs_per_group[((dir - 1) / img.ipg) as usize] += 1;
        let mut seen = Vec::new();
        for (name, ino, ft) in parse_dir(img, dir)? {
            if seen.contains(&name) {
                return Err(format!("dir {} duplicate entry {}", dir, name));
            }
            seen.push(name.clone());
            if !img.inode_in_use(ino) {
                return Err(format!(
                    "dir {} entry {} -> unused inode {}",
                    dir, name, ino
                ));
            }
            let raw = img.inode(ino);
            if ft != file_type_for(raw.mode) {
                return Err(format!(
                    "entry {} file_type {} vs mode {:o}",
                    name, ft, raw.mode
                ));
            }
            let first_visit = !refs.contains_key(&ino);
            *refs.entry(ino).or_insert(0) += 1;
            if raw.mode & 0xF000 == 0x4000 && first_visit && name != "." && name != ".." {
                stack.push(ino);
            }
        }
    }
    for (&ino, &count) in &refs {
        let links = img.inode(ino).links;
        if links != count {
            return Err(format!(
                "inode {} links {} but {} references",
                ino, links, count
            ));
        }
    }
    for g in 0..img.groups {
        if img.gd(g).used_dirs != dirs_per_group[g] {
            return Err(format!(
                "group {} used_dirs {} should be {}",
                g,
                img.gd(g).used_dirs,
                dirs_per_group[g]
            ));
        }
    }
    let total = img.ipg * img.groups as u32;
    for ino in FIRST_NON_RESERVED_INO..=total {
        if img.inode_in_use(ino) && !refs.contains_key(&ino) {
            return Err(format!("inode {} in use but unreachable", ino));
        }
    }
    Ok(())
}

fn check_image(data: &[u8]) -> Result<(), String> {
    let img = Img::new(data);
    let claimed = check_blocks(&img)?;
    check_bitmaps_and_counts(&img, &claimed)?;
    check_tree(&img)
}

fn scratch_dir() -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ext2-fsck-tmp");
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run the real `e2fsck -fn` over `data`; skipped when e2fsprogs is absent.
fn run_e2fsck(data: &[u8]) -> Result<(), String> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = scratch_dir().join(format!("img-{}-{}.ext2", std::process::id(), n));
    std::fs::write(&path, data).expect("write image");
    let out = std::process::Command::new("e2fsck")
        .arg("-fn")
        .arg(&path)
        .output();
    let _ = std::fs::remove_file(&path);
    match out {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("e2fsck not installed: oracle check skipped");
            Ok(())
        }
        Err(e) => Err(format!("e2fsck failed to run: {}", e)),
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "e2fsck -fn exit {:?}:\n{}",
            o.status.code(),
            String::from_utf8_lossy(&o.stdout)
        )),
    }
}

fn assert_clean(vol: &Volume) {
    let image = vol.image();
    check_image(&image).unwrap_or_else(|e| panic!("image invariant violated: {}", e));
    run_e2fsck(&image).unwrap_or_else(|e| panic!("{}", e));
}

// ─── Helpers for scenarios ───────────────────────────────────────────

const ROOT: u32 = EXT2_ROOT_INO;

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn sb_free_blocks() -> u32 {
    u32::from_le(fs().superblock.s_free_blocks_count)
}

fn sb_free_inodes() -> u32 {
    u32::from_le(fs().superblock.s_free_inodes_count)
}

fn mkfile(dir: u32, name: &str, data: &[u8]) -> u32 {
    create_backing_file(dir, name, data).expect("create file")
}

fn mkdir(dir: u32, name: &str) -> u32 {
    create_backing_dir(dir, name).expect("create dir")
}

fn i_blocks(ino: u32) -> u32 {
    u32::from_le(fs().read_inode(ino).expect("inode").i_blocks)
}

// ─── Scenarios ───────────────────────────────────────────────────────

#[test]
fn fixture_images_are_fsck_clean_before_any_write() {
    for fixture in [FIXTURE_1K, FIXTURE_4K] {
        with_volume(fixture, |vol| assert_clean(vol));
    }
}

#[test]
fn create_small_file_is_fsck_clean_and_counts_sectors() {
    with_volume(FIXTURE_1K, |vol| {
        let ino = mkfile(ROOT, "probe.txt", b"hello\n");
        assert_eq!(read_backing_file(ino).expect("read"), b"hello\n");
        assert_eq!(i_blocks(ino), 2, "one 1 KiB block is two 512-byte sectors");
        assert_clean(vol);
    });
}

#[test]
fn create_13_kib_file_uses_single_indirect() {
    with_volume(FIXTURE_1K, |vol| {
        let data = pattern(13 * KIB, 1);
        let ino = mkfile(ROOT, "ind.bin", &data);
        assert_eq!(read_backing_file(ino).expect("read"), data);
        assert_eq!(i_blocks(ino), (13 + 1) * 2);
        assert_clean(vol);
    });
}

#[test]
fn create_300_kib_file_uses_double_indirect() {
    with_volume(FIXTURE_1K, |vol| {
        let data = pattern(300 * KIB, 2);
        let ino = mkfile(ROOT, "dind.bin", &data);
        assert_eq!(read_backing_file(ino).expect("read"), data);
        assert_eq!(i_blocks(ino), (300 + 1 + 1 + 1) * 2);
        assert_clean(vol);
    });
}

#[test]
fn four_kib_blocks_large_file_roundtrip() {
    with_volume(FIXTURE_4K, |vol| {
        let small = mkfile(ROOT, "s", b"x");
        let data = pattern(5 * MIB, 3);
        let big = mkfile(ROOT, "big", &data);
        assert_eq!(read_backing_file(big).expect("read"), data);
        assert_eq!(read_backing_file(small).expect("read"), b"x");
        assert_clean(vol);
    });
}

#[test]
fn overwrite_with_shorter_content_frees_blocks() {
    with_volume(FIXTURE_1K, |vol| {
        let ino = mkfile(ROOT, "f", &pattern(50 * KIB, 4));
        let before = sb_free_blocks();
        write_backing_file(ino, &pattern(5 * KIB, 5)).expect("overwrite");
        assert_eq!(read_backing_file(ino).expect("read"), pattern(5 * KIB, 5));
        assert_eq!(i_blocks(ino), 5 * 2);
        assert_eq!(sb_free_blocks(), before + 45 + 1, "data and indirect freed");
        assert_clean(vol);
    });
}

#[test]
fn overwrite_with_longer_content_allocates_blocks() {
    with_volume(FIXTURE_1K, |vol| {
        let ino = mkfile(ROOT, "f", &pattern(2 * KIB, 6));
        write_backing_file(ino, &pattern(40 * KIB, 7)).expect("overwrite");
        assert_eq!(read_backing_file(ino).expect("read"), pattern(40 * KIB, 7));
        assert_eq!(i_blocks(ino), (40 + 1) * 2);
        assert_clean(vol);
    });
}

#[test]
fn overwrite_with_empty_content_truncates_to_zero_blocks() {
    with_volume(FIXTURE_1K, |vol| {
        let ino = mkfile(ROOT, "f", &pattern(20 * KIB, 8));
        write_backing_file(ino, b"").expect("truncate");
        assert_eq!(i_blocks(ino), 0);
        assert_clean(vol);
    });
}

#[test]
fn rewriting_same_content_is_idempotent() {
    with_volume(FIXTURE_1K, |vol| {
        let data = pattern(30 * KIB, 9);
        let ino = mkfile(ROOT, "f", &data);
        let (blocks, free) = (i_blocks(ino), sb_free_blocks());
        write_backing_file(ino, &data).expect("rewrite");
        write_backing_file(ino, &data).expect("rewrite again");
        assert_eq!((i_blocks(ino), sb_free_blocks()), (blocks, free));
        assert_eq!(read_backing_file(ino).expect("read"), data);
        assert_clean(vol);
    });
}

#[test]
fn mkdir_starts_with_dot_entries_and_updates_counts() {
    with_volume(FIXTURE_1K, |vol| {
        let root_links = u16::from_le(fs().read_inode(ROOT).expect("root").i_links_count);
        let dir = mkdir(ROOT, "home");
        let after = u16::from_le(fs().read_inode(ROOT).expect("root").i_links_count);
        assert_eq!(after, root_links + 1, "child '..' links the parent");
        assert_eq!(i_blocks(dir), 2);
        assert_clean(vol);
    });
}

#[test]
fn nested_mkdir_with_file_is_fsck_clean() {
    with_volume(FIXTURE_1K, |vol| {
        let home = mkdir(ROOT, "home");
        let user = mkdir(home, "user");
        let proj = mkdir(user, "proj");
        let file = mkfile(proj, "notes.txt", &pattern(3 * KIB, 10));
        assert_eq!(fs().lookup(proj, "notes.txt").expect("lookup"), file);
        assert_eq!(fs().lookup(ROOT, "home").expect("lookup"), home);
        assert_clean(vol);
    });
}

#[test]
fn unlink_file_releases_blocks_and_inode() {
    with_volume(FIXTURE_1K, |vol| {
        let (blocks, inodes) = (sb_free_blocks(), sb_free_inodes());
        let ino = mkfile(ROOT, "gone", &pattern(30 * KIB, 11));
        remove_backing_file(ROOT, "gone").expect("unlink");
        assert_eq!((sb_free_blocks(), sb_free_inodes()), (blocks, inodes));
        let raw = Img::new(&vol.image()).inode(ino);
        assert_ne!(raw.dtime, 0, "deleted inode carries a deletion time");
        assert_eq!(raw.links, 0);
        assert_eq!(fs().lookup(ROOT, "gone"), Err(VfsError::NotFound));
        assert_clean(vol);
    });
}

#[test]
fn rmdir_removes_empty_directory_and_refuses_non_empty() {
    with_volume(FIXTURE_1K, |vol| {
        let (blocks, inodes) = (sb_free_blocks(), sb_free_inodes());
        let dir = mkdir(ROOT, "d");
        mkfile(dir, "inner", b"data");
        assert_eq!(remove_backing_file(ROOT, "d"), Err(VfsError::NotEmpty));
        remove_backing_file(dir, "inner").expect("unlink inner");
        remove_backing_file(ROOT, "d").expect("rmdir");
        assert_eq!((sb_free_blocks(), sb_free_inodes()), (blocks, inodes));
        assert_clean(vol);
    });
}

#[test]
fn rename_within_directory_keeps_inode() {
    with_volume(FIXTURE_1K, |vol| {
        let ino = mkfile(ROOT, "old.txt", b"payload");
        mkfile(ROOT, "other.txt", b"x");
        rename_backing_file(ROOT, "old.txt", "new-longer-name.txt").expect("rename");
        assert_eq!(
            fs().lookup(ROOT, "new-longer-name.txt").expect("lookup"),
            ino
        );
        assert_eq!(fs().lookup(ROOT, "old.txt"), Err(VfsError::NotFound));
        assert_clean(vol);
    });
}

#[test]
fn rename_across_directories_moves_files_and_directories() {
    with_volume(FIXTURE_1K, |vol| {
        let (a, b) = (mkdir(ROOT, "a"), mkdir(ROOT, "b"));
        let file = mkfile(a, "f", b"payload");
        let sub = mkdir(a, "sub");
        rename_backing_entry(a, "f", b, "moved").expect("move file");
        rename_backing_entry(a, "sub", b, "sub2").expect("move dir");
        assert_eq!(fs().lookup(b, "moved").expect("lookup"), file);
        assert_eq!(fs().lookup(b, "sub2").expect("lookup"), sub);
        assert_eq!(fs().lookup(sub, "..").expect("dotdot"), b);
        assert_clean(vol);
    });
}

#[test]
fn rename_onto_existing_name_fails_without_losing_the_source() {
    with_volume(FIXTURE_1K, |vol| {
        mkfile(ROOT, "a", b"1");
        mkfile(ROOT, "b", b"2");
        assert_eq!(
            rename_backing_file(ROOT, "a", "b"),
            Err(VfsError::AlreadyExists)
        );
        assert!(fs().lookup(ROOT, "a").is_ok());
        assert_clean(vol);
    });
}

#[test]
fn directory_grows_past_one_block() {
    with_volume(FIXTURE_1K, |vol| {
        let dir = mkdir(ROOT, "big");
        let names: Vec<String> = (0..60)
            .map(|i| format!("a-rather-long-file-name-{:03}.txt", i))
            .collect();
        for (i, name) in names.iter().enumerate() {
            mkfile(dir, name, format!("file {}", i).as_bytes());
        }
        assert!(i_blocks(dir) >= 4, "needs a second directory block");
        for (i, name) in names.iter().enumerate() {
            let ino = fs().lookup(dir, name).expect("lookup");
            assert_eq!(
                read_backing_file(ino).expect("read"),
                format!("file {}", i).as_bytes()
            );
        }
        for name in names.iter().step_by(3) {
            remove_backing_file(dir, name).expect("unlink");
        }
        mkfile(dir, "reuses-freed-space", b"y");
        assert_clean(vol);
    });
}

#[test]
fn alloc_block_accounts_for_first_data_block() {
    with_volume(FIXTURE_1K, |vol| {
        let first_data = u32::from_le(fs().superblock.s_first_data_block) as u64;
        let image = vol.image();
        let img = Img::new(&image);
        let bitmap = img.gd(0).block_bitmap;
        let first_free_bit = (0..img.bpg as usize)
            .find(|&n| !img.bitmap_bit(bitmap, n))
            .expect("free bit") as u64;
        assert_eq!(
            fs().alloc_block().expect("alloc"),
            first_data + first_free_bit
        );
    });
}

#[test]
fn allocation_reaches_second_half_of_group_bitmap() {
    with_volume(FIXTURE_1K, |vol| {
        for i in 0..9 {
            mkfile(ROOT, &format!("chunk{}", i), &pattern(600 * KIB, i as u8));
        }
        let image = vol.image();
        let img = Img::new(&image);
        let bitmap = img.gd(0).block_bitmap;
        assert!((4096..img.bpg as usize).any(|n| img.bitmap_bit(bitmap, n)));
        assert_eq!(
            read_backing_file(fs().lookup(ROOT, "chunk8").expect("lookup")).expect("read"),
            pattern(600 * KIB, 8)
        );
        assert_clean(vol);
    });
}

#[test]
fn remount_reads_everything_back() {
    with_volume(FIXTURE_1K, |vol| {
        let dir = mkdir(ROOT, "d");
        let data = pattern(70 * KIB, 12);
        mkfile(dir, "f", &data);
        mkfile(ROOT, "g", b"small");
        vol.remount();
        let dir = fs().lookup(ROOT, "d").expect("dir");
        assert_eq!(
            read_backing_file(fs().lookup(dir, "f").expect("f")).expect("read"),
            data
        );
        assert_eq!(
            read_backing_file(fs().lookup(ROOT, "g").expect("g")).expect("read"),
            b"small"
        );
        mkfile(dir, "after-remount", b"ok");
        assert_clean(vol);
    });
}

#[test]
fn lookup_and_unlink_of_missing_name_report_not_found() {
    with_volume(FIXTURE_1K, |vol| {
        assert_eq!(fs().lookup(ROOT, "missing"), Err(VfsError::NotFound));
        assert_eq!(
            remove_backing_file(ROOT, "missing"),
            Err(VfsError::NotFound)
        );
        assert_eq!(
            rename_backing_file(ROOT, "missing", "x"),
            Err(VfsError::NotFound)
        );
        assert_clean(vol);
    });
}

#[test]
fn writing_to_a_full_volume_fails_cleanly() {
    for fixture in [FIXTURE_4K, FIXTURE_1K] {
        with_volume(fixture, |vol| {
            let keep = mkfile(ROOT, "keep", b"survivor");
            let too_big = pattern(24 * MIB, 13);
            assert!(create_backing_file(ROOT, "huge", &too_big).is_err());
            assert_eq!(read_backing_file(keep).expect("read"), b"survivor");
            assert_clean(vol);
        });
    }
}

#[test]
fn journal_never_writes_unowned_blocks() {
    with_volume(FIXTURE_1K, |vol| {
        let before = vol.image();
        let tail = before.len() - 64 * KIB;
        mkfile(ROOT, "f", &pattern(100 * KIB, 14));
        let after = vol.image();
        assert_eq!(
            before[tail..],
            after[tail..],
            "tail of the volume untouched"
        );
    });
}

#[test]
fn inode_writes_preserve_extra_inode_bytes() {
    with_volume(FIXTURE_1K, |vol| {
        let image = vol.image();
        let img = Img::new(&image);
        let table = img.gd(0).inode_table as usize * img.bs;
        let marker = |ino: usize| table + (ino - 1) * img.isz + 128;
        for ino in [12usize, 13] {
            DISK.data.lock().expect("lock")[marker(ino)..marker(ino) + 128].fill(0xA5);
        }
        vol.remount();
        let ino = mkfile(ROOT, "f", b"data");
        assert_eq!(ino, 12);
        let after = vol.image();
        for n in [12usize, 13] {
            assert!(
                after[marker(n)..marker(n) + 128].iter().all(|&b| b == 0xA5),
                "inode {} tail",
                n
            );
        }
    });
}

// ─── VFS-level scenarios (the path the kernel takes at boot) ─────────

use crate::fs::vfs;

const VFS_MOUNT: &str = "/sd";
const HOME_DIR: &str = "/sd/home";
const WALL_CLOCK_SECS: u64 = 1_900_000_000;
const MANY_FILES: usize = 40;
const HUGE_DIR_ENTRIES: usize = 4096;
const REAL_FILES: usize = 500;
const LONG_NAME_LEN: usize = 200;
const TOO_LONG_NAME_LEN: usize = 256;
const GROWTH_DIRS: usize = 30;
const GROWTH_FILES_PER_DIR: usize = 20;

/// Boot the kernel's VFS and mount the test volume at `/sd`, the way
/// `kernel_main` does. Calling it again simulates a reboot.
fn boot_vfs() {
    vfs::init();
    mount(device_index(), 0, VFS_MOUNT).expect("mount ext2 at /sd");
}

/// Sets the wall clock for one test and unsets it again afterwards.
struct ClockGuard;

impl ClockGuard {
    fn set(epoch_secs: u64) -> ClockGuard {
        crate::clock::set_boot_time(epoch_secs, 0, 0);
        ClockGuard
    }
}

impl Drop for ClockGuard {
    fn drop(&mut self) {
        crate::clock::set_boot_time(0, 0, 0);
    }
}

fn file_body(index: usize) -> Vec<u8> {
    format!("content{}\n", index).into_bytes()
}

fn listed_names(path: &str) -> Vec<String> {
    let mut names: Vec<String> = vfs::list_dir(path)
        .expect("list_dir")
        .iter()
        .map(|e| e.name_str().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn vfs_directory_with_40_files_lists_and_reads_all_after_remount() {
    with_volume(FIXTURE_4K, |vol| {
        mkdir(ROOT, "home");
        boot_vfs();
        vfs::mkdir("/sd/home/many").expect("mkdir");
        for i in 0..MANY_FILES {
            let path = format!("/sd/home/many/f{:02}.txt", i);
            vfs::write_file(&path, &file_body(i)).expect("write");
        }
        assert_eq!(
            vfs::list_dir("/sd/home/many").expect("list").len(),
            MANY_FILES
        );
        boot_vfs();
        let entries = vfs::list_dir("/sd/home/many").expect("list after remount");
        assert_eq!(entries.len(), MANY_FILES, "every file listed");
        for entry in &entries {
            let index: usize = entry.name_str()[1..3].parse().expect("index");
            assert_eq!(
                entry.size,
                file_body(index).len() as u64,
                "{}",
                entry.name_str()
            );
        }
        let last = vfs::read_file("/sd/home/many/f39.txt").expect("read 40th");
        assert_eq!(last, file_body(MANY_FILES - 1));
        assert_clean(vol);
    });
}

/// A directory of `HUGE_DIR_ENTRIES` names. The fixtures have only ~1000
/// inodes, so the first `REAL_FILES` entries are distinct files and the rest
/// are hard links to the last of them. Returns the body each entry reads as.
fn build_huge_dir(dir: u32) -> Vec<Vec<u8>> {
    let mut bodies = Vec::new();
    let mut last_ino = 0;
    for i in 0..HUGE_DIR_ENTRIES {
        let name = format!("e{:04}", i);
        if i < REAL_FILES {
            last_ino = mkfile(dir, &name, &file_body(i));
        } else {
            fs().add_dir_entry(dir, last_ino, &name, EXT2_FT_REG_FILE)
                .expect("hard link entry");
        }
        bodies.push(file_body(i.min(REAL_FILES - 1)));
    }
    let extra_links = (HUGE_DIR_ENTRIES - REAL_FILES) as i32;
    fs().adjust_links(last_ino, extra_links)
        .expect("link count");
    bodies
}

#[test]
fn vfs_directory_with_4096_entries_is_fully_reachable_after_remount() {
    with_volume(FIXTURE_1K, |vol| {
        let big = mkdir(ROOT, "big");
        let bodies = build_huge_dir(big);
        boot_vfs();
        let entries = vfs::list_dir("/sd/big").expect("list");
        assert_eq!(entries.len(), HUGE_DIR_ENTRIES);
        for (i, body) in bodies.iter().enumerate() {
            let path = format!("/sd/big/e{:04}", i);
            assert_eq!(&vfs::read_file(&path).expect(&path), body, "{}", path);
        }
        assert_clean(vol);
    });
}

#[test]
fn vfs_200_byte_file_name_roundtrips() {
    with_volume(FIXTURE_4K, |vol| {
        mkdir(ROOT, "home");
        boot_vfs();
        let name = "n".repeat(LONG_NAME_LEN);
        let path = format!("{}/{}", HOME_DIR, name);
        vfs::write_file(&path, b"long").expect("write");
        boot_vfs();
        assert_eq!(listed_names(HOME_DIR), vec![name]);
        assert_eq!(vfs::read_file(&path).expect("read"), b"long");
        assert_clean(vol);
    });
}

#[test]
fn vfs_lists_a_never_traversed_directory_after_remount() {
    with_volume(FIXTURE_4K, |_vol| {
        let home = mkdir(ROOT, "home");
        let cold = mkdir(home, "cold");
        for name in ["a", "b", "c"] {
            mkfile(cold, name, name.as_bytes());
        }
        boot_vfs();
        assert_eq!(listed_names("/sd/home/cold"), ["a", "b", "c"]);
    });
}

#[test]
fn vfs_stat_reports_real_metadata_before_any_open() {
    with_volume(FIXTURE_4K, |_vol| {
        let _clock = ClockGuard::set(WALL_CLOCK_SECS);
        let home = mkdir(ROOT, "home");
        mkfile(home, "sized.bin", &pattern(1234, 3));
        boot_vfs();
        let meta = vfs::stat("/sd/home/sized.bin").expect("stat");
        assert_eq!(meta.size, 1234);
        assert_eq!(meta.file_type, vfs::FileType::RegularFile);
        assert_eq!(meta.permissions, 0o644);
        assert_eq!(meta.modified_time, WALL_CLOCK_SECS);
        let entry = &vfs::list_dir(HOME_DIR).expect("list")[0];
        assert_eq!(entry.size, 1234, "ls -l size without opening the file");
    });
}

#[test]
fn vfs_created_inodes_carry_wall_clock_timestamps() {
    with_volume(FIXTURE_4K, |_vol| {
        mkdir(ROOT, "home");
        boot_vfs();
        let clock = ClockGuard::set(WALL_CLOCK_SECS);
        vfs::mkdir("/sd/home/d").expect("mkdir");
        vfs::write_file("/sd/home/d/f", b"one").expect("create");
        let (dir_ino, file_ino) = backing_inos("/sd/home/d", "/sd/home/d/f");
        let stamp = |ino| {
            let i = fs().read_inode(ino).expect("inode");
            (i.i_atime, i.i_ctime, i.i_mtime)
        };
        let now = WALL_CLOCK_SECS as u32;
        assert_eq!(stamp(file_ino), (now, now, now), "new file");
        assert_eq!(stamp(dir_ino), (now, now, now), "new directory");
        crate::clock::set_boot_time(WALL_CLOCK_SECS + 100, 0, 0);
        vfs::write_file("/sd/home/d/f", b"two!").expect("rewrite");
        let (atime, ctime, mtime) = stamp(file_ino);
        assert_eq!(
            (atime, ctime, mtime),
            (now, now + 100, now + 100),
            "rewrite"
        );
        drop(clock);
    });
}

#[test]
fn vfs_timestamps_stay_zero_without_a_clock() {
    with_volume(FIXTURE_4K, |_vol| {
        mkdir(ROOT, "home");
        boot_vfs();
        vfs::write_file("/sd/home/f", b"x").expect("create");
        let (_, file_ino) = backing_inos(HOME_DIR, "/sd/home/f");
        let inode = fs().read_inode(file_ino).expect("inode");
        assert_eq!((inode.i_ctime, inode.i_mtime), (0, 0));
    });
}

fn backing_inos(dir: &str, file: &str) -> (u32, u32) {
    let ino = |path: &str| {
        vfs::get_node(vfs::resolve_path(path).expect("resolve"))
            .expect("node")
            .backing_ino
    };
    (ino(dir), ino(file))
}

#[test]
fn vfs_node_table_grows_past_the_old_128_node_limit() {
    with_volume(FIXTURE_4K, |vol| {
        mkdir(ROOT, "home");
        boot_vfs();
        for d in 0..GROWTH_DIRS {
            vfs::mkdir(&format!("/sd/home/d{:02}", d)).expect("mkdir");
            for f in 0..GROWTH_FILES_PER_DIR {
                let path = format!("/sd/home/d{:02}/f{:02}", d, f);
                vfs::write_file(&path, &file_body(d * 100 + f)).expect(&path);
            }
        }
        boot_vfs();
        for d in 0..GROWTH_DIRS {
            let names = listed_names(&format!("/sd/home/d{:02}", d));
            assert_eq!(names.len(), GROWTH_FILES_PER_DIR, "dir d{:02}", d);
        }
        let last = format!(
            "/sd/home/d{:02}/f{:02}",
            GROWTH_DIRS - 1,
            GROWTH_FILES_PER_DIR - 1
        );
        assert_eq!(vfs::read_file(&last).expect("read"), file_body(2919));
        assert_clean(vol);
    });
}

#[test]
fn vfs_errors_are_reported_not_swallowed() {
    with_volume(FIXTURE_4K, |vol| {
        mkdir(ROOT, "home");
        boot_vfs();
        assert_eq!(
            vfs::write_file("/sd/home/missing/f", b"x"),
            Err(vfs::VfsError::NotFound)
        );
        let too_long = format!("{}/{}", HOME_DIR, "n".repeat(TOO_LONG_NAME_LEN));
        assert_eq!(
            vfs::write_file(&too_long, b"x"),
            Err(vfs::VfsError::InvalidPath)
        );
        vfs::mkdir("/sd/home/once").expect("first mkdir");
        assert_eq!(
            vfs::mkdir("/sd/home/once"),
            Err(vfs::VfsError::AlreadyExists)
        );
        assert_eq!(listed_names(HOME_DIR), ["once"]);
        assert_clean(vol);
    });
}
