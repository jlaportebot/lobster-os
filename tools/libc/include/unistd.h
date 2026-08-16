#ifndef _UNISTD_H
#define _UNISTD_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Process ID type */
typedef int pid_t;

/* File mode type */
typedef unsigned int mode_t;

/* Standard file descriptors */
#define STDIN_FILENO  0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

/* Used by libc internally */
#define STDOUT_FD  1
#define STDERR_FD  2

/* File access modes */
#define O_RDONLY  0
#define O_WRONLY  1
#define O_RDWR    2
#define O_CREAT   0100
#define O_TRUNC   01000
#define O_APPEND  02000

ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
int close(int fd);
int open(const char *pathname, int flags, ...);
int fork(void);
int execve(const char *path, char *const argv[], char *const envp[]);
unsigned int sleep(unsigned int seconds);
pid_t getpid(void);
int brk(void *addr);
void *sbrk(intptr_t increment);
void _exit(int status);
int access(const char *pathname, int mode);
int unlink(const char *pathname);

#ifdef __cplusplus
}
#endif

#endif /* _UNISTD_H */
