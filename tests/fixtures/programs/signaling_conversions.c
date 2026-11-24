/* Constant signaling NaNs through casts and conditionals. A conversion
   within one format (a cast to the operand's own type, a `?:` arm of the
   result's type) keeps the NaN signaling; a conversion to another format
   quiets it. Clang keeps the payload's leading bits; GCC drops the bit
   that only encodes `__builtin_nans("")`'s empty payload. Each value is
   folded in a static initializer and in an automatic one, and Clang's
   `__builtin_isfpclass` folds the same class. */
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
    return 0;
}
