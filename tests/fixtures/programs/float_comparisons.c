/* The comparison macros of C99 §7.12.14 as GCC's and Clang's builtins, which
 * glibc's <math.h> uses under both compilers' identities, with
 * `__builtin_issignaling` and `__builtin_roundeven`. Operands of different
 * types compare in their common real type; a NaN is unordered; constant
 * operands fold. `__builtin_iseqsig` is GCC 14's, so older references print
 * its values directly. */
#include <stdio.h>

#define COMPARE(x, y)                                                          \
    printf("%d%d%d%d%d%d ", __builtin_isgreater(x, y),                         \
           __builtin_isgreaterequal(x, y), __builtin_isless(x, y),             \
           __builtin_islessequal(x, y), __builtin_islessgreater(x, y),         \
           __builtin_isunordered(x, y))

_Static_assert(__builtin_isless(1.0, 2.0) && !__builtin_isless(2.0f, 1),
               "comparisons fold");
_Static_assert(__builtin_isunordered(__builtin_nan(""), 1.0), "NaN unordered");
_Static_assert(!__builtin_islessgreater(__builtin_nan(""), 1.0),
               "NaN is not less or greater");
static int folded[] = {__builtin_isgreaterequal(2.0L, 2), __builtin_islessequal(3, 2.5)};

/* Signaling constants fold from their own value, which a conversion to the
   operand type would quiet. */
_Static_assert(__builtin_issignaling(__builtin_nansf("")), "float");
_Static_assert(__builtin_issignaling(__builtin_nans("")), "double");
_Static_assert(__builtin_issignaling(-__builtin_nans("")), "negated");
_Static_assert(__builtin_issignaling(__builtin_nansl("")), "long double");
_Static_assert(!__builtin_issignaling(__builtin_nanl("")), "quiet");
static int signaling[] = {__builtin_issignaling(__builtin_nansf128("")),
                          __builtin_issignaling(__builtin_inff128())};

/* An x87 long double from its significand and sign-exponent words. The
   pseudo-NaN, pseudo-infinity and unnormal encodings are signaling for GCC
   and not for Clang. */
static long double x87(unsigned long long significand, unsigned short exponent) {
    long double value = 0;
    __builtin_memcpy(&value, &significand, 8);
    __builtin_memcpy((char *)&value + 8, &exponent, 2);
    return value;
}

static float signaling_float(void) {
    union { unsigned u; float f; } bits = {0x7fa00000u};
    return bits.f;
}

static double signaling_double(void) {
    union { unsigned long long u; double d; } bits = {0x7ff4000000000000ull};
    return bits.d;
}

int main(void) {
    volatile double one = 1.0, two = 2.0, nan = __builtin_nan("");
    volatile float half = 0.5f, fnan = __builtin_nanf("");
    volatile long double big = 1e300L * 1e300L;
    volatile int three = 3;
    volatile _Float16 small = 0.25f16;
    COMPARE(one, two);
    COMPARE(two, one);
    COMPARE(one, one);
    COMPARE(one, nan);
    COMPARE(fnan, half);
    COMPARE(half, one);
    COMPARE(big, two);
    COMPARE(three, two);
    COMPARE(one, three);
    COMPARE(small, half);
    COMPARE(small, three);
    printf("\n%d %d\n", folded[0], folded[1]);
    volatile float sf = signaling_float();
    volatile double sd = signaling_double();
    printf("%d %d %d %d %d %d\n", __builtin_issignaling(sf), __builtin_issignaling(sd),
           __builtin_issignaling(nan), __builtin_issignaling(fnan),
           __builtin_issignaling(one), __builtin_issignaling(big));
    static const struct { unsigned long long significand; unsigned short exponent; } encodings[] = {
        {0xC000000000000000ull, 0x7fff}, {0xA000000000000000ull, 0x7fff},
        {0x8000000000000001ull, 0xffff}, {0x8000000000000000ull, 0x7fff},
        {0x4000000000000000ull, 0x7fff}, {0x2000000000000000ull, 0x7fff},
        {0, 0x7fff}, {0x4000000000000000ull, 0x1234}, {0x8000000000000001ull, 0},
        {1, 0},
    };
    for (unsigned i = 0; i < sizeof encodings / sizeof encodings[0]; i++) {
        volatile long double value = x87(encodings[i].significand, encodings[i].exponent);
        printf("%d", __builtin_issignaling(value));
    }
    volatile __float128 quad_nan = __builtin_nanf128(""), quad_one = 1;
    printf(" %d %d %d %d %d\n", signaling[0], signaling[1], __builtin_isless(quad_nan, quad_one),
           __builtin_isgreaterequal(quad_one, quad_nan), __builtin_islessequal(quad_one, quad_one));
    volatile double ties[] = {0.5, 1.5, 2.5, -2.5, -0.5, 3.7, -0.0};
    for (int i = 0; i < 7; i++)
        printf("%g %g %Lg ", __builtin_roundeven(ties[i]),
               (double)__builtin_roundevenf((float)ties[i]),
               __builtin_roundevenl((long double)ties[i]));
    printf("\n");
#if !defined(__clang__) && __GNUC__ >= 14 /* GCC 14 added it */
    printf("%d %d %d\n", __builtin_iseqsig(one, 1), __builtin_iseqsig(one, two),
           __builtin_iseqsig(half, 0.5));
#else
    printf("1 0 1\n");
#endif
    return 0;
}
