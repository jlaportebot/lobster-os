//! Syscall dispatch — routes EL0 SVC calls to kernel handlers
//!
//! Syscall numbers:
//!  0 = exit(code)
//!  1 = write(fd, buf, len)
//!  2 = read(fd, buf, len)
//!  3 = open(path, flags)
//!  4 = close(fd)
//!  5 = yield()
//!  6 = getpid()
//!  7 = sleep(ms)
//!  8 = fork()
//!  9 = exec(path)
//! 10 = wait(pid)
//! 11 = mmap(addr, len, prot, flags)
//! 12 = munmap(addr, len)
//! 13 = pipe(read_fd_ptr, write_fd_ptr)
//! 14 = dup(old_fd)
//! 15 = dup2(old_fd, new_fd)
//! 16 = shm_create(size, key, mode)
//! 17 = shm_attach(shm_idx, writable)
//! 18 = shm_detach(shm_idx)
//! 19 = msgq_create(key)
//! 20 = msgq_send(queue_idx, mtype, data_ptr, data_len)
//! 21 = msgq_recv(queue_idx, mtype_filter, buf_ptr, buf_len)
//! 22 = sema_create(initial_value)
//! 23 = sema_wait(sema_idx)
//! 24 = sema_post(sema_idx)
//! 25 = sema_getvalue(sema_idx)
//! 26 = sema_destroy(sema_idx)
//! 27 = kill(pid, sig)
//! 28 = sigaction(sig, handler)
//! 29 = mkfifo(path_ptr, path_len, mode)
//! 30 = open_fifo(path_ptr, path_len, flags)
//! 31 = shm_ctl(shm_idx, cmd, arg)
//! 32 = unix_socket(sock_type)
//! 33 = unix_bind(sock_idx, path_ptr, path_len)
//! 34 = unix_listen(sock_idx, backlog)
//! 35 = unix_accept(sock_idx)
//! 36 = unix_connect(sock_idx, path_ptr, path_len)
//! 37 = unix_send(sock_idx, buf_ptr, len, task_idx)
//! 38 = unix_recv(sock_idx, buf_ptr, len, task_idx)
//! 39 = eventfd(initval, flags)
//! 40 = eventfd_read(efd_idx, task_idx)
//! 41 = eventfd_write(efd_idx, val, task_idx)
//! 42 = fcntl(fd, cmd, arg)

use crate::println;
use crate::sched;

/// Syscall numbers
pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_READ: u64 = 2;
pub const SYS_OPEN: u64 = 3;
pub const SYS_CLOSE: u64 = 4;
pub const SYS_YIELD: u64 = 5;
pub const SYS_GETPID: u64 = 6;
pub const SYS_SLEEP: u64 = 7;
pub const SYS_FORK: u64 = 8;
pub const SYS_EXEC: u64 = 9;
pub const SYS_WAIT: u64 = 10;
pub const SYS_MMAP: u64 = 11;
pub const SYS_MUNMAP: u64 = 12;
pub const SYS_PIPE: u64 = 13;
pub const SYS_DUP: u64 = 14;
pub const SYS_DUP2: u64 = 15;
pub const SYS_SHM_CREATE: u64 = 16;
pub const SYS_SHM_ATTACH: u64 = 17;
pub const SYS_SHM_DETACH: u64 = 18;
pub const SYS_MSGQ_CREATE: u64 = 19;
pub const SYS_MSGQ_SEND: u64 = 20;
pub const SYS_MSGQ_RECV: u64 = 21;
pub const SYS_SEMA_CREATE: u64 = 22;
pub const SYS_SEMA_WAIT: u64 = 23;
pub const SYS_SEMA_POST: u64 = 24;
pub const SYS_SEMA_GETVALUE: u64 = 25;
pub const SYS_SEMA_DESTROY: u64 = 26;
pub const SYS_KILL: u64 = 27;
pub const SYS_SIGACTION: u64 = 28;
pub const SYS_MKFIFO: u64 = 29;
pub const SYS_OPEN_FIFO: u64 = 30;
pub const SYS_SHM_CTL: u64 = 31;
pub const SYS_UNIX_SOCKET: u64 = 32;
pub const SYS_UNIX_BIND: u64 = 33;
pub const SYS_UNIX_LISTEN: u64 = 34;
pub const SYS_UNIX_ACCEPT: u64 = 35;
pub const SYS_UNIX_CONNECT: u64 = 36;
pub const SYS_UNIX_SEND: u64 = 37;
pub const SYS_UNIX_RECV: u64 = 38;
pub const SYS_EVENTFD: u64 = 39;
pub const SYS_EVENTFD_READ: u64 = 40;
pub const SYS_EVENTFD_WRITE: u64 = 41;
pub const SYS_FCNTL: u64 = 42;
pub const SYS_RT_SIGRETURN: u64 = 43;
pub const SYS_IPC_UNSHARE: u64 = 44;
pub const SYS_IPC_SETNS: u64 = 45;
pub const SYS_IPC_SET_MODE: u64 = 46;

