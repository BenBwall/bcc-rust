/* Clang's `__builtin_isfpclass`, `__builtin_issubnormal` and
 * `__builtin_iszero`, with the `__FPCLASS_*` masks, which GCC lacks: each
 * class of each real floating type, signaling NaNs and subnormals included,
 * read at run time through `volatile`, and constant operands folded. An x87
 * encoding the format leaves unsupported and a pseudo-denormal are in no
 * class alone, and a test holds for them depending on the whole mask, as
 * Clang tests them at -O0 (the reference). Lines that start with `x86:` are
 * left out on wasm32, whose `long double` is binary128. */
#include <stdio.h>
#include <string.h>

#ifdef __clang__
#define MASKS 18
static const int masks[MASKS] = {
    __FPCLASS_SNAN,      __FPCLASS_QNAN,         __FPCLASS_NEGINF,
    __FPCLASS_NEGNORMAL, __FPCLASS_NEGSUBNORMAL, __FPCLASS_NEGZERO,
    __FPCLASS_POSZERO,   __FPCLASS_POSSUBNORMAL, __FPCLASS_POSNORMAL,
    __FPCLASS_POSINF,    0x003,                  0x3fc,
    0x0f3,               0x30c,                  0x1f8,
    0x207,               0x090,                  0x060};

/* The mask must be a constant, so each is spelled out. */
#define TESTS(x)                                                               \
    __builtin_isfpclass(x, __FPCLASS_SNAN),                                    \
        __builtin_isfpclass(x, __FPCLASS_QNAN),                                \
        __builtin_isfpclass(x, __FPCLASS_NEGINF),                              \
        __builtin_isfpclass(x, __FPCLASS_NEGNORMAL),                           \
        __builtin_isfpclass(x, __FPCLASS_NEGSUBNORMAL),                        \
        __builtin_isfpclass(x, __FPCLASS_NEGZERO),                             \
        __builtin_isfpclass(x, __FPCLASS_POSZERO),                             \
        __builtin_isfpclass(x, __FPCLASS_POSSUBNORMAL),                        \
        __builtin_isfpclass(x, __FPCLASS_POSNORMAL),                           \
        __builtin_isfpclass(x, __FPCLASS_POSINF),                              \
        __builtin_isfpclass(x, 0x003), __builtin_isfpclass(x, 0x3fc),          \
        __builtin_isfpclass(x, 0x0f3), __builtin_isfpclass(x, 0x30c),          \
        __builtin_isfpclass(x, 0x1f8), __builtin_isfpclass(x, 0x207),          \
        __builtin_issubnormal(x), __builtin_iszero(x)

static void show(const char *prefix, const char *type, int index,
                 const int *results) {
    printf("%s%s %d ", prefix, type, index);
    for (int i = 0; i < MASKS; i++)
        putchar(results[i] == 1 ? '1' : results[i] == 0 ? '0' : '?');
    putchar('\n');
}

#define CLASSIFY(prefix, type, name, bits, count)                              \
    for (int i = 0; i < (count); i++) {                                        \
        type value;                                                            \
        memcpy(&value, &(bits)[i], sizeof value);                              \
        volatile type stored = value;                                          \
        type x = stored;                                                       \
        int results[MASKS] = {TESTS(x)};                                       \
        show(prefix, name, i, results);                                        \
    }

static const unsigned float_bits[] = {
    0x00000000, 0x80000000, 0x00000001, 0x807fffff, 0x00800000, 0xbf800000,
    0x7f7fffff, 0x7f800000, 0xff800000, 0x7fc00000, 0xffc00001, 0x7f800001,
    0xffa00000};
static const unsigned long long double_bits[] = {
    0x0000000000000000, 0x8000000000000000, 0x0000000000000001,
    0x800fffffffffffff, 0x0010000000000000, 0xbff0000000000000,
    0x7fefffffffffffff, 0x7ff0000000000000, 0xfff0000000000000,
    0x7ff8000000000000, 0xfff8000000000001, 0x7ff0000000000001,
    0xfff4000000000000};

/* Constant operands fold, a signaling NaN staying signaling. */
_Static_assert(__builtin_isfpclass(1.0, __FPCLASS_POSNORMAL), "normal");
_Static_assert(!__builtin_isfpclass(-1.0f, __FPCLASS_POSNORMAL), "sign");
_Static_assert(__builtin_isfpclass(-0.0L, __FPCLASS_NEGZERO), "zero");
_Static_assert(__builtin_isfpclass(1e-310, __FPCLASS_POSSUBNORMAL), "tiny");
_Static_assert(__builtin_isfpclass(__builtin_nans(""), __FPCLASS_SNAN),
               "signaling");
