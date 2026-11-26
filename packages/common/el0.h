/* el0.h - freestanding helpers for LobsterOS EL0 programs.
 *
 * Syscall ABI: x8 = number, x0..x5 = arguments, `svc #0`. The window
 * syscalls are described in src/wm/user_windows.rs: the kernel owns an ARGB32
 * buffer per window, SYS_WIN_MAP maps it into the process, SYS_WIN_FLUSH
 * publishes it and SYS_WIN_POLL returns packed mouse events.
 */
#ifndef LOBSTER_EL0_H
#define LOBSTER_EL0_H

#define SYS_EXIT 0
#define SYS_WRITE 1
#define SYS_SLEEP 7
#define SYS_GETTIMEOFDAY 69
#define SYS_WIN_CREATE 110
#define SYS_WIN_FILL 111
#define SYS_WIN_FLUSH 112
#define SYS_WIN_CLOSE 113
#define SYS_WIN_POLL 114
#define SYS_WIN_MAP 115
#define SYS_GETRANDOM 278

#define STDOUT_FD 1
#define WIN_ERROR (~0UL)
#define EV_KIND(ev) ((unsigned)((ev) & 0xFF))
#define EV_BUTTON(ev) ((unsigned)(((ev) >> 8) & 0xFF))
#define EV_X(ev) ((int)(((ev) >> 16) & 0xFFFF))
#define EV_Y(ev) ((int)(((ev) >> 32) & 0xFFFF))
#define EV_PRESS 1
#define BTN_LEFT 1
#define BTN_RIGHT 2

#define FONT_W 5
#define FONT_H 7
#define FONT_ADVANCE 6

typedef unsigned long u64;
typedef unsigned int u32;
typedef unsigned char u8;

static inline long sys3(long num, long a0, long a1, long a2)
{
    register long x8 __asm__("x8") = num;
    register long x0 __asm__("x0") = a0;
    register long x1 __asm__("x1") = a1;
    register long x2 __asm__("x2") = a2;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
    return x0;
}

static inline long sys6(long num, long a0, long a1, long a2, long a3, long a4, long a5)
{
    register long x8 __asm__("x8") = num;
    register long x0 __asm__("x0") = a0;
    register long x1 __asm__("x1") = a1;
    register long x2 __asm__("x2") = a2;
    register long x3 __asm__("x3") = a3;
    register long x4 __asm__("x4") = a4;
    register long x5 __asm__("x5") = a5;
    __asm__ volatile("svc #0"
                     : "+r"(x0)
                     : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5)
                     : "memory");
    return x0;
}

static inline long sys5(long num, long a0, long a1, long a2, long a3, long a4)
{
    register long x8 __asm__("x8") = num;
    register long x0 __asm__("x0") = a0;
    register long x1 __asm__("x1") = a1;
    register long x2 __asm__("x2") = a2;
    register long x3 __asm__("x3") = a3;
    register long x4 __asm__("x4") = a4;
    __asm__ volatile("svc #0"
                     : "+r"(x0)
                     : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4)
                     : "memory");
    return x0;
}

static inline void sys_exit(int code)
{
    sys3(SYS_EXIT, code, 0, 0);
    for (;;) {
    }
}

static inline unsigned long str_len(const char *s)
{
    unsigned long n = 0;
    while (s[n]) {
        n++;
    }
    return n;
}

static inline void out_str(const char *s)
{
    sys3(SYS_WRITE, STDOUT_FD, (long)s, (long)str_len(s));
}

/* Writes `value` in decimal into `buf`; returns the digit count. */
static inline int fmt_uint(unsigned long value, char *buf)
{
    char tmp[24];
    int n = 0;
    int len = 0;
    do {
        tmp[n++] = (char)('0' + value % 10);
        value /= 10;
    } while (value);
    while (n) {
        buf[len++] = tmp[--n];
    }
    buf[len] = 0;
    return len;
}

static inline void out_uint(unsigned long value)
{
    char buf[24];
    fmt_uint(value, buf);
    out_str(buf);
}

/* Milliseconds since the epoch from SYS_GETTIMEOFDAY (struct timeval). */
static inline unsigned long now_ms(void)
{
    unsigned long tv[2] = {0, 0};
    sys3(SYS_GETTIMEOFDAY, (long)tv, 0, 0);
    return tv[0] * 1000UL + tv[1] / 1000UL;
}

static inline unsigned long random_seed(void)
{
    unsigned long seed = 0;
    if (sys3(SYS_GETRANDOM, (long)&seed, (long)sizeof seed, 0) != (long)sizeof seed) {
        seed = now_ms();
    }
    return seed | 1UL;
}

/* Linear congruential step; plenty for mine placement. */
static inline unsigned long next_random(unsigned long *state)
{
    *state = *state * 6364136223846793005UL + 1442695040888963407UL;
    return *state >> 33;
}

/* A window whose pixels the program draws directly. */
struct win {
    long id;
    u32 *px;
    int w;
    int h;
};

static inline int win_open(struct win *win, int x, int y, int w, int h, const char *title)
{
    long id = sys5(SYS_WIN_CREATE, x, y, w, h, (long)title);
    if ((unsigned long)id == WIN_ERROR) {
        return -1;
    }
    unsigned long va = (unsigned long)sys3(SYS_WIN_MAP, id, 0, 0);
    if (va == WIN_ERROR) {
        return -1;
    }
    win->id = id;
    win->px = (u32 *)va;
    win->w = w;
    win->h = h;
    return 0;
}

/* The kernel fills the rectangle (clipped to the window): a pixel loop in
 * user mode between syscalls was long enough to stall the desktop. */
