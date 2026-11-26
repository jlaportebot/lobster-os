/* two_apps.c - EL0 test program for per-process address spaces (O5.1).
 *
 * Built twice from this file: -DROLE_A and -DROLE_B. Both keep a secret in
 * the same static buffer, so it sits at the same virtual address in each.
 * A stores "SECRET-A", sleeps, and re-checks it. B finds its own zeroed
 * buffer (not A's secret), writes "DATA-B", then reads a physical address
 * where A's frames live; the kernel must kill B with a data abort while A
 * keeps its secret.
 *
 * Build: tools/build_test_programs.sh
 */

#define SYS_EXIT 0
#define SYS_WRITE 1
#define SYS_SLEEP 7
#define STDOUT_FD 1
#define LINE_BUF_SIZE 96
#define SECRET_LEN 16
#define A_SLEEP_MS 6000
#define FRAME_PROBE_ADDRESS 0x01000000UL /* identity-mapped RAM, EL1 only */

static char secret[SECRET_LEN];

static long lobster_syscall(long number, long a0, long a1) {
    register long x8 asm("x8") = number;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    asm volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1) : "memory");
    return x0;
}

static long write_text(const char *text) {
    unsigned long len = 0;
    while (text[len] != '\0') {
        len++;
    }
    register long x8 asm("x8") = SYS_WRITE;
    register long x0 asm("x0") = STDOUT_FD;
    register long x1 asm("x1") = (long)text;
    register long x2 asm("x2") = (long)len;
    asm volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
    return x0;
}

static void store(const char *text) {
    unsigned long i = 0;
    for (; text[i] != '\0'; i++) {
        secret[i] = text[i];
    }
    secret[i] = '\0';
}

#ifdef ROLE_A
static int starts_with(const char *text, const char *prefix) {
    for (; *prefix != '\0'; prefix++, text++) {
        if (*text != *prefix) {
            return 0;
        }
    }
    return 1;
}

void _start(void) {
    store("SECRET-A");
    write_text("two_apps A: secret stored\n");
    lobster_syscall(SYS_SLEEP, A_SLEEP_MS, 0);
    write_text(starts_with(secret, "SECRET-A") ? "two_apps A: secret intact\n"
                                                : "two_apps A: secret CORRUPTED\n");
    lobster_syscall(SYS_EXIT, 0, 0);
    for (;;) {
    }
}
#else
void _start(void) {
    write_text(secret[0] == '\0' ? "two_apps B: own buffer is empty (A's secret not visible)\n"
                                 : "two_apps B: SAW FOREIGN DATA\n");
    store("DATA-B");
    write_text("two_apps B: probing a physical frame address\n");
    volatile unsigned long *probe = (volatile unsigned long *)FRAME_PROBE_ADDRESS;
    unsigned long leaked = *probe;
    write_text("two_apps B: probe NOT blocked\n");
    lobster_syscall(SYS_EXIT, (long)(leaked & 1), 0);
    for (;;) {
    }
}
#endif
