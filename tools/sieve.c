#include <stdio.h>

/* Sieve of Eratosthenes — downloaded from GitHub, compiled with LobsterOS cc */
int main(void) {
    int n = 100;
    int prime[101];
    int i, j;
    int count = 0;

    /* Initialize all as prime */
    for (i = 0; i <= n; i++) {
        prime[i] = 1;
    }
    prime[0] = 0;
    prime[1] = 0;

    /* Sieve */
    for (i = 2; i * i <= n; i++) {
        if (prime[i]) {
            for (j = i * i; j <= n; j += i) {
                prime[j] = 0;
            }
        }
    }

    /* Count and print primes */
    for (i = 2; i <= n; i++) {
        if (prime[i]) {
            printf("%d ", i);
            count++;
        }
    }
    printf("\nTotal primes up to %d: %d\n", n, count);
    return 0;
}