// ── Security / credential syscalls ──
pub const SYS_SETUID: u64 = 47;
pub const SYS_SETGID: u64 = 48;
pub const SYS_SETEUID: u64 = 49;
pub const SYS_SETEGID: u64 = 50;
pub const SYS_GETUID: u64 = 51;
pub const SYS_GETGID: u64 = 52;
pub const SYS_GETEUID: u64 = 53;
pub const SYS_GETEGID: u64 = 54;
pub const SYS_GETGROUPS: u64 = 55;
pub const SYS_SETGROUPS: u64 = 56;
pub const SYS_CAPGET: u64 = 57;
pub const SYS_CAPSET: u64 = 58;
pub const SYS_CHMOD: u64 = 59;
pub const SYS_CHOWN: u64 = 60;
pub const SYS_SET_NO_NEW_PRIVS: u64 = 61;
pub const SYS_GET_NO_NEW_PRIVS: u64 = 62;

/// Maximum syscall number
const MAX_SYSCALL: u64 = 62;

/// Dispatch a syscall from EL0
///
/// Called by the EL0 synchronous exception handler when it detects
/// an SVC instruction. Arguments come from the saved trap frame.
///
/// Returns the value to put in x0 of the calling process.
pub fn dispatch(syscall_num: u64, args: &[u64; 6]) -> u64 {
    if syscall_num > MAX_SYSCALL {
        println!(
            "[syscall] Invalid syscall #{} from PID {}",
            syscall_num,
            sched::current_pid()
        );
        return u64::MAX; // -1
    }

    match syscall_num {
        SYS_EXIT => {
            let code = args[0] as i64;
            sys_exit(code);
        }
        SYS_WRITE => sys_write(args[0] as i32, args[1], args[2] as usize),
        SYS_READ => sys_read(args[0] as i32, args[1], args[2] as usize),
        SYS_OPEN => sys_open(args[1] as *const u8, args[2] as u32),
        SYS_CLOSE => sys_close(args[0] as i32),
        SYS_YIELD => {
            sys_yield();
            0
        }
        SYS_GETPID => sys_getpid(),
        SYS_SLEEP => {
            sys_sleep(args[0]);
            0
        }
        SYS_FORK => sys_fork(),
        SYS_EXEC => {
            // args[1] is a user pointer to a null-terminated path string.
            // For now, construct a Rust &str from the raw pointer.
            let path_ptr = args[1] as *const u8;
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 {
                    len += 1;
                }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            sys_exec(path_str)
        }
        SYS_WAIT => sys_wait(args[0]),
        SYS_MMAP => sys_mmap(args[1], args[2] as usize, args[3] as u32, args[4] as u32),
        SYS_MUNMAP => sys_munmap(args[1], args[2] as usize),
        // ── IPC syscalls ──
        SYS_PIPE => sys_pipe(args[0], args[1]),
        SYS_DUP => sys_dup(args[0] as i32),
        SYS_DUP2 => sys_dup2(args[0] as i32, args[1] as i32),
        SYS_SHM_CREATE => sys_shm_create(args[0] as usize, args[1], args[2] as u32),
        SYS_SHM_ATTACH => sys_shm_attach(args[0] as usize, args[1] != 0),
        SYS_SHM_DETACH => sys_shm_detach(args[0] as usize),
        SYS_MSGQ_CREATE => sys_msgq_create(args[0]),
        SYS_MSGQ_SEND => sys_msgq_send(
            args[0] as usize,
            args[1],
            args[2], // data_ptr as u64
            args[3] as usize,
        ),
        SYS_MSGQ_RECV => sys_msgq_recv(
            args[0] as usize,
            args[1],
            args[2], // buf_ptr as u64
            args[3] as usize,
        ),
        SYS_SEMA_CREATE => sys_sema_create(args[0] as u32),
        SYS_SEMA_WAIT => sys_sema_wait(args[0] as usize),
        SYS_SEMA_POST => sys_sema_post(args[0] as usize),
        SYS_SEMA_GETVALUE => sys_sema_getvalue(args[0] as usize),
        SYS_SEMA_DESTROY => sys_sema_destroy(args[0] as usize),
        // ── Signal syscalls ──
        SYS_KILL => sys_kill(args[0] as u64, args[1] as u32),
        SYS_SIGACTION => sys_sigaction(args[0] as u32, args[1]),
        // ── Named pipe (FIFO) syscalls ──
        SYS_MKFIFO => sys_mkfifo(args[0], args[1] as usize, args[2] as u32),
        SYS_OPEN_FIFO => sys_open_fifo(args[0], args[1] as usize, args[2] as u32),
        // ── SHM control ──
        SYS_SHM_CTL => sys_shm_ctl(args[0] as usize, args[1] as u32, args[2] as u64),
        // ── Unix domain socket syscalls ──
        SYS_UNIX_SOCKET => sys_unix_socket(args[0] as u32),
        SYS_UNIX_BIND => sys_unix_bind(args[0] as usize, args[1], args[2] as usize),
        SYS_UNIX_LISTEN => sys_unix_listen(args[0] as usize, args[1] as usize),
        SYS_UNIX_ACCEPT => sys_unix_accept(args[0] as usize),
        SYS_UNIX_CONNECT => sys_unix_connect(args[0] as usize, args[1], args[2] as usize),
        SYS_UNIX_SEND => sys_unix_send(
            args[0] as usize,
            args[1],
            args[2] as usize,
            args[3] as usize,
        ),
        SYS_UNIX_RECV => sys_unix_recv(
            args[0] as usize,
            args[1],
            args[2] as usize,
            args[3] as usize,
        ),
        // ── Eventfd syscalls ──
        SYS_EVENTFD => sys_eventfd(args[0], args[1] as u32),
        SYS_EVENTFD_READ => sys_eventfd_read(args[0] as usize, args[1] as usize),
        SYS_EVENTFD_WRITE => sys_eventfd_write(args[0] as usize, args[1], args[2] as usize),
        // ── fcntl ──
        SYS_FCNTL => sys_fcntl(args[0] as i32, args[1] as u32, args[2]),
        // ── IPC namespace syscalls ──
        SYS_IPC_UNSHARE => sys_ipc_unshare(),
        SYS_IPC_SETNS => sys_ipc_setns(args[0] as u32),
        SYS_IPC_SET_MODE => sys_ipc_set_mode(args[0] as u32, args[1] as u32),
        // ── Security / credential syscalls ──
        SYS_SETUID => crate::security::sys_setuid(args[0] as u32) as u64,
        SYS_SETGID => crate::security::sys_setgid(args[0] as u32) as u64,
        SYS_SETEUID => crate::security::sys_seteuid(args[0] as u32) as u64,
        SYS_SETEGID => crate::security::sys_setegid(args[0] as u32) as u64,
        SYS_GETUID => crate::security::sys_getuid() as u64,
        SYS_GETGID => crate::security::sys_getgid() as u64,
        SYS_GETEUID => crate::security::sys_geteuid() as u64,
        SYS_GETEGID => crate::security::sys_getegid() as u64,
        SYS_GETGROUPS => {
            // args[0] = user-provided buffer pointer for u32 groups
            let buf_ptr = args[0] as *mut u32;
            let buf_len = args[1] as usize;
            let buf = if buf_ptr.is_null() || buf_len == 0 {
                &mut []
            } else {
                unsafe { core::slice::from_raw_parts_mut(buf_ptr, buf_len.min(4)) }
            };
            crate::security::sys_getgroups(buf) as u64
        }
        SYS_SETGROUPS => {
            // args[0] = pointer to u32 array, args[1] = count
            let grp_ptr = args[0] as *const u32;
            let grp_len = args[1] as usize;
            let groups = if grp_ptr.is_null() || grp_len == 0 {
                &[]
            } else {
                unsafe { core::slice::from_raw_parts(grp_ptr, grp_len.min(4)) }
            };
            crate::security::sys_setgroups(groups) as u64
        }
        SYS_CAPGET => {
            // Return CapSet as two u64 halves in x0/x1
            let caps = crate::security::sys_capget();
            (caps.permitted & 0xFFFF) | ((caps.effective & 0xFFFF) << 16)
        }
        SYS_CAPSET => {
            // args[0] = pointer to CapSet (5 × u64 = 40 bytes)
            let cap_ptr = args[0] as *const crate::security::CapSet;
            if cap_ptr.is_null() {
                u64::MAX
            } else {
                let new = unsafe { *cap_ptr };
                crate::security::sys_capset(&new) as u64
            }
        }
        SYS_CHMOD => {
            // args[0] = path ptr, args[1] = path len, args[2] = mode
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            let mode = args[2] as u32;
            if path_ptr.is_null() {
                u64::MAX
            } else {
                let path_bytes =
                    unsafe { core::slice::from_raw_parts(path_ptr, path_len.min(256)) };
                let path_str = core::str::from_utf8(path_bytes).unwrap_or("");
                match crate::fs::vfs::chmod(path_str, mode) {
                    Ok(()) => 0,
                    Err(_) => u64::MAX,
                }
            }
        }
        SYS_CHOWN => {
            // args[0] = path ptr, args[1] = path len, args[2] = uid, args[3] = gid
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            let uid = args[2] as u32;
            let gid = args[3] as u32;
            if path_ptr.is_null() {
                u64::MAX
            } else {
                let path_bytes =
                    unsafe { core::slice::from_raw_parts(path_ptr, path_len.min(256)) };
                let path_str = core::str::from_utf8(path_bytes).unwrap_or("");
                match crate::fs::vfs::chown(path_str, uid, gid) {
                    Ok(()) => 0,
                    Err(_) => u64::MAX,
                }
            }
        }
        SYS_SET_NO_NEW_PRIVS => crate::security::sys_set_no_new_privs() as u64,
        SYS_GET_NO_NEW_PRIVS => crate::security::sys_get_no_new_privs() as u64,
        // ── Signal return ──
        SYS_RT_SIGRETURN => sys_rt_sigreturn(),
        _ => u64::MAX, // -1 (ENOSYS)
    }
}

