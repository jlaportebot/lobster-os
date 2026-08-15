//! Virtual Filesystem Layer — abstract inode-based filesystem interface

use crate::ipc::{O_CLOEXEC, O_NONBLOCK};
use crate::println;
use alloc::string::ToString;

/// Maximum path depth for resolution
const MAX_PATH_DEPTH: usize = 16;

/// Maximum filename length (reduced from 255 to save kernel image size)
const MAX_NAME_LEN: usize = 32;

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

/// Directory entry
#[derive(Debug, Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
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
            name: [0u8; MAX_NAME_LEN],
            name_len: 0,
            file_type: FileType::RegularFile,
            inode_id: 0,
            size: 0,
            uid: 0,
            gid: 0,
            is_dir: false,
        }
    }

    pub fn set_name(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(MAX_NAME_LEN);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name_len = len;
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("?")
    }
}

/// Maximum directory entries per node
pub const MAX_DIR_ENTRIES: usize = 16;

/// Maximum inline file data size (4KB per file)
pub const MAX_FILE_DATA: usize = 4096;

/// VFS Node — represents a file, directory, device, or symlink
#[derive(Copy, Clone)]
pub struct VfsNode {
    pub id: u64,
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
    pub metadata: Metadata,
    pub fs_id: u32, // Which filesystem owns this node
    pub parent_id: u64,
    /// For directories: list of child entries
    pub dir_entries: [DirEntry; MAX_DIR_ENTRIES],
    pub dir_count: usize,
    /// For regular files: inline data storage
    pub data: [u8; MAX_FILE_DATA],
    pub data_len: u64,
}

impl VfsNode {
    pub const fn empty(id: u64) -> Self {
        Self {
            id,
            name: [0u8; MAX_NAME_LEN],
            name_len: 0,
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
            parent_id: 0,
            dir_entries: [DirEntry::empty(); MAX_DIR_ENTRIES],
            dir_count: 0,
            data: [0u8; MAX_FILE_DATA],
            data_len: 0,
        }
    }

    pub fn set_name(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(MAX_NAME_LEN);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name_len = len;
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("?")
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
        buf[..to_read].copy_from_slice(&self.data[src_start..src_start + to_read]);
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
        self.data[offset_usize..offset_usize + to_write].copy_from_slice(&buf[..to_write]);
        self.data_len = self.data_len.max(offset + to_write as u64);
        self.metadata.size = self.data_len;
        Ok(to_write)
    }

