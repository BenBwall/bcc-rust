/* <math.h> for wasm32-wasip1. sqrt, fabs, floor, ceil, trunc, rint,
   nearbyint and copysign are WebAssembly instructions, which the back end
   emits for calls to them; the definitions here serve calls through
   pointers. sin, cos and tan are in trig.c (fdlibm's, with Payne-Hanek
   reduction). The other transcendental functions reduce their argument
   and sum a series: accurate to about an ulp, but not correctly rounded.
   C99: §7.12, pp. 211-242. */
#include <errno.h>
#include <math.h>
#include <stdint.h>

static uint64_t bits_of(double x) {
    union {
        double d;
        uint64_t u;
    } v = {x};
    return v.u;
}

static double from_bits(uint64_t u) {
    union {
        uint64_t u;
        double d;
    } v = {u};
    return v.d;
}

double sqrt(double x) { return __builtin_sqrt(x); }
float sqrtf(float x) { return __builtin_sqrtf(x); }
double fabs(double x) { return __builtin_fabs(x); }
float fabsf(float x) { return __builtin_fabsf(x); }
long double fabsl(long double x) {
    union {
        long double ld;
        struct {
            uint64_t lo, hi;
        } w;
    } v;
    v.ld = x;
    v.w.hi &= ~(1ull << 63);
    return v.ld;
}
double floor(double x) { return __builtin_floor(x); }
float floorf(float x) { return __builtin_floorf(x); }
double ceil(double x) { return __builtin_ceil(x); }
float ceilf(float x) { return __builtin_ceilf(x); }
double trunc(double x) { return __builtin_trunc(x); }
float truncf(float x) { return __builtin_truncf(x); }
double rint(double x) { return __builtin_rint(x); }
float rintf(float x) { return __builtin_rintf(x); }
double nearbyint(double x) { return __builtin_nearbyint(x); }
float nearbyintf(float x) { return __builtin_nearbyintf(x); }
double copysign(double x, double y) { return __builtin_copysign(x, y); }
float copysignf(float x, float y) { return __builtin_copysignf(x, y); }

double round(double x) {
    double t = trunc(x);
    if (fabs(x - t) >= 0.5)
        t += copysign(1.0, x);
    return t;
}

float roundf(float x) { return (float)round(x); }
long lround(double x) { return (long)round(x); }
long long llround(double x) { return (long long)round(x); }
long lrint(double x) { return (long)rint(x); }

double fmin(double x, double y) {
    if (x != x)
        return y;
    if (y != y)
        return x;
    return x < y || (x == y && signbit(x)) ? x : y;
}

double fmax(double x, double y) {
    if (x != x)
        return y;
    if (y != y)
        return x;
    return x > y || (x == y && !signbit(x)) ? x : y;
}

float fminf(float x, float y) { return (float)fmin(x, y); }
float fmaxf(float x, float y) { return (float)fmax(x, y); }
double fdim(double x, double y) { return x > y ? x - y : 0.0; }

/* fmod is exact: the remainder of the aligned significands by repeated
   subtraction, one bit at a time. */
double fmod(double x, double y) {
    uint64_t ux = bits_of(x), uy = bits_of(y);
    int ex = (int)(ux >> 52 & 0x7FF), ey = (int)(uy >> 52 & 0x7FF);
    uint64_t sign = ux & (1ull << 63);
    if ((uy << 1) == 0 || ex == 0x7FF || y != y)
        return (x * y) / (x * y);
    if ((ux << 1) <= (uy << 1)) {
        if ((ux << 1) == (uy << 1))
            return 0 * x;
        return x;
    }
    uint64_t mx, my;
    if (ex == 0) {
        for (uint64_t i = ux << 12; i >> 63 == 0; ex--, i <<= 1)
            ;
        mx = ux << (1 - ex);
    } else {
        mx = (ux & ((1ull << 52) - 1)) | (1ull << 52);
    }
    if (ey == 0) {
        for (uint64_t i = uy << 12; i >> 63 == 0; ey--, i <<= 1)
            ;
        my = uy << (1 - ey);
    } else {
        my = (uy & ((1ull << 52) - 1)) | (1ull << 52);
    }
    for (; ex > ey; ex--) {
        uint64_t i = mx - my;
        if (i >> 63 == 0) {
            if (i == 0)
                return 0 * x;
            mx = i;
        }
        mx <<= 1;
    }
    uint64_t i = mx - my;
    if (i >> 63 == 0) {
        if (i == 0)
            return 0 * x;
        mx = i;
    }
    for (; mx >> 52 == 0; mx <<= 1, ex--)
        ;
    if (ex > 0) {
        mx -= 1ull << 52;
        mx |= (uint64_t)ex << 52;
    } else {
        mx >>= 1 - ex;
    }
    return from_bits(mx | sign);
}

