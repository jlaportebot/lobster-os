#include <unistd.h>
#include <fcntl.h>
#include "syscall.h"
#include "errno.h"

int open(const char *pathname, int flags, ...) {
    long ret = syscall3(3, (long)pathname, flags, 0);
    if (ret < 0) {
        errno = -ret;
        return -1;
    }
    return ret;
}

int creat(const char *pathname, mode_t mode) {
    (void)mode;
    return open(pathname, O_CREAT | O_WRONLY | O_TRUNC);
}
