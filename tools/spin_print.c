/* spin_print.c — EL0 test program for O5.2 preemption: spins and prints a
 * running counter every PRINT_EVERY iterations (one write syscall per
 * line), so progress is visible on the console while it competes with
 * other spinners. Runs until killed.
 *
 * Build: tools/build_test_programs.sh
 */

#define SYS_WRITE 1
#define STDOUT_FD 1
#define PRINT_EVERY (1UL << 22)
#define DECIMAL_BASE 10
#define LINE_BUF_SIZE 48

static long lobster_write(const char *buf, unsigned long len) {
    register long x8 asm("x8") = SYS_WRITE;
    register long x0 asm("x0") = STDOUT_FD;
    register long x1 asm("x1") = (long)buf;
    register long x2 asm("x2") = (long)len;
    asm volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
    return x0;
}

/* Writes "spin_print: <n>\n" into buf, returns its length. */
static unsigned long format_line(char *buf, unsigned long n) {
    static const char prefix[] = "spin_print: ";
    char digits[24];
    unsigned long len = 0, ndig = 0;
    for (; prefix[len] != '\0'; len++) {
        buf[len] = prefix[len];
    }
    do {
        digits[ndig++] = (char)('0' + n % DECIMAL_BASE);
        n /= DECIMAL_BASE;
    } while (n != 0);
    while (ndig > 0) {
        buf[len++] = digits[--ndig];
    }
    buf[len++] = '\n';
    return len;
}

void _start(void) {
    volatile unsigned long counter = 0;
    char line[LINE_BUF_SIZE];
    unsigned long reported = 0;
    for (;;) {
        counter++;
        if ((counter & (PRINT_EVERY - 1)) == 0) {
            reported++;
            lobster_write(line, format_line(line, reported));
        }
    }
}