// ═══════════════════════════════════════════════
// Classic process syscalls
// ═══════════════════════════════════════════════

/// sys_exit(code) — terminate the current process
fn sys_exit(code: i64) -> ! {
    println!("[syscall] exit({}) from PID {}", code, sched::current_pid());
    sched::exit_current(code)
}

/// sys_write(fd, buf_ptr, len) — write to file descriptor
fn sys_write(fd: i32, buf_ptr: u64, len: usize) -> u64 {
    // Check if this is an IPC fd first
    let pid = sched::current_pid();
    if let Some(task_idx) = sched::find_task_index(pid) {
        if let Some(fd_table) = crate::ipc::get_fd_table(task_idx) {
            if let Some(entry) = fd_table.get(fd as usize) {
                if entry.kind == crate::ipc::FdKind::PipeWrite {
                    // Write to pipe
                    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };
                    let result = crate::ipc::pipe_write(task_idx, fd as usize, buf);
                    return if result < 0 {
                        u64::MAX // Error
                    } else {
                        result as u64
                    };
                }
            }
        }
    }

    // Fallback: fd=1 (stdout) and fd=2 (stderr) → UART
    if fd != 1 && fd != 2 {
        return u64::MAX; // -1 (EBADF)
    }

    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };
    for &byte in buf {
        crate::drivers::uart::Uart10::putc(byte);
    }
    len as u64
}

