/* minesweeper.c - mouse-driven Minesweeper for the LobsterOS desktop.
 * Left click reveals a cell, right click plants a flag, a click on the
 * header starts a new game. The first reveal is always safe.
 */
#include "../../common/el0.h"

#define COLS 9
#define ROWS 9
#define MINES 10
#define CELL 26
#define HEADER_H 44
#define MARGIN 8
#define BOARD_W (COLS * CELL)
#define WIN_W (BOARD_W + 2 * MARGIN)
#define WIN_H (HEADER_H + ROWS * CELL + MARGIN)
#define WIN_X 180
#define WIN_Y 90
#define POLL_INTERVAL_MS 16
#define NEIGHBOURS 8

#define COLOR_BG 0xFFC9CED6u
#define COLOR_HIDDEN 0xFF8FA3C0u
#define COLOR_HIDDEN_EDGE 0xFFE7EEF8u
#define COLOR_OPEN 0xFFEEF1F6u
#define COLOR_MINE 0xFFD64545u
#define COLOR_FLAG 0xFFE8A317u
#define COLOR_TEXT 0xFF1B2433u
#define COLOR_WIN 0xFF2E8B57u

enum phase { READY, PLAYING, WON, LOST };

struct cell {
    u8 mine;
    u8 open;
    u8 flag;
    u8 near;
};

static struct cell board[ROWS][COLS];
static enum phase phase;
static unsigned long rng_state;
static unsigned long started_ms;
static unsigned long finished_ms;

static const u32 NUMBER_COLORS[NEIGHBOURS + 1] = {
    0, 0xFF1F5FBF, 0xFF2E8B57, 0xFFD64545, 0xFF5B2A86,
    0xFF8B2E2E, 0xFF1F8B8B, 0xFF1B2433, 0xFF666666,
};

static int in_board(int row, int col)
{
    return row >= 0 && row < ROWS && col >= 0 && col < COLS;
}

static void reset(void)
{
    for (int r = 0; r < ROWS; r++) {
        for (int c = 0; c < COLS; c++) {
            board[r][c].mine = board[r][c].open = board[r][c].flag = board[r][c].near = 0;
        }
    }
    phase = READY;
}

static void count_neighbours(void)
{
    for (int r = 0; r < ROWS; r++) {
        for (int c = 0; c < COLS; c++) {
            int near = 0;
            for (int dr = -1; dr <= 1; dr++) {
                for (int dc = -1; dc <= 1; dc++) {
                    near += in_board(r + dr, c + dc) && board[r + dr][c + dc].mine;
                }
            }
            board[r][c].near = (u8)near;
        }
    }
}

/* Places the mines after the first click so that cell is never one. */
static void place_mines(int safe_row, int safe_col)
{
    int placed = 0;
    while (placed < MINES) {
        int r = (int)(next_random(&rng_state) % ROWS);
        int c = (int)(next_random(&rng_state) % COLS);
        if (board[r][c].mine || (r == safe_row && c == safe_col)) {
            continue;
        }
        board[r][c].mine = 1;
        placed++;
    }
    count_neighbours();
}

static int closed_safe_cells(void)
{
    int closed = 0;
    for (int r = 0; r < ROWS; r++) {
        for (int c = 0; c < COLS; c++) {
            closed += !board[r][c].mine && !board[r][c].open;
        }
    }
    return closed;
}

static int flags_left(void)
{
    int left = MINES;
    for (int r = 0; r < ROWS; r++) {
        for (int c = 0; c < COLS; c++) {
            left -= board[r][c].flag;
        }
    }
    return left;
}

/* Flood fill without recursion: the stack at EL0 is small. */
static void reveal(int row, int col)
{
    static int stack[ROWS * COLS];
    int top = 0;
    stack[top++] = row * COLS + col;
    while (top > 0) {
        int at = stack[--top];
        struct cell *cell = &board[at / COLS][at % COLS];
        if (cell->open || cell->flag) {
            continue;
        }
        cell->open = 1;
        if (cell->mine || cell->near) {
            continue;
        }
        for (int dr = -1; dr <= 1; dr++) {
            for (int dc = -1; dc <= 1; dc++) {
                int r = at / COLS + dr;
                int c = at % COLS + dc;
                if (in_board(r, c) && !board[r][c].open) {
                    stack[top++] = r * COLS + c;
                }
            }
        }
    }
}