_Static_assert(!__builtin_isfpclass(__builtin_nans(""), __FPCLASS_QNAN),
               "not quiet");
_Static_assert(__builtin_isfpclass(-__builtin_inff(), __FPCLASS_NEGINF),
               "infinite");
_Static_assert(__builtin_issubnormal(1e-40f) && !__builtin_iszero(1e-40f),
               "float subnormal");
_Static_assert(__builtin_iszero(-0.0) && !__builtin_issubnormal(0.0),
               "zero");
static int folded[] = {__builtin_isfpclass(1e-320, 0x090),
                       __builtin_isfpclass(__builtin_nanl(""), 0x002),
                       __builtin_isfpclass(2.0, 0)};

#if defined(__x86_64__) && __LDBL_MANT_DIG__ == 64
/* Canonical x87 values, then unsupported encodings (pseudo-NaN,
   pseudo-infinity, unnormal) and pseudo-denormals. */
static const struct {
    unsigned long long significand;
    unsigned short sign_exponent;
} x87_bits[] = {
    {0, 0},
    {0, 0x8000},
    {1, 0},
    {0x7fffffffffffffff, 0x8000},
    {0x8000000000000000, 1},
    {0x8000000000000000, 0xbfff},
    {0x8000000000000000, 0x7fff},
    {0x8000000000000000, 0xffff},
    {0xc000000000000000, 0x7fff},
    {0xc000000000000001, 0xffff},
    {0x8000000000000001, 0x7fff},
    {0xa000000000000000, 0xffff},
    {0, 0x7fff},
    {0x4000000000000000, 0xffff},
    {0x4000000000000000, 0x3fff},
    {0, 0xbfff},
    {0x8000000000000000, 0},
    {0x8000000000000001, 0x8000},
};

/* Every mask for one x87 value, as a checksum. */
static unsigned long x87_masks(long double x);
#endif

#ifdef __SIZEOF_FLOAT128__
static const unsigned long long float128_bits[][2] = {
    {0, 0},
    {0, 0x8000000000000000},
    {1, 0},
    {0, 0x0001000000000000},
    {0, 0xffff000000000000},
    {0, 0x7fff800000000000},
    {1, 0x7fff000000000000},
};
#endif

static const unsigned short half_bits[] = {0x0000, 0x8000, 0x0001, 0x83ff,
                                           0x0400, 0xbc00, 0x7c00, 0xfc00,
                                           0x7e00, 0x7d00, 0xfc01};

int main(void) {
    CLASSIFY("", float, "float", float_bits,
             (int)(sizeof float_bits / sizeof *float_bits));
    CLASSIFY("", double, "double", double_bits,
             (int)(sizeof double_bits / sizeof *double_bits));
#ifdef __x86_64__
    CLASSIFY("x86:", _Float16, "_Float16", half_bits,
             (int)(sizeof half_bits / sizeof *half_bits));
#endif
#if defined(__x86_64__) && __LDBL_MANT_DIG__ == 64
    for (int i = 0; i < (int)(sizeof x87_bits / sizeof *x87_bits); i++) {
        long double value = 0;
        memcpy(&value, &x87_bits[i].significand, 8);
        memcpy((char *)&value + 8, &x87_bits[i].sign_exponent, 2);
        volatile long double stored = value;
        long double x = stored;
        int results[MASKS] = {TESTS(x)};
        show("x86:", "long double", i, results);
        printf("x86: all masks %lu\n", x87_masks(x));
    }
#endif
#if defined(__x86_64__) && defined(__SIZEOF_FLOAT128__)
    CLASSIFY("x86:", __float128, "__float128", float128_bits,
             (int)(sizeof float128_bits / sizeof *float128_bits));
#endif
    printf("folded %d %d %d\n", folded[0], folded[1], folded[2]);
    return 0;
}

#if defined(__x86_64__) && __LDBL_MANT_DIG__ == 64
#define M1(m) sum = sum * 3 + (unsigned long)__builtin_isfpclass(x, m);
#define M4(m) M1(m) M1(m + 1) M1(m + 2) M1(m + 3)
#define M16(m) M4(m) M4(m + 4) M4(m + 8) M4(m + 12)
#define M64(m) M16(m) M16(m + 16) M16(m + 32) M16(m + 48)
#define M256(m) M64(m) M64(m + 64) M64(m + 128) M64(m + 192)
static unsigned long x87_masks(long double x) {
    unsigned long sum = 0;
    M256(0) M256(256) M256(512) M256(768)
    return sum;
}
#endif
#else
int main(void) {
    puts("GCC has no __builtin_isfpclass");
    return 0;
}
#endif
