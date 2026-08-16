#ifndef _STDIO_H
#define _STDIO_H

#include <stddef.h>
#include <stdarg.h>

/* Opaque type for FILE */
typedef struct _IO_FILE {
    int dummy;
} FILE;

/* Standard I/O streams */
extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;

/* End-Of-Feof */
#define EOF (-1)

#ifdef __cplusplus
extern "C" {
#endif

int printf(const char *format, ...);
int fprintf(FILE *stream, const char *format, ...);
int sprintf(char *str, const char *format, ...);
int snprintf(char *str, size_t size, const char *format, ...);
int vprintf(const char *format, va_list arg);
int vsprintf(char *str, const char *format, va_list arg);
int vsnprintf(char *str, size_t size, const char *format, va_list arg);
int putchar(int c);
int puts(const char *s);
int fputc(int c, FILE *stream);
int fputs(const char *s, FILE *stream);
char *fgets(char *s, int n, FILE *stream);
int fflush(FILE *stream);

#ifdef __cplusplus
}
#endif

#endif // _STDIO_H