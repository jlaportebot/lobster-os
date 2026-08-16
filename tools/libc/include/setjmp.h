#ifndef _SETJMP_H
#define _SETJMP_H

// MicroPython on AArch64 uses its own NLR via inline assembly,
// not setjmp/longjmp. But some shared code references setjmp.
// We provide minimal stubs that are never actually called on AArch64.

typedef struct {
    unsigned long __jmpbuf[24]; // x19-x29, sp, lr + padding
} jmp_buf;

static inline int setjmp(jmp_buf env) {
    (void)env;
    return 0;
}

static inline void longjmp(jmp_buf env, int val) {
    (void)env;
    (void)val;
    while (1) { }
}

#endif // _SETJMP_H
