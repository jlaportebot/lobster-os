/*
 * hello_lobster.c — Test C program for LobsterOS EL0 execution.
 *
 * This is a freestanding C program (no libc) that uses LobsterOS's custom
 * syscall ABI: x8=syscall_number, x0-x5=args, svc #0.
 *
 * Syscall numbers (from src/sched/syscall.rs):
 *   0 = exit(code)
 *   1 = write(fd, buf, len)
 *   6 = getpid()
 *   7 = sleep(ms)
 *
 * Build:
 *   aarch64-linux-gnu-gcc -nostdlib -static -ffreestanding -O2 \
 *     -Wl,-Ttext=0x10000000 -o hello_lobster hello_lobster.c
 *
 * The linker loads the .text at 0x10000000 which matches USER_CODE_START.
 * The kernel ELF loader handles both ET_EXEC and ET_DYN.
 */

/* LobsterOS syscall numbers */
#define SYS_EXIT   0
#define SYS_WRITE  1
#define SYS_GETPID 6
#define SYS_SLEEP  7

/* LobsterOS custom syscall: x8=number, x0-x5=args, svc #0 */
static inline long lobster_syscall0(long num) {
    register long x8 asm("x8") = num;
    register long x0 asm("x0");
    asm volatile("svc #0" : "=r"(x0) : "r"(x8) : "memory");
    return x0;
}

static inline long lobster_syscall3(long num, long a0, long a1, long a2) {
    register long x8 asm("x8") = num;
    register long x0 asm("x0") = a0;
    register long x1 asm("x1") = a1;
    register long x2 asm("x2") = a2;
    asm volatile("svc #0" : "=r"(x0) : "r"(x8), "r"(x0), "r"(x1), "r"(x2) : "memory");
    return x0;
}

static void lobster_write(int fd, const char *buf, unsigned long len) {
    lobster_syscall3(SYS_WRITE, fd, (long)buf, len);
}

static void lobster_exit(int code) {
    lobster_syscall0(SYS_EXIT);
    /* Should not return */
    while (1);
}

static unsigned long lobster_strlen(const char *s) {
    unsigned long len = 0;
    while (s[len]) len++;
    return len;
}

/* Integer to decimal string conversion */
static int lobster_itoa(int value, char *buf) {
    if (value == 0) {
        buf[0] = '0';
        return 1;
    }

    char tmp[12];
    int i = 0;
    int neg = 0;

    if (value < 0) {
        neg = 1;
        /* Avoid overflow for INT_MIN */
        unsigned int uval = (unsigned int)value;
        while (uval > 0) {
            tmp[i++] = '0' + (uval % 10);
            uval /= 10;
        }
    } else {
        while (value > 0) {
            tmp[i++] = '0' + (value % 10);
            value /= 10;
        }
    }

    int pos = 0;
    if (neg) buf[pos++] = '-';
    /* Reverse tmp into buf */
    for (int j = i - 1; j >= 0; j--) {
        buf[pos++] = tmp[j];
    }
    return pos;
}

void _start(void) {
    /* Print a hello message */
    const char *msg1 = "Hello from C on LobsterOS!\n";
    lobster_write(1, msg1, lobster_strlen(msg1));

    /* Get our PID */
    long pid = lobster_syscall0(SYS_GETPID);
    char pid_buf[32];
    int pid_len = lobster_itoa((int)pid, pid_buf);
    pid_buf[pid_len++] = '\n';
    const char *pid_msg = "My PID is ";
    lobster_write(1, pid_msg, lobster_strlen(pid_msg));
    lobster_write(1, pid_buf, pid_len);

    /* Simple arithmetic to prove the compiler works */
    int a = 42;
    int b = 58;
    int sum = a + b;
    char sum_buf[32];
    int sum_len = lobster_itoa(sum, sum_buf);
    sum_buf[sum_len++] = '\n';
    const char *sum_msg = "42 + 58 = ";
    lobster_write(1, sum_msg, lobster_strlen(sum_msg));
    lobster_write(1, sum_buf, sum_len);

    /* Exit cleanly */
    lobster_exit(0);
}
