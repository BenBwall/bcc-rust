/* Every type-generic bit builtin GCC and Clang share and every function of
   C23's <stdbit.h>, at every unsigned width: the standard types,
   unsigned __int128 and unsigned _BitInt(1), (7), (63), (65), (200) and
   (1000), over edge values, bit patterns and pseudo-random values, with
   nonnegative rotation counts of narrow and wide types (GCC leaves a
   negative count undefined; stdbit_clang.c covers Clang's reduction). A result
   wider than 128 bits prints as a hash of its limbs. The values reach the
   builtins through a function's parameter, so each is computed at run
   time; the folded section repeats some on constants. GCC 15 and Clang 23
   print the same, and on wasm32 with a 128-bit BITINT_MAXWIDTH the wider
   widths are left out. */
#include <stdbit.h>
#include <stdio.h>

static unsigned long long state = 0x9e3779b97f4a7c15ull;

/* A pseudo-random bit (xorshift64). */
static int random_bit(void) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return (int)(state >> 31 & 1);
}

/* The printer of a value of `T`, `W` bits wide: hexadecimal up to 128
   bits, else an FNV-1a hash of its 64-bit limbs. */
#define PRINTER(name, T, W)                                                    \
    static void name(T v) {                                                    \
        if ((W) > 128) {                                                       \
            unsigned long long hash = 0xcbf29ce484222325ull;                   \
            for (int i = 0; i * 64 < (W); i++) {                               \
                hash = (hash ^ (unsigned long long)v) * 0x100000001b3ull;      \
                v >>= (W) > 64 ? 64 : 0;                                       \
            }                                                                  \
            printf("#%016llx", hash);                                          \
            return;                                                            \
        }                                                                      \
        if ((W) > 64)                                                          \
            printf("%llx:", (unsigned long long)(v >> ((W) > 64 ? 64 : 0)));   \
        printf("%llx", (unsigned long long)v);                                 \
    }

/* Runs every builtin on `x`, with rotations by each count. */
#define SUITE(name, T, W, print, runtime)                                      \
    static void name(T x) {                                                    \
        print(x);                                                              \
        printf(": pop %d clzg %d ctzg %d", __builtin_popcountg(x),             \
               __builtin_clzg(x, -1), __builtin_ctzg(x, (W) + 5));             \
        if (x)                                                                 \
            printf(" clz %d ctz %d", __builtin_clzg(x), __builtin_ctzg(x));    \
        printf(" lz %u lo %u tz %u to %u", __builtin_stdc_leading_zeros(x),    \
               __builtin_stdc_leading_ones(x), __builtin_stdc_trailing_zeros(x), \
               __builtin_stdc_trailing_ones(x));                               \
        printf(" flz %u flo %u ftz %u fto %u",                                 \
               __builtin_stdc_first_leading_zero(x),                           \
               __builtin_stdc_first_leading_one(x),                            \
               __builtin_stdc_first_trailing_zero(x),                          \
               __builtin_stdc_first_trailing_one(x));                          \
        printf(" cz %u co %u single %d width %u",                              \
               __builtin_stdc_count_zeros(x), __builtin_stdc_count_ones(x),    \
               (int)__builtin_stdc_has_single_bit(x),                          \
               __builtin_stdc_bit_width(x));                                   \
        printf(" floor ");                                                     \
        print(__builtin_stdc_bit_floor(x));                                    \
        printf(" ceil ");                                                      \
        print(__builtin_stdc_bit_ceil(x));                                     \
        printf("\n  rotations");                                               \
        ROTATE(x, 0, print)                                                    \
        ROTATE(x, 1, print)                                                    \
        ROTATE(x, 3, print)                                                    \
        ROTATE(x, (W) - 1, print)                                              \
        ROTATE(x, (W), print)                                                  \
        ROTATE(x, (W) + 1, print)                                              \
        ROTATE(x, 2 * (W) + 3, print)                                          \
        ROTATE(x, 5 * (W) - 1, print)                                          \
        runtime(x);                                                            \
        printf(" |");                                                          \
        print(__builtin_stdc_rotate_left(x, (signed char)121));                \
        printf(" ");                                                           \
        print(__builtin_stdc_rotate_right(x, (unsigned char)200));             \
        printf(" ");                                                           \
        print(__builtin_stdc_rotate_left(x, (bool)1));                         \
        printf(" ");                                                           \
        print(__builtin_stdc_rotate_left(x, ~0ull));                           \
        printf(" ");                                                           \
        print(__builtin_stdc_rotate_right(x, (__int128)1 << 100));             \
        WIDE_COUNTS(x, print)                                                  \
        printf("\n");                                                          \
    }

#if __BITINT_MAXWIDTH__ >= 1000
#define WIDE_COUNTS(x, print)                                                  \
    printf(" ");                                                               \
    print(__builtin_stdc_rotate_left(                                          \
        x, ((unsigned _BitInt(300))1 << 299) + 5));                            \
    printf(" ");                                                               \
    print(__builtin_stdc_rotate_right(                                         \
        x, ((_BitInt(300))1 << 298) + 7));
