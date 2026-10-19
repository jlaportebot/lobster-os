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
use crate::sched::{self, TrapFrame};

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
pub const SYS_SIGPROCMASK: u64 = 29;
pub const SYS_MKFIFO: u64 = 30;
pub const SYS_OPEN_FIFO: u64 = 31;
pub const SYS_SHM_CTL: u64 = 32;
pub const SYS_UNIX_SOCKET: u64 = 33;
pub const SYS_UNIX_BIND: u64 = 34;
pub const SYS_UNIX_LISTEN: u64 = 35;
pub const SYS_UNIX_ACCEPT: u64 = 36;
pub const SYS_UNIX_CONNECT: u64 = 37;
pub const SYS_UNIX_SEND: u64 = 38;
pub const SYS_UNIX_RECV: u64 = 39;
pub const SYS_EVENTFD: u64 = 40;
pub const SYS_EVENTFD_READ: u64 = 41;
pub const SYS_EVENTFD_WRITE: u64 = 42;
pub const SYS_FCNTL: u64 = 43;
pub const SYS_RT_SIGRETURN: u64 = 44;
pub const SYS_IPC_UNSHARE: u64 = 45;
pub const SYS_IPC_SETNS: u64 = 46;
pub const SYS_IPC_SET_MODE: u64 = 47;

// ── PTY syscalls ──
pub const SYS_OPENPTY: u64 = 48;
pub const SYS_FORKPTY: u64 = 49;
pub const SYS_IOCTL: u64 = 50;

// ── Security / credential syscalls ──
pub const SYS_SETUID: u64 = 51;
pub const SYS_SETGID: u64 = 52;
pub const SYS_SETEUID: u64 = 53;
pub const SYS_SETEGID: u64 = 54;
pub const SYS_GETUID: u64 = 55;
pub const SYS_GETGID: u64 = 56;
pub const SYS_GETEUID: u64 = 57;
pub const SYS_GETEGID: u64 = 58;
pub const SYS_GETGROUPS: u64 = 59;
pub const SYS_SETGROUPS: u64 = 60;
pub const SYS_CAPGET: u64 = 61;
pub const SYS_CAPSET: u64 = 62;
pub const SYS_CHMOD: u64 = 63;
pub const SYS_CHOWN: u64 = 64;
pub const SYS_SET_NO_NEW_PRIVS: u64 = 65;
pub const SYS_GET_NO_NEW_PRIVS: u64 = 66;
pub const SYS_KLOGCTL: u64 = 67;

// ── Filesystem / VFS syscalls ──
pub const SYS_BRK: u64 = 68;
pub const SYS_GETTIMEOFDAY: u64 = 69;
pub const SYS_NANOSLEEP: u64 = 70;
pub const SYS_STAT: u64 = 71;
pub const SYS_FSTAT: u64 = 72;
pub const SYS_LSTAT: u64 = 73;
pub const SYS_LSEEK: u64 = 74;
pub const SYS_GETDENTS: u64 = 75;
pub const SYS_MKDIR: u64 = 76;
pub const SYS_RMDIR: u64 = 77;
pub const SYS_UNLINK: u64 = 78;
pub const SYS_CREAT: u64 = 79;
pub const SYS_ACCESS: u64 = 80;
pub const SYS_GETCWD: u64 = 81;
pub const SYS_CHDIR: u64 = 82;
pub const SYS_TIME: u64 = 83;
pub const SYS_UTIME: u64 = 84;
pub const SYS_GETPPID: u64 = 85;
pub const SYS_SETSID: u64 = 86;
pub const SYS_UNAME: u64 = 87;
pub const SYS_SYNC: u64 = 88;
pub const SYS_FSYNC: u64 = 89;
pub const SYS_FDATASYNC: u64 = 90;
pub const SYS_TRUNCATE: u64 = 91;
pub const SYS_FTRUNCATE: u64 = 92;

// ── Dynamic linking syscalls ──
pub const SYS_DLOPEN: u64 = 93;
pub const SYS_DLSYM: u64 = 94;
pub const SYS_DLCLOSE: u64 = 95;
pub const SYS_WIN_CREATE: u64 = 110;
pub const SYS_WIN_FILL: u64 = 111;
pub const SYS_WIN_FLUSH: u64 = 112;
pub const SYS_WIN_CLOSE: u64 = 113;

/// Maximum syscall number
const MAX_SYSCALL: u64 = 113;

// Global pointer to current trap frame during syscall handling (for sigreturn)
static mut CURRENT_TRAP_FRAME: *mut TrapFrame = core::ptr::null_mut();

/// Set the current trap frame for syscall handling
pub fn set_current_trap_frame(regs: *mut TrapFrame) {
    unsafe {
        CURRENT_TRAP_FRAME = regs;
    }
}