    pub fn add_dir_entry(&mut self, entry: DirEntry) -> Result<(), VfsError> {
        if self.metadata.file_type != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }
        if self.dir_count >= MAX_DIR_ENTRIES {
            return Err(VfsError::OutOfMemory);
        }
        // Check for duplicate
        for i in 0..self.dir_count {
            if self.dir_entries[i].name_str() == entry.name_str() {
                return Err(VfsError::AlreadyExists);
            }
        }
        self.dir_entries[self.dir_count] = entry;
        self.dir_count += 1;
        Ok(())
    }

    pub fn find_child(&self, name: &str) -> Option<u64> {
        if self.metadata.file_type != FileType::Directory {
            return None;
        }
        for i in 0..self.dir_count {
            if self.dir_entries[i].name_str() == name {
                return Some(self.dir_entries[i].inode_id);
            }
        }
        None
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

/// Maximum number of nodes in the VFS (increased for ext2 rootfs support)
const MAX_VFS_NODES: usize = 64;

/// Maximum mount points
const MAX_MOUNT_POINTS: usize = 8;

/// Global VFS state
static mut VFS_NODES: [VfsNode; MAX_VFS_NODES] = {
    // Initialize all nodes as empty
    let mut arr: [VfsNode; MAX_VFS_NODES] = [VfsNode::empty(0); MAX_VFS_NODES];
    let mut i = 0;
    while i < MAX_VFS_NODES {
        arr[i].id = i as u64;
        i += 1;
    }
    arr
};
static mut VFS_NODE_COUNT: usize = 0;
static mut MOUNT_POINTS: [MountPoint; MAX_MOUNT_POINTS] = [MountPoint::empty(); MAX_MOUNT_POINTS];
static mut MOUNT_COUNT: usize = 0;
static mut NEXT_FS_ID: u32 = 1;

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

/// Global open file table
static mut OPEN_FILES: [Option<FileHandle>; MAX_OPEN_FILES] = [const { None }; MAX_OPEN_FILES];
static mut NEXT_FD: usize = 0;

/// Initialize the VFS
pub fn init() {
    unsafe {
        VFS_NODE_COUNT = 0;
        MOUNT_COUNT = 0;
        NEXT_FS_ID = 1;

        // Create root node
        let root = &mut VFS_NODES[0];
        root.id = 0;
        root.set_name("/");
        root.metadata = Metadata {
            size: 0,
            file_type: FileType::Directory,
            permissions: 0o755,
            modified_time: 0,
            inode_id: 0,
            uid: 0,
            gid: 0,
        };
        root.fs_id = 0;
        root.parent_id = 0;
        root.dir_count = 0;
        VFS_NODE_COUNT = 1;
    }
    println!("[vfs] Virtual filesystem initialized");
}

/// Allocate a new VFS node
pub fn alloc_node(name: &str, file_type: FileType, parent_id: u64) -> Result<u64, VfsError> {
    unsafe {
        if VFS_NODE_COUNT >= MAX_VFS_NODES {
            return Err(VfsError::OutOfMemory);
        }
        let idx = VFS_NODE_COUNT;
        let node = &mut VFS_NODES[idx];
        node.id = idx as u64;
        node.set_name(name);
        node.metadata = Metadata {
            size: 0,
            file_type,
            permissions: if file_type == FileType::Directory {
                0o755
            } else {
                0o644
            },
            modified_time: 0,
            inode_id: idx as u64,
            uid: 0,
            gid: 0,
        };
        node.fs_id = 0;
        node.parent_id = parent_id;
        node.dir_count = 0;
        node.data_len = 0;
        node.data = [0u8; MAX_FILE_DATA];
        VFS_NODE_COUNT += 1;

        // Add to parent directory
        if parent_id < VFS_NODE_COUNT as u64 && parent_id != idx as u64 {
            let parent = &mut VFS_NODES[parent_id as usize];
            let mut entry = DirEntry::empty();
            entry.set_name(name);
            entry.file_type = file_type;
            entry.inode_id = idx as u64;
            let _ = parent.add_dir_entry(entry);
        }

        Ok(idx as u64)
    }
}

/// Get a VFS node by ID
pub fn get_node(id: u64) -> Option<&'static VfsNode> {
    unsafe {
        if (id as usize) < VFS_NODE_COUNT {
            Some(&VFS_NODES[id as usize])
        } else {
            None
        }
    }
}

/// Get a mutable VFS node by ID
pub fn get_node_mut(id: u64) -> Option<&'static mut VfsNode> {
    unsafe {
        if (id as usize) < VFS_NODE_COUNT {
            Some(&mut VFS_NODES[id as usize])
        } else {
            None
        }
    }
}

/// Split a path into components
fn split_path(path: &str) -> [Option<&str>; MAX_PATH_DEPTH] {
    let mut parts: [Option<&str>; MAX_PATH_DEPTH] = [None; MAX_PATH_DEPTH];
    let mut idx = 0;
    for component in path.split('/') {
        if component.is_empty() {
            continue;
        }
        if idx >= MAX_PATH_DEPTH {
            break;
        }
        parts[idx] = Some(component);
        idx += 1;
    }
    parts
}

/// Resolve a path to a VFS node ID
pub fn resolve_path(path: &str) -> Result<u64, VfsError> {
    if path.is_empty() {
        return Err(VfsError::InvalidPath);
    }

    let start_node = if path.starts_with('/') {
        0u64
    } else {
        // Use current working directory (simplified: root)
        0u64
    };

    let parts = split_path(path);
    let mut current_id = start_node;

    for part_opt in &parts {
        if let Some(part) = part_opt {
            let node = get_node(current_id).ok_or(VfsError::NotFound)?;
            if node.metadata.file_type != FileType::Directory {
                return Err(VfsError::NotADirectory);
            }
            current_id = node.find_child(part).ok_or(VfsError::NotFound)?;
        }
    }

    Ok(current_id)
}

/// Create a directory
pub fn mkdir(path: &str) -> Result<u64, VfsError> {
    // Split path into parent + name
    let (parent_path, name) = if let Some(idx) = path.rfind('/') {
        (&path[..idx.max(1)], &path[idx + 1..])
    } else {
        ("", path)
    };

    let parent_id = if parent_path.is_empty() || parent_path == "/" {
        0u64
    } else {
        resolve_path(parent_path)?
    };

    alloc_node(name, FileType::Directory, parent_id)
}