/// sys_read(fd, buf_ptr, len) — read from file descriptor
fn sys_read(fd: i32, buf_ptr: u64, len: usize) -> u64 {
    // Check if this is an IPC fd first
    let pid = sched::current_pid();
    if let Some(task_idx) = sched::find_task_index(pid) {
        if let Some(fd_table) = crate::ipc::get_fd_table(task_idx) {
            if let Some(entry) = fd_table.get(fd as usize) {
                if entry.kind == crate::ipc::FdKind::PipeRead {
                    // Read from pipe
                    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };
                    let result = crate::ipc::pipe_read(task_idx, fd as usize, buf);
                    return if result < 0 {
                        u64::MAX // Error / would-block
                    } else {
                        result as u64
                    };
                }
            }
        }
    }

    // Fallback: fd=0 (stdin) → UART
    if fd != 0 {
        return u64::MAX; // -1 (EBADF)
    }
    0 // Return 0 bytes read (EOF)
}

/// sys_open(path, flags) — open a file
fn sys_open(_path: *const u8, _flags: u32) -> u64 {
    // TODO: implement via VFS
    u64::MAX // -1 (ENOSYS)
}

/// sys_close(fd) — close a file descriptor
fn sys_close(fd: i32) -> u64 {
    let pid = sched::current_pid();
    if let Some(task_idx) = sched::find_task_index(pid) {
        if crate::ipc::fd_close(task_idx, fd as usize) {
            return 0;
        }
    }
    u64::MAX // -1 (EBADF)
}

/// sys_yield() — yield the current time slice
fn sys_yield() {
    sched::yield_task();
}

/// sys_getpid() — return current PID
fn sys_getpid() -> u64 {
    sched::current_pid()
}