float fmodf(float x, float y) { return (float)fmod(x, y); }

double frexp(double x, int *e) {
    uint64_t u = bits_of(x);
    int ee = (int)(u >> 52 & 0x7FF);
    if (ee == 0) {
        if (x == 0) {
            *e = 0;
            return x;
        }
        x = frexp(x * 0x1p64, e);
        *e -= 64;
        return x;
    }
    if (ee == 0x7FF) {
        *e = 0;
        return x;
    }
    *e = ee - 1022;
    return from_bits((u & 0x800FFFFFFFFFFFFFull) | 0x3FE0000000000000ull);
}

double ldexp(double x, int n) {
    if (n > 1023) {
        x *= 0x1p1023;
        n -= 1023;
        if (n > 1023) {
            x *= 0x1p1023;
            n -= 1023;
            if (n > 1023)
                n = 1023;
        }
    } else if (n < -1022) {
        x *= 0x1p-1022 * 0x1p53;
        n += 1022 - 53;
        if (n < -1022) {
            x *= 0x1p-1022 * 0x1p53;
            n += 1022 - 53;
            if (n < -1022)
                n = -1022;
        }
    }
    return x * from_bits((uint64_t)(0x3FF + n) << 52);
}

float ldexpf(float x, int n) { return (float)ldexp(x, n); }
double scalbn(double x, int n) { return ldexp(x, n); }

double modf(double x, double *ip) {
    double t = trunc(x);
    *ip = t;
    if (x == t)
        return copysign(0.0, x);
    return x - t;
}

double hypot(double x, double y) {
    x = fabs(x);
    y = fabs(y);
    if (x == __builtin_inf() || y == __builtin_inf())
        return __builtin_inf();
    if (x < y) {
        double t = x;
        x = y;
        y = t;
    }
    if (x == 0)
        return 0;
    double r = y / x;
    return x * sqrt(1 + r * r);
}

static const double LN2_HI = 6.93147180369123816490e-01, LN2_LO = 1.90821492927058770002e-10;

double exp(double x) {
    if (x != x)
        return x;
    if (x > 709.782712893384)
        return __builtin_inf();
    if (x < -745.1332191019411)
        return 0;
    double k = nearbyint(x / 0.6931471805599453);
    double r = (x - k * LN2_HI) - k * LN2_LO;
    double term = 1, sum = 1;
    for (int i = 1; i < 30; i++) {
        term *= r / i;
        sum += term;
        if (fabs(term) < 1e-17 * fabs(sum))
            break;
    }
    return ldexp(sum, (int)k);
}

double expm1(double x) {
    if (fabs(x) < 1e-5)
        return x + x * x / 2 + x * x * x / 6;
    return exp(x) - 1;
}

double exp2(double x) { return exp(x * 0.69314718055994530942); }

double log(double x) {
    if (x != x || x < 0)
        return __builtin_nan("");
    if (x == 0)
        return -__builtin_inf();
    if (x == __builtin_inf())
        return x;
    int e;
    double m = frexp(x, &e);
    if (m < 0.70710678118654752440) {
        m *= 2;
        e--;
    }
    /* log(m) = 2 atanh((m - 1) / (m + 1)) */
    double s = (m - 1) / (m + 1), s2 = s * s, term = s, sum = 0;
    for (int i = 1; i < 60; i += 2) {
        sum += term / i;
        term *= s2;
        if (fabs(term) < 1e-18)
            break;
    }
    return 2 * sum + e * LN2_HI + e * LN2_LO;
}

