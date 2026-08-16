// LobsterOS MicroPython HAL port
#ifndef MPHALPORT_H
#define MPHALPORT_H

#include <stdint.h>
#include "py/mpconfig.h"

// Minimal timer — returns 0 (we don't have a real-time clock)
static inline mp_uint_t mp_hal_ticks_ms(void) {
    return 0;
}

static inline mp_uint_t mp_hal_ticks_us(void) {
    return 0;
}

static inline mp_uint_t mp_hal_ticks_cpu(void) {
    return 0;
}

static inline void mp_hal_set_interrupt_char(char c) {
    (void)c;
}

#endif // MPHALPORT_H