/// sys_sleep(ms) — sleep for N milliseconds
fn sys_sleep(ms: u64) {
    let target_tick = sched::tick_count() + (ms / sched::TICK_MS);
    let _ = target_tick;
    sched::yield_task();
}

/// sys_fork() — create a child process
fn sys_fork() -> u64 {
    crate::sched::process::fork() as u64
}

/// sys_exec(path) — replace current process image with ELF from VFS path
fn sys_exec(path: &str) -> u64 {
    match crate::sched::process::exec(path) {
        0 => 0,              // success (won't actually return in a real exec)
        code => code as u64, // error code
    }
}

/// sys_wait(pid) — wait for a child process
fn sys_wait(pid: u64) -> u64 {
    crate::sched::process::wait(pid) as u64
}

/// sys_mmap(addr, len, prot, flags) — map memory
fn sys_mmap(_addr: u64, _len: usize, _prot: u32, _flags: u32) -> u64 {
    crate::sched::process::mmap(_addr, _len, _prot, _flags) as u64
}

/// sys_munmap(addr, len) — unmap memory
fn sys_munmap(_addr: u64, _len: usize) -> u64 {
    crate::sched::process::munmap(_addr, _len) as u64
}

// ═══════════════════════════════════════════════
// IPC syscalls
// ═══════════════════════════════════════════════

/// sys_pipe(read_fd_ptr, write_fd_ptr) — create a pipe
/// Writes (read_fd, write_fd) to the user-provided addresses
fn sys_pipe(read_fd_ptr: u64, write_fd_ptr: u64) -> u64 {
    match crate::ipc::create_pipe() {
        Some((read_fd, write_fd)) => {
            // Store results in user memory
            unsafe {
                core::ptr::write(read_fd_ptr as *mut i32, read_fd as i32);
                core::ptr::write(write_fd_ptr as *mut i32, write_fd as i32);
            }
            0
        }
        None => u64::MAX, // -1
    }
}

/// sys_dup(old_fd) — duplicate a file descriptor
fn sys_dup(old_fd: i32) -> u64 {
    let pid = sched::current_pid();
    if let Some(task_idx) = sched::find_task_index(pid) {
        if let Some(new_fd) = crate::ipc::fd_dup(task_idx, old_fd as usize) {
            return new_fd as u64;
        }
    }
    u64::MAX // -1 (EBADF)
}

/// sys_dup2(old_fd, new_fd) — duplicate to a specific fd
fn sys_dup2(old_fd: i32, new_fd: i32) -> u64 {
    let pid = sched::current_pid();
    if let Some(task_idx) = sched::find_task_index(pid) {
        if let Some(result) = crate::ipc::fd_dup2(task_idx, old_fd as usize, new_fd as usize) {
            return result as u64;
        }
    }
    u64::MAX // -1 (EBADF)
}

/// sys_shm_create(size, key, mode) — create shared memory region
fn sys_shm_create(size: usize, key: u64, mode: u32) -> u64 {
    crate::ipc::shm_create(size, key, mode).map_or(u64::MAX, |idx| idx as u64)
}

/// sys_shm_attach(shm_idx, writable) — attach shared memory
fn sys_shm_attach(shm_idx: usize, writable: bool) -> u64 {
    crate::ipc::shm_attach(shm_idx, writable).map_or(u64::MAX, |addr| addr)
}

/// sys_shm_detach(shm_idx) — detach shared memory
fn sys_shm_detach(shm_idx: usize) -> u64 {
    crate::ipc::shm_detach(shm_idx);
    0
}

/// sys_msgq_create(key) — create a message queue
fn sys_msgq_create(key: u64) -> u64 {
    crate::ipc::msgq_create(key).map_or(u64::MAX, |idx| idx as u64)
}

/// sys_msgq_send(queue_idx, mtype, data_ptr, data_len) — send a message
fn sys_msgq_send(queue_idx: usize, mtype: u64, data_ptr: u64, data_len: usize) -> u64 {
    let data = unsafe { core::slice::from_raw_parts(data_ptr as *const u8, data_len.min(256)) };
    crate::ipc::msgq_send(queue_idx, mtype, data) as u64
}

/// sys_msgq_recv(queue_idx, mtype_filter, buf_ptr, buf_len) — receive a message
/// Returns (len << 32) | mtype on success, -1 on error
fn sys_msgq_recv(queue_idx: usize, mtype_filter: u64, buf_ptr: u64, buf_len: usize) -> u64 {
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len.min(256)) };
    let (len, mtype) = crate::ipc::msgq_recv(queue_idx, mtype_filter, buf);
    if len < 0 {
        u64::MAX
    } else {
        ((len as u64) << 32) | (mtype & 0xFFFFFFFF)
    }
}

