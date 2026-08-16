#ifndef _ASSERT_H
#define _ASSERT_H

// In freestanding mode, we define assert as a no-op or simple printf
#ifdef NDEBUG
#define assert(expr) ((void)0)
#else
#define assert(expr) \
    ((expr) ? (void)0 : __assert_fail(#expr, __FILE__, __LINE__, __func__))
#endif

void __assert_fail(const char *expr, const char *file, int line, const char *func);

#endif // _ASSERT_H
