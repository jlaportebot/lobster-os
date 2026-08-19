/* LobsterOS syscall definitions for NCC neatlibc */
/* Maps NCC syscall interface to LobsterOS kernel syscalls */

#ifndef __LOBO_SYSCALLS_H__
#define __LOBO_SYSCALLS_H__

#include <stddef.h>
#include <stdint.h>

/* LobsterOS syscall numbers (matching src/sched/syscall.rs) */
#define SYS_EXIT        0
#define SYS_WRITE       1
#define SYS_READ        2
#define SYS_OPEN        3
#define SYS_CLOSE       4
#define SYS_YIELD       5
#define SYS_GETPID      6
#define SYS_SLEEP       7
#define SYS_FORK        8
#define SYS_EXEC        9
#define SYS_WAIT        10
#define SYS_MMAP        11
#define SYS_MUNMAP      12
#define SYS_PIPE        13
#define SYS_DUP         14
#define SYS_DUP2        15
#define SYS_SHM_CREATE  16
#define SYS_SHM_ATTACH  17
#define SYS_SHM_DETACH  18
#define SYS_MSGQ_CREATE 19
#define SYS_MSGQ_SEND   20
#define SYS_MSGQ_RECV   21
#define SYS_SEMA_CREATE 22
#define SYS_SEMA_WAIT   23
#define SYS_SEMA_POST   24
#define SYS_SEMA_GETVAL 25
#define SYS_SEMA_DESTROY 26
#define SYS_KILL        27
#define SYS_SIGACTION   28
#define SYS_SIGPROCMASK 29
#define SYS_MKFIFO      30
#define SYS_OPEN_FIFO   31
#define SYS_SHM_CTL     32
#define SYS_UNIX_SOCKET 33
#define SYS_UNIX_BIND   34
#define SYS_UNIX_LISTEN 35
#define SYS_UNIX_ACCEPT 36
#define SYS_UNIX_CONNECT 37
#define SYS_UNIX_SEND   38
#define SYS_UNIX_RECV   39
#define SYS_EVENTFD     40
#define SYS_EVENTFD_READ 41
#define SYS_EVENTFD_WRITE 42
#define SYS_FCNTL       43
#define SYS_RT_SIGRETURN 44
#define SYS_IPC_UNSHARE 45
#define SYS_IPC_SETNS   46
#define SYS_IPC_SET_MODE 47
#define SYS_OPENPTY     48
#define SYS_FORKPTY     49
#define SYS_IOCTL       50
#define SYS_SETUID      51
#define SYS_SETGID      52
#define SYS_SETEUID     53
#define SYS_SETEGID     54
#define SYS_GETUID      55
#define SYS_GETGID      56
#define SYS_GETEUID     57
#define SYS_GETEGID     58
#define SYS_GETGROUPS   59
#define SYS_SETGROUPS   60
#define SYS_CAPGET      61
#define SYS_CAPSET      62
#define SYS_CHMOD       63
#define SYS_CHOWN       64
#define SYS_SET_NO_NEW_PRIVS 65
#define SYS_GET_NO_NEW_PRIVS 66
#define SYS_KLOGCTL     67

/* New syscalls for NCC compatibility - match kernel src/sched/syscall.rs */
#define SYS_BRK         68
#define SYS_GETTIMEOFDAY 69
#define SYS_NANOSLEEP    70
#define SYS_STAT         71
#define SYS_FSTAT        72
#define SYS_LSTAT        73
#define SYS_LSEEK        74
#define SYS_GETDENTS     75
#define SYS_MKDIR        76
#define SYS_RMDIR        77
#define SYS_UNLINK       78
#define SYS_CREAT        79
#define SYS_ACCESS       80
#define SYS_GETCWD       81
#define SYS_CHDIR        82
#define SYS_TIME         83
#define SYS_UTIME        84
#define SYS_GETPPID      85
#define SYS_SETSID       86
#define SYS_UNAME        87
#define SYS_SYNC         88
#define SYS_FSYNC        89
#define SYS_FDATASYNC    90
#define SYS_TRUNCATE     91
#define SYS_FTRUNCATE    92
#define SYS_DLOPEN       93
#define SYS_DLSYM        94
#define SYS_DLCLOSE      95

/* O_RDONLY, O_WRONLY, O_RDWR, O_CREAT, O_TRUNC, etc. */
#define O_RDONLY    0
#define O_WRONLY    1
#define O_RDWR      2
#define O_CREAT     0x40
#define O_TRUNC     0x200
#define O_APPEND    0x400