/// sys_sema_create(initial_value) — create a semaphore
fn sys_sema_create(initial_value: u32) -> u64 {
    crate::ipc::sema_create(initial_value).map_or(u64::MAX, |idx| idx as u64)
}

/// sys_sema_wait(sema_idx) — wait on a semaphore
fn sys_sema_wait(sema_idx: usize) -> u64 {
    crate::ipc::sema_wait(sema_idx) as u64
}

/// sys_sema_post(sema_idx) — post to a semaphore
fn sys_sema_post(sema_idx: usize) -> u64 {
    crate::ipc::sema_post(sema_idx) as u64
}

/// sys_sema_getvalue(sema_idx) — get semaphore value
fn sys_sema_getvalue(sema_idx: usize) -> u64 {
    crate::ipc::sema_getvalue(sema_idx).map_or(u64::MAX, |v| v as u64)
}

/// sys_sema_destroy(sema_idx) — destroy a semaphore
fn sys_sema_destroy(sema_idx: usize) -> u64 {
    if crate::ipc::sema_destroy(sema_idx) {
        0
    } else {
        u64::MAX
    }
}

// ═══════════════════════════════════════════════
// Signal syscalls
// ═══════════════════════════════════════════════

/// sys_kill(pid, sig) — send a signal to a process
fn sys_kill(pid: u64, sig: u32) -> u64 {
    // Permission check: CAP_KILL or same-uid required (POSIX rule)
    {
        let my_pid = crate::sched::current_pid() as u64;
        if pid != 0 && pid != my_pid {
            // Cross-process kill — check permission
            let my_uid = crate::security::sys_getuid();
            let target_uid = unsafe {
                crate::sched::TASK_TABLE
                    .iter()
                    .find(|t| t.as_ref().map_or(false, |t| t.pid as u64 == pid))
                    .and_then(|t| t.as_ref().map(|t| t.uid))
                    .unwrap_or(u32::MAX)
            };
            if my_uid != 0
                && my_uid != target_uid
                && !crate::security::has_capability(crate::security::CAP_KILL)
            {
                return u64::MAX; // -EPERM
            }
        }
    }
    crate::sched::signal::kill(pid, sig as u64) as u64
}

/// sys_sigaction(sig, handler) — set signal handler
/// Simplified syscall: handler=0 means SIG_DFL, handler=1 means SIG_IGN,
/// any other value is a user handler address.
fn sys_sigaction(sig: u32, handler: u64) -> u64 {
    let act = crate::sched::signal::SigAction {
        handler,
        sa_flags: 0,
        sa_mask: 0,
    };
    crate::sched::signal::sigaction(sig as u64, Some(&act), None) as u64
}

/// sys_rt_sigreturn() — return from signal handler
/// Restores the saved trap frame from the user stack
fn sys_rt_sigreturn() -> u64 {
    use crate::sched::signal::sigreturn;
    let pid = crate::sched::current_pid();
    let _task_idx = match crate::sched::find_task_index(pid) {
        Some(i) => i,
        None => return u64::MAX,
    };

    // Call sigreturn to restore the trap frame
    match sigreturn() {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

// ═══════════════════════════════════════════════
// Named pipe (FIFO) syscalls
// ═══════════════════════════════════════════════

/// sys_mkfifo(path_ptr, path_len, mode) — create a named pipe
fn sys_mkfifo(path_ptr: u64, path_len: usize, mode: u32) -> u64 {
    let path = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len.min(256)) };
    crate::ipc::mkfifo(path, mode).map_or(u64::MAX, |idx| idx as u64)
}

/// sys_open_fifo(path_ptr, path_len, flags) — open a named pipe
/// Uses find_named_pipe then open_fifo_read or open_fifo_write depending on flags
fn sys_open_fifo(path_ptr: u64, path_len: usize, flags: u32) -> u64 {
    let path = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len.min(256)) };
    let pid = sched::current_pid();
    let task_idx = match sched::find_task_index(pid) {
        Some(i) => i,
        None => return u64::MAX,
    };
    let np_idx = match crate::ipc::find_named_pipe(path) {
        Some(i) => i,
        None => return u64::MAX,
    };
    if flags & 0x1 != 0 {
        // O_WRONLY — open for writing
        crate::ipc::open_fifo_write(task_idx, np_idx, flags).map_or(u64::MAX, |fd| fd as u64)
    } else {
        // O_RDONLY — open for reading
        crate::ipc::open_fifo_read(task_idx, np_idx, flags).map_or(u64::MAX, |fd| fd as u64)
    }
}

