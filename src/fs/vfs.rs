//! Virtual Filesystem Layer — abstract inode-based filesystem interface
//!
//! Nodes live in a growable table. Every node is its own leaked heap box, so
//! the `&'static VfsNode` references handed out by [`get_node`] stay valid
//! while the table itself grows and reallocates. Directory contents are
//! lists of `(name, node id)` pairs sized by the heap, not by a fixed cap.

use crate::ipc::{O_CLOEXEC, O_NONBLOCK};
use crate::println;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Maximum filename length in bytes (the ext2 limit).
pub const MAX_NAME_LEN: usize = 255;

/// Maximum open files system-wide
const MAX_OPEN_FILES: usize = 32;

/// fcntl command constants (matching POSIX/Linux values)
const F_DUPFD: u32 = 0;
const F_GETFD: u32 = 1;
const F_SETFD: u32 = 2;
const F_GETFL: u32 = 3;
const F_SETFL: u32 = 4;

/// File types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    RegularFile,
    Directory,
    Device,
    Symlink,
}

/// VFS error types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VfsError {
    NotFound,
    NotADirectory,
    NotAFile,
    PermissionDenied,
    IOError,
    EndOfFile,
    InvalidOffset,
    AlreadyExists,
    NotEmpty,
    Busy,
    InvalidPath,
    OutOfMemory,
    ReadOnly,
}

impl core::fmt::Display for VfsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VfsError::NotFound => write!(f, "not found"),
            VfsError::NotADirectory => write!(f, "not a directory"),
            VfsError::NotAFile => write!(f, "not a file"),
            VfsError::PermissionDenied => write!(f, "permission denied"),
            VfsError::IOError => write!(f, "I/O error"),
            VfsError::EndOfFile => write!(f, "end of file"),
            VfsError::InvalidOffset => write!(f, "invalid offset"),
            VfsError::AlreadyExists => write!(f, "already exists"),
            VfsError::NotEmpty => write!(f, "directory not empty"),
            VfsError::Busy => write!(f, "device busy"),
            VfsError::InvalidPath => write!(f, "invalid path"),
            VfsError::OutOfMemory => write!(f, "out of memory"),
            VfsError::ReadOnly => write!(f, "read only"),
        }
    }
}

/// File metadata
#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    pub size: u64,
    pub file_type: FileType,
    pub permissions: u32,
    pub modified_time: u64,
    pub inode_id: u64,
    /// Owner user ID
    pub uid: u32,
    /// Owner group ID
    pub gid: u32,
}

/// Directory entry as returned by `list_dir`
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub file_type: FileType,
    pub inode_id: u64,
    /// Convenience: file size copied from metadata at enumeration time
    pub size: u64,
    /// Convenience: owner uid
    pub uid: u32,
    /// Convenience: owner gid
    pub gid: u32,
    /// Convenience: true if directory
    pub is_dir: bool,
}

impl DirEntry {
    pub const fn empty() -> Self {
        Self {
            name: String::new(),
            file_type: FileType::RegularFile,
            inode_id: 0,
            size: 0,
            uid: 0,
            gid: 0,
            is_dir: false,
        }
    }

    pub fn set_name(&mut self, s: &str) {
        self.name.clear();
        self.name.push_str(s);
    }

    pub fn name_str(&self) -> &str {
        &self.name
    }
}

/// Maximum inline file data size (256KB per file — supports libc-linked binaries)
pub const MAX_FILE_DATA: usize = 524288;

/// A child link inside a directory node: the name it is listed under and the
/// id of the node it leads to.
struct Child {
    name: String,
    id: u64,
}

/// VFS Node — represents a file, directory, device, or symlink
pub struct VfsNode {
    pub id: u64,
    name: String,
    pub metadata: Metadata,
    pub fs_id: u32, // Which filesystem owns this node
    /// Backing-store routing (ext2): filesystem id + ext2 inode. When
    /// backing_fs != 0, reads/writes are served by that filesystem lazily
    /// instead of the inline data buffer.
    pub backing_fs: u32,
    pub backing_ino: u32,
    /// Files: true once the content has been hydrated from the backing store
    /// into the inline buffer (subsequent writes go through + stay cached).
    /// Directories: true once their backing-store children were materialized.
    pub hydrated: bool,
    /// True while the inline buffer holds changes the backing store has not
    /// seen yet; only dirty files are written back (a read must not bump the
    /// file's mtime).
    pub dirty: bool,
    pub parent_id: u64,
    /// For directories: the children, in on-disk / creation order.
    children: Vec<Child>,
    /// For regular files: heap-backed data storage.
    /// `data_ptr` is null until the file is written; the buffer is lazily
    /// allocated so directories and empty nodes cost nothing.
    pub data_ptr: *mut u8,
    pub data_cap: usize,
    pub data_len: u64,
}

impl VfsNode {
    pub const fn empty(id: u64) -> Self {
        Self {
            id,
            name: String::new(),
            metadata: Metadata {
                size: 0,
                file_type: FileType::RegularFile,
                permissions: 0,
                modified_time: 0,
                inode_id: 0,
                uid: 0,
                gid: 0,
            },
            fs_id: 0,
            backing_fs: 0,
            backing_ino: 0,
            hydrated: false,
            dirty: false,
            parent_id: 0,
            children: Vec::new(),
            data_ptr: core::ptr::null_mut(),
            data_cap: 0,
            data_len: 0,
        }
    }

    pub fn set_name(&mut self, s: &str) {
        self.name.clear();
        self.name.push_str(s);
    }

    pub fn name_str(&self) -> &str {
        &self.name
    }

    /// Number of children (0 for anything but a directory).
    pub fn dir_count(&self) -> usize {
        self.children.len()
    }

    /// Current file contents (length = data_len). Returns empty slice
    /// when the file has not yet been written.
    pub fn data(&self) -> &[u8] {
        if self.data_ptr.is_null() {
            return &[];
        }
        // SAFETY: data_ptr/data_len describe the live heap buffer that
        // ensure_capacity allocated; data_len never exceeds data_cap.
        unsafe { core::slice::from_raw_parts(self.data_ptr, self.data_len as usize) }
    }

    /// Ensure the backing buffer can hold at least `cap` bytes, growing it
    /// on the heap if needed. Content already present is preserved.
    fn ensure_capacity(&mut self, cap: usize) -> Result<(), VfsError> {
        if cap <= self.data_cap {
            return Ok(());
        }
        // Lazily allocate and copy from the previous buffer (if any).
        let new_cap = cap; // exact-fit; callers already bound writes to MAX_FILE_DATA
        let mut new_box = alloc::vec![0u8; new_cap].into_boxed_slice();
        if !self.data_ptr.is_null() {
            let old_len = (self.data_len as usize).min(new_cap);
            // SAFETY: data_ptr is the live buffer and old_len <= data_len <= data_cap.
            let old = unsafe { core::slice::from_raw_parts(self.data_ptr, old_len) };
            new_box[..old_len].copy_from_slice(old);
            let kept_len = self.data_len;
            self.release_data();
            self.data_len = kept_len;
        }
        self.data_ptr = alloc::boxed::Box::into_raw(new_box) as *mut u8;
        self.data_cap = new_cap;
        Ok(())
    }

    /// Free the data buffer (if any). The recorded length is reset too.
    fn release_data(&mut self) {
        if self.data_ptr.is_null() {
            return;
        }
        // SAFETY: data_ptr/data_cap came from Box::<[u8]>::into_raw in
        // ensure_capacity and have not been freed since (data_ptr is nulled
        // right below).
        drop(unsafe {
            alloc::boxed::Box::<[u8]>::from_raw(core::ptr::slice_from_raw_parts_mut(
                self.data_ptr,
                self.data_cap,
            ))
        });
        self.data_ptr = core::ptr::null_mut();
        self.data_cap = 0;
        self.data_len = 0;
    }

