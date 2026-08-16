// LobsterOS libc math function implementations
// Using GCC builtins to satisfy function-pointer references
#include <math.h>

// These functions need to exist as real symbols because
// MicroPython passes them as function pointers (not calls)

double sqrt_val(double x) { return __builtin_sqrt(x); }
float sqrtf_val(float x) { return __builtin_sqrtf(x); }
double sin(double x) { return __builtin_sin(x); }
double cos(double x) { return __builtin_cos(x); }
double tan(double x) { return __builtin_tan(x); }
float sinf(float x) { return __builtin_sinf(x); }
float cosf(float x) { return __builtin_cosf(x); }
float tanf(float x) { return __builtin_tanf(x); }
double pow(double x, double y) { return __builtin_pow(x, y); }
float powf(float x, float y) { return __builtin_powf(x, y); }
double floor(double x) { return __builtin_floor(x); }
double ceil(double x) { return __builtin_ceil(x); }
float floorf(float x) { return __builtin_floorf(x); }
float ceilf(float x) { return __builtin_ceilf(x); }
double fabs(double x) { return __builtin_fabs(x); }
float fabsf(float x) { return __builtin_fabsf(x); }
double fmod(double x, double y) { return __builtin_fmod(x, y); }
float fmodf(float x, float y) { return __builtin_fmodf(x, y); }
double atan2(double y, double x) { return __builtin_atan2(y, x); }
float atan2f(float y, float x) { return __builtin_atan2f(y, x); }
double atan(double x) { return __builtin_atan(x); }
float atanf(float x) { return __builtin_atanf(x); }
double asin(double x) { return __builtin_asin(x); }
float asinf(float x) { return __builtin_asinf(x); }
double acos(double x) { return __builtin_acos(x); }
float acosf(float x) { return __builtin_acosf(x); }
double exp(double x) { return __builtin_exp(x); }
float expf(float x) { return __builtin_expf(x); }
double log(double x) { return __builtin_log(x); }
float logf(float x) { return __builtin_logf(x); }
double log10(double x) { return __builtin_log10(x); }
float log10f(float x) { return __builtin_log10f(x); }
double modf(double x, double *iptr) { return __builtin_modf(x, iptr); }
float modff(float x, float *iptr) { return __builtin_modff(x, iptr); }
float nearbyintf(float x) { return __builtin_nearbyintf(x); }
float truncf(float x) { return __builtin_truncf(x); }
float roundf(float x) { return __builtin_roundf(x); }
double sqrt(double x) { return __builtin_sqrt(x); }
float sqrtf(float x) { return __builtin_sqrtf(x); }
float copysignf(float x, float y) { return __builtin_copysignf(x, y); }
float ldexpf(float x, int exp) { return __builtin_ldexpf(x, exp); }
float frexpf(float x, int *exp) { return __builtin_frexpf(x, exp); }
float nanf(const char *s) { (void)s; return __builtin_nanf(""); }
double sinh(double x) { return __builtin_sinh(x); }
float sinhf(float x) { return __builtin_sinhf(x); }
double cosh(double x) { return __builtin_cosh(x); }
float coshf(float x) { return __builtin_coshf(x); }
double tanh(double x) { return __builtin_tanh(x); }
float tanhf(float x) { return __builtin_tanhf(x); }