// ═══════════════════════════════════════════════
// SHM control syscall
// ═══════════════════════════════════════════════

/// sys_shm_ctl(shm_idx, cmd, arg) — control shared memory region
fn sys_shm_ctl(shm_idx: usize, cmd: u32, arg: u64) -> u64 {
    crate::ipc::shm_ctl(shm_idx, cmd, arg) as u64
}

// ═══════════════════════════════════════════════
// Unix domain socket syscalls
// ═══════════════════════════════════════════════

/// sys_unix_socket(sock_type) — create a Unix domain socket
fn sys_unix_socket(sock_type: u32) -> u64 {
    crate::ipc::unix_socket(sock_type)
        .map_or(u64::MAX, |(idx, fd)| ((idx as u64) << 32) | (fd as u64))
}

/// sys_unix_bind(sock_idx, path_ptr, path_len) — bind a Unix socket to a path
fn sys_unix_bind(sock_idx: usize, path_ptr: u64, path_len: usize) -> u64 {
    let _ = (path_ptr, path_len);
    // TODO: implement unix_bind in ipc module
    let _ = sock_idx;
    u64::MAX
}

/// sys_unix_listen(sock_idx, backlog) — listen for connections
fn sys_unix_listen(sock_idx: usize, backlog: usize) -> u64 {
    let _ = backlog;
    let _ = sock_idx;
    // TODO: implement unix_listen in ipc module
    u64::MAX
}

/// sys_unix_accept(sock_idx) — accept a connection
fn sys_unix_accept(sock_idx: usize) -> u64 {
    let _ = sock_idx;
    // TODO: implement unix_accept in ipc module
    u64::MAX
}

/// sys_unix_connect(sock_idx, path_ptr, path_len) — connect to a Unix socket
fn sys_unix_connect(sock_idx: usize, path_ptr: u64, path_len: usize) -> u64 {
    let _ = (path_ptr, path_len);
    let _ = sock_idx;
    // unix_connect(sock_a, sock_b) takes two socket indices — path lookup TODO
    u64::MAX
}

/// sys_unix_send(sock_idx, buf_ptr, len, task_idx) — send on a Unix socket
fn sys_unix_send(sock_idx: usize, buf_ptr: u64, len: usize, task_idx: usize) -> u64 {
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len.min(4096)) };
    let pid = sched::current_pid();
    if let Some(tidx) = sched::find_task_index(pid) {
        if let Some(fd_table) = crate::ipc::get_fd_table(tidx) {
            for fd_num in 0..crate::ipc::MAX_FDS_PER_PROC {
                if let Some(entry) = fd_table.get(fd_num) {
                    if entry.kind == crate::ipc::FdKind::UnixSocket
                        && entry.index as usize == sock_idx
                    {
                        return crate::ipc::unix_send(task_idx, fd_num, buf) as u64;
                    }
                }
            }
        }
    }
    u64::MAX
}

/// sys_unix_recv(sock_idx, buf_ptr, len, task_idx) — recv on a Unix socket
fn sys_unix_recv(sock_idx: usize, buf_ptr: u64, len: usize, task_idx: usize) -> u64 {
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len.min(4096)) };
    let pid = sched::current_pid();
    if let Some(tidx) = sched::find_task_index(pid) {
        if let Some(fd_table) = crate::ipc::get_fd_table(tidx) {
            for fd_num in 0..crate::ipc::MAX_FDS_PER_PROC {
                if let Some(entry) = fd_table.get(fd_num) {
                    if entry.kind == crate::ipc::FdKind::UnixSocket
                        && entry.index as usize == sock_idx
                    {
                        return crate::ipc::unix_recv(task_idx, fd_num, buf) as u64;
                    }
                }
            }
        }
    }
    u64::MAX
}

// ═══════════════════════════════════════════════
// Eventfd syscalls
// ═══════════════════════════════════════════════

/// sys_eventfd(initval, flags) — create an eventfd
/// Returns (efd_idx << 32) | fd on success
fn sys_eventfd(initval: u64, flags: u32) -> u64 {
    crate::ipc::eventfd(initval, flags)
        .map_or(u64::MAX, |(idx, fd)| ((idx as u64) << 32) | (fd as u64))
}

/// sys_eventfd_read(efd_idx, task_idx) — read from an eventfd
fn sys_eventfd_read(efd_idx: usize, task_idx: usize) -> u64 {
    let result = crate::ipc::eventfd_read(task_idx, efd_idx);
    if result < 0 {
        u64::MAX
    } else {
        result as u64
    }
}