    /// Replace the entire file contents with `src` (used by initrd loader,
    /// symlink targets, and on-demand writers).
    pub fn set_data(&mut self, src: &[u8]) -> Result<(), VfsError> {
        if src.len() > MAX_FILE_DATA {
            return Err(VfsError::OutOfMemory);
        }
        self.ensure_capacity(src.len())?;
        if !self.data_ptr.is_null() {
            // SAFETY: ensure_capacity guarantees data_cap >= src.len().
            let dst = unsafe { core::slice::from_raw_parts_mut(self.data_ptr, src.len()) };
            dst.copy_from_slice(src);
        }
        self.data_len = src.len() as u64;
        self.metadata.size = src.len() as u64;
        Ok(())
    }

    pub fn read(&self, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
        if self.metadata.file_type != FileType::RegularFile {
            return Err(VfsError::NotAFile);
        }
        if offset >= self.data_len {
            return Err(VfsError::EndOfFile);
        }
        let remaining = (self.data_len - offset) as usize;
        let to_read = buf.len().min(remaining);
        let src_start = offset as usize;
        buf[..to_read].copy_from_slice(&self.data()[src_start..src_start + to_read]);
        Ok(to_read)
    }

    pub fn write(&mut self, offset: u64, buf: &[u8]) -> Result<usize, VfsError> {
        if self.metadata.file_type != FileType::RegularFile {
            return Err(VfsError::NotAFile);
        }
        let offset_usize = offset as usize;
        let available = MAX_FILE_DATA.saturating_sub(offset_usize);
        let to_write = buf.len().min(available);
        if to_write == 0 {
            return Err(VfsError::OutOfMemory);
        }
        self.ensure_capacity(offset_usize + to_write)?;
        if !self.data_ptr.is_null() {
            // SAFETY: ensure_capacity guarantees data_cap >= offset + to_write.
            let dst =
                unsafe { core::slice::from_raw_parts_mut(self.data_ptr, offset_usize + to_write) };
            dst[offset_usize..offset_usize + to_write].copy_from_slice(&buf[..to_write]);
        }
        self.data_len = self.data_len.max(offset + to_write as u64);
        self.metadata.size = self.data_len;
        self.dirty = true;
        Ok(to_write)
    }

    /// Link child node `id` into this directory under `name`. Fails with
    /// `AlreadyExists` for a duplicate name and `OutOfMemory` when the child
    /// list cannot grow; a failed call changes nothing.
    pub fn add_child(&mut self, name: &str, id: u64) -> Result<(), VfsError> {
        if self.metadata.file_type != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }
        if self.find_child(name).is_some() {
            return Err(VfsError::AlreadyExists);
        }
        self.push_child(name, id)
    }

    /// Append a child without the duplicate scan (the caller knows the name
    /// is new, e.g. while materializing a directory from disk).
    fn push_child(&mut self, name: &str, id: u64) -> Result<(), VfsError> {
        self.children
            .try_reserve(1)
            .map_err(|_| VfsError::OutOfMemory)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(name.len())
            .map_err(|_| VfsError::OutOfMemory)?;
        owned.push_str(name);
        self.children.push(Child { name: owned, id });
        Ok(())
    }

    /// Unlink the child called `name`; keeps the order of the others.
    fn remove_child(&mut self, name: &str) -> bool {
        match self.children.iter().position(|c| c.name == name) {
            Some(index) => {
                self.children.remove(index);
                true
            }
            None => false,
        }
    }

    pub fn find_child(&self, name: &str) -> Option<u64> {
        if self.metadata.file_type != FileType::Directory {
            return None;
        }
        self.children.iter().find(|c| c.name == name).map(|c| c.id)
    }
}

/// FileSystem trait — interface that concrete filesystems implement
pub trait FileSystem {
    fn name(&self) -> &str;
    fn fs_id(&self) -> u32;
    fn root_inode_id(&self) -> u64;
    fn read_node(&self, inode_id: u64, buf: &mut [u8], offset: u64) -> Result<usize, VfsError>;
    fn write_node(&self, inode_id: u64, buf: &[u8], offset: u64) -> Result<usize, VfsError>;
    fn get_metadata(&self, inode_id: u64) -> Result<Metadata, VfsError>;
}

/// Mount point entry
#[derive(Copy, Clone)]
struct MountPoint {
    path: [u8; 256],
    path_len: usize,
    fs_id: u32,
    root_inode: u64,
}

impl MountPoint {
    const fn empty() -> Self {
        Self {
            path: [0u8; 256],
            path_len: 0,
            fs_id: 0,
            root_inode: 0,
        }
    }
}

/// Maximum mount points
const MAX_MOUNT_POINTS: usize = 8;

/// Every node ever allocated, indexed by node id. Entries point at leaked
/// heap boxes and are never freed or moved: the table may reallocate, the
/// nodes it points to may not. Removed nodes stay as tombstones so ids are
/// never reused (a stale open fd cannot alias a newer file).
static mut NODE_TABLE: Vec<*mut VfsNode> = Vec::new();
static mut MOUNT_POINTS: [MountPoint; MAX_MOUNT_POINTS] = [MountPoint::empty(); MAX_MOUNT_POINTS];
static mut MOUNT_COUNT: usize = 0;
static mut NEXT_FS_ID: u32 = 1;

/// Test hook: refuse to grow the node table past this many nodes, to drive
/// the out-of-memory path deterministically.
#[cfg(test)]
pub(crate) static NODE_LIMIT: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(usize::MAX);

/// File handle for open files
pub struct FileHandle {
    pub inode_id: u64,
    pub offset: u64,
    pub flags: u32,
}

/// Open flags
pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_CREAT: u32 = 0x40;
pub const O_TRUNC: u32 = 0x200;
pub const O_APPEND: u32 = 0x400;

/// Global open file table
static mut OPEN_FILES: [Option<FileHandle>; MAX_OPEN_FILES] = [const { None }; MAX_OPEN_FILES];
static mut NEXT_FD: usize = 0;

/// Permission bits of a freshly created directory / file.
const DEFAULT_DIR_MODE: u32 = 0o755;
const DEFAULT_FILE_MODE: u32 = 0o644;

/// Directories every fresh boot gets (the VFS is volatile until a persistent
/// disk is mounted).
const STANDARD_DIRS: [&str; 7] = ["bin", "etc", "tmp", "root", "home", "var", "usr"];

/// Current wall-clock time in epoch seconds; 0 until a clock has been set.
pub(crate) fn now_secs() -> u32 {
    #[cfg(feature = "std")]
    let uptime_ms = 0;
    #[cfg(not(feature = "std"))]
    let uptime_ms = crate::arch::timer::Timer::uptime_ms();
    crate::clock::epoch_secs_at(uptime_ms).map_or(0, |secs| secs as u32)
}

/// Metadata for a node that has no backing inode yet.
fn default_metadata(file_type: FileType, id: u64) -> Metadata {
    let permissions = if file_type == FileType::Directory {
        DEFAULT_DIR_MODE
    } else {
        DEFAULT_FILE_MODE
    };
    Metadata {
        size: 0,
        file_type,
        permissions,
        modified_time: u64::from(now_secs()),
        inode_id: id,
        uid: 0,
        gid: 0,
    }
}

