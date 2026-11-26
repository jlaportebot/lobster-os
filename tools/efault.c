/* efault.c - EL0 test program for validated user pointers (O5.1): hands the
 * kernel pointers it must refuse and checks each call returns -EFAULT, then
 * proves a good pointer still works and the kernel survives.
 *
 * Build: tools/build_test_programs.sh
 */

#define SYS_EXIT 0
#define SYS_WRITE 1
#define SYS_GETRANDOM 278 /* Linux ABI number, served by the compat layer */
#define STDOUT_FD 1
#define EFAULT_RESULT (-14L)
#define SAMPLE_LEN 16
#define LINE_BUF_SIZE 96
#define UNMAPPED_POINTER 0x50000000UL
#define KERNEL_TEXT_POINTER 0x80000UL
#define PERIPHERAL_POINTER 0x7C000000UL
#define READ_ONLY_CODE_POINTER 0x10000000UL
#define STACK_TOP 0x7F000000UL
#define CROSS_PAGE_BACKOFF 8UL
#define CROSS_PAGE_LEN 64UL

static long lobster_syscall(long number, long a0, long a1, long a2) {
    register long x8 asm("x8") = number;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    asm volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
    return x0;
}

static unsigned long append(char *buf, unsigned long len, const char *text) {
    for (; *text != '\0'; text++) {
        buf[len++] = *text;
    }
    return len;
}

static void report(const char *label, int ok) {
    char line[LINE_BUF_SIZE];
    unsigned long len = append(line, 0, "efault: ");
    len = append(line, len, label);
    len = append(line, len, ok ? ": -EFAULT ok" : ": FAIL");
    line[len++] = '\n';
    lobster_syscall(SYS_WRITE, STDOUT_FD, (long)line, (long)len);
}

static int refuses(unsigned long pointer, unsigned long length) {
    return lobster_syscall(SYS_GETRANDOM, (long)pointer, (long)length, 0) == EFAULT_RESULT;
}

void _start(void) {
    unsigned char good[SAMPLE_LEN];
    report("unmapped 0x50000000", refuses(UNMAPPED_POINTER, SAMPLE_LEN));
    report("kernel text 0x80000", refuses(KERNEL_TEXT_POINTER, SAMPLE_LEN));
    report("null", refuses(0, SAMPLE_LEN));
    report("peripheral 0x7c000000", refuses(PERIPHERAL_POINTER, SAMPLE_LEN));
    report("page crossing into unmapped", refuses(STACK_TOP - CROSS_PAGE_BACKOFF, CROSS_PAGE_LEN));
    report("read-only code page", refuses(READ_ONLY_CODE_POINTER, SAMPLE_LEN));
    long got = lobster_syscall(SYS_GETRANDOM, (long)good, SAMPLE_LEN, 0);
    char line[LINE_BUF_SIZE];
    unsigned long len = append(line, 0, got == SAMPLE_LEN ? "efault: valid buffer ok\n" : "efault: valid buffer FAIL\n");
    lobster_syscall(SYS_WRITE, STDOUT_FD, (long)line, (long)len);
    lobster_syscall(SYS_EXIT, got == SAMPLE_LEN ? 0 : 1, 0, 0);
    for (;;) {
    }
}
