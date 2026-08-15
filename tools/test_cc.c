/* test_cc.c — Simple C program for in-emulator compilation test */

#define SYS_EXIT   0
#define SYS_WRITE  1

static inline long syscall3(long num, long a0, long a1, long a2) {
    register long x8 asm("x8") = num;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    asm volatile("svc #0" : "=r"(x0) : "r"(x8), "r"(x0), "r"(x1), "r"(x2) : "memory");
    return x0;
}

static void write_str(const char *s) {
    int len = 0;
    while (s[len]) len++;
    syscall3(SYS_WRITE, 1, (long)s, len);
}

void _start(void) {
    write_str("Compiled in-emulator!\n");
    syscall3(SYS_WRITE, 1, (long)"", 0);
    syscall3(SYS_EXIT, 0, 0, 0);
    while(1);
}