/// Initialize the VFS
pub fn init() {
    crate::fs::virtual_fs::reset();
    // SAFETY: boot-time, single-threaded. Old nodes (a second init() only
    // happens in host tests) are deliberately leaked, never freed, because
    // `&'static VfsNode` references to them may still exist.
    unsafe {
        NODE_TABLE.clear();
        MOUNT_COUNT = 0;
        NEXT_FS_ID = 1;
    }
    let mut root = VfsNode::empty(0);
    root.set_name("/");
    root.metadata = default_metadata(FileType::Directory, 0);
    if push_node(root).is_err() {
        println!("[vfs] out of memory creating the root node");
        return;
    }
    for dir in STANDARD_DIRS {
        if let Err(e) = alloc_node(dir, FileType::Directory, 0) {
            println!("[vfs] cannot create /{}: {}", dir, e);
        }
    }
    println!("[vfs] Virtual filesystem initialized (root + /bin /etc /tmp /root /home /var /usr)");
}

/// Number of nodes allocated so far (tombstones included).
fn node_count() -> usize {
    // SAFETY: the table is only mutated from task context, never concurrently.
    unsafe { NODE_TABLE.len() }
}

/// Put `node` on the heap, give it the next id and register it in the table.
fn push_node(mut node: VfsNode) -> Result<u64, VfsError> {
    #[cfg(test)]
    if node_count() >= NODE_LIMIT.load(core::sync::atomic::Ordering::Relaxed) {
        return Err(VfsError::OutOfMemory);
    }
    // SAFETY: only called from task context; no reference into the Vec
    // itself is held across this call (nodes are reached through raw pointers).
    let table = unsafe { &mut *core::ptr::addr_of_mut!(NODE_TABLE) };
    table.try_reserve(1).map_err(|_| VfsError::OutOfMemory)?;
    let id = table.len() as u64;
    node.id = id;
    node.metadata.inode_id = id;
    let layout = core::alloc::Layout::new::<VfsNode>();
    // SAFETY: VfsNode is not zero-sized, so the layout is valid for alloc.
    let ptr = unsafe { alloc::alloc::alloc(layout) } as *mut VfsNode;
    if ptr.is_null() {
        return Err(VfsError::OutOfMemory);
    }
    // SAFETY: ptr is non-null, aligned and large enough for a VfsNode, and
    // holds no value yet, so writing (not assigning) is correct.
    unsafe { ptr.write(node) };
    table.push(ptr);
    Ok(id)
}

/// Allocate a new VFS node with default metadata and link it into `parent_id`.
pub fn alloc_node(name: &str, file_type: FileType, parent_id: u64) -> Result<u64, VfsError> {
    alloc_node_with(name, file_type, parent_id, default_metadata(file_type, 0))
}

/// Allocate a node carrying `metadata` (as read from a backing store) and
/// link it into `parent_id`. Nothing is left behind when it fails: a bad
/// name, a missing or non-directory parent, a duplicate name or memory
/// exhaustion are all reported to the caller.
pub fn alloc_node_with(
    name: &str,
    file_type: FileType,
    parent_id: u64,
    metadata: Metadata,
) -> Result<u64, VfsError> {
    create_child(name, file_type, parent_id, metadata, true)
}

/// Shared body of node creation. `check_duplicates` is false only while a
/// directory is being materialized from disk, where names are known unique.
fn create_child(
    name: &str,
    file_type: FileType,
    parent_id: u64,
    metadata: Metadata,
    check_duplicates: bool,
) -> Result<u64, VfsError> {
    check_name(name)?;
    let parent = get_node(parent_id).ok_or(VfsError::NotFound)?;
    if parent.metadata.file_type != FileType::Directory {
        return Err(VfsError::NotADirectory);
    }
    if check_duplicates && parent.find_child(name).is_some() {
        return Err(VfsError::AlreadyExists);
    }
    link_new_node(name, file_type, parent_id, metadata)
}

/// Create the node and its parent link, undoing the node on link failure.
fn link_new_node(
    name: &str,
    file_type: FileType,
    parent_id: u64,
    metadata: Metadata,
) -> Result<u64, VfsError> {
    let mut node = VfsNode::empty(0);
    node.metadata = Metadata {
        file_type,
        ..metadata
    };
    node.parent_id = parent_id;
    node.name
        .try_reserve_exact(name.len())
        .map_err(|_| VfsError::OutOfMemory)?;
    node.set_name(name);
    let id = push_node(node)?;
    let parent = get_node_mut(parent_id).ok_or(VfsError::NotFound)?;
    if let Err(e) = parent.push_child(name, id) {
        retire_node(id);
        return Err(e);
    }
    Ok(id)
}

/// Reject names the VFS cannot store: empty, too long, or containing '/'.
fn check_name(name: &str) -> Result<(), VfsError> {
    if name.is_empty() || name.len() > MAX_NAME_LEN || name.contains('/') {
        return Err(VfsError::InvalidPath);
    }
    Ok(())
}

/// Turn a node into a tombstone: free its data and forget its identity. The
/// id stays allocated so it is never handed out again.
fn retire_node(id: u64) {
    if let Some(node) = get_node_mut(id) {
        node.release_data();
        node.children = Vec::new();
        node.name = String::new();
        node.metadata.file_type = FileType::RegularFile;
        node.metadata.size = 0;
        node.backing_fs = 0;
        node.backing_ino = 0;
    }
}

/// Get a VFS node by ID
pub fn get_node(id: u64) -> Option<&'static VfsNode> {
    // SAFETY: every table entry is a leaked heap box that is never freed or
    // moved (see NODE_TABLE), so the reference is valid for 'static even if
    // the table reallocates afterwards.
    unsafe {
        let ptr = *NODE_TABLE.get(id as usize)?;
        Some(&*ptr)
    }
}

/// Get a mutable VFS node by ID
pub fn get_node_mut(id: u64) -> Option<&'static mut VfsNode> {
    // SAFETY: as for get_node. Callers uphold the existing single-writer
    // discipline of the VFS (kernel task context, no concurrent mutation).
    unsafe {
        let ptr = *NODE_TABLE.get(id as usize)?;
        Some(&mut *ptr)
    }
}

/// The default user's home directory.
pub const HOME_PATH: &str = "/home/user";

/// Ensure the default user's home exists and holds a welcome file. With a
/// disk the home is persistent (the prefix map sends `/home` to `/sd/home`);
/// without one it is an ordinary RAM directory.
pub fn bootstrap_home() {
    let persistent = crate::fs::prefix_map::is_active("/home");
    if resolve_path(HOME_PATH).is_err() {
        for dir in ["/home", HOME_PATH] {
            if resolve_path(dir).is_err() {
                if let Err(e) = create_dir(dir) {
                    println!("[vfs] mkdir {} failed: {:?}", dir, e);
                }
            }
        }
    }
    let welcome = "/home/user/welcome.txt";
    if resolve_path(welcome).is_err() {
        let _ = write_file(
            welcome,
            b"Welcome to LobsterOS!\nThis is your home directory (/home/user).\nFiles saved here persist across reboots when a disk is attached.\n",
        );
    }
    println!(
        "[vfs] home: {} ({})",
        HOME_PATH,
        if persistent { "on disk" } else { "RAM only" }
    );
}

/// Resolve a path to a VFS node ID. Every backing-store directory walked
/// through is expanded first; the final node is returned as it is.
pub fn resolve_path(path: &str) -> Result<u64, VfsError> {
    let path = &*crate::fs::prefix_map::rewrite(path);
    crate::fs::virtual_fs::refresh_for(path);
    if path.is_empty() {
        return Err(VfsError::InvalidPath);
    }
    let mut current_id = 0u64;
    for part in path.split('/').filter(|part| !part.is_empty()) {
        ensure_expanded(current_id)?;
        let node = get_node(current_id).ok_or(VfsError::NotFound)?;
        if node.metadata.file_type != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }
        current_id = node.find_child(part).ok_or(VfsError::NotFound)?;
    }
    Ok(current_id)
}

/// Split `path` into its parent directory path and final component.
fn split_parent(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => (&path[..idx.max(1)], &path[idx + 1..]),
        None => ("", path),
    }
}

