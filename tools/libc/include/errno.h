
#ifndef _ERRNO_H
#define _ERRNO_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

extern int errno;

#define EPERM   1
#define ENOENT  2
#define ESRCH   3
#define EINTR   4
#define EIO     5
#define ENXIO   6
#define E2BIG   7
#define ENOMEM  8
#define ENOSYS  38  /* Function not implemented */
#define EACCES  13
#define EFAULT  14
#define EBUSY   16
#define EEXIST  17
#define EXDEV   18
#define ENODEV  19
#define ENOTDIR 20
#define EISDIR  21
#define EINVAL  22
#define ENFILE  23
#define EMFILE  24
#define ENOTTY  25
#define EFBIG   27
#define ENOSPC  28
#define ESPIPE  29
#define EROFS   30
#define EMLINK  31
#define EPIPE   32
#define EDOM    33
#define ERANGE  34
#define EAGAIN  35
#define EWOULDBLOCK EAGAIN
#define EINPROGRESS 36
#define EALREADY 37
#define ENOTSOCK 38
#define EDESTADDRREQ 39
#define EMSGSIZE 40
#define EPROTOTYPE 41
#define ENOPROTOOPT 42
#define EPROTONOSUPPORT 43
#define ESOCKTNOSUPPORT 44
#define EOPNOTSUPP 45
#define ENOSR   46
#define ENODATA 46
#define ETIME   47
#define ENOSR   46
#define ENONET  49
#define ENOPKG  50
#define EREMOTE 51
#define ENOLINK 52
#define EADV    53
#define ESRMNT  54
#define ECOMM   55
#define EPROTO  56
#define EMULTIHOP 57
#define EBADMSG 58
#define ENOTNAM 59
#define ENAVAIL 60
#define EISNAM  61
#define EREMOTEIO 62
#define EDQUOT  63
#define ENOMEDIUM 64
#define EMEDIUMTYPE 65
#define ECANCELED 66
#define ENOKEY  67
#define EKEYEXPIRED 68
#define EKEYREVOKED 69
#define EKEYREJECTED 70
#define EOWNERDEAD 71
#define ENOTRECOVERABLE 72
#define ERFKILL 73
#define EHWPOISON 133

#ifdef __cplusplus
}
#endif

#endif // _ERRNO_H
