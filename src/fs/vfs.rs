//! Virtual Filesystem Layer — abstract inode-based filesystem interface

use crate::println;

/// Maximum path depth for resolution
const MAX_PATH_DEPTH: usize = 16;

/// Maximum filename length
const MAX_NAME_LEN: usize = 255;

/// Maximum open files system-wide
const MAX_OPEN_FILES: usize = 128;

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

/// File metadata
#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    pub size: u64,
    pub file_type: FileType,
    pub permissions: u32,
    pub modified_time: u64,
    pub inode_id: u64,
}

/// Directory entry
#[derive(Debug, Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
    pub file_type: FileType,
    pub inode_id: u64,
}

impl DirEntry {
    pub const fn empty() -> Self {
        Self {
            name: [0u8; MAX_NAME_LEN],
            name_len: 0,
            file_type: FileType::RegularFile,
            inode_id: 0,
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
pub const MAX_DIR_ENTRIES: usize = 64;

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
    /// For regular files: data offset and length in the backing store
    pub data_offset: u64,
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
            },
            fs_id: 0,
            parent_id: 0,
            dir_entries: [DirEntry::empty(); MAX_DIR_ENTRIES],
            dir_count: 0,
            data_offset: 0,
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
        // In a real implementation, this would read from the backing store
        // For now, zero-fill as placeholder
        buf[..to_read].fill(0);
        Ok(to_read)
    }

    pub fn write(&mut self, offset: u64, buf: &[u8]) -> Result<usize, VfsError> {
        if self.metadata.file_type != FileType::RegularFile {
            return Err(VfsError::NotAFile);
        }
        // Placeholder — in real implementation, write to backing store
        let _ = offset;
        let written = buf.len();
        self.data_len = self.data_len.max(offset + written as u64);
        self.metadata.size = self.data_len;
        Ok(written)
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

/// Maximum number of nodes in the VFS
const MAX_VFS_NODES: usize = 512;

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
            permissions: if file_type == FileType::Directory { 0o755 } else { 0o644 },
            modified_time: 0,
            inode_id: idx as u64,
        };
        node.fs_id = 0;
        node.parent_id = parent_id;
        node.dir_count = 0;
        node.data_offset = 0;
        node.data_len = 0;
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

    let start_node = if path.starts_with('/') { 0u64 } else {
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
            create(path)?
        }
        Err(e) => return Err(e),
    };

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
        let node = get_node(handle.inode_id).ok_or(VfsError::NotFound)?;
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
            0 => offset as u64, // SEEK_SET
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
pub fn list_dir(path: &str) -> Result<&'static [DirEntry], VfsError> {
    let inode_id = resolve_path(path)?;
    let node = get_node(inode_id).ok_or(VfsError::NotFound)?;
    if node.metadata.file_type != FileType::Directory {
        return Err(VfsError::NotADirectory);
    }
    Ok(&node.dir_entries[..node.dir_count])
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
        println!("[vfs] {} nodes, {} mount points, {} open files",
            VFS_NODE_COUNT, MOUNT_COUNT,
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
            println!("[vfs]   {} {} ({} bytes)", node.name_str(), type_str, node.metadata.size);
        }
    }
}
