#ifndef _SYS_STAT_H
#define _SYS_STAT_H

#include <sys/types.h>

struct stat {
    dev_t     st_dev;
    ino_t     st_ino;
    mode_t    st_mode;
    nlink_t   st_nlink;
    uid_t     st_uid;
    gid_t     st_gid;
    dev_t     st_rdev;
    off_t     st_size;
    blksize_t st_blksize;
    blkcnt_t  st_blocks;
    time_t    st_atime;
    time_t    st_mtime;
    time_t    st_ctime;
};

#define S_IFMT   0xF000
#define S_IFDIR  0x4000
#define S_IFCHR  0x2000
#define S_IFREG  0x8000
#define S_IFBLK  0x6000
#define S_IFIFO  0x1000
#define S_IFLNK  0xA000
#define S_IFSOCK 0xC000

#define S_ISREG(m)  ((m & S_IFMT) == S_IFREG)
#define S_ISDIR(m)  ((m & S_IFMT) == S_IFDIR)
#define S_ISCHR(m)  ((m & S_IFMT) == S_IFCHR)

int stat(const char *path, struct stat *buf);
int fstat(int fd, struct stat *buf);

#endif // _SYS_STAT_H
