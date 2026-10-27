/* Binary128 (__float128) arithmetic, comparisons, conversions, calls,
   variadic arguments, folded static initializers and glibc's _Float128
   functions. Values print as their bits or through strfromf128, so the
   output is the same wherever the program is built against libgcc's
   soft-float routines and glibc. Every invalid operation is computed at
   run time from volatile operands: compilers fold a NaN's sign
   differently. */
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

typedef __float128 quad;
/* GCC spells the complex type only through a mode attribute. */
typedef __typeof__(0.0Qi) complex_quad;

quad strtof128(const char *, char **);
int strfromf128(char *, size_t, const char *, quad);
quad sqrtf128(quad);
quad fabsf128(quad);
quad copysignf128(quad, quad);
quad floorf128(quad);
quad ceilf128(quad);
quad truncf128(quad);
quad roundf128(quad);
quad rintf128(quad);
quad nearbyintf128(quad);
quad fmodf128(quad, quad);
quad fmaxf128(quad, quad);
quad fminf128(quad, quad);
quad fdimf128(quad, quad);
quad ldexpf128(quad, int);
quad scalbnf128(quad, int);
quad frexpf128(quad, int *);
quad nanf128(const char *);

static void bits(const char *label, quad value) {
    unsigned long long words[2];
    memcpy(words, &value, sizeof words);
    printf("%s %016llx%016llx\n", label, words[1], words[0]);
}

static void text(const char *label, const char *format, quad value) {
    char buffer[200];
    int length = strfromf128(buffer, sizeof buffer, format, value);
    printf("%s %s %d\n", label, buffer, length);
}

static quad from_bits(unsigned long long high, unsigned long long low) {
    unsigned long long words[2] = {low, high};
    quad value;
    memcpy(&value, words, sizeof value);
    return value;
}

struct pair {
    quad value;
    long tag;
};

struct mixed {
    double d;
    quad q;
};

static struct pair make_pair(quad value, long tag) {
    struct pair result = {value, tag};
    return result;
}

static quad pair_sum(struct pair p, struct mixed m) {
    return p.value + m.q + m.d + p.tag;
}

static quad many(quad a, quad b, quad c, quad d, quad e, quad f, quad g, quad h, quad i,
                 quad j) {
    return a - b + c - d + e - f + g - h + i - j * 2;
}

static quad sum(int count, ...) {
    va_list list;
    quad total = 0;
    va_start(list, count);
    for (int i = 0; i < count; i++) {
        if (i % 3 == 1) {
            total += va_arg(list, double);
        } else if (i % 3 == 2) {
            total += va_arg(list, struct pair).value;
        } else {
            total += va_arg(list, quad);
        }
    }
    va_end(list);
    return total;
}

static const quad folded[] = {
    1.0Q / 3,         0.1Q + 0.2Q,       -2.5Q * 4,      1e4932Q * 1e-10Q,
    0x1p-16494Q,      (quad)1 / 7 * 7,   (quad)(1.0 / 3), (quad)0.1f,
    -(quad)3 - 0.5Q,  (quad)123456789LL, 1.5Q > 1 ? 2.0Q : 3.0Q,
};
static const double narrowed = 1.0Q / 3;
static const float narrowed_float = 1.0Q / 3;
static const int truncated = (int)(-7.75Q);
static const unsigned long long wide = (unsigned long long)1.8446744073709551615e19Q;
static const _Bool truth = 0.5Q != 0;
static const complex_quad complex_folded = 1.5Q + 2.0Qi;
/* A converted operand's integer value does not survive the conversion:
   2^113 + 1 rounds to 2^113 in binary128, as 2^62 + 1 does in double. */