static void click_cell(int row, int col, unsigned button)
{
    struct cell *cell = &board[row][col];
    if (phase == WON || phase == LOST) {
        return;
    }
    if (button == BTN_RIGHT) {
        cell->flag = (u8)(!cell->flag && !cell->open);
        return;
    }
    if (cell->flag || cell->open) {
        return;
    }
    if (phase == READY) {
        place_mines(row, col);
        phase = PLAYING;
        started_ms = now_ms();
    }
    reveal(row, col);
    if (cell->mine) {
        phase = LOST;
    } else if (closed_safe_cells() == 0) {
        phase = WON;
    }
    if (phase == WON || phase == LOST) {
        finished_ms = now_ms();
    }
}

static void draw_cell(struct win *win, int row, int col)
{
    struct cell *cell = &board[row][col];
    int x = MARGIN + col * CELL;
    int y = HEADER_H + row * CELL;
    char digit[2] = {0, 0};
    int reveal_mine = cell->mine && (phase == LOST);
    if (!cell->open && !reveal_mine) {
        win_fill(win, x, y, CELL, CELL, COLOR_HIDDEN_EDGE);
        win_fill(win, x + 2, y + 2, CELL - 2, CELL - 2, COLOR_HIDDEN);
        if (cell->flag) {
            win_fill(win, x + 8, y + 6, 3, 14, COLOR_TEXT);
            win_fill(win, x + 11, y + 6, 8, 7, COLOR_FLAG);
        }
        return;
    }
    win_fill(win, x, y, CELL, CELL, COLOR_HIDDEN_EDGE);
    win_fill(win, x + 1, y + 1, CELL - 1, CELL - 1, cell->mine ? COLOR_MINE : COLOR_OPEN);
    if (cell->mine) {
        win_fill(win, x + 8, y + 8, 10, 10, COLOR_TEXT);
    } else if (cell->near) {
        digit[0] = (char)('0' + cell->near);
        win_text(win, x + 8, y + 6, digit, 2, NUMBER_COLORS[cell->near]);
    }
}

static const char *status_text(void)
{
    if (phase == WON) {
        return "YOU WIN!";
    }
    if (phase == LOST) {
        return "BOOM";
    }
    return phase == READY ? "CLICK TO START" : "MINES:";
}

static void draw_header(struct win *win)
{
    char digits[24];
    unsigned long end = phase == PLAYING ? now_ms() : finished_ms;
    unsigned long secs = phase == READY ? 0 : (end - started_ms) / 1000UL;
    win_fill(win, 0, 0, WIN_W, HEADER_H, COLOR_BG);
    int x = win_text(win, MARGIN, 14, status_text(), 2, phase == WON ? COLOR_WIN : COLOR_TEXT);
    if (phase == PLAYING) {
        fmt_uint((unsigned long)flags_left(), digits);
        win_text(win, x + 6, 14, digits, 2, COLOR_MINE);
    }
    fmt_uint(secs, digits);
    win_text(win, WIN_W - MARGIN - text_width(digits, 2), 14, digits, 2, COLOR_TEXT);
}

static void draw(struct win *win)
{
    win_fill(win, 0, HEADER_H, WIN_W, WIN_H - HEADER_H, COLOR_BG);
    draw_header(win);
    for (int r = 0; r < ROWS; r++) {
        for (int c = 0; c < COLS; c++) {
            draw_cell(win, r, c);
        }
    }
    win_flush(win);
}

static void handle_event(unsigned long ev)
{
    int x = EV_X(ev) - MARGIN;
    int y = EV_Y(ev) - HEADER_H;
    if (EV_KIND(ev) != EV_PRESS) {
        return;
    }
    if (y < 0) {
        reset();
        return;
    }
    if (x >= 0 && y >= 0 && x < BOARD_W && y < ROWS * CELL) {
        click_cell(y / CELL, x / CELL, EV_BUTTON(ev));
    }
}

void _start(void)
{
    struct win win;
    unsigned long last_second = 0;
    rng_state = random_seed();
    if (win_open(&win, WIN_X, WIN_Y, WIN_W, WIN_H, "Minesweeper") < 0) {
        out_str("minesweeper: cannot open a window\n");
        sys_exit(1);
    }
    reset();
    draw(&win);
    for (;;) {
        unsigned long ev = win_poll(&win);
        if (ev == WIN_ERROR) {
            sys_exit(0);
        }
        if (ev != 0) {
            handle_event(ev);
        } else {
            sleep_ms(POLL_INTERVAL_MS);
        }
        unsigned long second = now_ms() / 1000UL;
        if (ev != 0 || (phase == PLAYING && second != last_second)) {
            last_second = second;
            draw(&win);
        }
    }
}