/// Resolve the directory that holds (or will hold) `path`'s last component
/// and make sure its children are loaded. Returns the directory's node id
/// and the component's name. `path` must already be alias-rewritten.
fn resolve_parent(path: &str) -> Result<(u64, &str), VfsError> {
    let (parent_path, name) = split_parent(path);
    let parent_id = if parent_path.is_empty() || parent_path == "/" {
        0u64
    } else {
        resolve_path(parent_path)?
    };
    ensure_expanded(parent_id)?;
    Ok((parent_id, name))
}

/// Backing-store inode and fs id of a node, when it has one.
fn backing_of(node_id: u64) -> Option<(u32, u32)> {
    let node = get_node(node_id)?;
    if node.backing_ino == 0 || node.backing_fs == 0 {
        return None;
    }
    Some((node.backing_ino, node.backing_fs))
}

/// Remove `name` from its parent's child list and retire the node.
fn detach_node(parent_id: u64, name: &str, node_id: u64) {
    if let Some(parent) = get_node_mut(parent_id) {
        parent.remove_child(name);
    }
    retire_node(node_id);
}

/// Create a directory (also in the ext2 backing store when the parent is
/// ext2-backed, so it survives reboots like file write-through does).
pub fn mkdir(path: &str) -> Result<u64, VfsError> {
    let path = &*crate::fs::prefix_map::rewrite(path);
    let (parent_id, name) = resolve_parent(path)?;
    let node_id = alloc_node(name, FileType::Directory, parent_id)?;
    if let Err(e) = persist_new_dir(parent_id, node_id, name) {
        detach_node(parent_id, name, node_id);
        return Err(e);
    }
    Ok(node_id)
}

/// Create the ext2 directory behind a new VFS directory node.
fn persist_new_dir(parent_id: u64, node_id: u64, name: &str) -> Result<(), VfsError> {
    let Some((parent_ino, fs_id)) = backing_of(parent_id) else {
        return Ok(());
    };
    let new_ino = crate::fs::ext2::create_backing_dir(parent_ino, name)?;
    set_node_backing(node_id, fs_id, new_ino);
    // A directory that was just created has nothing on disk to expand.
    if let Some(node) = get_node_mut(node_id) {
        node.hydrated = true;
    }
    Ok(())
}

/// Create a file; an existing regular file is returned as it is.
pub fn create(path: &str) -> Result<u64, VfsError> {
    let path = &*crate::fs::prefix_map::rewrite(path);
    let (parent_id, name) = resolve_parent(path)?;
    let parent = get_node(parent_id).ok_or(VfsError::NotFound)?;
    match parent.find_child(name).and_then(get_node) {
        Some(existing) if existing.metadata.file_type == FileType::RegularFile => Ok(existing.id),
        Some(_) => Err(VfsError::AlreadyExists),
        None => alloc_node(name, FileType::RegularFile, parent_id),
    }
}

/// Permission bits `open` must check for the access mode in `flags`.
fn open_access_mask(flags: u32) -> u32 {
    let read = crate::security::PERM_OWNER_READ
        | crate::security::PERM_GROUP_READ
        | crate::security::PERM_OTHERS_READ;
    let write = crate::security::PERM_OWNER_WRITE
        | crate::security::PERM_GROUP_WRITE
        | crate::security::PERM_OTHERS_WRITE;
    match flags & 0x3 {
        O_RDONLY => read,
        O_WRONLY => write,
        O_RDWR => read | write,
        _ => 0,
    }
}

/// Create the file an `open(O_CREAT)` asked for, if the parent allows it.
fn create_for_open(path: &str) -> Result<u64, VfsError> {
    let (parent_id, _) = resolve_parent(path)?;
    if let Some(parent) = get_node(parent_id) {
        let write_access = crate::security::PERM_OWNER_WRITE
            | crate::security::PERM_GROUP_WRITE
            | crate::security::PERM_OTHERS_WRITE;
        if !check_permission(parent, write_access) {
            return Err(VfsError::PermissionDenied);
        }
    }
    create(path)
}

/// Open a file (resolve path, return file descriptor)
pub fn open(path: &str, flags: u32) -> Result<usize, VfsError> {
    let path = &*crate::fs::prefix_map::rewrite(path);
    let inode_id = match resolve_path(path) {
        Ok(id) => id,
        Err(VfsError::NotFound) if flags & O_CREAT != 0 => create_for_open(path)?,
        Err(e) => return Err(e),
    };
    if let Some(node) = get_node(inode_id) {
        if !check_permission(node, open_access_mask(flags)) {
            return Err(VfsError::PermissionDenied);
        }
    }
    open_handle(inode_id, flags)
}

/// Claim a free slot in the open-file table for `inode_id`.
fn open_handle(inode_id: u64, flags: u32) -> Result<usize, VfsError> {
    // SAFETY: the open-file table is only touched from kernel task context.
    let slot = unsafe { OPEN_FILES.iter().position(|f| f.is_none()) };
    let slot = slot.ok_or(VfsError::OutOfMemory)?;
    // Lazy-load content from the ext2 backing store on first open
    // so reads see the persistent file.
    let _ = hydrate_node(inode_id);
    let node = get_node_mut(inode_id).ok_or(VfsError::NotFound)?;
    // O_APPEND starts at end-of-file so `>> file` extends it.
    let offset = if flags & O_APPEND != 0 {
        node.data_len
    } else {
        0
    };
    if flags & O_TRUNC != 0 {
        node.data_len = 0;
        node.metadata.size = 0;
        node.dirty = true;
    }
    // SAFETY: as above; `slot` was just found free.
    unsafe {
        OPEN_FILES[slot] = Some(FileHandle {
            inode_id,
            offset,
            flags,
        });
    }
    Ok(slot)
}

/// Close a file descriptor
pub fn close(fd: usize) -> Result<(), VfsError> {
    // Write modified hydrated content back to the ext2 backing store so it
    // persists across reboots.
    unsafe {
        if fd < MAX_OPEN_FILES {
            if let Some(handle) = OPEN_FILES[fd].as_ref() {
                let _ = writeback_node(handle.inode_id);
            }
        }
    }

    unsafe {
        if fd < MAX_OPEN_FILES {
            OPEN_FILES[fd] = None;
            Ok(())
        } else {
            Err(VfsError::InvalidOffset)
        }
    }
}

/// Read from a file descriptor
pub fn read(fd: usize, buf: &mut [u8]) -> Result<usize, VfsError> {
    unsafe {
        if fd >= MAX_OPEN_FILES {
            return Err(VfsError::InvalidOffset);
        }
        let handle = OPEN_FILES[fd].as_ref().ok_or(VfsError::InvalidOffset)?;
        let inode_id = handle.inode_id;
        // Check read permission
        if let Some(node) = get_node(inode_id) {
            let read_access = crate::security::PERM_OWNER_READ
                | crate::security::PERM_GROUP_READ
                | crate::security::PERM_OTHERS_READ;
            if !check_permission(node, read_access) {
                return Err(VfsError::PermissionDenied);
            }
        }
        let node = get_node(inode_id).ok_or(VfsError::NotFound)?;
        let bytes_read = node.read(handle.offset, buf)?;
        // Update offset (need to reconstruct since we can't hold mutable refs)
        if let Some(h) = OPEN_FILES[fd].as_mut() {
            h.offset += bytes_read as u64;
        }
        Ok(bytes_read)
    }
}

