#include <stdio.h>
#include <string.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    printf("Hello from LobsterOS libc!\n");
    printf("argc = %d\n", argc);
    for (int i = 0; i < argc; i++) {
        printf("  argv[%d] = %s\n", i, argv[i]);
    }
    printf("Math: %d + %d = %d\n", 42, 58, 42 + 58);
    printf("String test: %s (len=%d)\n", "hello world", (int)strlen("hello world"));
    printf("Malloc test: allocating 64 bytes...\n");
    char *p = (char *)malloc(64);
    if (p) {
        strcpy(p, "malloc works!");
        printf("  p = %s\n", p);
        free(p);
    } else {
        printf("  malloc FAILED!\n");
    }
    printf("Done!\n");
    return 0;
}
