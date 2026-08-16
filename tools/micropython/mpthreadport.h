// LobsterOS MicroPython thread port — stub implementation
#ifndef MPTHREADPORT_H
#define MPTHREADPORT_H

#include <stdint.h>
typedef void *mp_thread_mutex_t;

// Thread state (minimal — single threaded)
struct _mp_state_thread_t;

static inline void mp_thread_init(void) {}
static inline void mp_thread_deinit(void) {}
static inline void mp_thread_gc_others(void) {}

static inline void mp_thread_mutex_init(mp_thread_mutex_t *mutex) {
    *mutex = (void*)1; // non-NULL = initialized
}

static inline int mp_thread_mutex_lock(mp_thread_mutex_t *mutex, int wait) {
    (void)mutex; (void)wait;
    return 1; // always succeeds (single-threaded)
}

static inline void mp_thread_mutex_unlock(mp_thread_mutex_t *mutex) {
    (void)mutex;
}

#endif // MPTHREADPORT_H
