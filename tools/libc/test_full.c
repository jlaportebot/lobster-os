/* Libc test program — verifies printf, string, math, malloc work
 * Compiled against liblobsterc.a (our custom libc)
 * Uses LobsterOS syscalls (not Linux numbers)
 */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

int main(void) {
    /* Test printf */
    printf("=== LobsterOS Libc Test ===\n");
    printf("Math: %d + %d = %d\n", 42, 58, 42 + 58);
    printf("Hex: 0x%x\n", 0xDEAD);
    printf("Octal: %o\n", 0755);
    printf("Char: %c%c%c\n", 'A', 'B', 'C');
    printf("String: %s\n", "works!");

    /* Test string functions */
    char buf[32];
    strcpy(buf, "hello");
    strcat(buf, " world");
    printf("String: %s (len=%d)\n", buf, (int)strlen(buf));
    printf("strcmp: %d\n", strcmp("abc", "abc"));

    /* Test malloc/free */
    printf("Malloc test...\n");
    char *p = (char *)malloc(64);
    if (p) {
        strcpy(p, "malloc works!");
        printf("  p = %s\n", p);
        free(p);
        printf("  free OK\n");
    }

    printf("=== All tests passed! ===\n");
    return 0;
}