/// Write to a file descriptor
pub fn write(fd: usize, buf: &[u8]) -> Result<usize, VfsError> {
    unsafe {
        if fd >= MAX_OPEN_FILES {
            return Err(VfsError::InvalidOffset);
        }
        let handle = OPEN_FILES[fd].as_ref().ok_or(VfsError::InvalidOffset)?;
        let inode_id = handle.inode_id;
        let offset = handle.offset;
        // Check write permission
        if let Some(node) = get_node(inode_id) {
            let write_access = crate::security::PERM_OWNER_WRITE
                | crate::security::PERM_GROUP_WRITE
                | crate::security::PERM_OTHERS_WRITE;
            if !check_permission(node, write_access) {
                return Err(VfsError::PermissionDenied);
            }
        }
        let node = get_node_mut(inode_id).ok_or(VfsError::NotFound)?;
        let written = node.write(offset, buf)?;
        if let Some(h) = OPEN_FILES[fd].as_mut() {
            h.offset += written as u64;
        }
        Ok(written)
    }
}

/// Seek to position in file
pub fn seek(fd: usize, offset: i64, whence: u32) -> Result<u64, VfsError> {
    unsafe {
        if fd >= MAX_OPEN_FILES {
            return Err(VfsError::InvalidOffset);
        }
        let handle = OPEN_FILES[fd].as_mut().ok_or(VfsError::InvalidOffset)?;
        let new_offset = match whence {
            0 => offset as u64,                             // SEEK_SET
            1 => handle.offset.wrapping_add(offset as u64), // SEEK_CUR
            2 => {
                let node = get_node(handle.inode_id).ok_or(VfsError::NotFound)?;
                node.data_len.wrapping_add(offset as u64) // SEEK_END
            }
            _ => return Err(VfsError::InvalidOffset),
        };
        handle.offset = new_offset;
        Ok(new_offset)
    }
}

/// If `path` is inside the host shared folder (/share or /share/...),
/// return the share-relative remainder ("" for the root).
fn share_rel_path(path: &str) -> Option<&str> {
    if path == "/share" {
        return Some("");
    }
    path.strip_prefix("/share/")
}

/// List the host shared folder directory `rel` (relative to /share).
fn list_share_dir(rel: &str) -> Result<Vec<DirEntry>, VfsError> {
    if !crate::drivers::lobster_share::detect() {
        return Err(VfsError::NotFound);
    }
    let mut entries = Vec::new();
    crate::drivers::lobster_share::list_dir(rel, |name| {
        let child_rel = if rel.is_empty() {
            name.to_string()
        } else {
            alloc::format!("{}/{}", rel, name)
        };
        let mut de = DirEntry::empty();
        de.set_name(name);
        de.size = crate::drivers::lobster_share::stat(&child_rel).unwrap_or(0);
        entries.push(de);
    })
    .map_err(|_| VfsError::NotFound)?;
    Ok(entries)
}

/// Copy a name into a fresh `String`, reporting allocation failure.
fn try_string(s: &str) -> Result<String, VfsError> {
    let mut owned = String::new();
    owned
        .try_reserve_exact(s.len())
        .map_err(|_| VfsError::OutOfMemory)?;
    owned.push_str(s);
    Ok(owned)
}

/// The listing entry for one child link, with the child's current metadata.
fn dir_entry_for(child: &Child) -> Result<DirEntry, VfsError> {
    let mut entry = DirEntry::empty();
    entry.name = try_string(&child.name)?;
    entry.inode_id = child.id;
    if let Some(node) = get_node(child.id) {
        entry.file_type = node.metadata.file_type;
        entry.size = node.metadata.size;
        entry.uid = node.metadata.uid;
        entry.gid = node.metadata.gid;
        entry.is_dir = node.metadata.file_type == FileType::Directory;
    }
    Ok(entry)
}

