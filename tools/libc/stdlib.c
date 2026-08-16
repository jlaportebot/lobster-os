#include <stddef.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include "syscall.h"
#include "errno.h"

#define SIZE_T_SIZE (sizeof(size_t))

static char *heap_end = NULL;

void *malloc(size_t size) {
    if (size == 0) {
        return NULL;
    }
    size_t total = SIZE_T_SIZE + size;
    char *ptr = (char *)sbrk(total);
    if (ptr == (char *)-1) {
        errno = ENOMEM;
        return NULL;
    }
    *(size_t *)ptr = size;
    return (void *)(ptr + SIZE_T_SIZE);
}

void free(void *ptr) {
    if (ptr == NULL) {
        return;
    }
    // No-op for bump allocator
    // We could add to a free list, but we don't implement that.
    (void)ptr;
}

void *realloc(void *ptr, size_t size) {
    if (ptr == NULL) {
        return malloc(size);
    }
    if (size == 0) {
        free(ptr);
        return NULL;
    }
    size_t *real_ptr = (size_t *)((char *)ptr - SIZE_T_SIZE);
    size_t old_size = *real_ptr;
    void *new_ptr = malloc(size);
    if (new_ptr == NULL) {
        return NULL;
    }
    size_t copy_size = old_size < size ? old_size : size;
    memcpy(new_ptr, ptr, copy_size);
    free(ptr);
    return new_ptr;
}

void *calloc(size_t nmemb, size_t size) {
    if (nmemb == 0 || size == 0) {
        return NULL;
    }
    size_t total = nmemb * size;
    void *ptr = malloc(total);
    if (ptr) {
        memset(ptr, 0, total);
    }
    return ptr;
}

int atoi(const char *nptr) {
    int val = 0;
    int sign = 1;
    while (*nptr == ' ' || *nptr == '\t' || *nptr == '\n' || *nptr == '\v' || *nptr == '\f' || *nptr == '\r') {
        nptr++;
    }
    if (*nptr == '-') {
        sign = -1;
        nptr++;
    } else if (*nptr == '+') {
        nptr++;
    }
    while (*nptr >= '0' && *nptr <= '9') {
        val = val * 10 + (*nptr - '0');
        nptr++;
    }
    return sign * val;
}

long atol(const char *nptr) {
    return strtol(nptr, NULL, 10);
}

long strtol(const char *nptr, char **endptr, int base) {
    if (endptr) {
        *endptr = (char *)nptr;
    }
    if (base < 0 || base == 1 || base > 36) {
        if (endptr) {
            *endptr = (char *)nptr;
        }
        errno = EINVAL;
        return 0;
    }
    while (*nptr == ' ' || *nptr == '\t' || *nptr == '\n' || *nptr == '\v' || *nptr == '\f' || *nptr == '\r') {
        nptr++;
    }
    int sign = 1;
    if (*nptr == '-') {
        sign = -1;
        nptr++;
    } else if (*nptr == '+') {
        nptr++;
    }
    if (base == 0) {
        if (*nptr == '0') {
            nptr++;
            if (*nptr == 'x' || *nptr == 'X') {
                base = 16;
                nptr++;
            } else {
                base = 8;
            }
        } else {
            base = 10;
        }
    }
    unsigned long val = 0;
    while (*nptr) {
        int digit;
        if (*nptr >= '0' && *nptr <= '9') {
            digit = *nptr - '0';
        } else if (*nptr >= 'a' && *nptr <= 'z') {
            digit = *nptr - 'a' + 10;
        } else if (*nptr >= 'A' && *nptr <= 'Z') {
            digit = *nptr - 'A' + 10;
        } else {
            break;
        }
        if (digit >= base) {
            break;
        }
        val = val * base + digit;
        nptr++;
    }
    if (endptr) {
        *endptr = (char *)nptr;
    }
    if (sign == -1) {
        return -val;
    }
    return val;
}

