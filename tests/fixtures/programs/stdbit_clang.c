/* The type-generic bit builtins as only Clang has them: bool and
   enumerated operands of __builtin_popcountg, __builtin_clzg,
   __builtin_ctzg and the rotations, computed at the width of bool (1) or
   of the enumeration's underlying type; bit-field operands counted at
   their declared type's width; and negative rotation counts, which Clang
   reduces to the width's residue (GCC leaves them undefined), of narrow
   and wide types. The bundled Clang's build is the reference. */
#include <stdio.h>

enum plain { Zero, Five = 5 };
enum __attribute__((packed)) small { Small = 0x81 };

struct fields {
    unsigned small : 3;
    unsigned _BitInt(5) bits : 4;
};

static void booleans(bool b) {
    printf("bool %d %d %d %d %d %d\n", __builtin_popcountg(b),
           __builtin_clzg(b, 9), __builtin_ctzg(b, 9),
           (int)__builtin_stdc_rotate_left(b, 3),
           (int)__builtin_stdc_rotate_right(b, -2),
           _Generic(__builtin_stdc_rotate_left(b, 1), bool: 1, default: 0));
}

static void enumerations(enum plain p, enum small s) {
    printf("enum %d %d %d %u %d %d %u\n", __builtin_popcountg(p),
           __builtin_clzg(p, -1), __builtin_ctzg(p, -1),
           (unsigned)__builtin_stdc_rotate_left(p, 1), __builtin_clzg(s, -1),
           __builtin_popcountg(s), (unsigned)__builtin_stdc_rotate_right(s, 1));
}

static void fields(struct fields f) {
    printf("fields %d %d %u %u %u %u %d\n", __builtin_clzg(f.small, -1),
           __builtin_clzg(f.bits, -1), __builtin_stdc_leading_zeros(f.small),
           (unsigned)__builtin_stdc_bit_ceil(f.small),
           (unsigned)__builtin_stdc_rotate_left(f.small, 1),
           (unsigned)__builtin_stdc_rotate_right(f.bits, 1),
           _Generic(__builtin_stdc_bit_ceil(f.small), unsigned: 1, default: 0));
}

/* Rotations by negative counts of several types, known at run time. */
#define NEGATIVE(T, x)                                                         \
    do {                                                                       \
        static volatile int ints[] = {-1, -2, -7, -64, -65, -1000};            \
        static volatile signed char chars[] = {-1, -128};                      \
        static volatile __int128 wides[] = {-1, -((__int128)1 << 100) - 9};    \
        T v = (x);                                                             \
        printf("negative %s", #T);                                             \
        for (unsigned i = 0; i < sizeof ints / sizeof ints[0]; i++)            \
            printf(" %llx/%llx",                                               \
                   (unsigned long long)__builtin_stdc_rotate_left(v, ints[i]), \
                   (unsigned long long)__builtin_stdc_rotate_right(v, ints[i])); \
        for (unsigned i = 0; i < 2; i++)                                       \
            printf(" %llx/%llx",                                               \
                   (unsigned long long)__builtin_stdc_rotate_left(v, chars[i]), \
                   (unsigned long long)__builtin_stdc_rotate_left(v, wides[i])); \
        printf(" %llx %llx\n",                                                 \
               (unsigned long long)__builtin_stdc_rotate_left(v, -3),          \
               (unsigned long long)__builtin_stdc_rotate_right(                \
                   v, -(((_BitInt(300))1 << 290) + 11)));                      \
    } while (0)

int main(void) {
    booleans(false);
    booleans(true);
    enumerations(Five, Small);
    enumerations(Zero, Small);
    fields((struct fields){1, 3});
    fields((struct fields){5, 9});
    NEGATIVE(unsigned char, 0x81);
    NEGATIVE(unsigned short, 0x8003);
    NEGATIVE(unsigned int, 0x80000005u);
    NEGATIVE(unsigned long long, 0x8000000000000009ull);
    NEGATIVE(unsigned _BitInt(7), 0x41);
    NEGATIVE(unsigned _BitInt(65), 0x8000000000000011ull);
    NEGATIVE(unsigned __int128, ((unsigned __int128)1 << 127) | 0x21);
    printf("constant %u %u %u\n", __builtin_stdc_rotate_left(1u, -1),
           (unsigned)__builtin_stdc_rotate_left((unsigned _BitInt(7))1, -1),
           (unsigned)__builtin_stdc_rotate_right((unsigned char)1, -9));
    return 0;
}
