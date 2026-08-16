#include <stdio.h>

int main(void) {
    printf("Hello from LobsterOS libc!\n");
    printf("Math: %d + %d = %d\n", 42, 58, 42 + 58);
    printf("Hex: 0x%x\n", 0xDEAD);
    printf("String: %s\n", "works!");
    return 0;
}