/* File types and permissions */
#define S_IFMT   0170000
#define S_IFREG  0100000
#define S_IFDIR  0040000
#define S_IFCHR  0020000
#define S_IFBLK  0060000
#define S_IFIFO  0010000
#define S_IFLNK  0120000
#define S_IFSOCK 0140000

/* Permission bits */
#define S_IRUSR 0400
#define S_IWUSR 0200
#define S_IXUSR 0100
#define S_IRGRP 0040
#define S_IWGRP 0020
#define S_IXGRP 0010
#define S_IROTH 0004
#define S_IWOTH 0002
#define S_IXOTH 0001

/* Seek constants */
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

/* errno values (subset) */
#define EPERM    1
#define ENOENT   2
#define ESRCH    3
#define EINTR    4
#define EIO      5
#define ENXIO    6
#define E2BIG    7
#define ENOEXEC  8
#define EBADF    9
#define ECHILD  10
#define EAGAIN  11
#define ENOMEM  12
#define EACCES  13
#define EFAULT  14
#define EBUSY   16
#define EEXIST  17
#define EXDEV   18
#define ENODEV  19
#define ENOTDIR 20
#define EISDIR  21
#define EINVAL  22
#define ENFILE  23
#define EMFILE  24
#define ENOTTY  25
#define EFBIG   27
#define ENOSPC  28
#define ESPIPE  29
#define EROFS   30
#define EMLINK  31
#define EPIPE   32
#define EDOM    33
#define ERANGE  34

/* Thread-local errno */
extern int errno;

/* Inline syscall wrappers for AArch64 */
static inline long syscall0(long n) {
    register long x8 asm("x8") = n;
    register long x0 asm("x0");
    asm volatile("svc #0" : "=r"(x0) : "r"(x8) : "memory");
    return x0;
}

static inline long syscall1(long n, long a0) {
    register long x8 asm("x8") = n;
    register long x0 asm("x0") = a0;
    asm volatile("svc #0" : "=r"(x0) : "r"(x8), "r"(x0) : "memory");
    return x0;
}

static inline long syscall2(long n, long a0, long a1) {
    register long x8 asm("x8") = n;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    asm volatile("svc #0" : "=r"(x0) : "r"(x8), "r"(x0), "r"(x1) : "memory");
    return x0;
}

static inline long syscall3(long n, long a0, long a1, long a2) {
    register long x8 asm("x8") = n;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    asm volatile("svc #0" : "=r"(x0) : "r"(x8), "r"(x0), "r"(x1), "r"(x2) : "memory");
    return x0;
}

static inline long syscall4(long n, long a0, long a1, long a2, long a3) {
    register long x8 asm("x8") = n;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    register long x3 asm("x3") = a3;
    asm volatile("svc #0" : "=r"(x0) : "r"(x8), "r"(x0), "r"(x1), "r"(x2), "r"(x3) : "memory");
    return x0;
}

static inline long syscall5(long n, long a0, long a1, long a2, long a3, long a4) {
    register long x8 asm("x8") = n;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    register long x3 asm("x3") = a3;
    register long x4 asm("x4") = a4;
    asm volatile("svc #0" : "=r"(x0) : "r"(x8), "r"(x0), "r"(x1), "r"(x2), "r"(x3), "r"(x4) : "memory");
    return x0;
}