/// Get the current trap frame
pub fn get_current_trap_frame() -> *mut TrapFrame {
    unsafe { CURRENT_TRAP_FRAME }
}

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
            sys_exit(code)
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
        SYS_SIGPROCMASK => sys_sigprocmask(args[0] as u32, args[1] as u64, args[2] as u64),
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
        // ── PTY syscalls ──
        SYS_OPENPTY => sys_openpty(args[0], args[1]),
        SYS_FORKPTY => sys_forkpty(args[0], args[1]),
        SYS_IOCTL => sys_ioctl(args[0] as i32, args[1] as u64, args[2]),
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
        // ── Kernel log control (dmesg) ──
        SYS_KLOGCTL => sys_klogctl(args[0] as i32, args[1] as u64, args[2] as usize),
        // ── Filesystem / VFS syscalls ──
        SYS_BRK => sys_brk(args[0]),
        SYS_GETTIMEOFDAY => {
            let tv_ptr = args[0] as *mut u64;
            let tz_ptr = args[1] as *mut u64;
            let now_us = crate::arch::timer::Timer::system_time_us();
            if !tv_ptr.is_null() {
                unsafe {
                    *tv_ptr = now_us / 1_000_000;
                    *tv_ptr.add(1) = now_us % 1_000_000;
                }
            }
            if !tz_ptr.is_null() {
                unsafe { *tz_ptr = 0; }
            }
            0
        }
        SYS_NANOSLEEP => {
            let req_ptr = args[0] as *const u64;
            let rem_ptr = args[1] as *mut u64;
            let secs = if !req_ptr.is_null() {
                unsafe { *req_ptr }
            } else { 0 };
            let nsecs = if !req_ptr.is_null() {
                unsafe { *req_ptr.add(1) }
            } else { 0 };
            let total_ms = (secs * 1000) + (nsecs / 1_000_000);
            sys_sleep(total_ms);
            if !rem_ptr.is_null() {
                unsafe { *rem_ptr = 0; *rem_ptr.add(1) = 0; }
            }
            0
        }
        SYS_STAT => {
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            let stat_ptr = args[2] as *mut u8;
            if path_ptr.is_null() || stat_ptr.is_null() { return u64::MAX; }
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 && len < 256 { len += 1; }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            sys_stat(path_str, stat_ptr)
        }
        SYS_FSTAT => sys_fstat(args[0] as i32, args[1] as *mut u8),
        SYS_LSTAT => {
            let path_ptr = args[0] as *const u8;
            if path_ptr.is_null() { return u64::MAX; }
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 && len < 256 { len += 1; }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            sys_stat(path_str, args[1] as *mut u8)
        }
        SYS_LSEEK => {
            crate::fs::vfs::seek(args[0] as usize, args[1] as i64, args[2] as u32)
                .unwrap_or(u64::MAX) as u64
        }
        SYS_GETDENTS => sys_getdents(args[0] as i32, args[1] as *mut u8, args[2] as usize),
        SYS_MKDIR => {
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            let mode = args[2] as u32;
            if path_ptr.is_null() { return u64::MAX; }
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 && len < 256 { len += 1; }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            match crate::fs::vfs::mkdir(path_str) { Ok(_) => 0, Err(_) => u64::MAX }
        }
        SYS_RMDIR => {
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            if path_ptr.is_null() { return u64::MAX; }
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 && len < 256 { len += 1; }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            match crate::fs::vfs::remove(path_str) { Ok(_) => 0, Err(_) => u64::MAX }
        }
        SYS_UNLINK => {
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            if path_ptr.is_null() { return u64::MAX; }
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 && len < 256 { len += 1; }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            match crate::fs::vfs::unlink(path_str) { Ok(_) => 0, Err(_) => u64::MAX }
        }
        SYS_CREAT => {
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            let mode = args[2] as u32;
            if path_ptr.is_null() { return u64::MAX; }
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 && len < 256 { len += 1; }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            match crate::fs::vfs::open(path_str, crate::fs::vfs::O_WRONLY | crate::fs::vfs::O_CREAT | crate::fs::vfs::O_TRUNC) {
                Ok(fd) => fd as u64, Err(_) => u64::MAX
            }
        }
        SYS_ACCESS => {
            // args[0] = path ptr, args[1] = mode (F_OK=0, R_OK=4, W_OK=2, X_OK=1)
            // For now just check if file exists
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            if path_ptr.is_null() { return u64::MAX; }
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 && len < 256 { len += 1; }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            match crate::fs::vfs::resolve_path(path_str) { Ok(_) => 0, Err(_) => u64::MAX }
        }
        SYS_GETCWD => {
            let buf_ptr = args[0] as *mut u8;
            let buf_size = args[1] as usize;
            if buf_ptr.is_null() || buf_size == 0 { return u64::MAX; }
            let cwd = crate::shell::cwd();
            let bytes = cwd.as_bytes();
            let copy_len = bytes.len().min(buf_size - 1);
            unsafe { core::slice::from_raw_parts_mut(buf_ptr, copy_len) }.copy_from_slice(&bytes[..copy_len]);
            unsafe { *buf_ptr.add(copy_len) = 0; }
            buf_ptr as u64
        }
        SYS_CHDIR => {
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            if path_ptr.is_null() { return u64::MAX; }
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 && len < 256 { len += 1; }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            match crate::fs::vfs::resolve_path(path_str) {
                Ok(_) => { crate::shell::set_cwd(path_str); 0 }
                Err(_) => u64::MAX
            }
        }
        SYS_TIME => {
            let tloc = args[0] as *mut u64;
            let now_sec = crate::arch::timer::Timer::system_time_us() / 1_000_000;
            if !tloc.is_null() {
                unsafe { *tloc = now_sec; }
            }
            now_sec
        }
        SYS_UTIME => {
            // args[0] = path, args[1] = utimbuf ptr
            // Stub for now
            0
        }
        SYS_GETPPID => crate::sched::get_parent_pid(),
        SYS_SETSID => crate::sched::create_session(),
        SYS_UNAME => {
            let buf_ptr = args[0] as *mut u8;
            if buf_ptr.is_null() { return u64::MAX; }
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr, 6 * 65) };
            for b in buf.iter_mut() { *b = 0; }
            let name = b"lobster-os\0"; buf[..name.len()].copy_from_slice(name);
            let node = b"lobster\0"; buf[65..65+node.len()].copy_from_slice(node);
            let rel = b"0.2.0\0"; buf[130..130+rel.len()].copy_from_slice(rel);
            let ver = b"lobster-os 0.2.0\0"; buf[195..195+ver.len()].copy_from_slice(ver);
            let mach = b"aarch64\0"; buf[260..260+mach.len()].copy_from_slice(mach);
            0
        }
        SYS_SYNC => { crate::fs::vfs::sync(); 0 },
        SYS_FSYNC => sys_fsync(args[0] as i32),
        SYS_FDATASYNC => sys_fsync(args[0] as i32),
        SYS_TRUNCATE => {
            let path_ptr = args[0] as *const u8;
            let path_len = args[1] as usize;
            let length = args[2] as u64;
            if path_ptr.is_null() { return u64::MAX; }
            let path_str = unsafe {
                let mut len = 0usize;
                while *path_ptr.add(len) != 0 && len < 256 { len += 1; }
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
            };
            sys_truncate(path_str, length)
        }
        SYS_FTRUNCATE => sys_ftruncate(args[0] as i32, args[1] as u64),
        // ── Dynamic linking syscalls ──
        SYS_DLOPEN => sys_dlopen(args[0], args[1] as u32),
        SYS_DLSYM => sys_dlsym(args[0] as u64, args[1]),
        SYS_DLCLOSE => sys_dlclose(args[0] as u64),
        SYS_WIN_CREATE => crate::wm::user_windows::sys_win_create(args[0], args[1], args[2], args[3], args[4]),
        SYS_WIN_FILL => crate::wm::user_windows::sys_win_fill(args[0], args[1], args[2], args[3], args[4], args[5]),
        SYS_WIN_FLUSH => crate::wm::user_windows::sys_win_flush(args[0]),
        SYS_WIN_CLOSE => crate::wm::user_windows::sys_win_close(args[0]),
        _ => {
            // Try Linux ABI compatibility layer for unrecognized syscalls
            linux_compat::dispatch_linux(syscall_num, args)
        }
    }
}