#else
#define WIDE_COUNTS(x, print)
#endif

/* Rotations of `x` left and right by the constant `count`. */
#define ROTATE(x, count, print)                                                \
    printf(" ");                                                               \
    print(__builtin_stdc_rotate_left(x, count));                               \
    printf("/");                                                               \
    print(__builtin_stdc_rotate_right(x, count));

/* Rotations of `x` by counts known only at run time; Clang 23 cannot
   compile them for a `_BitInt` of more than 128 bits ("unsupported library
   call operation"), so only narrower types run them. */
#define RUNTIME(name, T, W, print)                                             \
    static void name(T x) {                                                    \
        static volatile int counts[] = {0,           1,       3,  (W) - 1,     \
                                        (W),         (W) + 1, 2 * (W) + 3,     \
                                        5 * (W) - 1, 1 << 30};                 \
        printf(" @");                                                          \
        for (unsigned i = 0; i < sizeof counts / sizeof counts[0]; i++) {      \
            printf(" ");                                                       \
            print(__builtin_stdc_rotate_left(x, counts[i]));                   \
            printf("/");                                                       \
            print(__builtin_stdc_rotate_right(x, counts[i]));                  \
        }                                                                      \
        static volatile __int128 wide = ((__int128)1 << 100) + 9;              \
        printf(" ");                                                           \
        print(__builtin_stdc_rotate_left(x, wide));                            \
    }
#define NO_RUNTIME(name, T, W, print)                                          \
    static void name(T x) { (void)x; }

