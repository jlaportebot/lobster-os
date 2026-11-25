/* hello.c - the smallest LobsterOS GUI program: one window, one greeting,
 * a click counter. It shows the whole EL0 window API (create, map, draw,
 * flush, poll) in one screen of code.
 */
#include "../../common/el0.h"

#define WIN_W 320
#define WIN_H 160
#define WIN_X 140
#define WIN_Y 110
#define GREETING_SCALE 3
#define COUNT_SCALE 2
#define POLL_INTERVAL_MS 16
#define COLOR_BG 0xFF1E2A44u
#define COLOR_FG 0xFFF4F4F8u
#define COLOR_ACCENT 0xFFFF7A59u

static void draw(struct win *win, unsigned clicks)
{
    char digits[24];
    win_fill(win, 0, 0, WIN_W, WIN_H, COLOR_BG);
    int greet_w = text_width("HELLO LOBSTER", GREETING_SCALE);
    win_text(win, (WIN_W - greet_w) / 2, 40, "HELLO LOBSTER", GREETING_SCALE, COLOR_FG);
    fmt_uint(clicks, digits);
    win_text(win, 24, 110, "CLICKS:", COUNT_SCALE, COLOR_FG);
    win_text(win, 24 + 8 * FONT_ADVANCE * COUNT_SCALE, 110, digits, COUNT_SCALE, COLOR_ACCENT);
    win_flush(win);
}

void _start(void)
{
    struct win win;
    unsigned clicks = 0;
    if (win_open(&win, WIN_X, WIN_Y, WIN_W, WIN_H, "Hello") < 0) {
        out_str("hello: cannot open a window\n");
        sys_exit(1);
    }
    draw(&win, clicks);
    for (;;) {
        unsigned long ev = win_poll(&win);
        if (ev == WIN_ERROR) {
            sys_exit(0);
        }
        if (ev == 0) {
            sleep_ms(POLL_INTERVAL_MS);
            continue;
        }
        if (EV_KIND(ev) == EV_PRESS) {
            draw(&win, ++clicks);
        }
    }
}