#define ROUNDS ((unsigned __int128)1 << 113)
static const int converted_equal = (1 ? ROUNDS + 1 : 0.0Q) == (1 ? ROUNDS : 0.0Q);
static const int converted_double = (1 ? (1LL << 62) + 1 : 0.0) == (1 ? 1LL << 62 : 0.0);
static const unsigned __int128 converted_wide = 1 ? ROUNDS + 1 : 0.0Q;
static const long long converted_long = 1 ? (1LL << 62) + 1 : 0.0;
static const int converted_branch = (1 ? ROUNDS + 1 : 0.0Q) == ROUNDS ? 1 : 2;
/* Unsigned 128-bit destinations hold 2^127 and up. */
static const unsigned __int128 unsigned_top = 0x1p127Q;
static const unsigned __int128 unsigned_top_double = 0x1.8p127;
static const quad wide_nan = __builtin_nanf128("0x10000000000000000");
static const quad wide_signaling = __builtin_nansf128("0xffffffffffffffffffffffffffffffffff");

int main(void) {
    volatile quad values[] = {
        0,
        -0.0Q,
        1,
        -1.5Q,
        0.1Q,
        3,
        1e300Q,
        1e-4940Q,
        1.18973149535723176508575932662800702e4932Q,
        from_bits(0x7fff000000000000ull, 0),
        from_bits(0x7fff800000000000ull, 5),
        from_bits(0xffff800000000000ull, 9),
        from_bits(0x7fff000000000000ull, 3),
    };
    enum { COUNT = sizeof values / sizeof values[0] };
    char label[64];
    for (int i = 0; i < COUNT; i++) {
        for (int j = 0; j < COUNT; j++) {
            quad a = values[i], b = values[j];
            snprintf(label, sizeof label, "%d+%d", i, j);
            bits(label, a + b);
            snprintf(label, sizeof label, "%d-%d", i, j);
            bits(label, a - b);
            snprintf(label, sizeof label, "%d*%d", i, j);
            bits(label, a * b);
            snprintf(label, sizeof label, "%d/%d", i, j);
            bits(label, a / b);
            printf("%d?%d %d%d%d%d%d%d\n", i, j, a < b, a <= b, a > b, a >= b, a == b, a != b);
        }
        snprintf(label, sizeof label, "neg%d", i);
        bits(label, -values[i]);
        printf("truth%d %d\n", i, values[i] ? 1 : 0);
    }

    volatile quad in_range[] = {0, -0.75Q, 1.5Q, -123456.875Q, 2147483647.9Q, 1e18Q};
    for (int i = 0; i < 6; i++) {
        quad v = in_range[i];
        printf("int%d %d %lld %llx\n", i, v < 2147483648.0Q ? (int)v : 0, (long long)v,
               (unsigned long long)(__int128)v);
        printf("dbl%d %a %a\n", i, (double)v, (double)(float)v);
        long double extended = (long double)v;
        unsigned char raw[16] = {0};
        memcpy(raw, &extended, 10);
        printf("x87%d", i);
        for (int k = 9; k >= 0; k--) {
            printf("%02x", raw[k]);
        }
        printf("\n");
        if (v >= 0) {
            printf("uns%d %u %llu\n", i, v < 4294967296.0Q ? (unsigned)v : 0u,
                   (unsigned long long)v);
        }
    }
    volatile int small = -2147483647 - 1;
    volatile unsigned long long large = 18446744073709551615ull;
    volatile __int128 huge = (__int128)1 << 120;
    volatile unsigned __int128 unsigned_huge = ~(unsigned __int128)0;
    volatile double d = 0.1;
    volatile float f = -3.25f;
    volatile long double ld = 1.0L / 3;
    bits("from_int", small);
    bits("from_ull", large);
    bits("from_i128", huge);
    bits("from_u128", unsigned_huge);
    bits("from_double", d);
    bits("from_float", f);
    bits("from_x87", ld);
    unsigned long long nan_bits = 0x7ff0000000000003ull;
    double signaling;
    memcpy(&signaling, &nan_bits, sizeof signaling);
    volatile double signaling_double = signaling;
    bits("from_snan", signaling_double);
    printf("to_double_nan %a\n", (double)values[11]);

    for (int i = 0; i < (int)(sizeof folded / sizeof folded[0]); i++) {
        snprintf(label, sizeof label, "folded%d", i);
        bits(label, folded[i]);
    }
    printf("narrowed %a %a %d %llu %d\n", narrowed, narrowed_float, truncated, wide, truth);
    bits("complex_real", __real__ complex_folded);
    bits("complex_imag", __imag__ complex_folded);

    volatile complex_quad z = 1.5Q - 2.0Qi;
    complex_quad w = z + complex_folded;
    bits("csum_real", __real__ w);
    bits("csum_imag", __imag__ w);
    w = z * 2.0Q;
    bits("cscale_imag", __imag__ w);

    struct pair p = make_pair(values[4], 7);
    struct mixed m = {0.5, values[5]};
    bits("pair", pair_sum(p, m));
    bits("many", many(values[0], values[1], values[2], values[3], values[4], values[5],
                      values[6], values[7], values[2], values[3]));
    bits("sum", sum(5, values[4], 2.5, p, values[3], 0.25));

    volatile quad two = 2, x = values[4], y = values[3];
    text("sqrt", "%.36g", sqrtf128(two));
    bits("sqrt_neg", sqrtf128(y));
    bits("fabs", fabsf128(y));
    bits("copysign", copysignf128(two, y));
    text("floor", "%g", floorf128(y));
    text("ceil", "%g", ceilf128(y));
    text("trunc", "%g", truncf128(y));
    text("round", "%g", roundf128(y));
    text("round2", "%g", roundf128(2.5Q + x * 0));
    text("rint", "%g", rintf128(2.5Q + x * 0));
    text("nearbyint", "%g", nearbyintf128(-3.5Q + x * 0));
    text("fmod", "%.36g", fmodf128(values[6], x));
    text("fmax", "%g", fmaxf128(values[10], two));
    text("fmin", "%g", fminf128(two, y));
    text("fdim", "%g", fdimf128(two, y));
    text("ldexp", "%a", ldexpf128(x, -16400));
    text("scalbn", "%a", scalbnf128(x, 100));
    int exponent;
    quad fraction = frexpf128(values[6], &exponent);
    text("frexp", "%a", fraction);
    printf("frexp_exponent %d\n", exponent);
    bits("nan", nanf128("0x123"));

    static const char *const inputs[] = {
        "  -12.375e2xyz", "0x1.8p-16400", "1e5000", "-nan(42)", "infinityx", "  +.5e",
        "0x", "junk", "1e-5000", "3.141592653589793238462643383279502884197",
    };
    for (int i = 0; i < 10; i++) {
        char *end;
        quad value = strtof128(inputs[i], &end);
        snprintf(label, sizeof label, "strto%d", i);
        bits(label, value);
        printf("end%d %d\n", i, (int)(end - inputs[i]));
    }
    static const char *const formats[] = {"%a", "%A", "%.3a", "%e", "%.40e", "%f", "%.0f",
                                          "%g", "%.36g", "%G", "%E", "%F"};
    for (int i = 0; i < 12; i++) {
        snprintf(label, sizeof label, "fmt%d", i);
        text(label, formats[i], folded[0]);
        text(label, formats[i], values[3]);
        text(label, formats[i], values[7]);
        text(label, formats[i], values[9]);
        text(label, formats[i], values[11]);
    }
    printf("converted %d %d %llx %lld %d\n", converted_equal, converted_double,
           (unsigned long long)(converted_wide >> 64), converted_long, converted_branch);
    printf("unsigned %llx %llx\n", (unsigned long long)(unsigned_top >> 64),
           (unsigned long long)(unsigned_top_double >> 64));
    bits("wide_nan", wide_nan);
    bits("wide_signaling", wide_signaling);
    bits("wide_nan_dec", __builtin_nanf128("18446744073709551617"));
    volatile quad quad_signaling = __builtin_nansf128("3");
    bits("fmin_snan", fminf128(quad_signaling, 1.0Q));
    bits("fmin_snan2", fminf128(1.0Q, quad_signaling));
    bits("fmax_snan", fmaxf128(quad_signaling, 1.0Q));
    bits("fmax_qnan", fmaxf128(values[10], 1.0Q));
    bits("fmin_qnan", fminf128(1.0Q, values[11]));
    char small_buffer[4];
    printf("short %d ", strfromf128(small_buffer, sizeof small_buffer, "%f", values[5]));
    printf("%s\n", small_buffer);
    return 0;
}
