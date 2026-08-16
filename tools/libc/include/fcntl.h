#ifndef _FCNTL_H
#define _FCNTL_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Open file flags (same as in unistd.h — no redefine) */
#ifndef O_RDONLY
#define O_RDONLY    0
#define O_WRONLY    1
#define O_RDWR      2
#define O_CREAT     0100
#define O_EXCL      0200
#define O_TRUNC     01000
#define O_APPEND    02000
#define O_NONBLOCK  04000
#endif

int open(const char *pathname, int flags, ...);
int creat(const char *pathname, mode_t mode);

#ifdef __cplusplus
}
#endif

#endif /* _FCNTL_H */