// ═══════════════════════════════════════════════
// Linux ABI Compatibility Layer
// ═══════════════════════════════════════════════
// Maps standard AArch64 Linux syscall numbers to our
// existing VFS/scheduler implementations. This allows
// programs compiled with standard Linux headers (using
// Linux syscall numbers) to work on LobsterOS.

mod linux_compat {
    use super::*;

    // Linux AArch64 syscall numbers (from unistd.h)
    const LINUX_IOV_SETUP: u64 = 0;
    const LINUX_IOV_DESTROY: u64 = 1;
    const LINUX_IOV_RESUME: u64 = 2;
    const LINUX_IOV_REGISTER: u64 = 3;
    const LINUX_IOV_UNREGISTER: u64 = 4;
    const LINUX_OPENAT: u64 = 56;
    const LINUX_CLOSE: u64 = 57;
    const LINUX_READ: u64 = 63;
    const LINUX_WRITE: u64 = 64;
    const LINUX_WRITEV: u64 = 66;
    const LINUX_READV: u64 = 65;
    const LINUX_LSEEK: u64 = 62;
    const LINUX_PIPE2: u64 = 59;
    const LINUX_PIPE: u64 = 21;
    const LINUX_DUP: u64 = 23;
    const LINUX_DUP3: u64 = 24;
    const LINUX_FCNTL: u64 = 25;
    const LINUX_IOCTL: u64 = 29;
    const LINUX_NEWFSTATAT: u64 = 79;
    const LINUX_FSTAT: u64 = 80;
    const LINUX_STAT: u64 = 79; // same as newfstatat with AT_FDCWD
    const LINUX_STATFS: u64 = 44;
    const LINUX_FSTATFS: u64 = 45;
    const LINUX_TRUNCATE: u64 = 45;
    const LINUX_FTRUNCATE: u64 = 46;
    const LINUX_FALLOCATE: u64 = 47;
    const LINUX_FACCESSAT: u64 = 48;
    const LINUX_FCHDIR: u64 = 49;
    const LINUX_FCHOWN: u64 = 50;
    const LINUX_FCHMOD: u64 = 52;
    const LINUX_FCHOWNAT: u64 = 54;
    const LINUX_FCHMODAT: u64 = 53;
    const LINUX_OPEN: u64 = 22; // obsolete, maps to openat
    const LINUX_GETCWD: u64 = 17;
    const LINUX_CHDIR: u64 = 49;
    const LINUX_MKDIR: u64 = 53;
    const LINUX_RMDIR: u64 = 84;
    const LINUX_UNLINK: u64 = 87;
    const LINUX_UNLINKAT: u64 = 35;
    const LINUX_RENAME: u64 = 85;
    const LINUX_RENAMEAT: u64 = 38;
    const LINUX_SYMLINK: u64 = 88;
    const LINUX_SYMLINKAT: u64 = 86;
    const LINUX_READLINK: u64 = 89;
    const LINUX_READLINKAT: u64 = 78;
    const LINUX_LINK: u64 = 79;
    const LINUX_LINKAT: u64 = 38;
    const LINUX_GETPID: u64 = 172;
    const LINUX_GETTID: u64 = 178;
    const LINUX_GETUID: u64 = 174;
    const LINUX_GETGID: u64 = 176;
    const LINUX_GETEUID: u64 = 175;
    const LINUX_GETEGID: u64 = 177;
    const LINUX_EXIT: u64 = 93;
    const LINUX_EXIT_GROUP: u64 = 94;
    const LINUX_SCHED_YIELD: u64 = 124;
    const LINUX_NANOSLEEP: u64 = 101;
    const LINUX_CLONE: u64 = 220;
    const LINUX_FORK: u64 = 220;
    const LINUX_EXECVE: u64 = 221;
    const LINUX_WAIT4: u64 = 260;
    const LINUX_MMAP: u64 = 222;
    const LINUX_MUNMAP: u64 = 215;
    const LINUX_MPROTECT: u64 = 226;
    const LINUX_BRK: u64 = 214;
    const LINUX_GETRANDOM: u64 = 278;
    const LINUX_SOCKET: u64 = 198;
    const LINUX_BIND: u64 = 200;
    const LINUX_LISTEN: u64 = 201;
    const LINUX_ACCEPT: u64 = 202;
    const LINUX_CONNECT: u64 = 203;
    const LINUX_SENDTO: u64 = 206;
    const LINUX_RECVFROM: u64 = 207;
    const LINUX_SETSOCKOPT: u64 = 208;
    const LINUX_GETSOCKOPT: u64 = 209;
    const LINUX_SHUTDOWN: u64 = 210;
    const LINUX_GETSOCKNAME: u64 = 204;
    const LINUX_GETPEERNAME: u64 = 205;
    const LINUX_FUTEX: u64 = 98;
    const LINUX_SET_TID_ADDRESS: u64 = 96;
    const LINUX_RT_SIGPROCMASK: u64 = 135;
    const LINUX_RT_SIGACTION: u64 = 134;
    const LINUX_RT_SIGRETURN: u64 = 139;
    const LINUX_KILL: u64 = 129;
    const LINUX_TGKILL: u64 = 131;
    const LINUX_TKILL: u64 = 130;
    const LINUX_IOCTL_: u64 = 29;
    const LINUX_WRITEV_: u64 = 66;
    const LINUX_PREAD64: u64 = 67;
    const LINUX_PWRITE64: u64 = 68;
    const LINUX_READV_: u64 = 65;
    const LINUX_FSYNC: u64 = 82;
    const LINUX_FDATASYNC: u64 = 83;
    const LINUX_MADVISE: u64 = 233;
    const LINUX_MINCORE: u64 = 232;
    const LINUX_MLOCK: u64 = 229;
    const LINUX_MUNLOCK: u64 = 228;
    const LINUX_GETTIMEOFDAY: u64 = 169;
    const LINUX_SETTIMEOFDAY: u64 = 170;
    const LINUX_TIME: u64 = 169;
    const LINUX_CLOCK_GETTIME: u64 = 113;
    const LINUX_CLOCK_SETTIME: u64 = 112;
    const LINUX_UNAME: u64 = 160;
    const LINUX_UMASK: u64 = 167;
    const LINUX_GETRLIMIT: u64 = 163;
    const LINUX_SETRLIMIT: u64 = 164;
    const LINUX_GETRUSAGE: u64 = 165;
    const LINUX_SYSINFO: u64 = 179;
    const LINUX_PERSONALITY: u64 = 92;

