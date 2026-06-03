//! Syscall interface for user-space processes (EL0 → EL1)
//!
//! Syscalls are invoked via the `svc #N` instruction from EL0.
//! The exception handler dispatches to the appropriate syscall handler.
//!
//! Convention:
//!   x8 = syscall number
//!   x0-x5 = arguments (up to 6)
//!   x0 = return value
//!
//! Syscall numbers:
//!   0 = exit(code)
//!   1 = write(fd, buf, len)
//!   2 = read(fd, buf, len)
//!   3 = open(path, flags)
//!   4 = close(fd)
//!   5 = yield()
//!   6 = getpid()
//!   7 = sleep(ms)
//!   8 = fork()
//!   9 = exec(path)
//!  10 = wait(pid)
//!  11 = mmap(addr, len, prot, flags)
//!  12 = munmap(addr, len)

use core::arch::asm;
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

/// Maximum syscall number
const MAX_SYSCALL: u64 = 12;

/// Dispatch a syscall from EL0
///
/// Called by the EL0 synchronous exception handler when it detects
/// an SVC instruction. Arguments come from the saved trap frame.
///
/// Returns the value to put in x0 of the calling process.
pub fn dispatch(syscall_num: u64, args: &[u64; 6]) -> u64 {
    if syscall_num > MAX_SYSCALL {
        println!("[syscall] Invalid syscall #{} from PID {}", syscall_num, sched::current_pid());
        return u64::MAX; // -1
    }

    match syscall_num {
        SYS_EXIT => {
            let code = args[0] as i64;
            sys_exit(code);
            0 // Never reached
        }
        SYS_WRITE => {
            sys_write(args[0] as i32, args[1], args[2] as usize)
        }
        SYS_READ => {
            sys_read(args[0] as i32, args[1], args[2] as usize)
        }
        SYS_OPEN => {
            sys_open(args[1] as *const u8, args[2] as u32)
        }
        SYS_CLOSE => {
            sys_close(args[0] as i32)
        }
        SYS_YIELD => {
            sys_yield();
            0
        }
        SYS_GETPID => {
            sys_getpid()
        }
        SYS_SLEEP => {
            sys_sleep(args[0]);
            0
        }
        SYS_FORK => {
            sys_fork()
        }
        SYS_EXEC => {
            sys_exec(args[1] as *const u8);
            u64::MAX
        }
        SYS_WAIT => {
            sys_wait(args[0])
        }
        SYS_MMAP => {
            sys_mmap(args[1], args[2] as usize, args[3] as u32, args[4] as u32)
        }
        SYS_MUNMAP => {
            sys_munmap(args[1], args[2] as usize)
        }
        _ => {
            u64::MAX // -1 (ENOSYS)
        }
    }
}

/// sys_exit(code) — terminate the current process
fn sys_exit(code: i64) -> ! {
    println!("[syscall] exit({}) from PID {}", code, sched::current_pid());
    sched::exit_current(code)
}

/// sys_write(fd, buf_ptr, len) — write to file descriptor
fn sys_write(fd: i32, buf_ptr: u64, len: usize) -> u64 {
    // For now, only support fd=1 (stdout) and fd=2 (stderr) → UART
    if fd != 1 && fd != 2 {
        return u64::MAX; // -1 (EBADF)
    }

    // Safety: we trust the user pointer (for now — no VMA validation yet)
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };

    // Write to UART
    for &byte in buf {
        crate::drivers::uart::Uart10::putc(byte);
    }

    len as u64 // Return bytes written
}

/// sys_read(fd, buf_ptr, len) — read from file descriptor
fn sys_read(fd: i32, _buf_ptr: u64, _len: usize) -> u64 {
    // For now, only support fd=0 (stdin) → UART
    if fd != 0 {
        return u64::MAX; // -1 (EBADF)
    }

    // TODO: implement UART read with blocking
    0 // Return 0 bytes read (EOF)
}

/// sys_open(path, flags) — open a file
fn sys_open(_path: *const u8, _flags: u32) -> u64 {
    // TODO: implement via VFS
    u64::MAX // -1 (ENOSYS)
}

/// sys_close(fd) — close a file descriptor
fn sys_close(_fd: i32) -> u64 {
    // TODO: implement via VFS
    u64::MAX // -1 (ENOSYS)
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
    // Block the task; the timer tick will eventually unblock it
    // (Simplified: just yield for now)
    let _ = target_tick;
    sched::yield_task();
}

/// sys_fork() — create a child process
fn sys_fork() -> u64 {
    // TODO: implement process duplication
    println!("[syscall] fork() not yet implemented");
    u64::MAX // -1 (ENOSYS)
}

/// sys_exec(path) — replace current process image
fn sys_exec(_path: *const u8) {
    // TODO: implement via ELF loader
    println!("[syscall] exec() not yet implemented");
}

/// sys_wait(pid) — wait for a child process
fn sys_wait(_pid: u64) -> u64 {
    // TODO: implement
    u64::MAX // -1 (ENOSYS)
}

/// sys_mmap(addr, len, prot, flags) — map memory
fn sys_mmap(_addr: u64, _len: usize, _prot: u32, _flags: u32) -> u64 {
    // TODO: implement user-space memory mapping
    println!("[syscall] mmap() not yet implemented");
    u64::MAX // -1 (ENOSYS)
}

/// sys_munmap(addr, len) — unmap memory
fn sys_munmap(_addr: u64, _len: usize) -> u64 {
    // TODO: implement
    u64::MAX // -1 (ENOSYS)
}

/// Helper for user programs: invoke a syscall
/// Uses inline assembly to execute `svc #0` with arguments in registers
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