/// Every child of directory `node_id`, after loading it from its backing
/// store if that has not happened yet.
fn list_node(node_id: u64) -> Result<Vec<DirEntry>, VfsError> {
    ensure_expanded(node_id)?;
    let node = get_node(node_id).ok_or(VfsError::NotFound)?;
    if node.metadata.file_type != FileType::Directory {
        return Err(VfsError::NotADirectory);
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(node.children.len())
        .map_err(|_| VfsError::OutOfMemory)?;
    for child in &node.children {
        entries.push(dir_entry_for(child)?);
    }
    Ok(entries)
}

/// List a directory's entries (`/share/...` is served by the host share).
pub fn list_dir(path: &str) -> Result<Vec<DirEntry>, VfsError> {
    if let Some(rel) = share_rel_path(path) {
        return list_share_dir(rel);
    }
    list_node(resolve_path(path)?)
}

/// Read entire file contents into a Vec
pub fn read_file(path: &str) -> Result<alloc::vec::Vec<u8>, VfsError> {
    // Host shared folder
    if let Some(rel) = share_rel_path(path) {
        return crate::drivers::lobster_share::read_file(rel).ok_or(VfsError::NotFound);
    }
    let inode_id = resolve_path(path)?;
    // Materialize ext2-backed content before reading the inline buffer.
    let _ = hydrate_node(inode_id);
    let node = get_node(inode_id).ok_or(VfsError::NotFound)?;
    if node.metadata.file_type != FileType::RegularFile {
        return Err(VfsError::NotAFile);
    }
    let len = node.data_len as usize;
    let mut buf = alloc::vec::Vec::with_capacity(len);
    buf.resize(len, 0);
    node.read(0, &mut buf)?;
    Ok(buf)
}

/// Create the VFS node for a file that does not exist yet.
fn create_new_file(path: &str) -> Result<u64, VfsError> {
    let path = &*crate::fs::prefix_map::rewrite(path);
    let (parent_id, name) = resolve_parent(path)?;
    alloc_node(name, FileType::RegularFile, parent_id)
}

/// Replace a file node's in-memory contents with `data`.
fn store_contents(inode_id: u64, data: &[u8]) -> Result<(), VfsError> {
    let node = get_node_mut(inode_id).ok_or(VfsError::NotFound)?;
    node.data_len = 0;
    node.metadata.size = 0;
    node.dirty = true;
    if !data.is_empty() {
        node.write(0, data)?;
    }
    node.metadata.modified_time = u64::from(now_secs());
    Ok(())
}

/// Write data to a VFS file, creating it if it doesn't exist.
/// Truncates existing content. Bypasses permission checks (kernel use).
pub fn write_file(path: &str, data: &[u8]) -> Result<usize, VfsError> {
    if data.len() > MAX_FILE_DATA {
        return Err(VfsError::OutOfMemory);
    }
    let inode_id = match resolve_path(path) {
        Ok(id) => {
            // Hydrate ext2-backed files first so truncation applies to the
            // real content.
            let _ = hydrate_node(id);
            id
        }
        Err(VfsError::NotFound) => create_new_file(path)?,
        Err(e) => return Err(e),
    };
    store_contents(inode_id, data)?;
    // Write through to the ext2 backing store so kernel-side writes
    // (settings persistence etc.) actually land on disk.
    writeback_node(inode_id)?;
    Ok(data.len())
}

/// Write `data` to `path` only when the file does not exist yet. Returns
/// whether it was written. Used to seed defaults without clobbering state.
pub fn write_file_if_missing(path: &str, data: &[u8]) -> Result<bool, VfsError> {
    if resolve_path(path).is_ok() {
        return Ok(false);
    }
    write_file(path, data)?;
    Ok(true)
}

/// Write `data` to `path` unless it already holds exactly those bytes.
/// Returns whether it was written. Keeps boot from rewriting the disk.
pub fn write_file_if_changed(path: &str, data: &[u8]) -> Result<bool, VfsError> {
    if read_file(path).is_ok_and(|current| current == data) {
        return Ok(false);
    }
    write_file(path, data)?;
    Ok(true)
}

/// Create a directory (wrapper with shell-friendly name)
pub fn create_dir(path: &str) -> Result<u64, VfsError> {
    mkdir(path)
}

/// Remove a file or empty directory
pub fn remove(path: &str) -> Result<(), VfsError> {
    let inode_id = resolve_path(path)?;
    if inode_id == 0 {
        return Err(VfsError::Busy);
    }
    ensure_expanded(inode_id)?;
    let node = get_node(inode_id).ok_or(VfsError::NotFound)?;
    if node.metadata.file_type == FileType::Directory && node.dir_count() > 0 {
        return Err(VfsError::NotEmpty);
    }
    let parent_id = node.parent_id;
    let name = try_string(node.name_str())?;
    // Unlink on disk first: a failure leaves the VFS view untouched.
    persist_unlink(parent_id, &name)?;
    detach_node(parent_id, &name, inode_id);
    Ok(())
}

/// Write-through of a delete: unlink in the ext2 backing store when the
/// parent is ext2-backed, so deletes persist across reboots. An entry that
/// only ever existed in RAM (an empty file never written) is not an error.
fn persist_unlink(parent_id: u64, name: &str) -> Result<(), VfsError> {
    let Some((parent_ino, _)) = backing_of(parent_id) else {
        return Ok(());
    };
    match crate::fs::ext2::remove_backing_file(parent_ino, name) {
        Ok(()) | Err(VfsError::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Write-through of a rename when the source's parent is ext2-backed.
/// Parents on different backing stores cannot be renamed across.
fn persist_rename(
    old_parent: u64,
    old_name: &str,
    new_parent: u64,
    new_name: &str,
) -> Result<(), VfsError> {
    let Some((old_ino, old_fs)) = backing_of(old_parent) else {
        return Ok(());
    };
    let Some((new_ino, new_fs)) = backing_of(new_parent) else {
        return Err(VfsError::PermissionDenied);
    };
    if old_fs != new_fs {
        return Err(VfsError::PermissionDenied);
    }
    crate::fs::ext2::rename_backing_entry(old_ino, old_name, new_ino, new_name)
}

/// Rename / move a file or directory
pub fn rename(src_path: &str, dst_path: &str) -> Result<(), VfsError> {
    let src_id = resolve_path(src_path)?;
    let dst_path = &*crate::fs::prefix_map::rewrite(dst_path);
    let (dst_parent_id, dst_name) = resolve_parent(dst_path)?;
    check_name(dst_name)?;
    let src_node = get_node(src_id).ok_or(VfsError::NotFound)?;
    let old_parent_id = src_node.parent_id;
    // Own a copy: the node's name buffer is overwritten below.
    let src_name = try_string(src_node.name_str())?;
    if old_parent_id == dst_parent_id && src_name == dst_name {
        return Ok(());
    }
    let new_parent = get_node_mut(dst_parent_id).ok_or(VfsError::NotFound)?;
    new_parent.add_child(dst_name, src_id)?;
    if let Err(e) = persist_rename(old_parent_id, &src_name, dst_parent_id, dst_name) {
        new_parent.remove_child(dst_name);
        return Err(e);
    }
    if let Some(old_parent) = get_node_mut(old_parent_id) {
        old_parent.remove_child(&src_name);
    }
    let src = get_node_mut(src_id).ok_or(VfsError::NotFound)?;
    src.set_name(dst_name);
    src.parent_id = dst_parent_id;
    Ok(())
}

/// Get file metadata by path (stat)
pub fn stat(path: &str) -> Option<Metadata> {
    let id = resolve_path(path).ok()?;
    get_node(id).map(|n| n.metadata)
}

/// Change file permissions
pub fn chmod(path: &str, mode: u32) -> Result<(), VfsError> {
    let inode_id = resolve_path(path)?;
    let node = get_node_mut(inode_id).ok_or(VfsError::NotFound)?;

    // Permission check: owner or CAP_FOWNER
    {
        let pid = crate::sched::current_pid();
        if let Some(idx) = crate::sched::find_task_index(pid) {
            let euid = unsafe {
                crate::sched::TASK_TABLE[idx]
                    .as_ref()
                    .map(|t| t.euid)
                    .unwrap_or(0)
            };
            if euid != 0 && euid != node.metadata.uid {
                if !crate::security::has_capability(crate::security::CAP_FOWNER) {
                    return Err(VfsError::PermissionDenied);
                }
            }
        }
    }

    // Clear setuid/setgid bits when mode changes (POSIX)
    node.metadata.permissions = mode & 0o7777;
    Ok(())
}

/// Change file ownership
pub fn chown(path: &str, uid: u32, gid: u32) -> Result<(), VfsError> {
    let inode_id = resolve_path(path)?;
    let node = get_node_mut(inode_id).ok_or(VfsError::NotFound)?;

    // Permission check: owner or CAP_CHOWN (Linux: CAP_CHOWN required even for owner)
    {
        let pid = crate::sched::current_pid();
        if let Some(idx) = crate::sched::find_task_index(pid) {
            let euid = unsafe {
                crate::sched::TASK_TABLE[idx]
                    .as_ref()
                    .map(|t| t.euid)
                    .unwrap_or(0)
            };
            if euid != 0 {
                if !crate::security::has_capability(crate::security::CAP_CHOWN) {
                    return Err(VfsError::PermissionDenied);
                }
            }
        }
    }

    // POSIX: clear setuid/setgid bits when owner/group changes (unless root)
    let old_uid = node.metadata.uid;
    let old_gid = node.metadata.gid;
    node.metadata.uid = uid;
    node.metadata.gid = gid;
    if uid != old_uid {
        node.metadata.permissions &= !crate::security::PERM_SETUID;
    }
    if gid != old_gid {
        node.metadata.permissions &= !crate::security::PERM_SETGID;
    }
    Ok(())
}

/// Register a filesystem (assign an fs_id)
pub fn register_fs() -> u32 {
    unsafe {
        let id = NEXT_FS_ID;
        NEXT_FS_ID += 1;
        id
    }
}

/// Mount a filesystem at a path
pub fn mount(path: &str, fs_id: u32, root_inode: u64) -> Result<(), VfsError> {
    unsafe {
        if MOUNT_COUNT >= MAX_MOUNT_POINTS {
            return Err(VfsError::OutOfMemory);
        }
        let mp = &mut MOUNT_POINTS[MOUNT_COUNT];
        let bytes = path.as_bytes();
        let len = bytes.len().min(255);
        mp.path[..len].copy_from_slice(&bytes[..len]);
        mp.path_len = len;
        mp.fs_id = fs_id;
        mp.root_inode = root_inode;
        MOUNT_COUNT += 1;
    }
    println!("[vfs] Mounted fs {} at {}", fs_id, path);
    Ok(())
}

/// Hydrate a node's inline buffer from its ext2 backing store (once).
pub fn hydrate_node(node_id: u64) -> Result<(), VfsError> {
    let node = get_node(node_id).ok_or(VfsError::NotFound)?;
    if node.hydrated || backing_of(node_id).is_none() {
        return Ok(());
    }
    if node.metadata.file_type != FileType::RegularFile {
        return Ok(());
    }
    let data = crate::fs::ext2::read_backing_file(node.backing_ino)?;
    let node = get_node_mut(node_id).ok_or(VfsError::NotFound)?;
    let modified_time = node.metadata.modified_time;
    node.set_data(&data)?;
    node.metadata.modified_time = modified_time;
    node.hydrated = true;
    Ok(())
}

/// Whether a backing-store directory has been materialized into the VFS.
pub fn node_dir_expanded(node_id: u64) -> bool {
    get_node(node_id).map(|n| n.hydrated).unwrap_or(false)
}

/// Load an ext2-backed directory's children into the VFS the first time
/// anything needs them. Every reader of a directory's children goes through
/// this one function; on failure the directory stays unexpanded so the next
/// call retries, and the error reaches the caller.
pub fn ensure_expanded(node_id: u64) -> Result<(), VfsError> {
    let node = get_node(node_id).ok_or(VfsError::NotFound)?;
    if node.metadata.file_type != FileType::Directory || node.hydrated {
        return Ok(());
    }
    let Some((ino, fs_id)) = backing_of(node_id) else {
        return Ok(());
    };
    expand_backed_dir(node_id, fs_id, ino)?;
    if let Some(node) = get_node_mut(node_id) {
        node.hydrated = true;
    }
    Ok(())
}

/// Create a node for every entry of the backing directory `ino` under
/// `node_id`, carrying each inode's real size, type, mode, owner and mtime.
/// Entries that already exist (created in RAM before the expansion) are kept.
pub(crate) fn expand_backed_dir(node_id: u64, fs_id: u32, ino: u32) -> Result<(), VfsError> {
    let entries = crate::fs::ext2::read_backing_dir(ino)?;
    let check_duplicates = get_node(node_id).is_some_and(|n| n.dir_count() > 0);
    for entry in entries {
        let duplicate = check_duplicates
            && get_node(node_id).is_some_and(|n| n.find_child(&entry.name).is_some());
        if duplicate {
            continue;
        }
        let file_type = entry.metadata.file_type;
        let child_id = create_child(&entry.name, file_type, node_id, entry.metadata, false)?;
        set_node_backing(child_id, fs_id, entry.ino);
    }
    Ok(())
}

/// Tag a VFS node with its ext2 backing store (fs id + inode) so reads and
/// writes route through the ext2 instance instead of the inline buffer.
pub fn set_node_backing(node_id: u64, fs_id: u32, ext2_ino: u32) {
    if let Some(node) = get_node_mut(node_id) {
        node.backing_fs = fs_id;
        node.backing_ino = ext2_ino;
    }
}

/// Write a hydrated node's content back to its ext2 backing store. For new
/// files created under an ext2-backed directory (no backing inode yet),
/// create the file in ext2 first and tag the node.
pub fn writeback_node(node_id: u64) -> Result<(), VfsError> {
    let node = get_node(node_id).ok_or(VfsError::NotFound)?;
    if node.metadata.file_type != FileType::RegularFile {
        return Ok(());
    }
    if !node.dirty {
        return Ok(());
    }
    if node.backing_ino != 0 {
        crate::fs::ext2::write_backing_file(node.backing_ino, node.data())?;
    } else if node.data().is_empty() {
        return Ok(()); // nothing to persist for zero-length new files
    } else {
        create_backing_for_new_file(node_id)?;
    }
    if let Some(node) = get_node_mut(node_id) {
        node.dirty = false;
        node.metadata.modified_time = u64::from(now_secs());
    }
    Ok(())
}

/// Create the ext2 file behind a new VFS file whose parent is ext2-backed.
fn create_backing_for_new_file(node_id: u64) -> Result<(), VfsError> {
    let node = get_node(node_id).ok_or(VfsError::NotFound)?;
    let Some((parent_ino, fs_id)) = backing_of(node.parent_id) else {
        return Ok(());
    };
    let name = node.name_str();
    match crate::fs::ext2::create_backing_file(parent_ino, name, node.data()) {
        Ok(new_ino) => {
            set_node_backing(node_id, fs_id, new_ino);
            if let Some(node) = get_node_mut(node_id) {
                node.hydrated = true;
            }
            Ok(())
        }
        Err(e) => {
            println!("[vfs] ext2 create '{}' failed: {:?}", name, e);
            Err(e)
        }
    }
}

/// Print VFS debug info
pub fn print_debug() {
    println!(
        "[vfs] {} nodes, {} mount points, {} open files",
        node_count(),
        // SAFETY: read-only snapshot of kernel-task-context state.
        unsafe { MOUNT_COUNT },
        // SAFETY: as above.
        unsafe { OPEN_FILES.iter().filter(|f| f.is_some()).count() }
    );
    for id in 0..node_count() as u64 {
        let Some(node) = get_node(id) else { continue };
        let type_str = match node.metadata.file_type {
            FileType::RegularFile => "FILE",
            FileType::Directory => "DIR ",
            FileType::Device => "DEV ",
            FileType::Symlink => "LINK",
        };
        println!(
            "[vfs] {} {} ({} bytes)",
            node.name_str(),
            type_str,
            node.metadata.size
        );
    }
}

// ═══════════════════════════════════════════════════════════════
// Security: VFS permission checks
// ═══════════════════════════════════════════════════════════════

/// Check if the current process has permission to access a VFS node.
/// `access` is a bitmask of PERM_* bits to check (e.g., PERM_OWNER_READ | PERM_OWNER_WRITE).
/// Returns true if access is allowed.
pub fn check_permission(node: &VfsNode, access: u32) -> bool {
    use crate::security::{self, AccessMode};

    // If the process has CAP_DAC_OVERRIDE, bypass all permission checks
    if security::has_capability(security::CAP_DAC_OVERRIDE) {
        return true;
    }

    let perm = node.metadata.permissions;
    let file_uid = node.metadata.uid;
    let file_gid = node.metadata.gid;

    // Determine which permission bits apply based on the access type
    // access is a mask of specific permission bits to verify
    // We need to check the appropriate owner/group/other triplet

    // Use the security module's check_access for each requested bit
    if (access
        & (security::PERM_OWNER_READ | security::PERM_GROUP_READ | security::PERM_OTHERS_READ))
        != 0
    {
        if !security::check_access(file_uid, file_gid, perm, AccessMode::Read) {
            return false;
        }
    }
    if (access
        & (security::PERM_OWNER_WRITE | security::PERM_GROUP_WRITE | security::PERM_OTHERS_WRITE))
        != 0
    {
        if !security::check_access(file_uid, file_gid, perm, AccessMode::Write) {
            return false;
        }
    }
    if (access
        & (security::PERM_OWNER_EXEC | security::PERM_GROUP_EXEC | security::PERM_OTHERS_EXEC))
        != 0
    {
        if !security::check_access(file_uid, file_gid, perm, AccessMode::Execute) {
            return false;
        }
    }

    true
}

// ═══════════════════════════════════════════════════════════════
// Extended VFS Operations (symlink, readlink, link, unlink, fsync, fcntl, ioctl)
// ═══════════════════════════════════════════════════════════════

/// Check that the current process may create entries in directory `dir_id`.
fn check_dir_writable(dir_id: u64) -> Result<(), VfsError> {
    let write_access = crate::security::PERM_OWNER_WRITE
        | crate::security::PERM_GROUP_WRITE
        | crate::security::PERM_OTHERS_WRITE;
    match get_node(dir_id) {
        Some(dir) if !check_permission(dir, write_access) => Err(VfsError::PermissionDenied),
        _ => Ok(()),
    }
}

/// Create a symbolic link
pub fn symlink(target: &str, link_path: &str) -> Result<u64, VfsError> {
    if target.len() > MAX_NAME_LEN {
        return Err(VfsError::InvalidPath);
    }
    let link_path = &*crate::fs::prefix_map::rewrite(link_path);
    let (parent_id, name) = resolve_parent(link_path)?;
    check_dir_writable(parent_id)?;
    let link_id = alloc_node(name, FileType::Symlink, parent_id)?;
    // The target path lives in the node's data buffer.
    let stored = get_node_mut(link_id)
        .ok_or(VfsError::NotFound)
        .and_then(|node| node.set_data(target.as_bytes()));
    if let Err(e) = stored {
        detach_node(parent_id, name, link_id);
        return Err(e);
    }
    Ok(link_id)
}

/// Read a symbolic link target
pub fn readlink(path: &str) -> Result<alloc::string::String, VfsError> {
    let inode_id = resolve_path(path)?;
    let node = get_node(inode_id).ok_or(VfsError::NotFound)?;

    if node.metadata.file_type != FileType::Symlink {
        return Err(VfsError::NotAFile);
    }

    // For now, we need to store symlink target somewhere
    // Since we don't have a dedicated field yet, we'll use a simple approach
    // The target is stored in the node's backing data
    // We'll read from the node's data
    let mut buf = alloc::vec::Vec::with_capacity(node.data_len as usize);
    buf.resize(node.data_len as usize, 0);
    node.read(0, &mut buf)?;

    core::str::from_utf8(&buf)
        .map(|s| s.to_string())
        .map_err(|_| VfsError::InvalidPath)
}

/// Create a hard link to an existing file
pub fn link(src_path: &str, dst_path: &str) -> Result<(), VfsError> {
    let src_id = resolve_path(src_path)?;
    let src_node = get_node(src_id).ok_or(VfsError::NotFound)?;
    // Cannot link directories (POSIX restriction)
    if src_node.metadata.file_type == FileType::Directory {
        return Err(VfsError::PermissionDenied);
    }
    let dst_path = &*crate::fs::prefix_map::rewrite(dst_path);
    let (dst_parent_id, dst_name) = resolve_parent(dst_path)?;
    check_name(dst_name)?;
    check_dir_writable(dst_parent_id)?;
    let new_parent = get_node_mut(dst_parent_id).ok_or(VfsError::NotFound)?;
    new_parent.add_child(dst_name, src_id)
}

/// Remove a file or directory (unlink)
pub fn unlink(path: &str) -> Result<(), VfsError> {
    let inode_id = resolve_path(path)?;
    let node = get_node_mut(inode_id).ok_or(VfsError::NotFound)?;

    // Cannot unlink directories (use rmdir)
    if node.metadata.file_type == FileType::Directory {
        return Err(VfsError::PermissionDenied);
    }

    // Decrement link count and remove if zero
    // For now, just remove the directory entry and free if no more links
    // This is a simplified implementation
    remove(path)
}

/// Sync file data to storage (fsync)
pub fn fsync(fd: usize) -> Result<(), VfsError> {
    unsafe {
        if fd >= MAX_OPEN_FILES {
            return Err(VfsError::InvalidOffset);
        }
        let handle = OPEN_FILES[fd].as_ref().ok_or(VfsError::InvalidOffset)?;
        let inode_id = handle.inode_id;

        // In a real filesystem, this would flush to disk
        // For VFS with in-memory nodes, data is already in memory
        // We just ensure any pending writes are complete
        let _ = get_node(inode_id).ok_or(VfsError::NotFound)?;
    }
    Ok(())
}

/// File control operations (fcntl)
pub fn fcntl(fd: usize, cmd: u32, arg: u64) -> Result<u64, VfsError> {
    unsafe {
        if fd >= MAX_OPEN_FILES {
            return Err(VfsError::InvalidOffset);
        }
        let handle = OPEN_FILES[fd].as_mut().ok_or(VfsError::InvalidOffset)?;

        match cmd {
            F_DUPFD => {
                // Duplicate file descriptor
                let min_fd = arg as usize;
                for i in min_fd..MAX_OPEN_FILES {
                    if OPEN_FILES[i].is_none() {
                        OPEN_FILES[i] = Some(FileHandle {
                            inode_id: handle.inode_id,
                            offset: handle.offset,
                            flags: handle.flags,
                        });
                        return Ok(i as u64);
                    }
                }
                Err(VfsError::OutOfMemory)
            }
            F_GETFD => Ok((handle.flags & O_CLOEXEC) as u64),
            F_SETFD => {
                handle.flags = (handle.flags & !O_CLOEXEC) | (arg as u32 & O_CLOEXEC);
                Ok(0)
            }
            F_GETFL => Ok(handle.flags as u64),
            F_SETFL => {
                // Only allow changing O_NONBLOCK
                let allowed = O_NONBLOCK;
                handle.flags = (handle.flags & !allowed) | (arg as u32 & allowed);
                Ok(0)
            }
            _ => Err(VfsError::InvalidOffset),
        }
    }
}

/// I/O control operations (ioctl)
pub fn ioctl(fd: usize, _request: u64, _arg: u64) -> Result<u64, VfsError> {
    unsafe {
        if fd >= MAX_OPEN_FILES {
            return Err(VfsError::InvalidOffset);
        }
        let handle = OPEN_FILES[fd].as_mut().ok_or(VfsError::InvalidOffset)?;
        let inode_id = handle.inode_id;

        // Pass through to underlying filesystem/device
        if let Some(node) = get_node(inode_id) {
            if node.metadata.file_type == FileType::Device {
                // Device-specific ioctl - would call device driver
                // For now, return ENOTTY
                return Err(VfsError::InvalidOffset);
            }
        }
        Err(VfsError::InvalidOffset)
    }
}

/// Sync file data to storage (fdatasync)
pub fn fdatasync(fd: usize) -> Result<(), VfsError> {
    // For our in-memory VFS, fdatasync is same as fsync
    fsync(fd)
}

/// Get file status (stat) by file descriptor
pub fn fstat(fd: usize) -> Option<Metadata> {
    unsafe {
        if fd >= MAX_OPEN_FILES {
            return None;
        }
        let handle = OPEN_FILES[fd].as_ref()?;
        get_node(handle.inode_id).map(|n| n.metadata)
    }
}

/// Truncate/extend a file
pub fn ftruncate(fd: usize, length: u64) -> Result<(), VfsError> {
    unsafe {
        if fd >= MAX_OPEN_FILES {
            return Err(VfsError::InvalidOffset);
        }
        let handle = OPEN_FILES[fd].as_mut().ok_or(VfsError::InvalidOffset)?;
        let inode_id = handle.inode_id;

        if let Some(node) = get_node_mut(inode_id) {
            if node.metadata.file_type != FileType::RegularFile {
                return Err(VfsError::NotAFile);
            }
            // Check write permission
            let write_access = crate::security::PERM_OWNER_WRITE
                | crate::security::PERM_GROUP_WRITE
                | crate::security::PERM_OTHERS_WRITE;
            if !check_permission(node, write_access) {
                return Err(VfsError::PermissionDenied);
            }

            node.data_len = length;
            node.metadata.size = length;
            // If extending, zero-fill new space
            if length > node.data_len {
                // In a real implementation, we'd extend the backing store
            }
            Ok(())
        } else {
            Err(VfsError::NotFound)
        }
    }
}

/// Truncate a file to a specific length
pub fn truncate(path: &str, length: u64) -> Result<(), VfsError> {
    let inode_id = resolve_path(path)?;
    let node = get_node_mut(inode_id).ok_or(VfsError::NotFound)?;

    if node.metadata.file_type != FileType::RegularFile {
        return Err(VfsError::NotAFile);
    }

    // Check write permission
    let write_access = crate::security::PERM_OWNER_WRITE
        | crate::security::PERM_GROUP_WRITE
        | crate::security::PERM_OTHERS_WRITE;
    if !check_permission(node, write_access) {
        return Err(VfsError::PermissionDenied);
    }

    if length <= MAX_FILE_DATA as u64 {
        // Growing the file: ensure backing buffer covers the new length so
        // reads of the extended region stay in bounds (zero-filled).
        let need = length as usize;
        node.ensure_capacity(need)?;
        if length as usize > node.data_len as usize
            && need <= node.data_cap
            && !node.data_ptr.is_null()
        {
            let dst = unsafe { core::slice::from_raw_parts_mut(node.data_ptr, node.data_cap) };
            dst[node.data_len as usize..].fill(0);
        }
        node.data_len = length;
        node.metadata.size = length;
    } else {
        return Err(VfsError::InvalidOffset);
    }
    Ok(())
}

/// Sync all VFS data (no-op for in-memory VFS)
pub fn sync() {
    // VFS is in-memory, nothing to flush
}

/// Get directory entries for getdents
pub fn getdents(path: &str) -> Result<alloc::vec::Vec<DirEntry>, VfsError> {
    list_dir(path)
}