static inline long syscall6(long n, long a0, long a1, long a2, long a3, long a4, long a5) {
    register long x8 asm("x8") = n;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    register long x3 asm("x3") = a3;
    register long x4 asm("x4") = a4;
    register long x5 asm("x5") = a5;
    asm volatile("svc #0" : "=r"(x0) : "r"(x8), "r"(x0), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}

/* High-level wrappers matching NCC's expected interface */
static inline long Open(const char *path, int flags, int mode) {
    return syscall3(SYS_OPEN, (long)path, flags, mode);
}

static inline int Close(int fd) {
    return (int)syscall1(SYS_CLOSE, fd);
}

static inline long Read(int fd, void *data, unsigned long size) {
    return syscall3(SYS_READ, fd, (long)data, size);
}

static inline long Write(int fd, const void *data, unsigned long size) {
    return syscall3(SYS_WRITE, fd, (long)data, size);
}

static inline long Lseek(int fd, long offset, int whence) {
    return syscall3(SYS_LSEEK, fd, offset, whence);
}

static inline long Mmap(void *addr, unsigned long size, int prot, int flags, int fd, long off) {
    return syscall6(SYS_MMAP, (long)addr, size, prot, flags, fd, off);
}

static inline long Fork(void) {
    return syscall0(SYS_FORK);
}

static inline long WaitPid(int pid, int *status, int options) {
    return syscall3(SYS_WAIT, pid, (long)status, options);
}

static inline void Exit(int rc) {
    syscall1(SYS_EXIT, rc);
    while (1);
}

static inline long GetPid(void) {
    return syscall0(SYS_GETPID);
}

static inline long GetPPid(void) {
    return syscall0(SYS_GETPPID);
}

static inline long GetUid(void) {
    return syscall0(SYS_GETUID);
}

static inline long GetGid(void) {
    return syscall0(SYS_GETGID);
}

static inline long GetEUid(void) {
    return syscall0(SYS_GETEUID);
}

static inline long GetEGid(void) {
    return syscall0(SYS_GETEGID);
}

static inline long Brk(void *addr) {
    return syscall1(SYS_BRK, (long)addr);
}

static inline long GetTimeOfDay(void *tv, void *tz) {
    return syscall2(SYS_GETTIMEOFDAY, (long)tv, (long)tz);
}

static inline long ClockGettime(int clk_id, void *tp) {
    return syscall2(SYS_GETTIMEOFDAY, clk_id, (long)tp);
}

static inline long Nanosleep(const void *req, void *rem) {
    return syscall2(SYS_NANOSLEEP, (long)req, (long)rem);
}

static inline long Access(const char *path, int mode) {
    return syscall2(SYS_ACCESS, (long)path, mode);
}

static inline long Stat(const char *path, void *buf) {
    return syscall2(SYS_STAT, (long)path, (long)buf);
}

static inline long Fstat(int fd, void *buf) {
    return syscall2(SYS_FSTAT, fd, (long)buf);
}

static inline long Lstat(const char *path, void *buf) {
    return syscall2(SYS_LSTAT, (long)path, (long)buf);
}

static inline long GetDents(int fd, void *buf, unsigned long size) {
    return syscall3(SYS_GETDENTS, fd, (long)buf, size);
}

static inline long Unlink(const char *path) {
    return syscall1(SYS_UNLINK, (long)path);
}

static inline long Rmdir(const char *path) {
    return syscall1(SYS_RMDIR, (long)path);
}

static inline long Mkdir(const char *path, int mode) {
    return syscall2(SYS_MKDIR, (long)path, mode);
}

static inline long GetCwd(char *buf, long size) {
    return syscall2(SYS_GETCWD, (long)buf, size);
}

static inline long Chdir(const char *path) {
    return syscall1(SYS_CHDIR, (long)path);
}

static inline long Sync(void) {
    return syscall0(SYS_SYNC);
}

static inline long Fsync(int fd) {
    return syscall1(SYS_FSYNC, fd);
}

static inline long Fdatasync(int fd) {
    return syscall1(SYS_FDATASYNC, fd);
}

static inline long Truncate(const char *path, long length) {
    return syscall2(SYS_TRUNCATE, (long)path, length);
}

static inline long Ftruncate(int fd, long length) {
    return syscall2(SYS_FTRUNCATE, fd, length);
}

static inline long Setsid(void) {
    return syscall0(SYS_SETSID);
}

static inline long Uname(void *buf) {
    return syscall1(SYS_UNAME, (long)buf);
}

static inline long Ioctl(int fd, unsigned long request, void *arg) {
    return syscall3(SYS_IOCTL, fd, request, (long)arg);
}

static inline long Fcntl(int fd, int cmd, long arg) {
    return syscall3(SYS_FCNTL, fd, cmd, arg);
}

static inline long Dup(int oldfd) {
    return syscall1(SYS_DUP, oldfd);
}

static inline long Dup2(int oldfd, int newfd) {
    return syscall2(SYS_DUP2, oldfd, newfd);
}

static inline long Pipe(int *pipefd) {
    return syscall2(SYS_PIPE, (long)pipefd, 0);
}

static inline long Kill(int pid, int sig) {
    return syscall2(SYS_KILL, pid, sig);
}

static inline long Sleep(unsigned int seconds) {
    return syscall1(SYS_SLEEP, seconds * 1000); /* convert to ms */
}

static inline long Yield(void) {
    return syscall0(SYS_YIELD);
}

#endif /* __LOBO_SYSCALLS_H__ */