// LobsterOS MicroPython UART I/O — uses our libc read/write syscalls
#include <unistd.h>
#include "py/mpconfig.h"

// Receive single character from stdin
int mp_hal_stdin_rx_chr(void) {
    unsigned char c = 0;
    int r = read(0, &c, 1);  // stdin = fd 0
    (void)r;
    return c;
}

// Send string of given length to stdout
mp_uint_t mp_hal_stdout_tx_strn(const char *str, mp_uint_t len) {
    int r = write(1, str, len);  // stdout = fd 1
    (void)r;
    return len;
}
