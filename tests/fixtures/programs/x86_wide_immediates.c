/* Immediates wider than their builtin's `int` parameter, or outside a
   byte, take the value their conversion to that parameter gives (C99
   §6.5.2.2p7, §6.3.1.3), and the instruction encodes that value's low byte.
   GCC 14 accepts every one of these calls; Clang, under `-w`, encodes the
   same byte. Each line prints the result's lanes. */
#include <stdio.h>

typedef int v4si __attribute__((vector_size(16)));
typedef short v8hi __attribute__((vector_size(16)));
typedef float v4sf __attribute__((vector_size(16)));
typedef double v2df __attribute__((vector_size(16)));
typedef long long v2di __attribute__((vector_size(16)));

static void print4(const char *name, v4si v)
{
    printf("%s %d %d %d %d\n", name, v[0], v[1], v[2], v[3]);
}

static void print8(const char *name, v8hi v)
{
    printf("%s", name);
    for (int i = 0; i < 8; i++)
        printf(" %d", v[i]);
    printf("\n");
}

int main(void)
{
    v4si a = {1, 2, 3, 4};
    v4si b = {5, 6, 7, 8};
    v8hi h = {10, 11, 12, 13, 14, 15, 16, 17};
    v4sf f = {1.0f, 2.0f, 3.0f, 4.0f};
    v4sf g = {5.0f, 6.0f, 7.0f, 8.0f};
    v2df d = {1.0, 2.0};
    v2df e = {3.0, 4.0};
    v2di x = {3, 5};
    v2di y = {6, 7};

    /* 257 as `int`: lane 1, then lane 0 three times. */
    print4("pshufd-257", __builtin_ia32_pshufd(a, 0x8000000000000101ULL));
    /* 17 as `int`: lane 0 from `b`. */
    print4("pblendd-17", __builtin_ia32_pblendd128(a, b, 0x8000000000000011ULL));
    /* Negative values: -1 is 0xff, -255 is 0x01. */
    print4("pshufd-neg1", __builtin_ia32_pshufd(a, -1));
    print4("pshufd-neg255", __builtin_ia32_pshufd(a, -255));
    /* Above a byte: 0x31b encodes 0x1b, which reverses the lanes. */
    print4("pshufd-0x31b", __builtin_ia32_pshufd(a, 0x31b));
    print8("pshuflw-wide", __builtin_ia32_pshuflw(h, 0x700000000000011bLL));
    print8("pshufhw-wide", __builtin_ia32_pshufhw(h, 0x100001b));
    /* 64-bit values whose low 32 bits are the `int`. */
    v4sf s = __builtin_ia32_shufps(f, g, 0xffffffff0000004eULL);
    printf("shufps-wide %g %g %g %g\n", s[0], s[1], s[2], s[3]);
    v2df t = __builtin_ia32_shufpd(d, e, 0x100000001ULL);
    printf("shufpd-wide %g %g\n", t[0], t[1]);
    print8("pblendw-wide", __builtin_ia32_pblendw128(h, h + 100, 0xaaaaaaaa000000f0ULL));
    printf("vec-ext-wide %d\n", __builtin_ia32_vec_ext_v4si(a, 0x100000002ULL));
    /* A named instruction: 0x11 multiplies the high halves. */
    v2di p = __builtin_ia32_pclmulqdq128(x, y, 0x8000000000000011ULL);
    printf("pclmul-wide %lld %lld\n", p[0], p[1]);
    return 0;
}
