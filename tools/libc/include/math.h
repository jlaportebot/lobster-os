#ifndef _MATH_H
#define _MATH_H

#ifdef __cplusplus
extern "C" {
#endif

// Math functions — implemented in math.c using GCC builtins
// Declare as real functions (not macros) because MicroPython passes them as function pointers

double sqrt(double x);
float sqrtf(float x);
double sin(double x);
double cos(double x);
double tan(double x);
float sinf(float x);
float cosf(float x);
float tanf(float x);
double pow(double x, double y);
float powf(float x, float y);
double floor(double x);
double ceil(double x);
float floorf(float x);
float ceilf(float x);
double fabs(double x);
float fabsf(float x);
double fmod(double x, double y);
float fmodf(float x, float y);
double atan2(double y, double x);
float atan2f(float y, float x);
double atan(double x);
float atanf(float x);
double asin(double x);
float asinf(float x);
double acos(double x);
float acosf(float x);
double exp(double x);
float expf(float x);
double log(double x);
float logf(float x);
double log10(double x);
float log10f(float x);
double modf(double x, double *iptr);
float modff(float x, float *iptr);
float nearbyintf(float x);
float truncf(float x);
float roundf(float x);
float copysignf(float x, float y);
float ldexpf(float x, int exp);
float frexpf(float x, int *exp);
float nanf(const char *s);
double sinh(double x);
float sinhf(float x);
double cosh(double x);
float coshf(float x);
double tanh(double x);
float tanhf(float x);

// Float classification — inline using builtins
static inline int signbit(double x) { return __builtin_signbit(x); }
static inline int isnan(double x) { return __builtin_isnan(x); }
static inline int isinf(double x) { return __builtin_isinf(x); }
static inline int isfinite(double x) { return __builtin_isfinite(x); }
static inline int signbitf(float x) { return __builtin_signbit(x); }
static inline int isnanf(float x) { return __builtin_isnan(x); }
static inline int isinff(float x) { return __builtin_isinf(x); }

// Constants
#define M_PI       3.14159265358979323846
#define M_E        2.71828182845904523536
#define M_LN2      0.69314718055994530942
#define M_LN10     2.30258509299404568402
#define INFINITY   (__builtin_inf())
#define NAN        (__builtin_nan(""))
#define HUGE_VAL   (__builtin_inf())

#ifdef __cplusplus
}
#endif

#endif // _MATH_H
