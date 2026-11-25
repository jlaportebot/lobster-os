/* sieve.c - Sieve of Eratosthenes benchmark. Counts the primes below LIMIT
 * and reports how long that took, so two machines (or two kernels) can be
 * compared with one command.
 */
#include "../../common/el0.h"

#define LIMIT 2000000UL
#define ROUNDS 3

static unsigned char composite[LIMIT + 1];

static unsigned long count_primes(void)
{
    for (unsigned long i = 0; i <= LIMIT; i++) {
        composite[i] = 0;
    }
    for (unsigned long i = 2; i * i <= LIMIT; i++) {
        if (composite[i]) {
            continue;
        }
        for (unsigned long j = i * i; j <= LIMIT; j += i) {
            composite[j] = 1;
        }
    }
    unsigned long count = 0;
    for (unsigned long i = 2; i <= LIMIT; i++) {
        count += composite[i] ? 0 : 1;
    }
    return count;
}

void _start(void)
{
    out_str("sieve: primes below ");
    out_uint(LIMIT);
    out_str("\n");
    for (int round = 1; round <= ROUNDS; round++) {
        unsigned long start = now_ms();
        unsigned long count = count_primes();
        out_str("  round ");
        out_uint((unsigned long)round);
        out_str(": ");
        out_uint(count);
        out_str(" primes in ");
        out_uint(now_ms() - start);
        out_str(" ms\n");
    }
    sys_exit(0);
}
