/* The comparison macros of C99 §7.12.14 as GCC's and Clang's builtins, which
 * glibc's <math.h> uses under both compilers' identities, with
 * `__builtin_issignaling` and `__builtin_roundeven`. Operands of different
 * types compare in their common real type; a NaN is unordered; constant
 * operands fold. GCC's `__builtin_iseqsig` is GCC-only. */
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
    volatile double ties[] = {0.5, 1.5, 2.5, -2.5, -0.5, 3.7, -0.0};
    for (int i = 0; i < 7; i++)
        printf("%g %g %Lg ", __builtin_roundeven(ties[i]),
               (double)__builtin_roundevenf((float)ties[i]),
               __builtin_roundevenl((long double)ties[i]));
    printf("\n");
#ifndef __clang__
    printf("%d %d %d\n", __builtin_iseqsig(one, 1), __builtin_iseqsig(one, two),
           __builtin_iseqsig(half, 0.5));
#else
    printf("1 0 1\n");
#endif
    return 0;
}
