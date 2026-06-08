//! Inter-Process Communication — pipes, shared memory, message queues, semaphores,
//! named pipes (FIFOs), Unix domain sockets, eventfd, and IPC namespaces
//!
//! Provides kernel-managed IPC primitives:
//! - **Pipes**: unidirectional byte streams (anonymous, for fork+exec)
//! - **Named pipes (FIFOs)**: filesystem-resident pipes accessible by path
//! - **Shared memory**: mapped regions visible to multiple processes
//! - **Message queues**: typed message passing (System V style)
//! - **Semaphores**: counting semaphores for synchronization
//! - **Unix domain sockets**: bidirectional byte streams between local processes
//! - **Eventfd**: lightweight event notification mechanism
//!
//! All IPC objects are tracked in global tables with reference counting.
//! File-descriptor integration is via the per-process fd table in `ipc::FdTable`.

use crate::memory::frame_alloc;
use crate::memory::{phys_to_virt, PhysAddr, PAGE_SIZE};
use crate::println;
use crate::sched::{self, find_task_index, TaskState, TASK_TABLE};

// ═══════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════

pub const MAX_PIPES: usize = 16;
pub const PIPE_BUFFER_SIZE: usize = 4096; // One page per pipe buffer

pub const MAX_NAMED_PIPES: usize = 8;
pub const NAMED_PIPE_PATH_MAX: usize = 64;

pub const MAX_SHM_REGIONS: usize = 8;
pub const MAX_SHM_SIZE_PAGES: usize = 256; // 1 MB per region max

pub const MAX_MSG_QUEUES: usize = 8;
pub const MAX_MSG_PER_QUEUE: usize = 16;
pub const MAX_MSG_SIZE: usize = 256;

pub const MAX_SEMAS: usize = 16;
pub const MAX_SEMA_VALUE: u32 = 1024;

pub const MAX_UNIX_SOCKETS: usize = 8;
pub const UNIX_SOCK_BUF_SIZE: usize = 4096;

pub const MAX_EVENTFDS: usize = 16;

pub const MAX_PTYS: usize = 16;
pub const PTY_BUFFER_SIZE: usize = 4096;

pub const MAX_FDS_PER_PROC: usize = 32; // Increased from 16

pub const MAX_IPC_NAMESPACES: usize = 16;

// ═══════════════════════════════════════════════════════════════
// IPC Namespacing
// ═══════════════════════════════════════════════════════════════

/// An IPC namespace isolates IPC objects (pipes, SHM, msgqueues,
/// semaphores, unix sockets, eventfds) so that processes in different
/// namespaces cannot see or interact with each other's IPC resources.
///
/// Each IPC object records which namespace it was created in; lookups
/// and operations are gated by `ipc_ns_check()`.
#[derive(Clone, Copy)]
pub struct IpcNamespace {
    /// Unique namespace identifier (0 = global/init namespace)
    pub ns_id: u32,
    /// Reference count (number of processes in this namespace)
    pub refcount: u16,
    /// Whether this slot is in use
    pub in_use: bool,
    /// Permission mode: 0 = open (any ns can see), 1 = strict (same-ns only)
    pub mode: u32,
}

static mut IPC_NAMESPACES: [IpcNamespace; MAX_IPC_NAMESPACES] = [IpcNamespace {
    ns_id: 0,
    refcount: 0,
    in_use: false,
    mode: 0,
}; MAX_IPC_NAMESPACES];

/// Get the IPC namespace ID for a task. Returns 0 (global) if not found.
pub fn ipc_ns_id(task_idx: usize) -> u32 {
    if task_idx >= sched::MAX_TASKS {
        return 0;
    }
    unsafe {
        if let Some(ref task) = TASK_TABLE[task_idx] {
            task.ipc_ns
        } else {
            0
        }
    }
}

/// Create a new IPC namespace. Returns the namespace ID on success.
/// The calling process is moved into the new namespace.
pub fn ipc_ns_unshare() -> Option<u32> {
    let pid = sched::current_pid();
    let task_idx = find_task_index(pid)?;

    let idx = unsafe { IPC_NAMESPACES.iter().position(|ns| !ns.in_use)? };

    let new_ns_id = (idx as u32) + 1; // ns_id 0 is reserved for global

    unsafe {
        IPC_NAMESPACES[idx] = IpcNamespace {
            ns_id: new_ns_id,
            refcount: 1,
            in_use: true,
            mode: 1, // strict by default
        };

        // Move the task into the new namespace
        if let Some(ref mut task) = TASK_TABLE[task_idx] {
            task.ipc_ns = new_ns_id;
        }
    }

    println!(
        "[ipc] namespace: PID {} unshared into ns {}",
        pid, new_ns_id
    );
    Some(new_ns_id)
}

/// Set the IPC namespace for the current process (must have CAP_SYS_ADMIN in
/// a real kernel; here we allow it for any process as a simplification).
/// Returns true on success.
pub fn set_ipc_ns(ns_id: u32) -> bool {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(i) => i,
        None => return false,
    };

    // ns_id 0 always exists (global namespace)
    if ns_id == 0 {
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[task_idx] {
                task.ipc_ns = 0;
            }
        }
        return true;
    }

    // Find the target namespace
    let idx = (ns_id as usize) - 1;
    if idx >= MAX_IPC_NAMESPACES {
        return false;
    }

    unsafe {
        if !IPC_NAMESPACES[idx].in_use || IPC_NAMESPACES[idx].ns_id != ns_id {
            return false;
        }

        if let Some(ref mut task) = TASK_TABLE[task_idx] {
            task.ipc_ns = ns_id;
        }
    }

    true
}

/// Check whether a process (by task_idx) may access an IPC object
/// that was created in `object_ns`.
///
/// Rules:
/// - If the object is in namespace 0 (global), anyone can access it.
/// - If the process is in namespace 0 (global), it can access everything.
/// - If the namespace is in "open" mode (mode=0), anyone can access it.
/// - Otherwise, process must be in the same namespace as the object.
pub fn ipc_ns_check(task_idx: usize, object_ns: u32) -> bool {
    // Object in global namespace → always accessible
    if object_ns == 0 {
        return true;
    }

    let proc_ns = ipc_ns_id(task_idx);

    // Process in global namespace → can access everything
    if proc_ns == 0 {
        return true;
    }

    // Same namespace → always allowed
    if proc_ns == object_ns {
        return true;
    }

    // Check if the object's namespace is in "open" mode
    let ns_idx = (object_ns as usize) - 1;
    if ns_idx < MAX_IPC_NAMESPACES {
        unsafe {
            if IPC_NAMESPACES[ns_idx].in_use && IPC_NAMESPACES[ns_idx].mode == 0 {
                return true;
            }
        }
    }

    false
}

/// Set the permission mode of an IPC namespace (0=open, 1=strict).
/// Only a process in the namespace (or in ns 0) may change its mode.
pub fn ipc_ns_set_mode(ns_id: u32, mode: u32) -> bool {
    if ns_id == 0 {
        // Global namespace mode can be changed by any process
        // (in a real kernel, this would require CAP_SYS_ADMIN)
        return true;
    }

    let ns_idx = (ns_id as usize) - 1;
    if ns_idx >= MAX_IPC_NAMESPACES {
        return false;
    }

    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(i) => i,
        None => return false,
    };

    // Only processes in the namespace (or ns 0) may change its mode
    let proc_ns = ipc_ns_id(task_idx);
    if proc_ns != ns_id && proc_ns != 0 {
        return false;
    }

    unsafe {
        if IPC_NAMESPACES[ns_idx].in_use && IPC_NAMESPACES[ns_idx].ns_id == ns_id {
            IPC_NAMESPACES[ns_idx].mode = mode;
            return true;
        }
    }

    false
}

/// Decrement the refcount of an IPC namespace (called when a process exits).
/// If refcount reaches 0, the namespace is destroyed.
pub fn ipc_ns_release(ns_id: u32) {
    if ns_id == 0 {
        return; // Global namespace is never destroyed
    }

    let ns_idx = (ns_id as usize) - 1;
    if ns_idx >= MAX_IPC_NAMESPACES {
        return;
    }

    unsafe {
        if IPC_NAMESPACES[ns_idx].in_use && IPC_NAMESPACES[ns_idx].ns_id == ns_id {
            IPC_NAMESPACES[ns_idx].refcount = IPC_NAMESPACES[ns_idx].refcount.saturating_sub(1);
            if IPC_NAMESPACES[ns_idx].refcount == 0 {
                IPC_NAMESPACES[ns_idx].in_use = false;
                IPC_NAMESPACES[ns_idx].ns_id = 0;
                IPC_NAMESPACES[ns_idx].mode = 0;
                println!("[ipc] namespace {} destroyed (no processes remain)", ns_id);
            }
        }
    }
}