/// sys_eventfd_write(efd_idx, val, task_idx) — write to an eventfd
fn sys_eventfd_write(efd_idx: usize, val: u64, task_idx: usize) -> u64 {
    crate::ipc::eventfd_write(task_idx, efd_idx, val) as u64
}

// ═══════════════════════════════════════════════
// fcntl syscall
// ═══════════════════════════════════════════════

/// fcntl commands
const F_DUPFD: u32 = 0;
const F_GETFD: u32 = 1;
const F_SETFD: u32 = 2;
const F_GETFL: u32 = 3;
const F_SETFL: u32 = 4;

/// sys_fcntl(fd, cmd, arg) — file descriptor control
fn sys_fcntl(fd: i32, cmd: u32, arg: u64) -> u64 {
    let pid = sched::current_pid();
    if let Some(task_idx) = sched::find_task_index(pid) {
        if let Some(fd_table) = crate::ipc::get_fd_table(task_idx) {
            match cmd {
                F_DUPFD => {
                    // Duplicate fd to lowest available >= arg
                    if let Some(new_fd) = crate::ipc::fd_dup2(task_idx, fd as usize, arg as usize) {
                        return new_fd as u64;
                    }
                }
                F_GETFD => {
                    if let Some(entry) = fd_table.get(fd as usize) {
                        return entry.flags as u64;
                    }
                }
                F_SETFD => {
                    if let Some(entry) = fd_table.get_mut(fd as usize) {
                        entry.flags = arg as u32;
                        return 0;
                    }
                }
                F_GETFL => {
                    if let Some(entry) = fd_table.get(fd as usize) {
                        return entry.flags as u64; // Simplified: return raw flags
                    }
                }
                F_SETFL => {
                    // Only allow setting O_NONBLOCK
                    if let Some(entry) = fd_table.get_mut(fd as usize) {
                        entry.flags = (entry.flags & !0x04u32) | ((arg as u32) & 0x04);
                        return 0;
                    }
                }
                _ => {}
            }
        }
    }
    u64::MAX // -1 (EBADF / EINVAL)
}

// ═══════════════════════════════════════════════
// IPC namespace syscalls
// ═══════════════════════════════════════════════

/// sys_ipc_unshare() — create a new IPC namespace and move the calling process into it.
/// Returns the new namespace ID on success, or -1 on failure.
fn sys_ipc_unshare() -> u64 {
    crate::ipc::ipc_ns_unshare().map_or(u64::MAX, |ns_id| ns_id as u64)
}

/// sys_ipc_setns(ns_id) — move the calling process into an existing IPC namespace.
/// Returns 0 on success, or -1 on failure.
fn sys_ipc_setns(ns_id: u32) -> u64 {
    if crate::ipc::set_ipc_ns(ns_id) {
        0
    } else {
        u64::MAX
    }
}

/// sys_ipc_set_mode(ns_id, mode) — set the access mode of an IPC namespace.
/// mode: 0 = open (any process can access), 1 = strict (same-ns only).
/// Returns 0 on success, or -1 on failure.
fn sys_ipc_set_mode(ns_id: u32, mode: u32) -> u64 {
    if crate::ipc::ipc_ns_set_mode(ns_id, mode) {
        0
    } else {
        u64::MAX
    }
}

// ═══════════════════════════════════════════════
// Inline syscall helpers for user programs
// ═══════════════════════════════════════════════

/// Syscall helper functions (ARM64-specific)
#[cfg(target_arch = "aarch64")]
mod arch_syscalls {
    use core::arch::asm;

    #[inline(always)]
    pub unsafe fn syscall0(num: u64) -> u64 {
        let ret: u64;
        asm!(
            "svc #0",
            in("x8") num,
            out("x0") ret,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall1(num: u64, a0: u64) -> u64 {
        let ret: u64;
        asm!(
            "svc #0",
            in("x8") num,
            inlateout("x0") a0 => ret,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall3(num: u64, a0: u64, a1: u64, a2: u64) -> u64 {
        let ret: u64;
        asm!(
            "svc #0",
            in("x8") num,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall4(num: u64, a0: u64, a1: u64, a2: u64, a3: u64) -> u64 {
        let ret: u64;
        asm!(
            "svc #0",
            in("x8") num,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall6(num: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
        let ret: u64;
        asm!(
            "svc #0",
            in("x8") num,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            in("x4") a4,
            in("x5") a5,
        );
        ret
    }
}