/// Create a file
pub fn create(path: &str) -> Result<u64, VfsError> {
    let (parent_path, name) = if let Some(idx) = path.rfind('/') {
        (&path[..idx.max(1)], &path[idx + 1..])
    } else {
        ("", path)
    };

    let parent_id = if parent_path.is_empty() || parent_path == "/" {
        0u64
    } else {
        resolve_path(parent_path)?
    };

    alloc_node(name, FileType::RegularFile, parent_id)
}

/// Open a file (resolve path, return file descriptor)
pub fn open(path: &str, flags: u32) -> Result<usize, VfsError> {
    let inode_id = resolve_path(path);

    let inode_id = match inode_id {
        Ok(id) => id,
        Err(VfsError::NotFound) if flags & O_CREAT != 0 => {
            // Check write permission on parent directory
            let (parent_path, _) = if let Some(idx) = path.rfind('/') {
                (&path[..idx.max(1)], &path[idx + 1..])
            } else {
                ("", path)
            };
            let parent_id = if parent_path.is_empty() || parent_path == "/" {
                0u64
            } else {
                resolve_path(parent_path)?
            };
            if let Some(parent) = get_node(parent_id) {
                let write_access = crate::security::PERM_OWNER_WRITE
                    | crate::security::PERM_GROUP_WRITE
                    | crate::security::PERM_OTHERS_WRITE;
                if !check_permission(parent, write_access) {
                    return Err(VfsError::PermissionDenied);
                }
            }
            create(path)?
        }
        Err(e) => return Err(e),
    };

    // Check access permissions on the file
    if let Some(node) = get_node(inode_id) {
        let access = match flags & 0x3 {
            O_RDONLY => {
                crate::security::PERM_OWNER_READ
                    | crate::security::PERM_GROUP_READ
                    | crate::security::PERM_OTHERS_READ
            }
            O_WRONLY => {
                crate::security::PERM_OWNER_WRITE
                    | crate::security::PERM_GROUP_WRITE
                    | crate::security::PERM_OTHERS_WRITE
            }
            O_RDWR => {
                crate::security::PERM_OWNER_READ
                    | crate::security::PERM_GROUP_READ
                    | crate::security::PERM_OTHERS_READ
                    | crate::security::PERM_OWNER_WRITE
                    | crate::security::PERM_GROUP_WRITE
                    | crate::security::PERM_OTHERS_WRITE
            }
            _ => 0,
        };
        if !check_permission(node, access) {
            return Err(VfsError::PermissionDenied);
        }
    }

    unsafe {
        for i in 0..MAX_OPEN_FILES {
            if OPEN_FILES[i].is_none() {
                OPEN_FILES[i] = Some(FileHandle {
                    inode_id,
                    offset: 0,
                    flags,
                });
                if flags & O_TRUNC != 0 {
                    if let Some(node) = get_node_mut(inode_id) {
                        node.data_len = 0;
                        node.metadata.size = 0;
                    }
                }
                return Ok(i);
            }
        }
    }
    Err(VfsError::OutOfMemory)
}

