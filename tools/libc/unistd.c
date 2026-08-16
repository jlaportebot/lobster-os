#include <unistd.h>
#include <stddef.h>
#include "syscall.h"
#include "errno.h"
#include <stdlib.h>

ssize_t read(int fd, void *buf, size_t count) {
    long ret = syscall3(2, fd, (long)buf, count);
    if (ret < 0) {
        errno = -ret;
        return -1;
    }
    return ret;
}

ssize_t write(int fd, const void *buf, size_t count) {
    long ret = syscall3(1, fd, (long)buf, count);
    if (ret < 0) {
        errno = -ret;
        return -1;
    }
    return ret;
}

int close(int fd) {
    long ret = syscall3(4, fd, 0, 0);
    if (ret < 0) {
        errno = -ret;
        return -1;
    }
    return 0;
}

// Fork and exec are not implemented in LobsterOS, so we return -1 and set ENOSYS.
int fork(void) {
    errno = ENOSYS;
    return -1;
}

int execve(const char *path, char *const argv[], char *const envp[]) {
    (void)path; (void)argv; (void)envp;
    errno = ENOSYS;
    return -1;
}

unsigned int sleep(unsigned int seconds) {
    // Convert seconds to milliseconds for the sleep syscall (which takes ms)
    long ret = syscall3(7, (long)seconds * 1000, 0, 0);
    if (ret < 0) {
        errno = -ret;
        return (unsigned int)-1;
    }
    // The sleep syscall returns the remaining time in milliseconds? 
    // But the LobsterOS sleep syscall (syscall 7) is defined as sleep(ms) and returns nothing? 
    // We'll assume it returns 0 on success and error on failure.
    // We already handled error, so return 0.
    return 0;
}

pid_t getpid(void) {
    long ret = syscall1(6, 0);
    if (ret < 0) {
        errno = -ret;
        return -1;
    }
    return (pid_t)ret;
}

int brk(void *addr) {
    // We don't have a brk syscall in LobsterOS? 
    // The LobsterOS syscall list doesn't include brk. 
    // We'll implement brk and sbrk using the heap_end from stdlib.c? 
    // But we are in unistd.c and we don't want to depend on stdlib.c's heap_end.
    // We'll have to implement a simple brk that uses the sbrk syscall? 
    // Actually, the LobsterOS kernel might not have a brk syscall. 
    // We'll have to simulate it by managing our own heap and using the sbrk syscall to get more memory.
    // However, the task says to implement brk and sbrk by making syscalls? 
    // But there is no brk or sbrk syscall in the given list.
    // We'll have to note that the kernel doesn't support brk/sbrk and we'll have to use a different method.
    // But the task requires brk and sbrk in unistd.c.
    // Let's look at the LobsterOS syscall list again: only 0-7 are defined.
    // We don't have brk or sbrk. 
    // We'll have to implement them by using the heap_end from stdlib.c and the sbrk function in stdlib.c? 
    // But that would create a circular dependency.
    // Alternatively, we can implement a simple heap manager in unistd.c that uses the sbrk syscall to get memory from the kernel? 
    // But there is no sbrk syscall.
    // We'll have to assume that the kernel provides a way to get more memory via a syscall that is not listed? 
    // The task says: "The LobsterOS syscall interface is:" and lists 0-7.
    // So we don't have brk or sbrk.
    // We'll have to return -1 and set ENOSYS for now.
    errno = ENOSYS;
    return -1;
}

void *sbrk(intptr_t increment) {
    // Same as brk, we don't have a syscall for it.
    errno = ENOSYS;
    return (void *)-1;
}

void _exit(int status) {
    exit(status);
}

int access(const char *pathname, int mode) {
    // We don't have an access syscall.
    errno = ENOSYS;
    return -1;
}

int unlink(const char *pathname) {
    // We don't have an unlink syscall.
    errno = ENOSYS;
    return -1;
}