/* Builds the values of `T`, `W` bits wide, and runs the suite on each. */
#define VALUES(name, T, W, suite)                                              \
    static void name(void) {                                                   \
        T high = (T)1 << ((W) - 1), all = (T)-1;                               \
        T values[18] = {0,        1,         (T)2,    (T)3,    high,          \
                        high | 1, all,       all - 1, all / 2};              \
        int count = 9;                                                         \
        for (int step = 2; step <= 8; step *= 2) {                             \
            T pattern = 0;                                                     \
            for (int i = 0; i < (W); i++)                                      \
                if (i % step < step / 2)                                       \
                    pattern |= (T)1 << i;                                      \
            values[count++] = pattern;                                         \
            values[count++] = (T)~pattern;                                     \
        }                                                                      \
        values[count++] = (T)1 << ((W) / 2);                                   \
        for (; count < 18; count++) {                                          \
            T random = 0;                                                      \
            for (int i = 0; i < (W); i++)                                      \
                if (random_bit())                                              \
                    random |= (T)1 << i;                                       \
            values[count] = random;                                            \
        }                                                                      \
        printf("%s\n", #T);                                                    \
        for (int i = 0; i < 18; i++)                                           \
            suite(values[i]);                                                  \
    }

#define TYPE(T, W, print, suite, values, runtime, rotations)                    \
    PRINTER(print, T, W)                                                       \
    rotations(runtime, T, W, print)                                            \
    SUITE(suite, T, W, print, runtime)                                         \
    VALUES(values, T, W, suite)

TYPE(unsigned char, __CHAR_BIT__, print_uc, suite_uc, values_uc, runtime_uc, RUNTIME)
TYPE(unsigned short, 16, print_us, suite_us, values_us, runtime_us, RUNTIME)
TYPE(unsigned int, 32, print_ui, suite_ui, values_ui, runtime_ui, RUNTIME)
TYPE(unsigned long, __SIZEOF_LONG__ * 8, print_ul, suite_ul, values_ul, runtime_ul, RUNTIME)
TYPE(unsigned long long, 64, print_ull, suite_ull, values_ull, runtime_ull, RUNTIME)
TYPE(unsigned __int128, 128, print_u128, suite_u128, values_u128, runtime_u128, RUNTIME)
TYPE(unsigned _BitInt(1), 1, print_b1, suite_b1, values_b1, runtime_b1, RUNTIME)
TYPE(unsigned _BitInt(7), 7, print_b7, suite_b7, values_b7, runtime_b7, RUNTIME)
TYPE(unsigned _BitInt(63), 63, print_b63, suite_b63, values_b63, runtime_b63, RUNTIME)
TYPE(unsigned _BitInt(65), 65, print_b65, suite_b65, values_b65, runtime_b65, RUNTIME)
#if __BITINT_MAXWIDTH__ >= 1000
TYPE(unsigned _BitInt(200), 200, print_b200, suite_b200, values_b200, runtime_b200,
     NO_RUNTIME)
TYPE(unsigned _BitInt(1000), 1000, print_b1000, suite_b1000, values_b1000, runtime_b1000,
     NO_RUNTIME)
#endif

/* Each <stdbit.h> function of each standard type, as the macro the header
   defines, as a call that suppresses the macro, and through a pointer to
   the function; and the type-generic macros. */
#define FUNCTION(name, suffix, T, x)                                           \
    do {                                                                       \
        __typeof__(stdc_##name##suffix(x)) (*f)(T) = stdc_##name##suffix;      \
        printf(" %llu/%llu/%llu", (unsigned long long)stdc_##name##suffix(x),  \
               (unsigned long long)(stdc_##name##suffix)(x),                   \
               (unsigned long long)f(x));                                      \
    } while (0)

#define FUNCTIONS(suffix, T, x)                                                \
    do {                                                                       \
        FUNCTION(leading_zeros, suffix, T, x);                                 \
        FUNCTION(leading_ones, suffix, T, x);                                  \
        FUNCTION(trailing_zeros, suffix, T, x);                                \
        FUNCTION(trailing_ones, suffix, T, x);                                 \
        FUNCTION(first_leading_zero, suffix, T, x);                            \
        FUNCTION(first_leading_one, suffix, T, x);                             \
        FUNCTION(first_trailing_zero, suffix, T, x);                           \
        FUNCTION(first_trailing_one, suffix, T, x);                            \
        FUNCTION(count_zeros, suffix, T, x);                                   \
        FUNCTION(count_ones, suffix, T, x);                                    \
        FUNCTION(has_single_bit, suffix, T, x);                                \
        FUNCTION(bit_width, suffix, T, x);                                     \
        FUNCTION(bit_floor, suffix, T, x);                                     \
        FUNCTION(bit_ceil, suffix, T, x);                                      \
        printf(" |");                                                          \
        T y = (T)(x);                                                          \
        printf(" %u %u %u %u %u %u %u %u %u %u %d %u %llu %llu",               \
               stdc_leading_zeros(y), stdc_leading_ones(y),                    \
               stdc_trailing_zeros(y), stdc_trailing_ones(y),                  \
               stdc_first_leading_zero(y), stdc_first_leading_one(y),          \
               stdc_first_trailing_zero(y), stdc_first_trailing_one(y),        \
               stdc_count_zeros(y), stdc_count_ones(y),                        \
               (int)stdc_has_single_bit(y), stdc_bit_width(y),                 \
               (unsigned long long)stdc_bit_floor(y),                          \
               (unsigned long long)stdc_bit_ceil(y));                          \
    } while (0)

static void functions(unsigned long long x) {
    printf("functions %llx:", x);
    FUNCTIONS(_uc, unsigned char, x);
    FUNCTIONS(_us, unsigned short, x);
    FUNCTIONS(_ui, unsigned int, x);
    FUNCTIONS(_ul, unsigned long, x);
    FUNCTIONS(_ull, unsigned long long, x);
    printf("\n");
}

/* Calls whose operands are constants, which fold. */
static void folded(void) {
    printf("folded %d %d %d %d %u %u %u %u\n", __builtin_popcountg(0xf0u),
           __builtin_clzg((unsigned char)0, 77), __builtin_ctzg(0x100ull),
           __builtin_clzg((unsigned __int128)1 << 90),
           __builtin_stdc_leading_ones((unsigned _BitInt(63))-1),
           (unsigned)__builtin_stdc_bit_ceil((unsigned short)300),
           (unsigned)__builtin_stdc_rotate_left((unsigned char)0x81, 15),
           (unsigned)__builtin_stdc_rotate_right(0x80000001u, 33));
#if __BITINT_MAXWIDTH__ >= 1000
    printf("folded wide %d %d %u %u %u %d\n",
           __builtin_popcountg((unsigned _BitInt(1000))-1),
           __builtin_ctzg((unsigned _BitInt(1000))1 << 999),
           __builtin_stdc_first_leading_zero((unsigned _BitInt(200))-1 >> 3),
           __builtin_stdc_bit_width((unsigned _BitInt(200))1 << 150),
           __builtin_stdc_count_zeros((unsigned _BitInt(1000))0),
           __builtin_stdc_bit_ceil((unsigned _BitInt(200))5) == 8);
#endif
}

int main(void) {
    printf("stdbit %ld endian %d %d %d\n", (long)__STDC_VERSION_STDBIT_H__,
           __STDC_ENDIAN_LITTLE__, __STDC_ENDIAN_BIG__, __STDC_ENDIAN_NATIVE__);
    folded();
    values_uc();
    values_us();
    values_ui();
    values_ul();
    values_ull();
    values_u128();
    values_b1();
    values_b7();
    values_b63();
    values_b65();
#if __BITINT_MAXWIDTH__ >= 1000
    values_b200();
    values_b1000();
#endif
    static const unsigned long long inputs[] = {
        0, 1, 2, 3, 0x80, 0xff, 0x100, 0x8000, 0xffff, 0x10000, 0x7fffffff,
        0x80000000, 0xffffffff, 0x100000000, 0x8000000000000000,
        0xffffffffffffffff, 0x123456789abcdef0, 0x0f0f0f0f0f0f0f0f};
    for (unsigned i = 0; i < sizeof inputs / sizeof inputs[0]; i++)
        functions(inputs[i]);
    return 0;
}