unsigned long strtoul(const char *nptr, char **endptr, int base) {
    if (endptr) {
        *endptr = (char *)nptr;
    }
    if (base < 0 || base == 1 || base > 36) {
        if (endptr) {
            *endptr = (char *)nptr;
        }
        errno = EINVAL;
        return 0;
    }
    while (*nptr == ' ' || *nptr == '\t' || *nptr == '\n' || *nptr == '\v' || *nptr == '\f' || *nptr == '\r') {
        nptr++;
    }
    unsigned long val = 0;
    if (base == 0) {
        if (*nptr == '0') {
            nptr++;
            if (*nptr == 'x' || *nptr == 'X') {
                base = 16;
                nptr++;
            } else {
                base = 8;
            }
        } else {
            base = 10;
        }
    }
    while (*nptr) {
        int digit;
        if (*nptr >= '0' && *nptr <= '9') {
            digit = *nptr - '0';
        } else if (*nptr >= 'a' && *nptr <= 'z') {
            digit = *nptr - 'a' + 10;
        } else if (*nptr >= 'A' && *nptr <= 'Z') {
            digit = *nptr - 'A' + 10;
        } else {
            break;
        }
        if (digit >= base) {
            break;
        }
        val = val * base + digit;
        nptr++;
    }
    if (endptr) {
        *endptr = (char *)nptr;
    }
    return val;
}

void exit(int status) {
    syscall3(0, status, 0, 0);
    // Should not return
    while (1) {}
}

void abort(void) {
    // We'll just exit with a non-zero status
    exit(1);
}

char *getenv(const char *name) {
    // We don't implement environment variables, so return NULL
    (void)name;
    return NULL;
}

void qsort(void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *)) {
    // Simple bubble sort for demonstration (not efficient)
    if (nmemb < 2 || !compar) return;
    char *arr = base;
    for (size_t i = 0; i < nmemb - 1; i++) {
        for (size_t j = 0; j < nmemb - i - 1; j++) {
            if (compar(arr + j * size, arr + (j + 1) * size) > 0) {
                // Swap
                for (size_t k = 0; k < size; k++) {
                    char tmp = arr[j * size + k];
                    arr[j * size + k] = arr[(j + 1) * size + k];
                    arr[(j + 1) * size + k] = tmp;
                }
            }
        }
    }
}

void *bsearch(const void *key, const void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *)) {
    if (!key || !base || !compar || nmemb == 0) return NULL;
    const char *arr = base;
    size_t left = 0;
    size_t right = nmemb;
    while (left < right) {
        size_t mid = left + (right - left) / 2;
        int cmp = compar(key, arr + mid * size);
        if (cmp == 0) {
            return (void *)(arr + mid * size);
        } else if (cmp < 0) {
            right = mid;
        } else {
            left = mid + 1;
        }
    }
    return NULL;
}

static unsigned int rand_seed = 1;

int rand(void) {
    rand_seed = rand_seed * 1103515245 + 12345;
    return (unsigned int)(rand_seed / 65536) % 32768;
}

void srand(unsigned int seed) {
    rand_seed = seed;
}

char *itoa(int value, char *str, int base) {
    if (base < 2 || base > 36) {
        *str = '\0';
        return str;
    }
    char *ptr = str;
    char *ptr1 = str;
    char tmp_char;
    int tmp_value;
    int sign = 1;
    if (value < 0 && base == 10) {
        sign = -1;
        value = -value;
    }
    do {
        tmp_value = value % base;
        *ptr++ = (tmp_value < 10) ? (tmp_value + '0') : (tmp_value - 10 + 'a');
        value /= base;
    } while (value);
    if (sign < 0) {
        *ptr++ = '-';
    }
    *ptr-- = '\0';
    while (ptr1 < ptr) {
        tmp_char = *ptr;
        *ptr-- = *ptr1;
        *ptr1++ = tmp_char;
    }
    return str;
}

int abs(int j) {
    return (j < 0) ? -j : j;
}