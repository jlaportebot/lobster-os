/* regcheck.c — EL0 test program for context-switch fidelity: every general
 * register must keep its value while the task is preempted and resumed.
 * check_regs() loads x0-x28 and x30 with distinct patterns, spins in x29 long
 * enough for many timer preemptions, then compares every register with its
 * pattern: 0 = all intact, 100 + n = register xn was corrupted.
 * _start() runs ROUNDS rounds, prints "regcheck: OK <round>" after each, or
 * "regcheck: CORRUPT x<n>" and exits 1.
 *
 * Build: tools/build_test_programs.sh
 */
#define SYS_EXIT 0
#define SYS_WRITE 1
#define STDOUT_FD 1
#define ROUNDS 10
#define CORRUPT_BASE 100
#define DECIMAL_BASE 10
#define LINE_BUF_SIZE 48

long check_regs(void);

__asm__(
    ".text\n"
    ".global check_regs\n"
    ".type check_regs, %function\n"
    "check_regs:\n"
    "  stp x29, x30, [sp, #-16]!\n"
    "  stp x19, x20, [sp, #-16]!\n"
    "  stp x21, x22, [sp, #-16]!\n"
    "  stp x23, x24, [sp, #-16]!\n"
    "  stp x25, x26, [sp, #-16]!\n"
    "  stp x27, x28, [sp, #-16]!\n"
    "  .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,30\n"
    "  movz x\\n, #(0xA500 + \\n), lsl #48\n"
    "  movk x\\n, #0x5A5A, lsl #32\n"
    "  movk x\\n, #(0x3C00 + \\n), lsl #16\n"
    "  movk x\\n, #0xC3C3\n"
    "  .endr\n"
    "  movz x29, #0x0BEB, lsl #16\n" /* 200,000,000 iterations */
    "  movk x29, #0xC200\n"
    "1:\n"
    "  subs x29, x29, #1\n"
    "  b.ne 1b\n"
    "  movz x29, #0xA500, lsl #48\n"
    "  movk x29, #0x5A5A, lsl #32\n"
    "  movk x29, #0x3C00, lsl #16\n"
    "  movk x29, #0xC3C3\n"
    "  cmp x0, x29\n"
    "  mov x0, #100\n"
    "  b.ne 3f\n"
    "  .irp n, 1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,30\n"
    "  movz x29, #(0xA500 + \\n), lsl #48\n"
    "  movk x29, #0x5A5A, lsl #32\n"
    "  movk x29, #(0x3C00 + \\n), lsl #16\n"
    "  movk x29, #0xC3C3\n"
    "  mov x0, #(100 + \\n)\n"
    "  cmp x\\n, x29\n"
    "  b.ne 3f\n"
    "  .endr\n"
    "  mov x0, #0\n"
    "3:\n"
    "  ldp x27, x28, [sp], #16\n"
    "  ldp x25, x26, [sp], #16\n"
    "  ldp x23, x24, [sp], #16\n"
    "  ldp x21, x22, [sp], #16\n"
    "  ldp x19, x20, [sp], #16\n"
    "  ldp x29, x30, [sp], #16\n"
    "  ret\n"
    ".size check_regs, .-check_regs\n");

static long sys3(long nr, long a0, long a1, long a2) {
    register long x8 asm("x8") = nr;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    asm volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
    return x0;
}

static unsigned long append_str(char *buf, unsigned long len, const char *s) {
    for (; *s != '\0' && len < LINE_BUF_SIZE - 1; s++) {
        buf[len++] = *s;
    }
    return len;
}

static unsigned long append_dec(char *buf, unsigned long len, unsigned long v) {
    char digits[24];
    unsigned long n = 0;
    do {
        digits[n++] = (char)('0' + v % DECIMAL_BASE);
        v /= DECIMAL_BASE;
    } while (v != 0 && n < sizeof(digits));
    while (n > 0 && len < LINE_BUF_SIZE - 1) {
        buf[len++] = digits[--n];
    }
    return len;
}

static void report(const char *prefix, unsigned long value) {
    char line[LINE_BUF_SIZE];
    unsigned long len = append_str(line, 0, prefix);
    len = append_dec(line, len, value);
    line[len++] = '\n';
    sys3(SYS_WRITE, STDOUT_FD, (long)line, (long)len);
}

void _start(void) {
    for (unsigned long round = 1; round <= ROUNDS; round++) {
        long bad = check_regs();
        if (bad != 0) {
            report("regcheck: CORRUPT x", (unsigned long)(bad - CORRUPT_BASE));
            sys3(SYS_EXIT, 1, 0, 0);
        }
        report("regcheck: OK ", round);
    }
    sys3(SYS_EXIT, 0, 0, 0);
    for (;;) {
    }
}