/// Increment the refcount of an IPC namespace (called when a process
/// joins a namespace, e.g. via fork inheriting parent's ns).
pub fn ipc_ns_incref(ns_id: u32) {
    if ns_id == 0 {
        return;
    }

    let ns_idx = (ns_id as usize) - 1;
    if ns_idx >= MAX_IPC_NAMESPACES {
        return;
    }

    unsafe {
        if IPC_NAMESPACES[ns_idx].in_use && IPC_NAMESPACES[ns_idx].ns_id == ns_id {
            IPC_NAMESPACES[ns_idx].refcount = IPC_NAMESPACES[ns_idx].refcount.saturating_add(1);
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// File Descriptor Table (per-process)
// ═══════════════════════════════════════════════════════════════

/// IPC object types that can be referenced by file descriptors
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FdKind {
    None,           // Unused slot
    PipeRead,       // Read end of a pipe
    PipeWrite,      // Write end of a pipe
    NamedPipeRead,  // Read end of a named pipe (FIFO)
    NamedPipeWrite, // Write end of a named pipe (FIFO)
    Shm,            // Shared memory region
    MsgQueue,       // Message queue
    UnixSocket,     // Unix domain socket
    Eventfd,        // Eventfd
    VfsFile,        // VFS-backed file (future)
    PtyMaster,      // PTY master (terminal emulator side)
    PtySlave,       // PTY slave (shell side)
}

/// FD flags
pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_NONBLOCK: u32 = 0o4000;
pub const O_CLOEXEC: u32 = 0o2000000;

/// A single file descriptor entry
#[derive(Clone, Copy)]
pub struct FdEntry {
    pub kind: FdKind,
    pub index: u64, // Index into the corresponding IPC table
    pub flags: u32, // O_RDONLY, O_NONBLOCK, O_CLOEXEC, etc.
}

impl FdEntry {
    pub const fn empty() -> Self {
        Self {
            kind: FdKind::None,
            index: 0,
            flags: 0,
        }
    }
}

/// Per-process file descriptor table
#[derive(Clone, Copy)]
pub struct FdTable {
    pub entries: [FdEntry; MAX_FDS_PER_PROC],
}

impl FdTable {
    pub const fn new() -> Self {
        Self {
            entries: [FdEntry::empty(); MAX_FDS_PER_PROC],
        }
    }

    /// Allocate the lowest available fd
    pub fn alloc_fd(&mut self, kind: FdKind, index: u64, flags: u32) -> Option<usize> {
        for i in 0..MAX_FDS_PER_PROC {
            if self.entries[i].kind == FdKind::None {
                self.entries[i] = FdEntry { kind, index, flags };
                return Some(i);
            }
        }
        None
    }

    /// Get a reference to an fd entry
    pub fn get(&self, fd: usize) -> Option<&FdEntry> {
        if fd < MAX_FDS_PER_PROC && self.entries[fd].kind != FdKind::None {
            Some(&self.entries[fd])
        } else {
            None
        }
    }

    /// Get a mutable reference to an fd entry
    pub fn get_mut(&mut self, fd: usize) -> Option<&mut FdEntry> {
        if fd < MAX_FDS_PER_PROC && self.entries[fd].kind != FdKind::None {
            Some(&mut self.entries[fd])
        } else {
            None
        }
    }

    /// Release an fd, returns the released entry
    pub fn release(&mut self, fd: usize) -> Option<FdEntry> {
        if fd < MAX_FDS_PER_PROC && self.entries[fd].kind != FdKind::None {
            let entry = self.entries[fd];
            self.entries[fd] = FdEntry::empty();
            Some(entry)
        } else {
            None
        }
    }

    /// Check if fd has O_CLOEXEC flag set
    pub fn is_cloexec(&self, fd: usize) -> bool {
        if fd < MAX_FDS_PER_PROC {
            self.entries[fd].flags & O_CLOEXEC != 0
        } else {
            false
        }
    }

    /// Get/set flags on an fd (F_GETFD / F_SETFD)
    pub fn get_flags(&self, fd: usize) -> Option<u32> {
        if fd < MAX_FDS_PER_PROC && self.entries[fd].kind != FdKind::None {
            Some(self.entries[fd].flags)
        } else {
            None
        }
    }

    pub fn set_flags(&mut self, fd: usize, flags: u32) -> bool {
        if fd < MAX_FDS_PER_PROC && self.entries[fd].kind != FdKind::None {
            self.entries[fd].flags = flags;
            true
        } else {
            false
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Global FD table (one per process, indexed by PID slot)
// ═══════════════════════════════════════════════════════════════

const MAX_FD_TABLES: usize = 32; // Must match MAX_TASKS

static mut FD_TABLES: [FdTable; MAX_FD_TABLES] = [FdTable::new(); MAX_FD_TABLES];

/// Get the FdTable for a task (by task table index)
pub fn get_fd_table(task_idx: usize) -> Option<&'static mut FdTable> {
    if task_idx < MAX_FD_TABLES {
        Some(unsafe { &mut FD_TABLES[task_idx] })
    } else {
        None
    }
}

/// Clone the parent's fd table for fork (with O_CLOEXEC handling).
/// If `cloexec_mode` is true, close all fds with O_CLOEXEC set.
/// If `cloexec_mode` is false, inherit all fds (increment refcounts).
pub fn clone_fd_table(parent_idx: usize, child_idx: usize, cloexec_mode: bool) -> bool {
    if parent_idx >= MAX_FD_TABLES || child_idx >= MAX_FD_TABLES {
        return false;
    }

    let parent_entries;
    unsafe {
        parent_entries = FD_TABLES[parent_idx].entries;
    }

    for i in 0..MAX_FDS_PER_PROC {
        let entry = parent_entries[i];
        if entry.kind == FdKind::None {
            continue;
        }

        // In cloexec mode (execve), skip fds with O_CLOEXEC
        if cloexec_mode && (entry.flags & O_CLOEXEC) != 0 {
            continue;
        }

        // Copy the fd entry to child
        unsafe {
            FD_TABLES[child_idx].entries[i] = entry;
        }

        // Increment refcounts for shared IPC objects
        increment_ipc_refcount(entry);
    }

    true
}

/// Increment the refcount of an IPC object referenced by an FdEntry
fn increment_ipc_refcount(entry: FdEntry) {
    match entry.kind {
        FdKind::PipeRead | FdKind::PipeWrite => {
            let pipe_idx = entry.index as usize;
            if pipe_idx < MAX_PIPES {
                unsafe {
                    PIPES[pipe_idx].refcount = PIPES[pipe_idx].refcount.saturating_add(1);
                }
            }
        }
        FdKind::NamedPipeRead | FdKind::NamedPipeWrite => {
            let np_idx = entry.index as usize;
            if np_idx < MAX_NAMED_PIPES {
                unsafe {
                    NAMED_PIPES[np_idx].refcount = NAMED_PIPES[np_idx].refcount.saturating_add(1);
                }
            }
        }
        FdKind::Shm => {
            let shm_idx = entry.index as usize;
            if shm_idx < MAX_SHM_REGIONS {
                unsafe {
                    SHM_REGIONS[shm_idx].refcount = SHM_REGIONS[shm_idx].refcount.saturating_add(1);
                }
            }
        }
        FdKind::MsgQueue => {
            let qi = entry.index as usize;
            if qi < MAX_MSG_QUEUES {
                unsafe {
                    MSG_QUEUES[qi].refcount = MSG_QUEUES[qi].refcount.saturating_add(1);
                }
            }
        }
        FdKind::UnixSocket => {
            let si = entry.index as usize;
            if si < MAX_UNIX_SOCKETS {
                unsafe {
                    UNIX_SOCKETS[si].refcount = UNIX_SOCKETS[si].refcount.saturating_add(1);
                }
            }
        }
        FdKind::Eventfd => {
            let ei = entry.index as usize;
            if ei < MAX_EVENTFDS {
                unsafe {
                    EVENTFDS[ei].refcount = EVENTFDS[ei].refcount.saturating_add(1);
                }
            }
        }
        FdKind::VfsFile | FdKind::None | FdKind::PtyMaster | FdKind::PtySlave => {}
    }
}

/// Close all fds for a process (called on exit).
/// Iterates through the fd table, closing each open fd.
pub fn close_all_fds(task_idx: usize) {
    if task_idx >= MAX_FD_TABLES {
        return;
    }

    // Collect all open fds first to avoid borrow issues
    let mut open_fds: [Option<FdEntry>; MAX_FDS_PER_PROC] = [None; MAX_FDS_PER_PROC];
    unsafe {
        for i in 0..MAX_FDS_PER_PROC {
            if FD_TABLES[task_idx].entries[i].kind != FdKind::None {
                open_fds[i] = Some(FD_TABLES[task_idx].entries[i]);
                FD_TABLES[task_idx].entries[i] = FdEntry::empty();
            }
        }
    }

    // Now close each one
    for entry_opt in open_fds.iter() {
        if let Some(entry) = entry_opt {
            close_ipc_object(entry);
        }
    }
}

/// Close the underlying IPC object for an FdEntry (decrement refcount, free if zero)
fn close_ipc_object(entry: &FdEntry) {
    match entry.kind {
        FdKind::PipeRead => pipe_close_end(entry.index as usize, true),
        FdKind::PipeWrite => pipe_close_end(entry.index as usize, false),
        FdKind::NamedPipeRead => named_pipe_close_end(entry.index as usize, true),
        FdKind::NamedPipeWrite => named_pipe_close_end(entry.index as usize, false),
        FdKind::Shm => shm_detach(entry.index as usize),
        FdKind::MsgQueue => {
            let qi = entry.index as usize;
            if qi < MAX_MSG_QUEUES {
                unsafe {
                    MSG_QUEUES[qi].refcount = MSG_QUEUES[qi].refcount.saturating_sub(1);
                    if MSG_QUEUES[qi].refcount == 0 {
                        MSG_QUEUES[qi].in_use = false;
                    }
                }
            }
        }
        FdKind::UnixSocket => {
            let si = entry.index as usize;
            if si < MAX_UNIX_SOCKETS {
                unsafe {
                    UNIX_SOCKETS[si].refcount = UNIX_SOCKETS[si].refcount.saturating_sub(1);
                    if UNIX_SOCKETS[si].refcount == 0 {
                        // Free buffer
                        if UNIX_SOCKETS[si].buffer_phys != 0 {
                            frame_alloc::deallocate_frame(PhysAddr::new(
                                UNIX_SOCKETS[si].buffer_phys,
                            ));
                        }
                        UNIX_SOCKETS[si].in_use = false;
                    }
                }
            }
        }
        FdKind::Eventfd => {
            let ei = entry.index as usize;
            if ei < MAX_EVENTFDS {
                unsafe {
                    EVENTFDS[ei].refcount = EVENTFDS[ei].refcount.saturating_sub(1);
                    if EVENTFDS[ei].refcount == 0 {
                        EVENTFDS[ei].in_use = false;
                    }
                }
            }
        }
        FdKind::VfsFile | FdKind::None | FdKind::PtyMaster | FdKind::PtySlave => {}
    }
}

// ═══════════════════════════════════════════════════════════════
// PIPES (anonymous)
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
struct Pipe {
    buffer_phys: u64,
    read_pos: usize,
    write_pos: usize,
    count: usize,
    refcount: u16,
    in_use: bool,
    creator_pid: u64,
    readers_waiting: u64,
    writers_waiting: u64,
    /// IPC namespace this pipe was created in
    ns_id: u32,
}

static mut PIPES: [Pipe; MAX_PIPES] = [Pipe {
    buffer_phys: 0,
    read_pos: 0,
    write_pos: 0,
    count: 0,
    refcount: 0,
    in_use: false,
    creator_pid: 0,
    readers_waiting: 0,
    writers_waiting: 0,
    ns_id: 0,
}; MAX_PIPES];

/// Create an anonymous pipe, returns (read_fd, write_fd) for current process
pub fn create_pipe() -> Option<(usize, usize)> {
    let pid = sched::current_pid();
    let task_idx = find_task_index(pid)?;
    let ns = ipc_ns_id(task_idx);

    // Find a free pipe slot
    let pipe_idx = unsafe { PIPES.iter().position(|p| !p.in_use)? };

    // Allocate a page for the pipe buffer
    let frame = frame_alloc::allocate_frame()?;
    let buffer_phys = frame.0;

    // Zero the buffer
    let buffer_virt = phys_to_virt(PhysAddr::new(buffer_phys)).0;
    unsafe {
        core::ptr::write_bytes(buffer_virt as *mut u8, 0, PIPE_BUFFER_SIZE);
    }

    // Initialize the pipe
    unsafe {
        PIPES[pipe_idx] = Pipe {
            buffer_phys,
            read_pos: 0,
            write_pos: 0,
            count: 0,
            refcount: 2, // One read + one write
            in_use: true,
            creator_pid: pid,
            readers_waiting: 0,
            writers_waiting: 0,
            ns_id: ns,
        };
    }

    // Allocate fds for current process
    let fd_table = get_fd_table(task_idx)?;
    let read_fd = fd_table.alloc_fd(FdKind::PipeRead, pipe_idx as u64, 0)?;
    let write_fd = fd_table.alloc_fd(FdKind::PipeWrite, pipe_idx as u64, 0)?;

    println!(
        "[ipc] pipe: created pipe {} (read_fd={}, write_fd={}) for PID {}",
        pipe_idx, read_fd, write_fd, pid
    );

    Some((read_fd, write_fd))
}

/// Read from a pipe fd. Returns bytes read, or 0 if empty / invalid.
pub fn pipe_read(task_idx: usize, fd: usize, buf: &mut [u8]) -> isize {
    // Namespace check: verify process can access this pipe's namespace
    {
        let obj_ns = unsafe {
            PIPES[get_fd_table(task_idx)
                .and_then(|t| t.get(fd))
                .map(|e| e.index as usize)
                .unwrap_or(0)]
            .ns_id
        };
        if !ipc_ns_check(task_idx, obj_ns) {
            return -1; // -EACCES: wrong namespace
        }
    }

    let (pipe_idx, _flags) = {
        let fd_table = match get_fd_table(task_idx) {
            Some(t) => t,
            None => return -1,
        };
        let entry = match fd_table.get(fd) {
            Some(e) => e,
            None => return -1,
        };
        if entry.kind != FdKind::PipeRead {
            return -1;
        }
        (entry.index as usize, entry.flags)
    };

    unsafe {
        let pipe = &mut PIPES[pipe_idx];

        if pipe.count == 0 {
            if pipe.refcount <= 1 {
                return 0; // EOF
            }
            pipe.readers_waiting |= 1 << task_idx;
            return -2; // Would block
        }

        let to_read = buf.len().min(pipe.count);
        let buffer_virt = phys_to_virt(PhysAddr::new(pipe.buffer_phys)).0 as *const u8;

        for i in 0..to_read {
            buf[i] = *buffer_virt.add(pipe.read_pos);
            pipe.read_pos = (pipe.read_pos + 1) % PIPE_BUFFER_SIZE;
        }
        pipe.count -= to_read;

        if pipe.writers_waiting != 0 {
            let writers = pipe.writers_waiting;
            pipe.writers_waiting = 0;
            wake_tasks(writers);
        }

        to_read as isize
    }
}

/// Write to a pipe fd. Returns bytes written, or negative on error.
pub fn pipe_write(task_idx: usize, fd: usize, buf: &[u8]) -> isize {
    // Namespace check: verify process can access this pipe's namespace
    {
        let obj_ns = unsafe {
            PIPES[get_fd_table(task_idx)
                .and_then(|t| t.get(fd))
                .map(|e| e.index as usize)
                .unwrap_or(0)]
            .ns_id
        };
        if !ipc_ns_check(task_idx, obj_ns) {
            return -1; // -EACCES: wrong namespace
        }
    }

    let (pipe_idx, _flags) = {
        let fd_table = match get_fd_table(task_idx) {
            Some(t) => t,
            None => return -1,
        };
        let entry = match fd_table.get(fd) {
            Some(e) => e,
            None => return -1,
        };
        if entry.kind != FdKind::PipeWrite {
            return -1;
        }
        (entry.index as usize, entry.flags)
    };

    unsafe {
        let pipe = &mut PIPES[pipe_idx];

        if pipe.refcount <= 1 {
            return -32; // -EPIPE
        }

        let available = PIPE_BUFFER_SIZE - pipe.count;
        if available == 0 {
            pipe.writers_waiting |= 1 << task_idx;
            return -2; // Would block
        }

        let to_write = buf.len().min(available);
        let buffer_virt = phys_to_virt(PhysAddr::new(pipe.buffer_phys)).0 as *mut u8;

        for i in 0..to_write {
            *buffer_virt.add(pipe.write_pos) = buf[i];
            pipe.write_pos = (pipe.write_pos + 1) % PIPE_BUFFER_SIZE;
        }
        pipe.count += to_write;

        if pipe.readers_waiting != 0 {
            let readers = pipe.readers_waiting;
            pipe.readers_waiting = 0;
            wake_tasks(readers);
        }

        to_write as isize
    }
}

/// Close one end of a pipe (called when fd is released)
fn pipe_close_end(pipe_idx: usize, is_read: bool) {
    unsafe {
        let pipe = &mut PIPES[pipe_idx];
        if !pipe.in_use {
            return;
        }

        pipe.refcount = pipe.refcount.saturating_sub(1);

        if pipe.refcount == 0 {
            frame_alloc::deallocate_frame(PhysAddr::new(pipe.buffer_phys));
            pipe.in_use = false;
            pipe.buffer_phys = 0;
            pipe.count = 0;
            pipe.read_pos = 0;
            pipe.write_pos = 0;
            println!("[ipc] pipe: destroyed pipe {}", pipe_idx);
        } else if is_read {
            if pipe.writers_waiting != 0 {
                let writers = pipe.writers_waiting;
                pipe.writers_waiting = 0;
                wake_tasks(writers);
            }
        } else {
            if pipe.readers_waiting != 0 {
                let readers = pipe.readers_waiting;
                pipe.readers_waiting = 0;
                wake_tasks(readers);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// NAMED PIPES (FIFOs)
// ═══════════════════════════════════════════════════════════════

/// A named pipe (FIFO) — same circular buffer as anonymous pipe,
/// but has a filesystem path so unrelated processes can open it.
#[derive(Clone, Copy)]
struct NamedPipe {
    /// Backing pipe (index into PIPES table)
    pipe_idx: usize,
    /// Path name for lookup
    path: [u8; NAMED_PIPE_PATH_MAX],
    path_len: usize,
    /// Whether this slot is in use
    in_use: bool,
    /// Creator PID
    creator_pid: u64,
    /// Reference count (readers + writers with this FIFO open)
    refcount: u16,
    /// Mode/permissions
    mode: u32,
    /// Tasks waiting for a reader to open
    wait_readers: u64,
    /// Tasks waiting for a writer to open
    wait_writers: u64,
    /// IPC namespace this FIFO was created in
    ns_id: u32,
}

static mut NAMED_PIPES: [NamedPipe; MAX_NAMED_PIPES] = {
    let path = [0u8; NAMED_PIPE_PATH_MAX];
    [NamedPipe {
        pipe_idx: 0,
        path,
        path_len: 0,
        in_use: false,
        creator_pid: 0,
        refcount: 0,
        mode: 0,
        wait_readers: 0,
        wait_writers: 0,
        ns_id: 0,
    }; MAX_NAMED_PIPES]
};

/// Create a named pipe (FIFO) at the given path.
/// Returns the named pipe index on success.
pub fn mkfifo(path: &[u8], mode: u32) -> Option<usize> {
    let pid = sched::current_pid();
    let task_idx = find_task_index(pid)?;
    let ns = ipc_ns_id(task_idx);

    if path.len() >= NAMED_PIPE_PATH_MAX {
        return None;
    }

    // Check path doesn't already exist
    if find_named_pipe(path).is_some() {
        return None; // EEXIST
    }

    // Find a free named pipe slot
    let np_idx = unsafe { NAMED_PIPES.iter().position(|np| !np.in_use)? };

    // Also allocate an anonymous pipe for the buffer
    let pipe_idx = unsafe { PIPES.iter().position(|p| !p.in_use)? };

    // Allocate pipe buffer page
    let frame = frame_alloc::allocate_frame()?;
    let buffer_phys = frame.0;
    let buffer_virt = phys_to_virt(PhysAddr::new(buffer_phys)).0;
    unsafe {
        core::ptr::write_bytes(buffer_virt as *mut u8, 0, PIPE_BUFFER_SIZE);
        PIPES[pipe_idx] = Pipe {
            buffer_phys,
            read_pos: 0,
            write_pos: 0,
            count: 0,
            refcount: 0, // Will be incremented on open
            in_use: true,
            creator_pid: pid,
            readers_waiting: 0,
            writers_waiting: 0,
            ns_id: ns,
        };
    }

    // Initialize named pipe
    let mut path_buf = [0u8; NAMED_PIPE_PATH_MAX];
    path_buf[..path.len()].copy_from_slice(path);

    unsafe {
        NAMED_PIPES[np_idx] = NamedPipe {
            pipe_idx,
            path: path_buf,
            path_len: path.len(),
            in_use: true,
            creator_pid: pid,
            refcount: 0,
            mode,
            wait_readers: 0,
            wait_writers: 0,
            ns_id: ns,
        };
    }

    println!(
        "[ipc] mkfifo: created FIFO {} at '{}' for PID {}",
        np_idx,
        core::str::from_utf8(path).unwrap_or("?"),
        pid
    );

    Some(np_idx)
}

/// Find a named pipe by path
pub fn find_named_pipe(path: &[u8]) -> Option<usize> {
    unsafe {
        NAMED_PIPES.iter().position(|np| {
            np.in_use && np.path_len == path.len() && np.path[..np.path_len] == path[..]
        })
    }
}

/// Open a named pipe for reading. Returns (fd, named_pipe_idx).
/// Blocks if no writer has the pipe open (unless O_NONBLOCK).
pub fn open_fifo_read(task_idx: usize, np_idx: usize, flags: u32) -> Option<usize> {
    if np_idx >= MAX_NAMED_PIPES {
        return None;
    }

    // Namespace check: verify process can access this FIFO's namespace
    unsafe {
        let obj_ns = NAMED_PIPES[np_idx].ns_id;
        if !ipc_ns_check(task_idx, obj_ns) {
            return None;
        }
    }

    unsafe {
        let np = &mut NAMED_PIPES[np_idx];
        if !np.in_use {
            return None;
        }

        let pipe_idx = np.pipe_idx;

        // Increment refcounts
        PIPES[pipe_idx].refcount = PIPES[pipe_idx].refcount.saturating_add(1);
        np.refcount = np.refcount.saturating_add(1);

        // Check if any writer has this open (at least 2 refs: reader + writer)
        // If no writer and not O_NONBLOCK, block
        let has_writer = PIPES[pipe_idx].refcount >= 2 ||
            np.wait_writers != 0 ||
            // Check if any fd references this pipe as PipeWrite or NamedPipeWrite
            count_pipe_writers(pipe_idx) > 0;

        if !has_writer && (flags & O_NONBLOCK) == 0 {
            // Block until a writer opens
            np.wait_readers |= 1 << task_idx;
            // For now, return the fd — the task will yield when it reads
        }

        // Wake any waiting writers
        if np.wait_writers != 0 {
            let waiters = np.wait_writers;
            np.wait_writers = 0;
            wake_tasks(waiters);
        }

        let fd_table = get_fd_table(task_idx)?;
        fd_table.alloc_fd(FdKind::NamedPipeRead, np_idx as u64, flags)
    }
}

/// Open a named pipe for writing. Returns (fd, named_pipe_idx).
pub fn open_fifo_write(task_idx: usize, np_idx: usize, flags: u32) -> Option<usize> {
    if np_idx >= MAX_NAMED_PIPES {
        return None;
    }

    // Namespace check: verify process can access this FIFO's namespace
    unsafe {
        let obj_ns = NAMED_PIPES[np_idx].ns_id;
        if !ipc_ns_check(task_idx, obj_ns) {
            return None;
        }
    }

    unsafe {
        let np = &mut NAMED_PIPES[np_idx];
        if !np.in_use {
            return None;
        }

        let pipe_idx = np.pipe_idx;

        PIPES[pipe_idx].refcount = PIPES[pipe_idx].refcount.saturating_add(1);
        np.refcount = np.refcount.saturating_add(1);

        // Wake any waiting readers
        if np.wait_readers != 0 {
            let waiters = np.wait_readers;
            np.wait_readers = 0;
            wake_tasks(waiters);
        }

        let fd_table = get_fd_table(task_idx)?;
        fd_table.alloc_fd(FdKind::NamedPipeWrite, np_idx as u64, flags)
    }
}

/// Count how many writer fds reference a given pipe index
fn count_pipe_writers(pipe_idx: usize) -> usize {
    let mut count = 0;
    unsafe {
        for t in 0..MAX_FD_TABLES {
            for f in 0..MAX_FDS_PER_PROC {
                let e = &FD_TABLES[t].entries[f];
                if (e.kind == FdKind::PipeWrite || e.kind == FdKind::NamedPipeWrite)
                    && e.index as usize == pipe_idx
                {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Read from a named pipe fd
pub fn fifo_read(task_idx: usize, fd: usize, buf: &mut [u8]) -> isize {
    let (np_idx, flags) = {
        let fd_table = match get_fd_table(task_idx) {
            Some(t) => t,
            None => return -1,
        };
        let entry = match fd_table.get(fd) {
            Some(e) => e,
            None => return -1,
        };
        if entry.kind != FdKind::NamedPipeRead {
            return -1;
        }
        (entry.index as usize, entry.flags)
    };

    if np_idx >= MAX_NAMED_PIPES {
        return -1;
    }

    let _pipe_idx = unsafe { NAMED_PIPES[np_idx].pipe_idx };

    // Delegate to the underlying pipe
    let result = pipe_read(task_idx, fd, buf);

    // If would-block and O_NONBLOCK, return -11 (EAGAIN)
    if result == -2 && (flags & O_NONBLOCK) != 0 {
        return -11; // -EAGAIN
    }

    result
}

/// Write to a named pipe fd
pub fn fifo_write(task_idx: usize, fd: usize, buf: &[u8]) -> isize {
    let (np_idx, flags) = {
        let fd_table = match get_fd_table(task_idx) {
            Some(t) => t,
            None => return -1,
        };
        let entry = match fd_table.get(fd) {
            Some(e) => e,
            None => return -1,
        };
        if entry.kind != FdKind::NamedPipeWrite {
            return -1;
        }
        (entry.index as usize, entry.flags)
    };

    if np_idx >= MAX_NAMED_PIPES {
        return -1;
    }

    // For named pipe, signal SIGPIPE if no readers
    let pipe_idx = unsafe { NAMED_PIPES[np_idx].pipe_idx };
    let has_readers = unsafe { PIPES[pipe_idx].refcount > 0 && count_pipe_readers(pipe_idx) > 0 };

    if !has_readers {
        // Send SIGPIPE to the writing process
        unsafe {
            if let Some(ref mut task) = TASK_TABLE[task_idx] {
                task.pending_signals |= 1 << 13; // SIGPIPE = 13
            }
        }
        return -32; // -EPIPE
    }

    let result = pipe_write(task_idx, fd, buf);

    if result == -2 && (flags & O_NONBLOCK) != 0 {
        return -11; // -EAGAIN
    }

    result
}

/// Count how many reader fds reference a given pipe index
fn count_pipe_readers(pipe_idx: usize) -> usize {
    let mut count = 0;
    unsafe {
        for t in 0..MAX_FD_TABLES {
            for f in 0..MAX_FDS_PER_PROC {
                let e = &FD_TABLES[t].entries[f];
                if (e.kind == FdKind::PipeRead || e.kind == FdKind::NamedPipeRead)
                    && e.index as usize == pipe_idx
                {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Close one end of a named pipe
fn named_pipe_close_end(np_idx: usize, is_read: bool) {
    if np_idx >= MAX_NAMED_PIPES {
        return;
    }

    unsafe {
        let np = &mut NAMED_PIPES[np_idx];
        if !np.in_use {
            return;
        }

        np.refcount = np.refcount.saturating_sub(1);

        let pipe_idx = np.pipe_idx;
        pipe_close_end(pipe_idx, is_read);

        // If no more references, destroy the named pipe
        if np.refcount == 0 {
            np.in_use = false;
            np.path_len = 0;
            println!("[ipc] mkfifo: destroyed FIFO {}", np_idx);
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// SHARED MEMORY
// ═══════════════════════════════════════════════════════════════

/// shmctl operations
pub const SHM_CTL_STAT: u32 = 0; // Get status
pub const SHM_CTL_SET: u32 = 1; // Set permissions
pub const SHM_CTL_RMID: u32 = 2; // Mark for removal
pub const SHM_CTL_LOCK: u32 = 3; // Lock in memory (no-op, stub)
pub const SHM_CTL_UNLOCK: u32 = 4; // Unlock (no-op, stub)
pub const SHM_CTL_INFO: u32 = 5; // System-wide info

/// shmctl result structure (returned via user memory pointer)
#[derive(Clone, Copy)]
#[repr(C)]
pub struct ShmIdDs {
    pub shm_idx: u64,
    pub key: u64,
    pub size_pages: usize,
    pub refcount: u16,
    pub mode: u32,
    pub creator_pid: u64,
    pub attach_count: u16,
    pub is_marked_for_removal: bool,
}

#[derive(Clone, Copy)]
struct ShmRegion {
    base_phys: u64,
    size_pages: usize,
    refcount: u16,
    in_use: bool,
    key: u64,
    creator_pid: u64,
    mode: u32,
    marked_for_removal: bool,
    /// IPC namespace this SHM region was created in
    ns_id: u32,
}

static mut SHM_REGIONS: [ShmRegion; MAX_SHM_REGIONS] = [ShmRegion {
    base_phys: 0,
    size_pages: 0,
    refcount: 0,
    in_use: false,
    key: 0,
    creator_pid: 0,
    mode: 0,
    marked_for_removal: false,
    ns_id: 0,
}; MAX_SHM_REGIONS];

/// Create a shared memory region. Returns the shm index.
pub fn shm_create(size_bytes: usize, key: u64, mode: u32) -> Option<usize> {
    let pid = sched::current_pid();
    let task_idx = find_task_index(pid)?;
    let ns = ipc_ns_id(task_idx);
    let size_pages = (size_bytes + PAGE_SIZE as usize - 1) / (PAGE_SIZE as usize);

    if size_pages == 0 || size_pages > MAX_SHM_SIZE_PAGES {
        return None;
    }

    let idx = unsafe { SHM_REGIONS.iter().position(|r| !r.in_use)? };

    let base = frame_alloc::allocate_contiguous(size_pages)?;
    let base_phys = base.0;

    for i in 0..size_pages {
        let virt = phys_to_virt(PhysAddr::new(base_phys + (i as u64) * PAGE_SIZE)).0;
        unsafe {
            core::ptr::write_bytes(virt as *mut u8, 0, PAGE_SIZE as usize);
        }
    }

    unsafe {
        SHM_REGIONS[idx] = ShmRegion {
            base_phys,
            size_pages,
            refcount: 1,
            in_use: true,
            key,
            creator_pid: pid,
            mode,
            marked_for_removal: false,
            ns_id: ns,
        };
    }

    println!(
        "[ipc] shm: created region {} ({} pages, key={:#x}) for PID {}",
        idx, size_pages, key, pid
    );

    Some(idx)
}

/// Look up a shared memory region by key
pub fn shm_find(key: u64) -> Option<usize> {
    unsafe {
        SHM_REGIONS
            .iter()
            .position(|r| r.in_use && r.key == key && r.key != 0)
    }
}

/// Attach a shared memory region to the current process.
pub fn shm_attach(shm_idx: usize, writable: bool) -> Option<u64> {
    let pid = sched::current_pid();
    let task_idx = find_task_index(pid)?;

    // Namespace check: process must be in the same namespace as the SHM region
    {
        let obj_ns = unsafe { SHM_REGIONS[shm_idx].ns_id };
        if !ipc_ns_check(task_idx, obj_ns) {
            return None;
        }
    }

    if shm_idx >= MAX_SHM_REGIONS {
        return None;
    }

    let (base_phys, size_pages, marked) = unsafe {
        let region = &mut SHM_REGIONS[shm_idx];
        if !region.in_use {
            return None;
        }
        if region.marked_for_removal && region.refcount == 0 {
            return None;
        }
        region.refcount = region.refcount.saturating_add(1);
        (
            region.base_phys,
            region.size_pages,
            region.marked_for_removal,
        )
    };

    let fd_table = get_fd_table(task_idx)?;
    let _fd = fd_table.alloc_fd(FdKind::Shm, shm_idx as u64, if writable { 1 } else { 0 })?;

    let ttbr0 = unsafe { TASK_TABLE[task_idx].as_ref()?.ttbr0_phys };

    if ttbr0 == 0 {
        return None;
    }

    let map_addr = crate::sched::process::mmap_shm(ttbr0, base_phys, size_pages, writable)?;

    println!(
        "[ipc] shm: attached region {} to PID {} at {:#018x}{}",
        shm_idx,
        pid,
        map_addr,
        if marked { " (marked for removal)" } else { "" }
    );

    Some(map_addr)
}

/// Detach a shared memory region (decrement refcount)
pub fn shm_detach(shm_idx: usize) {
    if shm_idx >= MAX_SHM_REGIONS {
        return;
    }

    unsafe {
        let region = &mut SHM_REGIONS[shm_idx];
        if !region.in_use {
            return;
        }
        region.refcount = region.refcount.saturating_sub(1);

        if region.refcount == 0 {
            if region.marked_for_removal {
                // Free the physical frames now
                frame_alloc::deallocate_frame(PhysAddr::new(region.base_phys));
                region.in_use = false;
                region.base_phys = 0;
                region.size_pages = 0;
                region.marked_for_removal = false;
                println!("[ipc] shm: destroyed marked region {}", shm_idx);
            } else {
                println!(
                    "[ipc] shm: region {} refcount=0, still alive (not marked)",
                    shm_idx
                );
            }
        }
    }
}

/// Control operations on shared memory (shmctl)
/// Returns 0 on success, negative on error.
pub fn shm_ctl(shm_idx: usize, cmd: u32, result_ptr: u64) -> isize {
    if shm_idx >= MAX_SHM_REGIONS {
        return -1;
    }

    unsafe {
        let region = &mut SHM_REGIONS[shm_idx];
        if !region.in_use {
            return -1;
        }

        match cmd {
            SHM_CTL_STAT | SHM_CTL_INFO => {
                let ds = ShmIdDs {
                    shm_idx: shm_idx as u64,
                    key: region.key,
                    size_pages: region.size_pages,
                    refcount: region.refcount,
                    mode: region.mode,
                    creator_pid: region.creator_pid,
                    attach_count: region.refcount,
                    is_marked_for_removal: region.marked_for_removal,
                };
                // Copy to user memory
                if result_ptr != 0 {
                    let dst = result_ptr as *mut ShmIdDs;
                    core::ptr::write(dst, ds);
                }
                0
            }
            SHM_CTL_SET => {
                // Read new mode from user memory
                if result_ptr != 0 {
                    let src = result_ptr as *const u32;
                    region.mode = core::ptr::read(src);
                }
                0
            }
            SHM_CTL_RMID => {
                region.marked_for_removal = true;
                if region.refcount == 0 {
                    // No attachments — free immediately
                    frame_alloc::deallocate_frame(PhysAddr::new(region.base_phys));
                    region.in_use = false;
                    region.base_phys = 0;
                    region.size_pages = 0;
                    region.marked_for_removal = false;
                    println!("[ipc] shm: destroyed region {} (IPCRM)", shm_idx);
                } else {
                    println!(
                        "[ipc] shm: region {} marked for removal ({} attachments remain)",
                        shm_idx, region.refcount
                    );
                }
                0
            }
            SHM_CTL_LOCK | SHM_CTL_UNLOCK => {
                // No-op stub — we don't have swapping, so all memory is "locked"
                0
            }
            _ => -22, // -EINVAL
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// MESSAGE QUEUES
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
struct MsgQueue {
    messages: [MsgFrame; MAX_MSG_PER_QUEUE],
    msg_count: usize,
    in_use: bool,
    key: u64,
    creator_pid: u64,
    refcount: u16,
    receivers_waiting: u64,
    /// IPC namespace this message queue was created in
    ns_id: u32,
}

#[derive(Clone, Copy)]
struct MsgFrame {
    mtype: u64,
    data: [u8; MAX_MSG_SIZE],
    data_len: usize,
    valid: bool,
}

static mut MSG_QUEUES: [MsgQueue; MAX_MSG_QUEUES] = [MsgQueue {
    messages: [MsgFrame {
        mtype: 0,
        data: [0u8; MAX_MSG_SIZE],
        data_len: 0,
        valid: false,
    }; MAX_MSG_PER_QUEUE],
    msg_count: 0,
    in_use: false,
    key: 0,
    creator_pid: 0,
    refcount: 0,
    receivers_waiting: 0,
    ns_id: 0,
}; MAX_MSG_QUEUES];

/// Create a message queue. Returns queue index.
pub fn msgq_create(key: u64) -> Option<usize> {
    let pid = sched::current_pid();
    let task_idx = find_task_index(pid)?;
    let ns = ipc_ns_id(task_idx);

    let idx = unsafe { MSG_QUEUES.iter().position(|q| !q.in_use)? };

    unsafe {
        MSG_QUEUES[idx] = MsgQueue {
            messages: [MsgFrame {
                mtype: 0,
                data: [0u8; MAX_MSG_SIZE],
                data_len: 0,
                valid: false,
            }; MAX_MSG_PER_QUEUE],
            msg_count: 0,
            in_use: true,
            key,
            creator_pid: pid,
            refcount: 1,
            receivers_waiting: 0,
            ns_id: ns,
        };
    }

    let _task_idx2 = find_task_index(pid)?;
    let fd_table = get_fd_table(task_idx)?;
    fd_table.alloc_fd(FdKind::MsgQueue, idx as u64, 0);

    println!(
        "[ipc] msgq: created queue {} (key={:#x}) for PID {}",
        idx, key, pid
    );
    Some(idx)
}

/// Find a message queue by key
pub fn msgq_find(key: u64) -> Option<usize> {
    unsafe {
        MSG_QUEUES
            .iter()
            .position(|q| q.in_use && q.key == key && q.key != 0)
    }
}

/// Send a message to a queue. Returns 0 on success, negative on error.
pub fn msgq_send(queue_idx: usize, mtype: u64, data: &[u8]) -> isize {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(i) => i,
        None => return -1,
    };
    // Namespace check
    let obj_ns = unsafe { MSG_QUEUES[queue_idx].ns_id };
    if !ipc_ns_check(task_idx, obj_ns) {
        return -1; // -EACCES: wrong namespace
    }
    if queue_idx >= MAX_MSG_QUEUES {
        return -1;
    }

    unsafe {
        let queue = &mut MSG_QUEUES[queue_idx];
        if !queue.in_use {
            return -1;
        }

        if queue.msg_count >= MAX_MSG_PER_QUEUE {
            return -2; // Queue full
        }

        if data.len() > MAX_MSG_SIZE {
            return -22; // -EINVAL: message too large
        }

        let slot = queue
            .messages
            .iter()
            .position(|m| !m.valid)
            .unwrap_or(MAX_MSG_PER_QUEUE);
        if slot == MAX_MSG_PER_QUEUE {
            return -2;
        }

        queue.messages[slot] = MsgFrame {
            mtype,
            data: {
                let mut d = [0u8; MAX_MSG_SIZE];
                d[..data.len()].copy_from_slice(data);
                d
            },
            data_len: data.len(),
            valid: true,
        };
        queue.msg_count += 1;

        if queue.receivers_waiting != 0 {
            let waiters = queue.receivers_waiting;
            queue.receivers_waiting = 0;
            wake_tasks(waiters);
        }

        0
    }
}

/// Receive a message from a queue.
pub fn msgq_recv(queue_idx: usize, mtype_filter: u64, buf: &mut [u8]) -> (isize, u64) {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(i) => i,
        None => return (-1, 0),
    };
    // Namespace check
    let obj_ns = unsafe { MSG_QUEUES[queue_idx].ns_id };
    if !ipc_ns_check(task_idx, obj_ns) {
        return (-1, 0); // -EACCES: wrong namespace
    }
    if queue_idx >= MAX_MSG_QUEUES {
        return (-1, 0);
    }

    unsafe {
        let queue = &mut MSG_QUEUES[queue_idx];
        if !queue.in_use {
            return (-1, 0);
        }

        for i in 0..MAX_MSG_PER_QUEUE {
            let msg = &queue.messages[i];
            if !msg.valid {
                continue;
            }
            if mtype_filter != 0 && msg.mtype != mtype_filter {
                continue;
            }

            let to_copy = buf.len().min(msg.data_len);
            buf[..to_copy].copy_from_slice(&msg.data[..to_copy]);
            let mtype = msg.mtype;
            let len = msg.data_len;

            queue.messages[i].valid = false;
            queue.msg_count = queue.msg_count.saturating_sub(1);

            return (len as isize, mtype);
        }

        let task_idx = find_task_index(sched::current_pid()).unwrap_or(0);
        queue.receivers_waiting |= 1 << task_idx;
        (-2, 0) // Would block
    }
}

// ═══════════════════════════════════════════════════════════════
// SEMAPHORES
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
struct Semaphore {
    value: u32,
    in_use: bool,
    creator_pid: u64,
    waiters: u64,
    /// IPC namespace this semaphore was created in
    ns_id: u32,
}

static mut SEMAPHORES: [Semaphore; MAX_SEMAS] = [Semaphore {
    value: 0,
    in_use: false,
    creator_pid: 0,
    waiters: 0,
    ns_id: 0,
}; MAX_SEMAS];

/// Create a semaphore with initial value. Returns semaphore index.
pub fn sema_create(initial_value: u32) -> Option<usize> {
    let pid = sched::current_pid();
    let task_idx = find_task_index(pid)?;
    let ns = ipc_ns_id(task_idx);

    if initial_value > MAX_SEMA_VALUE {
        return None;
    }

    let idx = unsafe { SEMAPHORES.iter().position(|s| !s.in_use)? };

    unsafe {
        SEMAPHORES[idx] = Semaphore {
            value: initial_value,
            in_use: true,
            creator_pid: pid,
            waiters: 0,
            ns_id: ns,
        };
    }
    println!(
        "[ipc] sema: created semaphore {} (value={}) for PID {}",
        idx, initial_value, pid
    );
    Some(idx)
}

/// Wait (P) operation — decrement, or block if zero
pub fn sema_wait(sema_idx: usize) -> isize {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(i) => i,
        None => return -1,
    };
    // Namespace check
    let obj_ns = unsafe { SEMAPHORES[sema_idx].ns_id };
    if !ipc_ns_check(task_idx, obj_ns) {
        return -1; // -EACCES
    }
    if sema_idx >= MAX_SEMAS {
        return -1;
    }

    unsafe {
        let sema = &mut SEMAPHORES[sema_idx];
        if !sema.in_use {
            return -1;
        }

        if sema.value > 0 {
            sema.value -= 1;
            0
        } else {
            let task_idx = find_task_index(sched::current_pid()).unwrap_or(0);
            sema.waiters |= 1 << task_idx;
            -2 // Would block
        }
    }
}

/// Post (V) operation — increment, wake one waiter if any
pub fn sema_post(sema_idx: usize) -> isize {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(i) => i,
        None => return -1,
    };
    // Namespace check
    let obj_ns = unsafe { SEMAPHORES[sema_idx].ns_id };
    if !ipc_ns_check(task_idx, obj_ns) {
        return -1; // -EACCES
    }
    if sema_idx >= MAX_SEMAS {
        return -1;
    }

    unsafe {
        let sema = &mut SEMAPHORES[sema_idx];
        if !sema.in_use {
            return -1;
        }

        if sema.waiters != 0 {
            let waiter = sema.waiters.trailing_zeros() as usize;
            sema.waiters &= !(1 << waiter);

            if let Some(ref mut task) = TASK_TABLE[waiter] {
                if task.state == TaskState::Blocked {
                    task.state = TaskState::Ready;
                }
            }
        } else if sema.value < MAX_SEMA_VALUE {
            sema.value += 1;
        }
    }

    0
}

/// Get current semaphore value
pub fn sema_getvalue(sema_idx: usize) -> Option<u32> {
    if sema_idx >= MAX_SEMAS {
        return None;
    }
    unsafe {
        if SEMAPHORES[sema_idx].in_use {
            Some(SEMAPHORES[sema_idx].value)
        } else {
            None
        }
    }
}

/// Destroy a semaphore
pub fn sema_destroy(sema_idx: usize) -> bool {
    if sema_idx >= MAX_SEMAS {
        return false;
    }

    unsafe {
        let sema = &mut SEMAPHORES[sema_idx];
        if !sema.in_use {
            return false;
        }

        if sema.waiters != 0 {
            wake_tasks(sema.waiters);
        }

        sema.in_use = false;
        sema.value = 0;
        sema.waiters = 0;
    }

    true
}

// ═══════════════════════════════════════════════════════════════
// UNIX DOMAIN SOCKETS
// ═══════════════════════════════════════════════════════════════

/// Socket types
pub const SOCK_STREAM: u32 = 1;
pub const SOCK_DGRAM: u32 = 2;

/// Socket states
#[derive(Clone, Copy, PartialEq)]
enum SockState {
    Unconnected,
    Listening,
    Connected,
    Disconnected,
}

#[derive(Clone, Copy)]
struct UnixSocket {
    /// Backing circular buffer (one page)
    buffer_phys: u64,
    read_pos: usize,
    write_pos: usize,
    count: usize,
    /// Socket type (SOCK_STREAM or SOCK_DGRAM)
    sock_type: u32,
    /// Current state
    state: SockState,
    /// Connected peer socket index (MAX_UNIX_SOCKETS if none)
    peer_idx: usize,
    /// Tasks waiting to read
    readers_waiting: u64,
    /// Tasks waiting to write
    writers_waiting: u64,
    /// Tasks waiting to connect/accept
    connect_waiters: u64,
    /// Reference count
    refcount: u16,
    /// Is this slot in use?
    in_use: bool,
    /// Creator PID
    creator_pid: u64,
    /// IPC namespace this socket was created in
    ns_id: u32,
}

static mut UNIX_SOCKETS: [UnixSocket; MAX_UNIX_SOCKETS] = [UnixSocket {
    buffer_phys: 0,
    read_pos: 0,
    write_pos: 0,
    count: 0,
    sock_type: 0,
    state: SockState::Unconnected,
    peer_idx: MAX_UNIX_SOCKETS,
    readers_waiting: 0,
    writers_waiting: 0,
    connect_waiters: 0,
    refcount: 0,
    in_use: false,
    creator_pid: 0,
    ns_id: 0,
}; MAX_UNIX_SOCKETS];

/// Create a Unix domain socket. Returns (socket_idx, fd).
pub fn unix_socket(sock_type: u32) -> Option<(usize, usize)> {
    let pid = sched::current_pid();
    let task_idx = find_task_index(pid)?;
    let ns = ipc_ns_id(task_idx);

    if sock_type != SOCK_STREAM && sock_type != SOCK_DGRAM {
        return None;
    }

    let idx = unsafe { UNIX_SOCKETS.iter().position(|s| !s.in_use)? };

    // Allocate buffer page
    let frame = frame_alloc::allocate_frame()?;
    let buffer_phys = frame.0;
    let buffer_virt = phys_to_virt(PhysAddr::new(buffer_phys)).0;
    unsafe {
        core::ptr::write_bytes(buffer_virt as *mut u8, 0, UNIX_SOCK_BUF_SIZE);
        UNIX_SOCKETS[idx] = UnixSocket {
            buffer_phys,
            read_pos: 0,
            write_pos: 0,
            count: 0,
            sock_type,
            state: SockState::Unconnected,
            peer_idx: MAX_UNIX_SOCKETS,
            readers_waiting: 0,
            writers_waiting: 0,
            connect_waiters: 0,
            refcount: 1,
            in_use: true,
            creator_pid: pid,
            ns_id: ns,
        };
    }

    let fd_table = get_fd_table(task_idx)?;
    let fd = fd_table.alloc_fd(FdKind::UnixSocket, idx as u64, O_RDWR)?;

    println!(
        "[ipc] unix_socket: created socket {} (type={}) for PID {}",
        idx, sock_type, pid
    );

    Some((idx, fd))
}

/// Connect two Unix sockets (creates a socketpair-like connection).
/// Both sockets must be unconnected.
pub fn unix_connect(sock_a: usize, sock_b: usize) -> isize {
    if sock_a >= MAX_UNIX_SOCKETS || sock_b >= MAX_UNIX_SOCKETS || sock_a == sock_b {
        return -22; // -EINVAL
    }

    unsafe {
        let sa = &mut UNIX_SOCKETS[sock_a];
        let sb = &mut UNIX_SOCKETS[sock_b];

        if !sa.in_use || !sb.in_use {
            return -1; // -EBADF
        }

        if sa.state != SockState::Unconnected || sb.state != SockState::Unconnected {
            return -106; // -EISCONN
        }

        if sa.sock_type != sb.sock_type {
            return -22; // -EINVAL
        }

        sa.peer_idx = sock_b;
        sb.peer_idx = sock_a;
        sa.state = SockState::Connected;
        sb.state = SockState::Connected;

        // Wake any tasks waiting for connection
        if sa.connect_waiters != 0 {
            let w = sa.connect_waiters;
            sa.connect_waiters = 0;
            wake_tasks(w);
        }
        if sb.connect_waiters != 0 {
            let w = sb.connect_waiters;
            sb.connect_waiters = 0;
            wake_tasks(w);
        }
    }

    println!("[ipc] unix_socket: connected {} <-> {}", sock_a, sock_b);
    0
}

/// Read from a Unix socket. For connected sockets, reads from peer's buffer.
pub fn unix_recv(task_idx: usize, fd: usize, buf: &mut [u8]) -> isize {
    // Namespace check
    {
        let sock_idx_pre = get_fd_table(task_idx)
            .and_then(|t| t.get(fd))
            .map(|e| e.index as usize)
            .unwrap_or(MAX_UNIX_SOCKETS);
        if sock_idx_pre < MAX_UNIX_SOCKETS {
            let obj_ns = unsafe { UNIX_SOCKETS[sock_idx_pre].ns_id };
            if !ipc_ns_check(task_idx, obj_ns) {
                return -1; // -EACCES: wrong namespace
            }
        }
    }

    let (sock_idx, _flags) = {
        let fd_table = match get_fd_table(task_idx) {
            Some(t) => t,
            None => return -1,
        };
        let entry = match fd_table.get(fd) {
            Some(e) => e,
            None => return -1,
        };
        if entry.kind != FdKind::UnixSocket {
            return -1;
        }
        (entry.index as usize, entry.flags)
    };

    if sock_idx >= MAX_UNIX_SOCKETS {
        return -1;
    }

    unsafe {
        let sock = &mut UNIX_SOCKETS[sock_idx];
        if !sock.in_use {
            return -1;
        }

        // For connected stream sockets, read from own buffer (data was written by peer)
        if sock.count == 0 {
            if sock.state == SockState::Disconnected {
                return 0; // EOF
            }
            if sock.state == SockState::Connected {
                sock.readers_waiting |= 1 << task_idx;
                return -2; // Would block
            }
            return -107; // -ENOTCONN
        }

        let to_read = buf.len().min(sock.count);
        let buffer_virt = phys_to_virt(PhysAddr::new(sock.buffer_phys)).0 as *const u8;

        for i in 0..to_read {
            buf[i] = *buffer_virt.add(sock.read_pos);
            sock.read_pos = (sock.read_pos + 1) % UNIX_SOCK_BUF_SIZE;
        }
        sock.count -= to_read;

        // Wake peer if they were waiting to write
        if sock.peer_idx < MAX_UNIX_SOCKETS {
            let peer = &mut UNIX_SOCKETS[sock.peer_idx];
            if peer.writers_waiting != 0 {
                let w = peer.writers_waiting;
                peer.writers_waiting = 0;
                wake_tasks(w);
            }
        }

        to_read as isize
    }
}

/// Write to a Unix socket. For connected sockets, writes to peer's buffer.
pub fn unix_send(task_idx: usize, fd: usize, buf: &[u8]) -> isize {
    // Namespace check
    {
        let sock_idx_pre = get_fd_table(task_idx)
            .and_then(|t| t.get(fd))
            .map(|e| e.index as usize)
            .unwrap_or(MAX_UNIX_SOCKETS);
        if sock_idx_pre < MAX_UNIX_SOCKETS {
            let obj_ns = unsafe { UNIX_SOCKETS[sock_idx_pre].ns_id };
            if !ipc_ns_check(task_idx, obj_ns) {
                return -1; // -EACCES: wrong namespace
            }
        }
    }

    let (sock_idx, _flags) = {
        let fd_table = match get_fd_table(task_idx) {
            Some(t) => t,
            None => return -1,
        };
        let entry = match fd_table.get(fd) {
            Some(e) => e,
            None => return -1,
        };
        if entry.kind != FdKind::UnixSocket {
            return -1;
        }
        (entry.index as usize, entry.flags)
    };

    if sock_idx >= MAX_UNIX_SOCKETS {
        return -1;
    }

    unsafe {
        let (in_use, state, peer_idx_val) = {
            let sock = &UNIX_SOCKETS[sock_idx];
            (sock.in_use, sock.state, sock.peer_idx)
        };

        if !in_use {
            return -1;
        }

        if state != SockState::Connected {
            return -107; // -ENOTCONN
        }

        if peer_idx_val >= MAX_UNIX_SOCKETS {
            return -107;
        }

        // Write to peer's buffer
        let peer = &mut UNIX_SOCKETS[peer_idx_val];
        let available = UNIX_SOCK_BUF_SIZE - peer.count;

        if available == 0 {
            UNIX_SOCKETS[sock_idx].writers_waiting |= 1 << task_idx;
            return -2; // Would block
        }

        let to_write = buf.len().min(available);
        let buffer_virt = phys_to_virt(PhysAddr::new(peer.buffer_phys)).0 as *mut u8;

        for i in 0..to_write {
            *buffer_virt.add(peer.write_pos) = buf[i];
            peer.write_pos = (peer.write_pos + 1) % UNIX_SOCK_BUF_SIZE;
        }
        peer.count += to_write;

        // Wake peer if they were waiting to read
        if peer.readers_waiting != 0 {
            let w = peer.readers_waiting;
            peer.readers_waiting = 0;
            wake_tasks(w);
        }

        to_write as isize
    }
}

// ═══════════════════════════════════════════════════════════════
// EVENTFD
// ═══════════════════════════════════════════════════════════════

/// Eventfd flags
pub const EFD_SEMAPHORE: u32 = 1;
pub const EFD_NONBLOCK: u32 = 0o4000;
pub const EFD_CLOEXEC: u32 = 0o2000000;

#[derive(Clone, Copy)]
struct EventfdObj {
    /// Current counter value
    counter: u64,
    /// Is this slot in use?
    in_use: bool,
    /// Creator PID
    creator_pid: u64,
    /// Reference count
    refcount: u16,
    /// Flags (EFD_SEMAPHORE, EFD_NONBLOCK)
    flags: u32,
    /// Tasks waiting for read (counter > 0)
    readers_waiting: u64,
    /// Tasks waiting for write (counter < u64::MAX)
    writers_waiting: u64,
    /// IPC namespace this eventfd was created in
    ns_id: u32,
}

static mut EVENTFDS: [EventfdObj; MAX_EVENTFDS] = [EventfdObj {
    counter: 0,
    in_use: false,
    creator_pid: 0,
    refcount: 0,
    flags: 0,
    readers_waiting: 0,
    writers_waiting: 0,
    ns_id: 0,
}; MAX_EVENTFDS];

/// Create an eventfd. Returns (eventfd_idx, fd).
pub fn eventfd(initval: u64, flags: u32) -> Option<(usize, usize)> {
    let pid = sched::current_pid();
    let task_idx = find_task_index(pid)?;
    let ns = ipc_ns_id(task_idx);

    let idx = unsafe { EVENTFDS.iter().position(|e| !e.in_use)? };

    unsafe {
        EVENTFDS[idx] = EventfdObj {
            counter: initval,
            in_use: true,
            creator_pid: pid,
            refcount: 1,
            flags,
            readers_waiting: 0,
            writers_waiting: 0,
            ns_id: ns,
        };
    }

    let fd_table = get_fd_table(task_idx)?;
    let fd = fd_table.alloc_fd(FdKind::Eventfd, idx as u64, flags)?;

    println!(
        "[ipc] eventfd: created efd {} (init={}, flags={:#x}) for PID {}",
        idx, initval, flags, pid
    );

    Some((idx, fd))
}

/// Read from eventfd: returns the counter value and resets it to 0
/// (or decrements by 1 in semaphore mode).
/// Returns the value on success, or negative on error.
pub fn eventfd_read(task_idx: usize, fd: usize) -> i64 {
    // Namespace check
    {
        let efd_idx_pre = get_fd_table(task_idx)
            .and_then(|t| t.get(fd))
            .map(|e| e.index as usize)
            .unwrap_or(MAX_EVENTFDS);
        if efd_idx_pre < MAX_EVENTFDS {
            let obj_ns = unsafe { EVENTFDS[efd_idx_pre].ns_id };
            if !ipc_ns_check(task_idx, obj_ns) {
                return -1; // -EACCES: wrong namespace
            }
        }
    }

    let efd_idx = {
        let fd_table = match get_fd_table(task_idx) {
            Some(t) => t,
            None => return -1,
        };
        let entry = match fd_table.get(fd) {
            Some(e) => e,
            None => return -1,
        };
        if entry.kind != FdKind::Eventfd {
            return -1;
        }
        entry.index as usize
    };

    if efd_idx >= MAX_EVENTFDS {
        return -1;
    }

    unsafe {
        let efd = &mut EVENTFDS[efd_idx];
        if !efd.in_use {
            return -1;
        }

        if efd.counter == 0 {
            if efd.flags & EFD_NONBLOCK != 0 {
                return -11; // -EAGAIN
            }
            efd.readers_waiting |= 1 << task_idx;
            return -2; // Would block
        }

        let value = if efd.flags & EFD_SEMAPHORE != 0 {
            // Semaphore mode: return 1 and decrement by 1
            efd.counter = efd.counter.saturating_sub(1);
            1u64
        } else {
            // Normal mode: return counter and reset to 0
            let v = efd.counter;
            efd.counter = 0;
            v
        };

        // Wake any waiting writers
        if efd.writers_waiting != 0 {
            let w = efd.writers_waiting;
            efd.writers_waiting = 0;
            wake_tasks(w);
        }

        value as i64
    }
}

/// Write to eventfd: add value to the counter.
/// Returns 0 on success, negative on error.
pub fn eventfd_write(task_idx: usize, fd: usize, value: u64) -> i64 {
    if value == u64::MAX {
        return -22; // -EINVAL: can't write u64::MAX
    }

    // Namespace check
    {
        let efd_idx_pre = get_fd_table(task_idx)
            .and_then(|t| t.get(fd))
            .map(|e| e.index as usize)
            .unwrap_or(MAX_EVENTFDS);
        if efd_idx_pre < MAX_EVENTFDS {
            let obj_ns = unsafe { EVENTFDS[efd_idx_pre].ns_id };
            if !ipc_ns_check(task_idx, obj_ns) {
                return -1; // -EACCES: wrong namespace
            }
        }
    }

    let efd_idx = {
        let fd_table = match get_fd_table(task_idx) {
            Some(t) => t,
            None => return -1,
        };
        let entry = match fd_table.get(fd) {
            Some(e) => e,
            None => return -1,
        };
        if entry.kind != FdKind::Eventfd {
            return -1;
        }
        entry.index as usize
    };

    if efd_idx >= MAX_EVENTFDS {
        return -1;
    }

    unsafe {
        let efd = &mut EVENTFDS[efd_idx];
        if !efd.in_use {
            return -1;
        }

        // Check for overflow
        if efd.counter.checked_add(value).is_none() || efd.counter + value > u64::MAX - 1 {
            if efd.flags & EFD_NONBLOCK != 0 {
                return -11; // -EAGAIN
            }
            efd.writers_waiting |= 1 << task_idx;
            return -2; // Would block
        }

        efd.counter += value;

        // Wake any waiting readers
        if efd.readers_waiting != 0 {
            let w = efd.readers_waiting;
            efd.readers_waiting = 0;
            wake_tasks(w);
        }
    }

    0
}

// ═══════════════════════════════════════════════════════════════
// PSEUDO-TERMINAL (PTY) SUPPORT
// ═══════════════════════════════════════════════════════════════

/// PTY object — master/slave pair for terminal emulation
#[derive(Clone, Copy)]
struct PtyObj {
    /// IPC namespace this PTY was created in
    ns_id: u32,
    /// Master side buffer (written by terminal emulator, read by shell)
    master_buffer: [u8; PTY_BUFFER_SIZE],
    master_read_pos: usize,
    master_write_pos: usize,
    /// Slave side buffer (written by shell, read by terminal emulator)
    slave_buffer: [u8; PTY_BUFFER_SIZE],
    slave_read_pos: usize,
    slave_write_pos: usize,
    /// Window size (for TIOCGWINSZ/TIOCSWINSZ)
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
    /// Process group for SIGWINCH
    pgrp: u32,
    /// Reference count
    refcount: u16,
    /// Whether this slot is in use
    in_use: bool,
    /// Master fd open count
    master_open: bool,
    /// Slave fd open count
    slave_open: bool,
}

impl PtyObj {
    const fn empty() -> Self {
        Self {
            ns_id: 0,
            master_buffer: [0; PTY_BUFFER_SIZE],
            master_read_pos: 0,
            master_write_pos: 0,
            slave_buffer: [0; PTY_BUFFER_SIZE],
            slave_read_pos: 0,
            slave_write_pos: 0,
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
            pgrp: 0,
            refcount: 0,
            in_use: false,
            master_open: false,
            slave_open: false,
        }
    }

    /// Read from master (terminal emulator reads shell output)
    fn master_read(&mut self, buf: &mut [u8]) -> usize {
        let available = self.master_write_pos.saturating_sub(self.master_read_pos);
        if available == 0 {
            return 0;
        }
        let len = available.min(buf.len());
        buf[..len].copy_from_slice(&self.master_buffer[self.master_read_pos..self.master_read_pos + len]);
        self.master_read_pos += len;
        if self.master_read_pos == self.master_write_pos {
            self.master_read_pos = 0;
            self.master_write_pos = 0;
        }
        len
    }

    /// Write to master (shell writes output)
    fn master_write(&mut self, buf: &[u8]) -> usize {
        let available = PTY_BUFFER_SIZE.saturating_sub(self.master_write_pos);
        if available == 0 {
            return 0;
        }
        let len = available.min(buf.len());
        self.master_buffer[self.master_write_pos..self.master_write_pos + len].copy_from_slice(&buf[..len]);
        self.master_write_pos += len;
        len
    }

    /// Read from slave (shell reads input from terminal)
    fn slave_read(&mut self, buf: &mut [u8]) -> usize {
        let available = self.slave_write_pos.saturating_sub(self.slave_read_pos);
        if available == 0 {
            return 0;
        }
        let len = available.min(buf.len());
        buf[..len].copy_from_slice(&self.slave_buffer[self.slave_read_pos..self.slave_read_pos + len]);
        self.slave_read_pos += len;
        if self.slave_read_pos == self.slave_write_pos {
            self.slave_read_pos = 0;
            self.slave_write_pos = 0;
        }
        len
    }

    /// Write to slave (terminal emulator writes input)
    fn slave_write(&mut self, buf: &[u8]) -> usize {
        let available = PTY_BUFFER_SIZE.saturating_sub(self.slave_write_pos);
        if available == 0 {
            return 0;
        }
        let len = available.min(buf.len());
        self.slave_buffer[self.slave_write_pos..self.slave_write_pos + len].copy_from_slice(&buf[..len]);
        self.slave_write_pos += len;
        len
    }
}

static mut PTYS: [PtyObj; MAX_PTYS] = [PtyObj::empty(); MAX_PTYS];

/// Create a new PTY master/slave pair. Returns (master_idx, slave_idx).
/// The master is typically opened by the terminal emulator, the slave by the shell.
pub fn pty_open() -> Option<(usize, usize)> {
    let ns_id = ipc_ns_id(sched::find_task_index(sched::current_pid())?);

    // Find free PTY slot
    let idx = unsafe { PTYS.iter().position(|p| !p.in_use)? };

    unsafe {
        let pty = &mut PTYS[idx];
        pty.ns_id = ns_id;
        pty.master_read_pos = 0;
        pty.master_write_pos = 0;
        pty.slave_read_pos = 0;
        pty.slave_write_pos = 0;
        pty.ws_row = 24;
        pty.ws_col = 80;
        pty.pgrp = sched::current_pid() as u32;
        pty.refcount = 2; // master + slave
        pty.in_use = true;
        pty.master_open = true;
        pty.slave_open = true;
    }

    // Create master and slave fd entries
    let task_idx = sched::find_task_index(sched::current_pid())?;
    let fd_table = get_fd_table(task_idx)?;

    // Master fd (for terminal emulator)
    let master_fd = fd_table.alloc_fd(FdKind::PtyMaster, idx as u64, 0)?;
    // Slave fd (for shell)
    let slave_fd = fd_table.alloc_fd(FdKind::PtySlave, idx as u64, 0)?;

    Some((master_fd, slave_fd))
}

/// Get PTY object by index
fn get_pty(idx: usize) -> Option<&'static mut PtyObj> {
    if idx < MAX_PTYS {
        unsafe {
            if PTYS[idx].in_use {
                return Some(&mut PTYS[idx]);
            }
        }
    }
    None
}

/// Read from PTY master (terminal emulator reads shell output)
pub fn pty_master_read(master_idx: usize, buf: &mut [u8]) -> usize {
    if let Some(pty) = get_pty(master_idx) {
        pty.master_read(buf)
    } else {
        0
    }
}

/// Write to PTY master (shell writes output)
pub fn pty_master_write(master_idx: usize, buf: &[u8]) -> usize {
    if let Some(pty) = get_pty(master_idx) {
        pty.master_write(buf)
    } else {
        0
    }
}

/// Read from PTY slave (shell reads input)
pub fn pty_slave_read(slave_idx: usize, buf: &mut [u8]) -> usize {
    if let Some(pty) = get_pty(slave_idx) {
        pty.slave_read(buf)
    } else {
        0
    }
}

/// Write to PTY slave (terminal emulator writes input)
pub fn pty_slave_write(slave_idx: usize, buf: &[u8]) -> usize {
    if let Some(pty) = get_pty(slave_idx) {
        pty.slave_write(buf)
    } else {
        0
    }
}

/// Close PTY master/slave
pub fn pty_close(idx: usize, is_master: bool) {
    if let Some(pty) = get_pty(idx) {
        if is_master {
            pty.master_open = false;
        } else {
            pty.slave_open = false;
        }
        if !pty.master_open && !pty.slave_open {
            // Both closed, free the PTY
            pty.in_use = false;
        }
    }
}

/// PTY IOCTL for window size, etc.
pub fn pty_ioctl(idx: usize, cmd: u64, arg: u64) -> i64 {
    use crate::sched;
    if let Some(pty) = get_pty(idx) {
        match cmd {
            // TIOCGWINSZ - get window size
            0x5413 => {
                // arg is pointer to winsize struct
                // For now, just return success
                0
            }
            // TIOCSWINSZ - set window size
            0x5414 => {
                0
            }
            // TIOCGPGRP - get process group
            0x540F => {
                // arg is pointer to pid_t
                0
            }
            // TIOCSPGRP - set process group
            0x5410 => {
                // arg is pid_t
                if let Ok(pgrp) = u32::try_from(arg) {
                    pty.pgrp = pgrp;
                }
                0
            }
            _ => -25, // -ENOTTY
        }
    } else {
        -1
    }
}

// ═══════════════════════════════════════════════════════════════
// FD MANAGEMENT — close, dup, dup2
// ═══════════════════════════════════════════════════════════════

/// Close a file descriptor, releasing the underlying IPC object if refcount hits 0
pub fn fd_close(task_idx: usize, fd: usize) -> bool {
    let entry = {
        let fd_table = match get_fd_table(task_idx) {
            Some(t) => t,
            None => return false,
        };
        match fd_table.release(fd) {
            Some(e) => e,
            None => return false,
        }
    };

    close_ipc_object(&entry);
    true
}

/// Duplicate a file descriptor (dup). Returns new fd.
pub fn fd_dup(task_idx: usize, old_fd: usize) -> Option<usize> {
    let fd_table = get_fd_table(task_idx)?;
    let entry = fd_table.get(old_fd)?.clone();
    let new_fd = fd_table.alloc_fd(entry.kind, entry.index, entry.flags)?;
    increment_ipc_refcount(entry);
    Some(new_fd)
}

/// Duplicate to a specific fd (dup2). Returns new_fd.
pub fn fd_dup2(task_idx: usize, old_fd: usize, new_fd: usize) -> Option<usize> {
    if new_fd >= MAX_FDS_PER_PROC {
        return None;
    }

    let fd_table = get_fd_table(task_idx)?;
    let entry = fd_table.get(old_fd)?.clone();

    // Close new_fd first (ignore errors if not open)
    fd_close(task_idx, new_fd);

    let fd_table = get_fd_table(task_idx)?;
    fd_table.entries[new_fd] = entry;

    increment_ipc_refcount(entry);
    Some(new_fd)
}

// ═══════════════════════════════════════════════════════════════
// HELPER: wake tasks from a bitmask
// ═══════════════════════════════════════════════════════════════

fn wake_tasks(task_mask: u64) {
    for i in 0..64 {
        if task_mask & (1 << i) != 0 {
            unsafe {
                if let Some(ref mut task) = TASK_TABLE[i] {
                    if task.state == TaskState::Blocked {
                        task.state = TaskState::Ready;
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// INITIALIZATION
// ═══════════════════════════════════════════════════════════════

/// Initialize the IPC subsystem (called during kernel boot)
pub fn init() {
    println!("[ipc] Initializing IPC subsystem...");
    println!(
        "[ipc] Pipes: {} slots × {} byte buffer",
        MAX_PIPES, PIPE_BUFFER_SIZE
    );
    println!(
        "[ipc] Named pipes (FIFOs): {} slots × {} byte path",
        MAX_NAMED_PIPES, NAMED_PIPE_PATH_MAX
    );
    println!(
        "[ipc] Shared memory: {} regions × {} pages max",
        MAX_SHM_REGIONS, MAX_SHM_SIZE_PAGES
    );
    println!(
        "[ipc] Message queues: {} × {} messages × {} bytes",
        MAX_MSG_QUEUES, MAX_MSG_PER_QUEUE, MAX_MSG_SIZE
    );
    println!(
        "[ipc] Semaphores: {} slots (max value {})",
        MAX_SEMAS, MAX_SEMA_VALUE
    );
    println!(
        "[ipc] Unix domain sockets: {} slots × {} byte buffer",
        MAX_UNIX_SOCKETS, UNIX_SOCK_BUF_SIZE
    );
    println!("[ipc] Eventfds: {} slots", MAX_EVENTFDS);
    println!(
        "[ipc] FD tables: {} × {} entries (O_CLOEXEC supported)",
        MAX_FD_TABLES, MAX_FDS_PER_PROC
    );
}