static inline void win_fill(struct win *win, int x, int y, int w, int h, u32 argb)
{
    if (x < 0 || y < 0 || w <= 0 || h <= 0) {
        return;
    }
    sys6(SYS_WIN_FILL, win->id, x, y, w, h, (long)argb);
}

static inline void win_flush(struct win *win)
{
    sys3(SYS_WIN_FLUSH, win->id, 0, 0);
}

static inline unsigned long win_poll(struct win *win)
{
    return (unsigned long)sys3(SYS_WIN_POLL, win->id, 0, 0);
}

static inline void sleep_ms(unsigned ms)
{
    sys3(SYS_SLEEP, ms, 0, 0);
}

/* 5x7 column-major glyphs: bit 0 of each byte is the top row. */
static const u8 FONT_5X7[][FONT_W] = {
    {0x00, 0x00, 0x00, 0x00, 0x00}, /* space */
    {0x00, 0x00, 0x5F, 0x00, 0x00}, /* ! */
    {0x08, 0x08, 0x08, 0x08, 0x08}, /* - */
    {0x00, 0x36, 0x36, 0x00, 0x00}, /* : */
    {0x3E, 0x51, 0x49, 0x45, 0x3E}, /* 0 */
    {0x00, 0x42, 0x7F, 0x40, 0x00}, /* 1 */
    {0x42, 0x61, 0x51, 0x49, 0x46}, /* 2 */
    {0x21, 0x41, 0x45, 0x4B, 0x31}, /* 3 */
    {0x18, 0x14, 0x12, 0x7F, 0x10}, /* 4 */
    {0x27, 0x45, 0x45, 0x45, 0x39}, /* 5 */
    {0x3C, 0x4A, 0x49, 0x49, 0x30}, /* 6 */
    {0x01, 0x71, 0x09, 0x05, 0x03}, /* 7 */
    {0x36, 0x49, 0x49, 0x49, 0x36}, /* 8 */
    {0x06, 0x49, 0x49, 0x29, 0x1E}, /* 9 */
    {0x7E, 0x11, 0x11, 0x11, 0x7E}, /* A */
    {0x7F, 0x49, 0x49, 0x49, 0x36}, /* B */
    {0x3E, 0x41, 0x41, 0x41, 0x22}, /* C */
    {0x7F, 0x41, 0x41, 0x22, 0x1C}, /* D */
    {0x7F, 0x49, 0x49, 0x49, 0x41}, /* E */
    {0x7F, 0x09, 0x09, 0x09, 0x01}, /* F */
    {0x3E, 0x41, 0x49, 0x49, 0x7A}, /* G */
    {0x7F, 0x08, 0x08, 0x08, 0x7F}, /* H */
    {0x00, 0x41, 0x7F, 0x41, 0x00}, /* I */
    {0x20, 0x40, 0x41, 0x3F, 0x01}, /* J */
    {0x7F, 0x08, 0x14, 0x22, 0x41}, /* K */
    {0x7F, 0x40, 0x40, 0x40, 0x40}, /* L */
    {0x7F, 0x02, 0x0C, 0x02, 0x7F}, /* M */
    {0x7F, 0x04, 0x08, 0x10, 0x7F}, /* N */
    {0x3E, 0x41, 0x41, 0x41, 0x3E}, /* O */
    {0x7F, 0x09, 0x09, 0x09, 0x06}, /* P */
    {0x3E, 0x41, 0x51, 0x21, 0x5E}, /* Q */
    {0x7F, 0x09, 0x19, 0x29, 0x46}, /* R */
    {0x46, 0x49, 0x49, 0x49, 0x31}, /* S */
    {0x01, 0x01, 0x7F, 0x01, 0x01}, /* T */
    {0x3F, 0x40, 0x40, 0x40, 0x3F}, /* U */
    {0x1F, 0x20, 0x40, 0x20, 0x1F}, /* V */
    {0x3F, 0x40, 0x38, 0x40, 0x3F}, /* W */
    {0x63, 0x14, 0x08, 0x14, 0x63}, /* X */
    {0x07, 0x08, 0x70, 0x08, 0x07}, /* Y */
    {0x61, 0x51, 0x49, 0x45, 0x43}, /* Z */
};

static inline int glyph_index(char c)
{
    if (c == '!') {
        return 1;
    }
    if (c == '-') {
        return 2;
    }
    if (c == ':') {
        return 3;
    }
    if (c >= '0' && c <= '9') {
        return 4 + (c - '0');
    }
    if (c >= 'a' && c <= 'z') {
        c = (char)(c - 'a' + 'A');
    }
    if (c >= 'A' && c <= 'Z') {
        return 14 + (c - 'A');
    }
    return 0;
}

/* Draws `text` at `scale`x magnification; returns the x past the last glyph. */
static inline int win_text(struct win *win, int x, int y, const char *text, int scale, u32 argb)
{
    for (; *text; text++) {
        const u8 *cols = FONT_5X7[glyph_index(*text)];
        for (int cx = 0; cx < FONT_W; cx++) {
            for (int cy = 0; cy < FONT_H; cy++) {
                if (cols[cx] & (1 << cy)) {
                    win_fill(win, x + cx * scale, y + cy * scale, scale, scale, argb);
                }
            }
        }
        x += FONT_ADVANCE * scale;
    }
    return x;
}

static inline int text_width(const char *text, int scale)
{
    return (int)str_len(text) * FONT_ADVANCE * scale - scale;
}

/* gcc may lower loops to these even with -ffreestanding. */
void *memset(void *dst, int value, unsigned long n)
{
    u8 *p = dst;
    while (n--) {
        *p++ = (u8)value;
    }
    return dst;
}

void *memcpy(void *dst, const void *src, unsigned long n)
{
    u8 *d = dst;
    const u8 *s = src;
    while (n--) {
        *d++ = *s++;
    }
    return dst;
}

#endif
