/* Constant signaling NaNs through casts and conditionals. A conversion
   within one format (a cast to the operand's own type, a `?:` arm of the
   result's type) keeps the NaN signaling; a conversion to another format
   quiets it. Clang keeps the payload's leading bits; GCC drops the bit
   that only encodes `__builtin_nans("")`'s empty payload. Each value is
   folded in a static initializer and in an automatic one, and Clang's
   `__builtin_isfpclass` folds the same class. An initializer of an object
   of another format converts the value as by assignment, which quiets it:
   a scalar, a member, an `x87` array element, or a complex part. A real
   value's imaginary part is +0 in a complex object, a negated NaN's too.
   A `_Float16` NaN's real part in a `_Complex _Float16` keeps its format,
   so it stays signaling. */
#include <stdio.h>
#include <string.h>

static void dump(const char *name, const void *object, int size,
                 int signaling) {
    unsigned char bytes[16];
    memcpy(bytes, object, (size_t)size);
    printf("%-40s %d ", name, signaling);
    for (int i = size - 1; i >= 0; i--)
        printf("%02x", bytes[i]);
    printf("\n");
}

#define T(ty, size, e)                                                         \
    {                                                                          \
        static ty fixed = e;                                                   \
        static const int folded = __builtin_issignaling(e);                    \
        ty automatic = e;                                                      \
        dump(#e, &fixed, size, folded);                                        \
        dump(#e, &automatic, size, __builtin_issignaling(automatic));          \
    }

/* `e` stored in an object of the other real type `ty`. */
#define S(ty, size, e)                                                         \
    {                                                                          \
        static ty fixed = e;                                                   \
        ty automatic = e;                                                      \
        dump(#ty " = " #e, &fixed, size, __builtin_issignaling(fixed));        \
        dump(#ty " = " #e, &automatic, size,                                   \
             __builtin_issignaling(automatic));                                \
    }

/* `e` stored in an object of the complex type `ty`, of whose parts `size`
   bytes each are shown. */
#define C(ty, size, e)                                                         \
    {                                                                          \
        static ty fixed = e;                                                   \
        ty automatic = e;                                                      \
        dump(#ty " = " #e, &fixed, size,                                       \
             __builtin_issignaling(__real__ fixed));                           \
        dump(#ty " = " #e, (char *)&fixed + sizeof fixed / 2, size,            \
             __builtin_issignaling(__imag__ fixed));                           \
        dump(#ty " = " #e, &automatic, size,                                   \
             __builtin_issignaling(__real__ automatic));                       \
        dump(#ty " = " #e, (char *)&automatic + sizeof automatic / 2, size,    \
             __builtin_issignaling(__imag__ automatic));                       \
    }

struct members {
    float f;
    double d;
    long double l;
};

#define MEMBERS                                                                \
    { __builtin_nans(""), __builtin_nansf(""), (double)__builtin_nans("") }
#define ELEMENTS { __builtin_nansf(""), (double)__builtin_nans("3") }

static void dump_members(const char *name, const struct members *m) {
    dump(name, &m->f, 4, __builtin_issignaling(m->f));
    dump(name, &m->d, 8, __builtin_issignaling(m->d));
    dump(name, &m->l, 10, __builtin_issignaling(m->l));
}

#ifdef __clang__
_Static_assert(__builtin_isfpclass((float)__builtin_nansf(""), 1), "kept");
_Static_assert(__builtin_isfpclass(1 ? __builtin_nansf("") : 0.0f, 1),
               "kept");
_Static_assert(__builtin_isfpclass((long double)__builtin_nans(""), 2),
               "quiet");
_Static_assert(__builtin_isfpclass(1 ? __builtin_nansf("") : 0.0, 2),
               "quiet");
#endif

int main(void) {
    T(float, 4, __builtin_nansf(""))
    T(float, 4, (float)__builtin_nansf(""))
    T(double, 8, (double)__builtin_nans(""))
    T(long double, 10, (long double)__builtin_nansl(""))
    T(float, 4, (float)__builtin_nans(""))
    T(float, 4, (float)-__builtin_nans(""))
    T(double, 8, (double)__builtin_nansf(""))
    T(double, 8, (double)__builtin_nansl(""))
    T(long double, 10, (long double)__builtin_nans(""))
    T(long double, 10, (long double)__builtin_nans("3"))
    T(float, 4, (float)(double)__builtin_nansf(""))
    T(float, 4, 1 ? __builtin_nansf("") : 0.0f)
    T(float, 4, -__builtin_nansf(""))
    T(double, 8, 1 ? __builtin_nansf("") : 0.0)
    T(double, 8, 0 ? 0.0 : __builtin_nans(""))
    T(double, 8, 0 ? 1.0f : __builtin_nans(""))
#ifdef __x86_64__
    T(_Float16, 2, (_Float16)__builtin_nans(""))
    T(_Float16, 2, (_Float16)__builtin_nansf("0x100000"))
#endif
#ifdef __SIZEOF_FLOAT128__
    T(__float128, 16, (__float128)__builtin_nans(""))
    T(__float128, 16, 1 ? __builtin_nansf128("") : 0)
    T(double, 8, (double)__builtin_nansf128(""))
#endif
    S(double, 8, (float)__builtin_nansf(""))
    S(double, 8, 1 ? __builtin_nansf("") : 0)
    S(double, 8, __builtin_nansf("0x5"))
    S(float, 4, __builtin_nans(""))
    S(float, 4, -__builtin_nans("0x123"))
    S(float, 4, __builtin_nansl(""))
    S(long double, 10, (double)__builtin_nans(""))
    S(long double, 10, __builtin_nansf(""))
    S(double, 8, __builtin_complex(__builtin_nansf(""), 0.0f))
#ifdef __x86_64__
    S(_Float16, 2, __builtin_nans(""))
    S(_Float16, 2, 1 ? __builtin_nans("0x123") : 0)
    S(float, 4, __builtin_nansf16(""))
    S(float, 4, __builtin_nansf16("0x5"))
    S(float, 4, (float)__builtin_nansf16(""))
    S(double, 8, __builtin_nansf16(""))
    S(__bf16, 2, __builtin_nansf16(""))
    S(_Float16, 2, __builtin_nansf16(""))
#endif
#ifdef __SIZEOF_FLOAT128__
    S(__float128, 16, __builtin_nans(""))
    S(double, 8, __builtin_nansf128(""))
    S(long double, 10, __builtin_nansf128("0x5"))
#endif
    C(_Complex double, 8, __builtin_nansf(""))
    C(_Complex double, 8, __builtin_complex(__builtin_nansf(""), 0.0f))
    C(_Complex float, 4, __builtin_complex(0.0, -__builtin_nans("")))
    C(_Complex long double, 10, __builtin_complex(__builtin_nans(""), 1.0))
    C(_Complex double, 8, -__builtin_nan(""))
    C(_Complex double, 8, -__builtin_nans(""))
    C(_Complex float, 4, -__builtin_nansf(""))
    C(_Complex long double, 10, -__builtin_nanl(""))
    C(_Complex double, 8, -1.0)
#ifdef __x86_64__
    C(_Complex float, 4, __builtin_nansf16(""))
    C(_Complex float, 4, -(_Float16)1.0)
    C(_Complex float, 4, -__builtin_nansf16(""))
    C(_Complex _Float16, 2, __builtin_nansf16(""))
    C(_Complex _Float16, 2, __builtin_nansf16("0x3"))
    C(_Complex _Float16, 2, -__builtin_nansf16(""))
    C(_Complex _Float16, 2, (_Float16)__builtin_nansf16(""))
#endif
    {
        static _Complex long double fixed[1] = {-__builtin_nansl("")};
        _Complex long double automatic[1] = {-__builtin_nansl("")};
        dump("static complex element", (char *)fixed + 16, 10,
             __builtin_issignaling(__imag__ fixed[0]));
        dump("automatic complex element", (char *)automatic + 16, 10,
             __builtin_issignaling(__imag__ automatic[0]));
    }
    {
        static struct members fixed = MEMBERS;
        struct members automatic = MEMBERS;
        dump_members("static members", &fixed);
        dump_members("automatic members", &automatic);
    }
    {
        static long double fixed[2] = ELEMENTS;
        long double automatic[2] = ELEMENTS;
        for (int i = 0; i < 2; i++) {
            dump("static elements", &fixed[i], 10,
                 __builtin_issignaling(fixed[i]));
            dump("automatic elements", &automatic[i], 10,
                 __builtin_issignaling(automatic[i]));
        }
    }
    return 0;
}
