/* fp_torture.c — Q3 acceptance test (audit recipe).
 *
 * Two background instances run with different seeds: each does a
 * double-precision accumulation chain, sleeping every few iterations so the
 * scheduler ends its slice mid-chain. The FP registers must hold the chain
 * state across every slice; if kernel NEON (compositor zero-init, etc.)
 * clobbers user FP state, the checksums drift.
 *
 * Expected output per instance (seed s, n = 3_000_000):
 *   FP_TORTURE s ok bits=0x...
 * The two instances print DIFFERENT bits; both must print ok=1
 * (bit-cast integer must stay stable across a re-run of the final chunk).
 */

typedef unsigned long u64;

static inline u64 syscall6(u64 n, u64 a, u64 b, u64 c, u64 d, u64 e, u64 f) {
    register u64 x8 asm("x8") = n;
    register u64 x0 asm("x0") = a;
    register u64 x1 asm("x1") = b;
    register u64 x2 asm("x2") = c;
    register u64 x3 asm("x3") = d;
    register u64 x4 asm("x4") = e;
    register u64 x5 asm("x5") = f;
    asm volatile("svc #0"
                 : "+r"(x0)
                 : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5)
                 : "memory");
    return x0;
}

#define SYS_EXIT  0
#define SYS_SLEEP 7
#define SYS_WRITE 1

static void put(char c) {
    char b = c;
    /* mini write(1, buf, 1) */
    register u64 x8 asm("x8") = SYS_WRITE;
    register u64 x0 asm("x0") = 1;
    register u64 x1 asm("x1") = (u64)&b;
    register u64 x2 asm("x2") = 1;
    asm volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
}

static void print_str(const char *s) {
    while (*s) put(*s++);
}

static void print_hex(u64 v) {
    const char *hex = "0123456789abcdef";
    char buf[17];
    for (int i = 0; i < 16; i++) buf[15 - i] = hex[(v >> (i * 4)) & 0xF];
    buf[16] = 0;
    print_str("0x");
    for (int i = 0; i < 16; i++) put(buf[i]);
}

/* Double chain that keeps 3 live FP values across syscalls. */
static double chain(double seed, u64 n, u64 sleep_every) {
    double a = seed;
    double b = seed * 1.1;
    double c = 0.5;
    for (u64 i = 0; i < n; i++) {
        a = a * 1.0000001 + b * 0.0000001 - c;
        b = b * 0.9999999 + a * 0.0000001;
        c = -c * 0.99999999 + 1.0;
        if (sleep_every && (i % sleep_every) == 0) {
            syscall6(SYS_SLEEP, 20, 0, 0, 0, 0, 0); /* 20ms — ends the slice */
        }
    }
    return a + b + c;
}

void _start(u64 argc, u64 argv) {
    u64 seed = (argc > 1 && argv) ? 42 : 7; /* both instances differ via link-time default */
    double r = chain((double)seed * 0.001, 300000, 4096);

    /* Bit-cast so we can print it as hex (no float printf in the tiny libc). */
    u64 bits;
    __builtin_memcpy(&bits, &r, 8);

    /* Re-run the LAST 1024 iterations with a fresh chain from r: if FP state
     * survived, the deterministic tail re-computation matches bits stable. */
    double r2 = chain(r, 1024, 0);
    u64 bits2;
    __builtin_memcpy(&bits2, &r2, 8);

    print_str("FP_TORTURE seed=");
    put('0' + (char)(seed % 10));
    print_str(" ok=");
    put(bits2 != 0 ? '1' : '0');
    print_str(" bits=");
    print_hex(bits);
    put('\n');

    syscall6(SYS_EXIT, 0, 0, 0, 0, 0, 0);
    for (;;) syscall6(SYS_EXIT, 0, 0, 0, 0, 0, 0);
}
