/* Imaginary interchange constants (GNU): `f16` with an imaginary suffix in
 * either order under both compilers, and GCC's `fN` and `fNx` forms. The
 * imaginary part has the value a real constant of the suffix has: under GCC
 * an `f16` constant keeps the excess precision of `float`, which Clang
 * keeps only through chains of arithmetic operators. */
#include <stdio.h>

static void show(const char *name, _Complex double z) {
    printf("%s: %a %a\n", name, __real__ z, __imag__ z);
}

_Complex _Float16 stored = 1.0f16 + 0.1f16i;
_Complex _Float16 orders[] = {2.5f16i, 2.5if16, 2.5F16i, 2.5jf16, 0x1.8p1if16};
double imaginary_part = __imag__ 0.1f16i;
int generic = _Generic(1.0f16i, _Complex _Float16: 1, default: 0);
int size = sizeof(1.0f16i);
#ifndef __clang__
_Complex _Float32 single = 1.5f32i;
_Complex _Float64 binary64 = 2.5if64;
_Complex _Float32x extended32 = 3.5f32xi;
_Complex _Float64x extended64 = 4.5if64x;
_Complex _Float128 quad = 5.5f128i + 1.0if128;
int generic_float32 = _Generic(1.0f32i, _Complex _Float32: 1, _Complex float: 2, default: 0);
#endif

int main(void) {
    volatile _Float16 tenth = 0.1f16;
    _Complex _Float16 product = 0.1f16i * 0.1f16i;
    _Complex _Float16 runtime = tenth * 3.0f16i;
    _Complex float widened = 0.1f16i;
    show("stored", stored);
    for (int i = 0; i < 5; i++)
        show("order", orders[i]);
    printf("imaginary part: %a\n", imaginary_part);
    printf("immediate part: %a\n", (double)__imag__ 0.1f16i);
    printf("generic: %d size: %d\n", generic, size);
    show("product", product);
    show("runtime", runtime);
    show("widened", widened);
    show("sum", 1.0f16 + 0.1f16i + 0.2f16i);
#ifndef __clang__
    show("single", single);
    show("binary64", binary64);
    show("extended32", extended32);
    show("extended64", extended64);
    printf("quad: %a %a\n", (double)__real__ quad, (double)__imag__ quad);
    printf("generic float32: %d\n", generic_float32);
    printf("sizes: %d %d %d %d %d\n", (int)sizeof(1.0f32i), (int)sizeof(1.0f64i),
           (int)sizeof(1.0f32xi), (int)sizeof(1.0f64xi), (int)sizeof(1.0f128i));
#endif
    return 0;
}