    /// Dispatch Linux ABI syscall
    pub fn dispatch_linux(syscall_num: u64, args: &[u64; 6]) -> u64 {
        match syscall_num {
            LINUX_WRITE => sys_write(args[0] as i32, args[1], args[2] as usize),
            LINUX_READ => sys_read(args[0] as i32, args[1], args[2] as usize),
            LINUX_CLOSE => sys_close(args[0] as i32),

            LINUX_EXIT | LINUX_EXIT_GROUP => {
                sys_exit(args[0] as i64)
            }

            LINUX_GETPID => sched::current_pid(),
            LINUX_GETTID => sched::current_pid(),
            LINUX_SCHED_YIELD => {
                sys_yield();
                0
            }

            LINUX_GETUID => crate::security::sys_getuid() as u64,
            LINUX_GETGID => crate::security::sys_getgid() as u64,
            LINUX_GETEUID => crate::security::sys_geteuid() as u64,
            LINUX_GETEGID => crate::security::sys_getegid() as u64,

            LINUX_BRK => sys_brk(args[0]),
            LINUX_GETRANDOM => sys_getrandom(args[0], args[1] as usize, args[2] as u32),

            LINUX_RT_SIGRETURN => sys_rt_sigreturn(),

            // Stubbed syscalls — return 0 (success) to prevent crashes
            LINUX_RT_SIGPROCMASK | LINUX_RT_SIGACTION => 0,
            LINUX_SET_TID_ADDRESS => 0,
            LINUX_FUTEX => 0,
            LINUX_MPROTECT => 0,
            LINUX_MADVISE => 0,
            LINUX_PERSONALITY => 0,
            LINUX_UMASK => 0o022,
            LINUX_GETRLIMIT | LINUX_SETRLIMIT => 0,
            LINUX_GETRUSAGE => 0,
            LINUX_SYSINFO => 0,
            LINUX_MLOCK | LINUX_MUNLOCK => 0,
            LINUX_MINCORE => 0,
            LINUX_IOCTL | LINUX_IOCTL_ => 0,
            LINUX_FSYNC | LINUX_FDATASYNC => 0,
            LINUX_FACCESSAT => 0,
            LINUX_STATFS | LINUX_FSTATFS => 0,

            LINUX_NANOSLEEP => {
                // args[0] = timespec ptr {sec, nsec}, args[1] = rem ptr
                let secs = if args[0] != 0 {
                    unsafe { *(args[0] as *const u64) }
                } else { 0 };
                sys_sleep(secs * 1000);
                0
            }

            LINUX_UNAME => {
                // Fill utsname struct (6 × 65-byte fields)
                if args[0] != 0 {
                    let buf = unsafe {
                        core::slice::from_raw_parts_mut(
                            args[0] as *mut u8,
                            6 * 65,
                        )
                    };
                    for b in buf.iter_mut() { *b = 0; }
                    // sysname
                    let name = b"lobster-os\0";
                    buf[..name.len()].copy_from_slice(name);
                    // nodename
                    let node = b"lobster\0";
                    buf[65..65+node.len()].copy_from_slice(node);
                    // release
                    let rel = b"0.2.0\0";
                    buf[130..130+rel.len()].copy_from_slice(rel);
                    // version
                    let ver = b"lobster-os 0.2.0\0";
                    buf[195..195+ver.len()].copy_from_slice(ver);
                    // machine
                    let mach = b"aarch64\0";
                    buf[260..260+mach.len()].copy_from_slice(mach);
                }
                0
            }

            LINUX_CLOCK_GETTIME | LINUX_GETTIMEOFDAY | LINUX_TIME => {
                // Return system uptime in appropriate format
                let now_us = crate::arch::timer::Timer::system_time_us();
                if syscall_num == LINUX_CLOCK_GETTIME {
                    // args[1] = timespec ptr {sec, nsec}
                    if args[1] != 0 {
                        unsafe {
                            *(args[1] as *mut u64) = now_us / 1_000_000;
                            *((args[1] as *mut u64).add(1)) = (now_us % 1_000_000) * 1000;
                        }
                    }
                } else if syscall_num == LINUX_GETTIMEOFDAY {
                    // args[0] = timeval ptr {sec, usec}
                    if args[0] != 0 {
                        unsafe {
                            *(args[0] as *mut u64) = now_us / 1_000_000;
                            *((args[0] as *mut u64).add(1)) = now_us % 1_000_000;
                        }
                    }
                }
                0
            }

            LINUX_OPENAT => {
                // args[0] = dirfd (AT_FDCWD = -100), args[1] = path ptr, args[2] = flags, args[3] = mode
                let path_ptr = args[1] as *const u8;
                let flags = args[2] as u32;
                if path_ptr.is_null() {
                    return u64::MAX;
                }
                // Read null-terminated path
                let path_str = unsafe {
                    let mut len = 0usize;
                    while *path_ptr.add(len) != 0 && len < 256 {
                        len += 1;
                    }
                    core::str::from_utf8_unchecked(
                        core::slice::from_raw_parts(path_ptr, len)
                    )
                };
                sys_open_vfs(path_str, flags)
            }

            LINUX_OPEN => {
                // Legacy open(path, flags, mode)
                let path_ptr = args[0] as *const u8;
                let flags = args[1] as u32;
                if path_ptr.is_null() {
                    return u64::MAX;
                }
                let path_str = unsafe {
                    let mut len = 0usize;
                    while *path_ptr.add(len) != 0 && len < 256 {
                        len += 1;
                    }
                    core::str::from_utf8_unchecked(
                        core::slice::from_raw_parts(path_ptr, len)
                    )
                };
                sys_open_vfs(path_str, flags)
            }

            LINUX_FSTAT | LINUX_NEWFSTATAT => {
                // Fill stat struct — we return a minimal struct
                let fd = args[0] as i32;
                let stat_ptr = if syscall_num == LINUX_FSTAT {
                    args[1]
                } else {
                    args[2] // newfstatat: dirfd, path, statbuf, flag
                };
                if stat_ptr != 0 {
                    // Linux stat struct (128 bytes on AArch64)
                    let buf = unsafe {
                        core::slice::from_raw_parts_mut(stat_ptr as *mut u8, 128)
                    };
                    for b in buf.iter_mut() { *b = 0; }
                    // st_mode = 0100644 (regular file, rw-r--r--)
                    unsafe { *(stat_ptr as *mut u32) = 0o100644; }
                    // st_size — get from VFS fd
                    // For stdout/stderr (fd 1, 2), return 0
                    let _ = fd;
                }
                0
            }

            LINUX_WRITEV => {
                // args[0] = fd, args[1] = iov ptr, args[2] = iovcnt
                let fd = args[0] as i32;
                let iov_ptr = args[1] as *const u64;
                let iovcnt = args[2] as usize;
                let mut total = 0u64;
                for i in 0..iovcnt {
                    unsafe {
                        let buf_ptr = *iov_ptr.add(i * 2);
                        let len = *iov_ptr.add(i * 2 + 1) as usize;
                        if len > 0 && buf_ptr != 0 {
                            total += sys_write(fd, buf_ptr, len);
                        }
                    }
                }
                total
            }

            LINUX_READV => {
                0 // Stub — return 0 bytes read
            }

            LINUX_PREAD64 => {
                // Same as read but with offset — we don't support seeking from syscall yet
                sys_read(args[0] as i32, args[1], args[2] as usize)
            }

            LINUX_PWRITE64 => {
                sys_write(args[0] as i32, args[1], args[2] as usize)
            }

            LINUX_LSEEK => {
                // args[0] = fd, args[1] = offset, args[2] = whence
                crate::fs::vfs::seek(args[0] as usize, args[1] as i64, args[2] as u32)
                    .unwrap_or(0)
            }

            LINUX_PIPE | LINUX_PIPE2 => {
                sys_pipe(args[0], args[1])
            }

            LINUX_DUP => sys_dup(args[0] as i32),
            LINUX_FCNTL => sys_fcntl(args[0] as i32, args[1] as u32, args[2]),

            LINUX_MMAP => {
                // args[0]=addr, args[1]=len, args[2]=prot, args[3]=flags, args[4]=fd, args[5]=offset
                sys_mmap(args[1], args[1] as usize, args[2] as u32, args[3] as u32)
            }

            LINUX_MUNMAP => {
                // Stub — we don't actually track user mmaps
                0
            }

            LINUX_CLONE => {
                // Stub — return child PID 0 (as child)
                // Real implementation would call sys_fork()
                sys_fork()
            }

            LINUX_EXECVE => {
                // args[0] = path ptr, args[1] = argv ptr, args[2] = envp ptr
                let path_ptr = args[0] as *const u8;
                if path_ptr.is_null() { return u64::MAX; }
                let path_str = unsafe {
                    let mut len = 0usize;
                    while *path_ptr.add(len) != 0 && len < 256 {
                        len += 1;
                    }
                    core::str::from_utf8_unchecked(
                        core::slice::from_raw_parts(path_ptr, len)
                    )
                };
                sys_exec(path_str)
            }

            LINUX_WAIT4 => {
                // args[0] = pid, args[1] = status ptr, args[2] = options, args[3] = rusage ptr
                sys_wait(args[0])
            }

            LINUX_SOCKET => u64::MAX, // No network yet
            LINUX_CONNECT => u64::MAX,
            LINUX_BIND => u64::MAX,
            LINUX_LISTEN => u64::MAX,
            LINUX_ACCEPT => u64::MAX,
            LINUX_SENDTO => u64::MAX,
            LINUX_RECVFROM => u64::MAX,
            LINUX_SETSOCKOPT => 0,
            LINUX_GETSOCKOPT => 0,
            LINUX_SHUTDOWN => 0,

            _ => {
                // Unknown — log and return -1
                #[cfg(feature = "debug_syscalls")]
                println!(
                    "[syscall] Unhandled Linux syscall #{} from PID {}",
                    syscall_num, sched::current_pid()
                );
                let _ = syscall_num;
                u64::MAX
            }
        }
    }
}

