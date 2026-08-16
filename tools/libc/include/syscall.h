#ifndef _SYSCALL_H
#define _SYSCALL_H

static inline long syscall3(long num, long a0, long a1, long a2) {
    register long x8 asm("x8") = num;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    asm volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
    return x0;
}

static inline long syscall2(long num, long a0, long a1) {
    return syscall3(num, a0, a1, 0);
}

static inline long syscall1(long num, long a0) {
    return syscall3(num, a0, 0, 0);
}

static inline long syscall0(long num) {
    return syscall3(num, 0, 0, 0);
}

#endif // _SYSCALL_H