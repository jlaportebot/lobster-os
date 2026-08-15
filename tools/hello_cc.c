#define SYS_EXIT   0
#define SYS_WRITE  1
#define SYS_GETPID 6

static inline long sc3(long n, long a0, long a1, long a2) {
    register long x8 asm("x8") = n;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    asm volatile("svc #0" : "=r"(x0) : "r"(x8), "r"(x0), "r"(x1), "r"(x2) : "memory");
    return x0;
}

static inline long sc0(long n) {
    register long x8 asm("x8") = n;
    register long x0 asm("x0");
    asm volatile("svc #0" : "=r"(x0) : "r"(x8) : "memory");
    return x0;
}

static int slen(const char *s) { int l = 0; while (s[l]) l++; return l; }

static int itoa(int v, char *b) {
    if (v == 0) { b[0] = '0'; return 1; }
    char t[12]; int i = 0;
    while (v > 0) { t[i++] = '0' + v % 10; v /= 10; }
    int p = 0; for (int j = i-1; j >= 0; j--) b[p++] = t[j];
    return p;
}

void _start(void) {
    const char *m = "Hello from in-emulator cc!\n";
    sc3(SYS_WRITE, 1, (long)m, slen(m));

    long pid = sc0(SYS_GETPID);
    char pb[16]; int pl = itoa((int)pid, pb);
    pb[pl++] = '\n';
    const char *pm = "PID: ";
    sc3(SYS_WRITE, 1, (long)pm, slen(pm));
    sc3(SYS_WRITE, 1, (long)pb, pl);

    int a = 42, b = 58, sum = a + b;
    char sb[16]; int sl = itoa(sum, sb);
    sb[sl++] = '\n';
    const char *sm = "42 + 58 = ";
    sc3(SYS_WRITE, 1, (long)sm, slen(sm));
    sc3(SYS_WRITE, 1, (long)sb, sl);

    sc0(SYS_EXIT);
    while (1);
}