double log1p(double x) {
    if (fabs(x) < 1e-5)
        return x - x * x / 2 + x * x * x / 3;
    return log(1 + x);
}

double log2(double x) { return log(x) / 0.69314718055994530942; }
double log10(double x) { return log(x) / 2.30258509299404568402; }

double pow(double x, double y) {
    if (y == 0)
        return 1;
    if (x == 1)
        return 1;
    if (x != x || y != y)
        return x + y;
    double iy = trunc(y);
    int integer = iy == y;
    if (x == 0) {
        int odd = integer && fmod(fabs(y), 2) == 1;
        if (y < 0)
            return odd ? copysign(__builtin_inf(), x) : __builtin_inf();
        return odd ? x : 0;
    }
    if (integer && fabs(y) <= 1024) {
        double base = y < 0 ? 1 / x : x, result = 1;
        unsigned long long n = (unsigned long long)fabs(y);
        while (n) {
            if (n & 1)
                result *= base;
            base *= base;
            n >>= 1;
        }
        return result;
    }
    if (x < 0) {
        if (!integer)
            return __builtin_nan("");
        double r = exp(y * log(-x));
        return fmod(fabs(y), 2) == 1 ? -r : r;
    }
    return exp(y * log(x));
}

double cbrt(double x) {
    if (x == 0 || x != x || fabs(x) == __builtin_inf())
        return x;
    double r = copysign(exp(log(fabs(x)) / 3), x);
    r = r - (r * r * r - x) / (3 * r * r);
    return r;
}

double atan(double x) {
    if (x != x)
        return x;
    double a = fabs(x);
    int invert = a > 1;
    if (invert)
        a = 1 / a;
    /* atan(a) = 2 atan(a / (1 + sqrt(1 + a^2))), applied twice, then the
       series. */
    a = a / (1 + sqrt(1 + a * a));
    a = a / (1 + sqrt(1 + a * a));
    double a2 = a * a, term = a, sum = 0;
    for (int i = 1; i < 60; i += 2) {
        sum += term / i;
        term *= -a2;
        if (fabs(term) < 1e-18)
            break;
    }
    sum *= 4;
    if (invert)
        sum = 1.57079632679489661923 - sum;
    return copysign(sum, x);
}

double atan2(double y, double x) {
    if (x != x || y != y)
        return x + y;
    if (x == 0 && y == 0)
        return signbit(x) ? copysign(3.14159265358979323846, y) : copysign(0.0, y);
    if (x == 0)
        return copysign(1.57079632679489661923, y);
    double a = atan(y / x);
    if (x > 0)
        return a;
    return y >= 0 && !signbit(y) ? a + 3.14159265358979323846 : a - 3.14159265358979323846;
}

double asin(double x) {
    if (fabs(x) > 1)
        return __builtin_nan("");
    return atan2(x, sqrt((1 - x) * (1 + x)));
}

double acos(double x) {
    if (fabs(x) > 1)
        return __builtin_nan("");
    return atan2(sqrt((1 - x) * (1 + x)), x);
}

double sinh(double x) {
    if (fabs(x) < 1e-5)
        return x;
    double e = exp(fabs(x));
    return copysign((e - 1 / e) / 2, x);
}

double cosh(double x) {
    double e = exp(fabs(x));
    return (e + 1 / e) / 2;
}

double tanh(double x) {
    if (fabs(x) > 20)
        return copysign(1.0, x);
    if (fabs(x) < 1e-5)
        return x;
    double e = exp(2 * x);
    return (e - 1) / (e + 1);
}

float expf(float x) { return (float)exp(x); }
float logf(float x) { return (float)log(x); }
float powf(float x, float y) { return (float)pow(x, y); }
float atan2f(float y, float x) { return (float)atan2(y, x); }
