#include <stdio.h>
#include <stdlib.h>

void __assert_fail(const char *expr, const char *file, int line, const char *func) {
    if (func) {
        printf("Assertion failed: %s (%s: %s: %d)\n", expr, file, func, line);
    } else {
        printf("Assertion failed: %s (%s: %d)\n", expr, file, line);
    }
    abort();
}
