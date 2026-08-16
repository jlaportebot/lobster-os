#ifndef _STDDEF_H
#define _STDDEF_H

#undef NULL
#ifdef __cplusplus
#define NULL nullptr
#else
#define NULL ((void*)0)
#endif

#undef offsetof
#ifdef __compiler_offsetof
#define offsetof(TYPE,MEMBER) __compiler_offsetof(TYPE,MEMBER)
#else
#define offsetof(TYPE, MEMBER) ((size_t) &((TYPE *)0)->MEMBER)
#endif

/* Define size_t, ssize_t, and ptrdiff_t */
typedef unsigned long size_t;
typedef long ssize_t;
typedef long ptrdiff_t;

#endif // _STDDEF_H