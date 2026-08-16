#include <stdio.h>
#include <stdlib.h>

extern int main(int argc, char *argv[]);

void __libc_init(void) {
    // Initialize anything needed, for now we do nothing.
}

void _start(void) {
    __libc_init();
    // We cannot get argc and argv easily in a freestanding environment without the kernel providing them.
    // For simplicity, we assume argc=0 and argv=NULL.
    // In a real system, the kernel would set up the stack with argc and argv.
    // We'll just call main with 0 and NULL.
    int ret = main(0, NULL);
    exit(ret);
}