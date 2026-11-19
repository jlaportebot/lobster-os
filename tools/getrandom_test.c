/* getrandom_test.c — EL0 test program for the kernel CSPRNG (O1.5): calls
 * getrandom(2) twice, prints both samples and whether they differ, then
 * passes a kernel-space pointer, which the kernel must refuse with -EFAULT.
 *
 * Build: tools/build_test_programs.sh
 */

#define SYS_EXIT 0
#define SYS_WRITE 1
#define SYS_GETRANDOM 278 /* Linux ABI number, served by the compat layer */
#define STDOUT_FD 1
#define SAMPLE_LEN 16
#define KERNEL_POINTER 0xFFFF000000100000UL
#define LINE_BUF_SIZE 96
#define HEX_DIGITS "0123456789abcdef"

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

static unsigned long append_hex(char *buf, unsigned long len, const unsigned char *bytes) {
    for (unsigned long i = 0; i < SAMPLE_LEN; i++) {
        buf[len++] = HEX_DIGITS[bytes[i] >> 4];
        buf[len++] = HEX_DIGITS[bytes[i] & 0xF];
    }
    return len;
}

static void print_sample(const char *label, const unsigned char *bytes) {
    char line[LINE_BUF_SIZE];
    unsigned long len = append(line, 0, label);
    len = append_hex(line, len, bytes);
    line[len++] = '\n';
    lobster_syscall(SYS_WRITE, STDOUT_FD, (long)line, (long)len);
}

static int samples_differ(const unsigned char *a, const unsigned char *b) {
    for (unsigned long i = 0; i < SAMPLE_LEN; i++) {
        if (a[i] != b[i]) {
            return 1;
        }
    }
    return 0;
}

static void print_result(const char *label, const char *verdict) {
    char line[LINE_BUF_SIZE];
    unsigned long len = append(line, 0, label);
    len = append(line, len, verdict);
    line[len++] = '\n';
    lobster_syscall(SYS_WRITE, STDOUT_FD, (long)line, (long)len);
}

void _start(void) {
    unsigned char first[SAMPLE_LEN], second[SAMPLE_LEN];
    long got1 = lobster_syscall(SYS_GETRANDOM, (long)first, SAMPLE_LEN, 0);
    long got2 = lobster_syscall(SYS_GETRANDOM, (long)second, SAMPLE_LEN, 0);
    long kernel = lobster_syscall(SYS_GETRANDOM, (long)KERNEL_POINTER, SAMPLE_LEN, 0);
    print_sample("getrandom_test: first  = ", first);
    print_sample("getrandom_test: second = ", second);
    print_result("getrandom_test: lengths ", (got1 == SAMPLE_LEN && got2 == SAMPLE_LEN) ? "ok" : "FAIL");
    print_result("getrandom_test: samples differ: ", samples_differ(first, second) ? "yes" : "NO");
    print_result("getrandom_test: kernel pointer: ", kernel == -14 ? "rejected (-EFAULT)" : "ACCEPTED");
    lobster_syscall(SYS_EXIT, 0, 0, 0);
    for (;;) {
    }
}
