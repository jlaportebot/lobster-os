#ifndef _STDINT_H
#define _STDINT_H

/* size_t, ptrdiff_t, ssize_t come from stddef.h */
#ifndef _SIZE_T_DEFINED
typedef unsigned long  size_t;
#define _SIZE_T_DEFINED
#endif

#ifndef _PTRDIFF_T_DEFINED
typedef long           ptrdiff_t;
#define _PTRDIFF_T_DEFINED
#endif

#ifndef _SSIZE_T_DEFINED
typedef long           ssize_t;
#define _SSIZE_T_DEFINED
#endif

/* Exact-width integer types */
typedef signed char         int8_t;
typedef short               int16_t;
typedef int                 int32_t;
typedef long long           int64_t;
typedef unsigned char       uint8_t;
typedef unsigned short      uint16_t;
typedef unsigned int        uint32_t;
typedef unsigned long long  uint64_t;

/* Pointer-holding integer types */
typedef long           intptr_t;
typedef unsigned long uintptr_t;

/* Pointer integer limits */
#define INTPTR_MIN (-9223372036854775807L - 1L)
#define INTPTR_MAX 9223372036854775807L
#define UINTPTR_MAX (18446744073709551615UL)

/* Size limits */
#define SIZE_MAX (18446744073709551615UL)

/* NULL */
#ifndef NULL
#define NULL ((void *)0)
#endif

/* Limits */
#define INT8_MIN   (-128)
#define INT16_MIN  (-32768)
#define INT32_MIN  (-2147483647 - 1)
#define INT64_MIN  (-9223372036854775807LL - 1)
#define INT8_MAX   (127)
#define INT16_MAX  (32767)
#define INT32_MAX  (2147483647)
#define INT64_MAX  (9223372036854775807LL)
#define UINT8_MAX  (0xff)
#define UINT16_MAX (0xffff)
#define UINT32_MAX (0xffffffffU)
#define UINT64_MAX (0xffffffffffffffffULL)

#endif /* _STDINT_H */
