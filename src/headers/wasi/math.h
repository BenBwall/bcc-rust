/* bcc's bundled C library for wasm32-wasip1: <math.h>. */
#ifndef _BCC_WASI_MATH_H
#define _BCC_WASI_MATH_H
#define HUGE_VAL __builtin_huge_val()
#define HUGE_VALF __builtin_huge_valf()
#define HUGE_VALL __builtin_huge_vall()
#define INFINITY __builtin_inff()
#define NAN __builtin_nanf("")
#define M_PI 3.14159265358979323846
#define M_E 2.7182818284590452354
#define FP_NAN 0
#define FP_INFINITE 1
#define FP_ZERO 2
#define FP_SUBNORMAL 3
#define FP_NORMAL 4
#define isnan(x) __builtin_isnan(x)
#define isinf(x) __builtin_isinf(x)
#define isfinite(x) __builtin_isfinite(x)
#define isnormal(x) __builtin_isnormal(x)
#define signbit(x) __builtin_signbit(x)
#define fpclassify(x) __builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, x)
/* WebAssembly raises no floating exceptions, so the quiet comparisons are
   the ordinary ones. islessgreater and isunordered evaluate an argument
   more than once. */
#define isgreater(x, y) ((x) > (y))
#define isgreaterequal(x, y) ((x) >= (y))
#define isless(x, y) ((x) < (y))
#define islessequal(x, y) ((x) <= (y))
#define islessgreater(x, y) ((x) < (y) || (x) > (y))
#define isunordered(x, y) ((x) != (x) || (y) != (y))
typedef float float_t;
typedef double double_t;

double sqrt(double x);
float sqrtf(float x);
double fabs(double x);
float fabsf(float x);
long double fabsl(long double x);
double floor(double x);
float floorf(float x);
double ceil(double x);
float ceilf(float x);
double trunc(double x);
float truncf(float x);
double round(double x);
float roundf(float x);
long lround(double x);
long long llround(double x);
double rint(double x);
float rintf(float x);
double nearbyint(double x);
float nearbyintf(float x);
long lrint(double x);
double copysign(double x, double y);
float copysignf(float x, float y);
double fmod(double x, double y);
float fmodf(float x, float y);
long double fmodl(long double x, long double y);
double fmin(double x, double y);
double fmax(double x, double y);
float fminf(float x, float y);
float fmaxf(float x, float y);
double fdim(double x, double y);
double nextafter(double x, double y);
float nextafterf(float x, float y);
long double nextafterl(long double x, long double y);
double nan(const char *tagp);
float nanf(const char *tagp);
long double nanl(const char *tagp);
double frexp(double value, int *exp);
double ldexp(double x, int exp);
float ldexpf(float x, int exp);
double scalbn(double x, int n);
double modf(double value, double *iptr);
double hypot(double x, double y);
double exp(double x);
double exp2(double x);
double expm1(double x);
double log(double x);
double log2(double x);
double log10(double x);
double log1p(double x);
double pow(double x, double y);
double cbrt(double x);
double sin(double x);
double cos(double x);
double tan(double x);
double asin(double x);
double acos(double x);
double atan(double x);
double atan2(double y, double x);
double sinh(double x);
double cosh(double x);
double tanh(double x);
float expf(float x);
float logf(float x);
float powf(float x, float y);
float sinf(float x);
float cosf(float x);
float tanf(float x);
float atan2f(float y, float x);
#endif