/// Close a file descriptor
pub fn close(fd: usize) -> Result<(), VfsError> {
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

/// List directory contents
pub fn list_dir(path: &str) -> Result<alloc::vec::Vec<DirEntry>, VfsError> {
    let inode_id = resolve_path(path)?;
    let node = get_node(inode_id).ok_or(VfsError::NotFound)?;
    if node.metadata.file_type != FileType::Directory {
        return Err(VfsError::NotADirectory);
    }
    let mut entries = alloc::vec::Vec::new();
    for i in 0..node.dir_count {
        let de = &node.dir_entries[i];
        let (size, uid, gid, is_dir) = if let Some(child) = get_node(de.inode_id) {
            (
                child.metadata.size,
                child.metadata.uid,
                child.metadata.gid,
                child.metadata.file_type == FileType::Directory,
            )
        } else {
            (0, 0, 0, false)
        };
        let mut enriched = *de;
        enriched.size = size;
        enriched.uid = uid;
        enriched.gid = gid;
        enriched.is_dir = is_dir;
        entries.push(enriched);
    }
    Ok(entries)
}

/// Read entire file contents into a Vec
pub fn read_file(path: &str) -> Result<alloc::vec::Vec<u8>, VfsError> {
    let inode_id = resolve_path(path)?;
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

/// Write data to a VFS file, creating it if it doesn't exist.
/// Truncates existing content. Bypasses permission checks (kernel use).
pub fn write_file(path: &str, data: &[u8]) -> Result<usize, VfsError> {
    if data.len() > MAX_FILE_DATA {
        return Err(VfsError::OutOfMemory);
    }
    let inode_id = match resolve_path(path) {
        Ok(id) => id,
        Err(VfsError::NotFound) => {
            // Create the file
            let parent_path = parent_dir(path);
            let parent_id = resolve_path(&parent_path)?;
            let name = file_name(path);
            alloc_node(name, FileType::RegularFile, parent_id)?
        }
        Err(e) => return Err(e),
    };
    let node = get_node_mut(inode_id).ok_or(VfsError::NotFound)?;
    node.data_len = 0;
    let written = node.write(0, data)?;
    node.metadata.size = written as u64;
    Ok(written)
}

/// Extract parent directory from a path
fn parent_dir(path: &str) -> alloc::string::String {
    match path.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => alloc::string::String::from(&path[..i]),
        None => "/".into(),
    }
}

/// Extract file name from a path
fn file_name(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) if i < path.len() - 1 => &path[i + 1..],
        _ => path,
    }
}

/// Create a directory (wrapper with shell-friendly name)
pub fn create_dir(path: &str) -> Result<u64, VfsError> {
    mkdir(path)
}

/// Remove a file or empty directory
pub fn remove(path: &str) -> Result<(), VfsError> {
    let inode_id = resolve_path(path)?;
    let node = get_node_mut(inode_id).ok_or(VfsError::NotFound)?;
    if node.metadata.file_type == FileType::Directory && node.dir_count > 0 {
        return Err(VfsError::NotEmpty);
    }
    // Remove from parent
    if node.parent_id < unsafe { VFS_NODE_COUNT } as u64 {
        let parent = unsafe { &mut VFS_NODES[node.parent_id as usize] };
        let name = node.name_str();
        let mut shift = false;
        for i in 0..parent.dir_count {
            if shift {
                parent.dir_entries[i - 1] = parent.dir_entries[i];
            } else if parent.dir_entries[i].name_str() == name {
                shift = true;
            }
        }
        if shift && parent.dir_count > 0 {
            parent.dir_count -= 1;
        }
    }
    // Mark node as free (zero name)
    node.name_len = 0;
    node.metadata.file_type = FileType::RegularFile;
    node.data_len = 0;
    Ok(())
}