/// sys_open_vfs — real VFS open that returns a file descriptor
fn sys_open_vfs(path: &str, flags: u32) -> u64 {
    match crate::fs::vfs::open(path, flags) {
        Ok(fd) => fd as u64,
        Err(e) => {
            #[cfg(feature = "debug_syscalls")]
            println!("[syscall] open({}) failed: {:?}", path, e);
            u64::MAX
        }
    }
}

/// sys_brk — set/get program break (heap pointer)
fn sys_brk(addr: u64) -> u64 {
    if addr == 0 {
        crate::sched::get_user_brk()
    } else {
        crate::sched::set_user_brk(addr);
        addr
    }
}

/// sys_getrandom — fill buffer with random bytes
fn sys_getrandom(buf_ptr: u64, len: usize, _flags: u32) -> u64 {
    if buf_ptr == 0 || len == 0 {
        return 0;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };
    let mut seed = crate::arch::timer::Timer::system_time_us() as u32;
    for b in buf.iter_mut() {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        *b = (seed >> 16) as u8;
    }
    len as u64
}

// ═══════════════════════════════════════════════
// Classic process syscalls
// ═══════════════════════════════════════════════

/// sys_exit(code) — terminate the current process
/// In cooperative (QEMU) mode: marks task as zombie and requests cooperative exit
/// In preemptive mode: marks task as zombie and loops forever
fn sys_exit(code: i64) -> u64 {
    println!("[syscall] exit({}) from PID {}", code, sched::current_pid());

    let pid = sched::current_pid();
    if let Some(idx) = sched::find_task_index(pid) {
        unsafe {
            if let Some(ref mut task) = crate::sched::TASK_TABLE[idx] {
                task.state = crate::sched::TaskState::Zombie;
                task.exit_code = code;
                crate::sched::set_coop_exit_code(code);
                println!("[sched] PID {} exited with code {}", pid, code);
            }
        }
    }

    // Request cooperative exit — the exception handler will set SPSR to EL1h
    // so eret returns to kernel mode instead of EL0
    crate::sched::request_cooperative_exit();

    0 // exit doesn't really return, but we return 0 to satisfy the type
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
    #[cfg(target_arch = "aarch64")]
    {
        for &byte in buf {
            crate::drivers::uart::Uart10::putc(byte);
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        for &byte in buf {
            crate::arch::x86_64::console::serial_putc(byte);
        }
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
    if fd == 0 {
        // Read from UART — non-blocking poll for data
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };
        let mut count = 0usize;
        for slot in buf.iter_mut() {
            // Poll UART for a byte
            match crate::drivers::uart::Uart10::read_byte_nb() {
                Some(byte) => {
                    *slot = byte;
                    count += 1;
                    if byte == b'\r' || byte == b'\n' {
                        break; // Line-buffered: stop at newline
                    }
                }
                None => break, // No more data available
            }
        }
        if count == 0 {
            // No data available — yield to scheduler and return 0
            crate::arch::cpu::wfe();
            return 0;
        }
        return count as u64;
    }
    u64::MAX // -1 (EBADF)
}

/// sys_open(path, flags) — open a file
fn sys_open(path: *const u8, flags: u32) -> u64 {
    if path.is_null() {
        return u64::MAX;
    }
    let path_str = unsafe {
        let mut len = 0usize;
        while *path.add(len) != 0 && len < 256 {
            len += 1;
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(path, len))
    };
    sys_open_vfs(path_str, flags)
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
    // Cooperative mode (run_and_wait): the caller blocks, so make the
    // wait REAL — advance the CNTVCT clock while keeping the desktop
    // alive with WM ticks (~60fps). Preemptive mode falls back to a
    // yield. KERNEL_RETURN_ADDR is only set while a cooperative
    // run_and_wait trampoline owns the current task.
    if sched::coop_active() {
        let start = crate::arch::ArchTimer::uptime_ms();
        let mut now = start;
        let mut last_tick_ms = start;
        while now.saturating_sub(start) < ms {
            if now.saturating_sub(last_tick_ms) >= 16 {
                #[cfg(feature = "qemu_raspi5b")]
                crate::wm::tick();
                last_tick_ms = now;
            }
            crate::arch::cpu::relax();
            now = crate::arch::ArchTimer::uptime_ms();
        }
        return;
    }
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

/// sys_sigprocmask(how, set, oldset) — examine and change blocked signals
fn sys_sigprocmask(how: u32, set: u64, oldset: u64) -> u64 {
    let mut old_mask = 0u64;
    let result = crate::sched::signal::sigprocmask(how as u64, Some(set), Some(&mut old_mask));
    if result < 0 {
        return u64::MAX;
    }
    // Write old mask to user-provided pointer if provided
    if oldset != 0 {
        unsafe {
            *(oldset as *mut u64) = old_mask;
        }
    }
    0
}

/// sys_rt_sigreturn() — return from signal handler
/// Restores the saved trap frame from the user stack
fn sys_rt_sigreturn() -> u64 {
    use crate::sched::signal::sigreturn;
    let regs = crate::sched::syscall::get_current_trap_frame();
    if regs.is_null() {
        return u64::MAX;
    }

    // SAFETY: `regs` is a valid pointer to the current trap frame
    match unsafe { sigreturn(regs) } {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

/// sys_klogctl(type, buf, len) — kernel log control (dmesg)
/// type 2: read log into buffer (dmesg -r)
/// type 3: read and clear log (dmesg -c)
/// type 4: get log buffer size
/// type 5: get log buffer len
/// type 6: clear ring buffer (CAP_SYS_ADMIN)
/// type 8: wait for new message (blocking)
/// type 9: read next message (non-blocking)
fn sys_klogctl(tp: i32, buf: u64, len: usize) -> u64 {
    use crate::klog::{klog_clear, klog_read};

    match tp {
        2 | 3 => {
            // Read log (and optionally clear)
            let mut user_buf = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, len) };
            let mut seq = 0;
            let read = klog_read(&mut user_buf, &mut seq);
            if tp == 3 {
                klog_clear();
            }
            read as u64
        }
        4 => {
            // Get buffer size
            crate::klog::klog_len() as u64
        }
        5 => {
            // Get buffer len (same as 4 for us)
            crate::klog::klog_len() as u64
        }
        6 => {
            // Clear ring buffer
            klog_clear();
            0
        }
        8 | 9 => {
            // Wait for/read message (simplified - just read available)
            let mut user_buf = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, len) };
            let mut seq = 0;
            klog_read(&mut user_buf, &mut seq) as u64
        }
        _ => u64::MAX,
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
// PTY syscalls
// ═══════════════════════════════════════════════

/// sys_openpty(amaster_ptr, aslave_ptr) — create a new PTY pair
/// Writes master_fd to amaster_ptr and slave_fd to aslave_ptr
fn sys_openpty(amaster_ptr: u64, aslave_ptr: u64) -> u64 {
    let pid = sched::current_pid();
    let _task_idx = match sched::find_task_index(pid) {
        Some(i) => i,
        None => return u64::MAX,
    };

    match crate::ipc::pty_open() {
        Some((master_fd, slave_fd)) => {
            unsafe {
                core::ptr::write(amaster_ptr as *mut i32, master_fd as i32);
                core::ptr::write(aslave_ptr as *mut i32, slave_fd as i32);
            }
            0
        }
        None => u64::MAX,
    }
}

/// sys_forkpty(amaster_ptr, aslave_ptr) — create a new PTY pair and fork
/// The child process will have the slave as its controlling terminal
fn sys_forkpty(amaster_ptr: u64, aslave_ptr: u64) -> u64 {
    // For now, just call openpty - forkpty requires process creation
    // which is more complex. Userspace can implement forkpty using openpty + fork.
    sys_openpty(amaster_ptr, aslave_ptr)
}

/// sys_ioctl(fd, cmd, arg) — I/O control for devices
/// Handles PTY ioctls: TIOCSCTTY, TIOCGWINSZ, TIOCSWINSZ
fn sys_ioctl(fd: i32, cmd: u64, arg: u64) -> u64 {
    let pid = sched::current_pid();
    let task_idx = match sched::find_task_index(pid) {
        Some(i) => i,
        None => return u64::MAX,
    };

    // First check if this is an IPC fd (PTY)
    if let Some(fd_table) = crate::ipc::get_fd_table(task_idx) {
        if let Some(entry) = fd_table.get(fd as usize) {
            match entry.kind {
                crate::ipc::FdKind::PtyMaster | crate::ipc::FdKind::PtySlave => {
                    let result = crate::ipc::pty_ioctl(entry.index as usize, cmd, arg);
                    return if result < 0 { u64::MAX } else { result as u64 };
                }
                crate::ipc::FdKind::VfsFile => {
                    // TODO: VFS ioctl
                }
                _ => {}
            }
        }
    }

    // Fallback for special fds (stdin=0, stdout=1, stderr=2)
    if fd == 0 || fd == 1 || fd == 2 {
        // These are UART, handle basic ioctls
        match cmd {
            0x5413 => {
                // TIOCGWINSZ
                // Return window size: 24 rows, 80 cols
                let winsize = [24u16, 80u16, 0u16, 0u16];
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        winsize.as_ptr() as *const u8,
                        arg as *mut u8,
                        8,
                    );
                }
                return 0;
            }
            0x5414 => {
                // TIOCSWINSZ
                // Window size change - ignore for UART
                return 0;
            }
            _ => {}
        }
    }

    u64::MAX // -1 (ENOTTY / EBADF)
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

/// sys_stat — fill stat struct for a path
fn sys_stat(path: &str, stat_ptr: *mut u8) -> u64 {
    match crate::fs::vfs::stat(path) {
        Some(meta) => {
            unsafe {
                // Linux stat struct (128 bytes on AArch64)
                let buf = core::slice::from_raw_parts_mut(stat_ptr, 128);
                for b in buf.iter_mut() { *b = 0; }
                // st_mode
                *(stat_ptr as *mut u32) = meta.file_type as u32 | meta.permissions;
                // st_size
                *(stat_ptr.add(48) as *mut u64) = meta.size;
                // st_uid, st_gid
                *(stat_ptr.add(64) as *mut u32) = meta.uid;
                *(stat_ptr.add(68) as *mut u32) = meta.gid;
                // st_mtime (seconds)
                *(stat_ptr.add(88) as *mut u64) = meta.modified_time;
            }
            0
        }
        None => u64::MAX
    }
}

/// sys_fstat — fill stat struct for an open fd
fn sys_fstat(fd: i32, stat_ptr: *mut u8) -> u64 {
    if fd < 0 || fd > 2 { return u64::MAX; }
    unsafe {
        let buf = core::slice::from_raw_parts_mut(stat_ptr, 128);
        for b in buf.iter_mut() { *b = 0; }
        // stdout/stderr/stdin are char devices
        *(stat_ptr as *mut u32) = 0o100644; // regular file for simplicity
        *(stat_ptr.add(48) as *mut u64) = 0;
        *(stat_ptr.add(64) as *mut u32) = 0;
        *(stat_ptr.add(68) as *mut u32) = 0;
        *(stat_ptr.add(88) as *mut u64) = 0;
    }
    0
}

/// sys_getdents — read directory entries
fn sys_getdents(fd: i32, buf_ptr: *mut u8, buf_size: usize) -> u64 {
    // For now, only support fd opened via VFS
    // This is a simplified implementation
    if buf_ptr.is_null() || buf_size == 0 { return u64::MAX; }
    0 // Return 0 entries (end of directory)
}

/// sys_fsync — sync file data (already in memory for VFS)
fn sys_fsync(fd: i32) -> u64 {
    if fd < 0 { return u64::MAX; }
    0 // VFS is in-memory, nothing to flush
}

/// sys_truncate — truncate file by path
fn sys_truncate(path: &str, length: u64) -> u64 {
    match crate::fs::vfs::truncate(path, length) {
        Ok(_) => 0,
        Err(_) => u64::MAX
    }
}

/// sys_ftruncate — truncate file by fd
fn sys_ftruncate(_fd: i32, _length: u64) -> u64 {
    // VFS ftruncate not yet implemented
    u64::MAX
}

/// sys_dlopen — load shared library
fn sys_dlopen(path_ptr: u64, flags: u32) -> u64 {
    crate::sched::shared_lib::sys_dlopen(path_ptr, flags)
}

/// sys_dlsym — lookup symbol in shared library
fn sys_dlsym(handle: u64, symbol_ptr: u64) -> u64 {
    crate::sched::shared_lib::sys_dlsym(handle, symbol_ptr)
}

/// sys_dlclose — unload shared library
fn sys_dlclose(handle: u64) -> u64 {
    crate::sched::shared_lib::sys_dlclose(handle)
}
