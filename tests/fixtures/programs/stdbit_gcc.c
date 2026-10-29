/* The type-generic bit builtins as only GCC has them: __builtin_parityg,
   and __builtin_ffsg and __builtin_clrsbg of signed operands, at every
   width; bit-field operands counted at the field's width, with the
   results of the value-returning forms computed there; __builtin_clzg and
   __builtin_ctzg of zero without a result for zero, which GCC folds to
   the width; and rotations of a _BitInt wider than 128 bits by counts
   known only at run time, which Clang 23 cannot compile.
   stdbit_gcc.expected is what GCC 15 prints. */
#include <stdio.h>

static unsigned long long state = 0x2545f4914f6cdd1dull;

static int random_bit(void) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return (int)(state >> 31 & 1);
}

/* Runs the signed builtins on edge and random values of `T`, `W` bits. */
#define SIGNED(name, T, W)                                                     \
    static void name##_one(T x) {                                              \
        printf(" %d/%d", __builtin_ffsg(x), __builtin_clrsbg(x));              \
    }                                                                          \
    static void name(void) {                                                   \
        T values[12] = {0, 1, -1, 2, -2, 3};                                   \
        values[6] = (T)((T)1 << ((W) - 2));                                    \
        values[7] = (T)(-values[6] - values[6]);                               \
        values[8] = (T)(values[6] - 1 + values[6]);                            \
        for (int count = 9; count < 12; count++) {                             \
            T random = 0;                                                      \
            for (int i = 0; i < (W) - 1; i++)                                  \
                if (random_bit())                                              \
                    random |= (T)1 << i;                                       \
            values[count] = random_bit() ? (T)(-random - 1) : random;          \
        }                                                                      \
        printf("%s", #T);                                                      \
        for (int i = 0; i < 12; i++)                                           \
            name##_one(values[i]);                                             \
        printf("\n");                                                          \
    }

SIGNED(signed_sc, signed char, 8)
SIGNED(signed_s, short, 16)
SIGNED(signed_i, int, 32)
SIGNED(signed_l, long, 64)
SIGNED(signed_ll, long long, 64)
SIGNED(signed_128, __int128, 128)
SIGNED(signed_b2, _BitInt(2), 2)
SIGNED(signed_b7, _BitInt(7), 7)
SIGNED(signed_b63, _BitInt(63), 63)
SIGNED(signed_b65, _BitInt(65), 65)
SIGNED(signed_b200, _BitInt(200), 200)
SIGNED(signed_b1000, _BitInt(1000), 1000)

/* __builtin_parityg of the values 0 to 9 and all ones of `T`. */
#define PARITY(T)                                                              \
    do {                                                                       \
        printf("parity %s", #T);                                               \
        for (int i = 0; i < 10; i++)                                           \
            printf(" %d", __builtin_parityg((T)i));                            \
        printf(" %d\n", __builtin_parityg((T)-1));                             \
    } while (0)

struct fields {
    unsigned small : 3;
    unsigned long long middle : 40;
    unsigned _BitInt(5) bits : 4;
    unsigned _BitInt(200) wide : 150;
    int signed_small : 5;
    int one : 1;
};

/* The builtins on bit-field operands, which GCC counts at their width. */
static void fields(struct fields f) {
    printf("fields %d %d %d %u %u %u %u %u", __builtin_popcountg(f.small),
           __builtin_clzg(f.small, -1), __builtin_ctzg(f.middle, -1),
           __builtin_stdc_leading_zeros(f.middle),
           __builtin_stdc_leading_zeros(f.bits),
           __builtin_stdc_first_leading_one(f.wide),
           __builtin_stdc_count_zeros(f.wide), __builtin_stdc_bit_width(f.wide));
    printf(" %u %u %u %u", (unsigned)__builtin_stdc_bit_ceil(f.small),
           (unsigned)__builtin_stdc_bit_floor(f.small),
           (unsigned)__builtin_stdc_rotate_left(f.small, 1),
           (unsigned)__builtin_stdc_rotate_right(f.bits, 1));
    printf(" %llx %d", (unsigned long long)__builtin_stdc_bit_ceil(f.middle),
           __builtin_stdc_bit_ceil(f.wide) == 0);
    printf(" %d %d %d %d\n", __builtin_ffsg(f.signed_small),
           __builtin_clrsbg(f.signed_small), __builtin_ffsg(f.one),
           __builtin_clrsbg(f.one));
}

/* Rotations of wide values by counts known only at run time. */
#define WIDE_ROTATIONS(T, W)                                                   \
    do {                                                                       \
        static volatile int counts[] = {0, 1, 63, 64, 65, (W) - 1, (W),       \
                                        3 * (W) + 7};                          \
        T x = ((T)0x123456789abcdefull << 70) | 0xfedcba9876543210ull;         \
        printf("rotations %s", #T);                                            \
        for (unsigned i = 0; i < sizeof counts / sizeof counts[0]; i++) {      \
            T left = __builtin_stdc_rotate_left(x, counts[i]);                 \
            T right = __builtin_stdc_rotate_right(x, counts[i]);               \
            printf(" %d:%d:%llx/%d:%d:%llx", __builtin_popcountg(left),        \
                   __builtin_ctzg(left, -1), (unsigned long long)(left >> 64), \
                   __builtin_popcountg(right), __builtin_ctzg(right, -1),      \
                   (unsigned long long)(right >> 64));                         \
        }                                                                      \
        static volatile unsigned _BitInt(300) wide = ((unsigned _BitInt(300))1 << 280) + 3; \
        T left = __builtin_stdc_rotate_left(x, wide);                          \
        printf(" %d:%llx\n", __builtin_ctzg(left, -1),                         \
               (unsigned long long)(left >> 64));                              \
    } while (0)

int main(void) {
    signed_sc();
    signed_s();
    signed_i();
    signed_l();
    signed_ll();
    signed_128();
    signed_b2();
    signed_b7();
    signed_b63();
    signed_b65();
    signed_b200();
    signed_b1000();
    PARITY(unsigned char);
    PARITY(unsigned int);
    PARITY(unsigned __int128);
    PARITY(unsigned _BitInt(1));
    PARITY(unsigned _BitInt(65));
    PARITY(unsigned _BitInt(1000));
    fields((struct fields){1, 0x100, 3, (unsigned _BitInt(200))1 << 100, -3, -1});
    fields((struct fields){5, 0xffffffffff, 9, ((unsigned _BitInt(200))1 << 149) | 1, 7, 0});
    enum {
        Clz = __builtin_clzg(0u),
        Ctz = __builtin_ctzg((unsigned char)0),
        Wide = __builtin_clzg((unsigned _BitInt(1000))0),
    };
    printf("zero %d %d %d\n", Clz, Ctz, Wide);
    WIDE_ROTATIONS(unsigned _BitInt(200), 200);
    WIDE_ROTATIONS(unsigned _BitInt(1000), 1000);
    return 0;
}