/// Rename / move a file or directory
pub fn rename(src_path: &str, dst_path: &str) -> Result<(), VfsError> {
    let src_id = resolve_path(src_path)?;
    let (dst_parent, dst_name) = if let Some(idx) = dst_path.rfind('/') {
        (&dst_path[..idx.max(1)], &dst_path[idx + 1..])
    } else {
        ("", dst_path)
    };
    let dst_parent_id = if dst_parent.is_empty() || dst_parent == "/" {
        0u64
    } else {
        resolve_path(dst_parent)?
    };
    // Remove from old parent
    let src_node = get_node(src_id).ok_or(VfsError::NotFound)?;
    let old_parent_id = src_node.parent_id;
    let src_name = src_node.name_str();
    if old_parent_id < unsafe { VFS_NODE_COUNT } as u64 {
        let old_parent = unsafe { &mut VFS_NODES[old_parent_id as usize] };
        let mut shift = false;
        for i in 0..old_parent.dir_count {
            if shift {
                old_parent.dir_entries[i - 1] = old_parent.dir_entries[i];
            } else if old_parent.dir_entries[i].name_str() == src_name {
                shift = true;
            }
        }
        if shift && old_parent.dir_count > 0 {
            old_parent.dir_count -= 1;
        }
    }
    // Update node name and parent
    let src = get_node_mut(src_id).ok_or(VfsError::NotFound)?;
    src.set_name(dst_name);
    src.parent_id = dst_parent_id;
    // Add to new parent
    if dst_parent_id < unsafe { VFS_NODE_COUNT } as u64 {
        let new_parent = unsafe { &mut VFS_NODES[dst_parent_id as usize] };
        let mut entry = DirEntry::empty();
        entry.set_name(dst_name);
        entry.file_type = src.metadata.file_type;
        entry.inode_id = src_id;
        let _ = new_parent.add_dir_entry(entry);
    }
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

/// Print VFS debug info
pub fn print_debug() {
    unsafe {
        println!(
            "[vfs] {} nodes, {} mount points, {} open files",
            VFS_NODE_COUNT,
            MOUNT_COUNT,
            OPEN_FILES.iter().filter(|f| f.is_some()).count()
        );
        for i in 0..VFS_NODE_COUNT {
            let node = &VFS_NODES[i];
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

/// Create a symbolic link
pub fn symlink(target: &str, link_path: &str) -> Result<u64, VfsError> {
    // Resolve target path (store as-is, don't resolve)
    let (parent_path, name) = if let Some(idx) = link_path.rfind('/') {
        (&link_path[..idx.max(1)], &link_path[idx + 1..])
    } else {
        ("", link_path)
    };

    let parent_id = if parent_path.is_empty() || parent_path == "/" {
        0u64
    } else {
        resolve_path(parent_path)?
    };

    // Check write permission on parent directory
    if let Some(parent) = get_node(parent_id) {
        let write_access = crate::security::PERM_OWNER_WRITE
            | crate::security::PERM_GROUP_WRITE
            | crate::security::PERM_OTHERS_WRITE;
        if !check_permission(parent, write_access) {
            return Err(VfsError::PermissionDenied);
        }
    }

    // Create symlink node
    let link_id = alloc_node(name, FileType::Symlink, parent_id)?;
    if let Some(node) = get_node_mut(link_id) {
        // Store target path in data field
        let target_bytes = target.as_bytes();
        if target_bytes.len() > 255 {
            return Err(VfsError::InvalidPath);
        }
        node.data_len = target_bytes.len() as u64;
        node.metadata.size = target_bytes.len() as u64;
        // Store symlink target in inline data buffer
        node.data[..target_bytes.len()].copy_from_slice(target_bytes);
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

    let (dst_parent, dst_name) = if let Some(idx) = dst_path.rfind('/') {
        (&dst_path[..idx.max(1)], &dst_path[idx + 1..])
    } else {
        ("", dst_path)
    };

    let dst_parent_id = if dst_parent.is_empty() || dst_parent == "/" {
        0u64
    } else {
        resolve_path(dst_parent)?
    };

    // Check write permission on destination parent
    if let Some(parent) = get_node(dst_parent_id) {
        let write_access = crate::security::PERM_OWNER_WRITE
            | crate::security::PERM_GROUP_WRITE
            | crate::security::PERM_OTHERS_WRITE;
        if !check_permission(parent, write_access) {
            return Err(VfsError::PermissionDenied);
        }
    }

    // Create directory entry pointing to same inode
    let mut entry = DirEntry::empty();
    entry.set_name(dst_name);
    entry.file_type = src_node.metadata.file_type;
    entry.inode_id = src_id;
    entry.size = src_node.metadata.size;
    entry.uid = src_node.metadata.uid;
    entry.gid = src_node.metadata.gid;
    entry.is_dir = src_node.metadata.file_type == FileType::Directory;

    if let Some(new_parent) = get_node_mut(dst_parent_id) {
        new_parent.add_dir_entry(entry)?;
    }

    // Increment link count in metadata (we'll add nlink field)
    if let Some(_node) = get_node_mut(src_id) {
        // For now, just track in metadata
        // TODO: add nlink field to Metadata
    }

    Ok(())
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

/// Mount virtual filesystems (/proc, /dev, /sys) and register them with the VFS.
/// Called from kernel_init after vfs::init().
pub fn mount_virtual_fs() {
    // Create mount-point directories in VFS root
    let proc_id = alloc_node("proc", FileType::Directory, 0);
    let dev_id = alloc_node("dev", FileType::Directory, 0);
    let sys_id = alloc_node("sys", FileType::Directory, 0);

    // Register and mount each virtual filesystem
    let proc_fs_id = register_fs();
    let dev_fs_id = register_fs();
    let sys_fs_id = register_fs();

    if let Ok(id) = proc_id {
        let _ = mount("/proc", proc_fs_id, id);
    }
    if let Ok(id) = dev_id {
        let _ = mount("/dev", dev_fs_id, id);
    }
    if let Ok(id) = sys_id {
        let _ = mount("/sys", sys_fs_id, id);
    }

    println!("[vfs] Virtual filesystems mounted: /proc, /dev, /sys");
}
