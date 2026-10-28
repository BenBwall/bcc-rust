/* __bf16 on every back end: excess precision in each context where GCC
   and Clang keep or drop it, conversions to and from every arithmetic type
   (Clang rounds an integer through float, GCC once), the mix with
   _Float16, classification and folded initializers. GCC's bf16 constants,
   which Clang lacks, are tested where __BFLT16_MANT_DIG__ says they exist.
   The program prints bits and values; the test compares them with the same
   program built by the compiler bcc's identity names. Nothing here is
   undefined: no out-of-range conversion to an integer, no contraction-
   sensitive form, and no signaling NaN, which GCC quiets when it widens one
   and Clang does not. */
#include <stdio.h>
#include <string.h>

typedef __bf16 B;

static unsigned bits(B b) {
    unsigned short u;
    memcpy(&u, &b, 2);
    return u;
}

static unsigned hbits(_Float16 h) {
    unsigned short u;
    memcpy(&u, &h, 2);
    return u;
}

/* 1 + 2^-7 squared is 1 + 2^-6 + 2^-14 in float and 1 + 2^-6 in __bf16. */
B va = 1.0078125f, vb = 1.0078125f, vc = 4.0f, ve = 0.1f, v1 = 1.0f;
_Float16 vh = 1.0009765625f;
volatile int one = 1;
static float takef(float f) { return f; }
static B takeb(B b) { return b; }
#define X (va * vb)

static void contexts(void) {
    printf("compare %d %d %d\n", X / vc < (B)0.25390625f, X > 1.015625f, X - (B)1.015625f == 0);
    printf("truth %d %d\n", (X - (B)1.015625f) && 1, (X - (B)1.015625f) ? 1 : 0);
    printf("casts %a %a %a\n", (double)(float)X, (double)X, (double)((B)X / vc));
    {
        float f = X;
        printf("assign %a\n", (double)f);
    }
    printf("mixed %a %a\n", (double)(X + 1.0f), X + 1.0);
    printf("arguments %a %a\n", (double)takef(X), (double)takeb(X / vc));
    {
        int i = X * 100;
        printf("integer %d\n", i);
    }
    printf("conditional %a\n", (double)((one ? X : vc) / vc));
    printf("comma %a\n", (double)((0, X) / vc));
    printf("sign %a %a\n", (double)(-X / vc), (double)(+X / vc));
    {
        B b = vc;
        b += X - (B)1.015625f;
        printf("compound %a", (double)b);
        float f = 1;
        f += X;
        printf(" %a\n", (double)f);
    }
    printf("operands %a %a %a\n", (double)(va + 257), (double)(va * 2 / vc),
           (double)(257 / vc / vc));
    {
        float f = (B)0.1f, h = ve / 3;
        printf("literals %a %a\n", f, h);
    }
    printf("products %a %a\n", (double)(ve * ve), (double)(float)(ve * ve));
    {
        static B s = (B)1.0078125f * (B)1.0078125f / (B)4.0f;
        static float t = (B)0.1f * (B)0.1f;
        static B u = (B)0.1f * 3 * 1000;
        printf("statics %a %a %a\n", (double)s, t, (double)u);
    }
    {
        B b = (B)255.0f;
        b++;
        B k = vb;
        --k;
        printf("steps %04x %04x\n", bits(b), bits(k));
    }
    printf("generic %d %zu\n", _Generic(X, B: 1, float: 2, default: 3), sizeof(X));
    /* `_Float16` outranks `__bf16`; GCC keeps both excesses, Clang rounds
       each operand to its own type and the __bf16 one to _Float16. */
    printf("half %a %a %d %04x\n", (double)(va * vb + vh), (double)(vh * vh + va),
           _Generic(va + vh, _Float16: 1, B: 2, default: 3), hbits((_Float16)X));
    printf("from half %04x %04x\n", bits((B)vh), bits((B)(vh * vh)));
    /* GCC evaluates __bf16 + _Float16 in float, so a __bf16 beyond
       binary16's range stays finite; Clang converts it to _Float16 first. */
    {
        volatile B big = 0x1p20f;
        volatile _Float16 h1 = 1;
        printf("range %a %a %d %a\n", (double)(big + h1), (double)(h1 - big), big > h1,
               (double)(one ? big : h1));
    }
#ifdef __BFLT16_MANT_DIG__
    {
        float f = 0.1bf16, g = 1.0bf16 / 3.0BF16, h = 0x1.01p0bf16;
        static B s = 1.0078125bf16 * 1.0078125bf16 / 4.0bf16;
        static float t = 0.1bf16 * 0.1bf16;
        printf("gcc constants %a %a %a %a %a %d\n", f, g, h, (double)s, t,
               _Generic(1.0bf16, B: 1, default: 2));
    }
#endif
}

static unsigned long long state = 0x9e3779b97f4a7c15ull;
static unsigned long long next(void) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return state;
}

__attribute__((noinline)) static B from_float(float x) { return x; }
__attribute__((noinline)) static B from_double(double x) { return x; }
__attribute__((noinline)) static B from_long_double(long double x) { return x; }
__attribute__((noinline)) static B from_quad(__float128 x) { return x; }
__attribute__((noinline)) static B from_half(_Float16 x) { return x; }
__attribute__((noinline)) static B from_int(int x) { return x; }
__attribute__((noinline)) static B from_unsigned(unsigned x) { return x; }
__attribute__((noinline)) static B from_long(long x) { return x; }
__attribute__((noinline)) static B from_unsigned_long(unsigned long x) { return x; }
__attribute__((noinline)) static B from_int128(__int128 x) { return x; }
__attribute__((noinline)) static B from_short(short x) { return x; }
__attribute__((noinline)) static B from_bool(_Bool x) { return x; }
__attribute__((noinline)) static float to_float(B x) { return x; }
__attribute__((noinline)) static double to_double(B x) { return x; }
__attribute__((noinline)) static long double to_long_double(B x) { return x; }
__attribute__((noinline)) static __float128 to_quad(B x) { return x; }
__attribute__((noinline)) static _Float16 to_half(B x) { return x; }
__attribute__((noinline)) static int to_int(B x) { return x; }
__attribute__((noinline)) static long to_long(B x) { return x; }
__attribute__((noinline)) static __int128 to_int128(B x) { return x; }
__attribute__((noinline)) static _Bool to_bool(B x) { return x; }

static void conversions(void) {
    unsigned long long hash = 0;
    for (int i = 0; i < 4000; i++) {
        unsigned long long r = next();
        unsigned fb = (unsigned)r;
        if (i & 1)
            fb = (fb & 0x807fffffu) | (unsigned)(100 + (r >> 40) % 60) << 23;
        float f;
        memcpy(&f, &fb, 4);
        if (f != f)
            f = 1;
        unsigned long long db = next();
        if (i & 1)
            db = (db & 0x800fffffffffffffull) | (1023ull - 30 + (r >> 33) % 50) << 52;
        double d;
        memcpy(&d, &db, 8);
        if (d != d)
            d = 1;
        long double ld = (long double)d * (1 + (long double)(r & 0xffff) / 0x1p40L);
        __float128 q = (__float128)d * (1 + (__float128)(r & 0xffffff) / 0x1p80Q);
        int iv = (int)(r >> 32) >> (r % 31);
        long lv = (long)next() >> (r % 63);
        unsigned short hb = (unsigned short)(r >> 17);
        _Float16 h;
        memcpy(&h, &hb, 2);
        hash = hash * 31 + bits(from_float(f));
        hash = hash * 31 + bits(from_double(d));
        hash = hash * 31 + bits(from_long_double(ld));
        hash = hash * 31 + bits(from_quad(q));
        if ((hb & 0x7c00) != 0x7c00)
            hash = hash * 31 + bits(from_half(h));
        hash = hash * 31 + bits(from_int(iv)) + bits(from_unsigned((unsigned)iv)) * 3;
        hash = hash * 31 + bits(from_long(lv)) + bits(from_unsigned_long((unsigned long)lv)) * 5;
        hash = hash * 31 + bits(from_int128((__int128)lv << (r % 60)));
        hash = hash * 31 + bits(from_short((short)iv)) + bits(from_bool(iv & 1));
        unsigned short bb = (unsigned short)r;
        B b;
        memcpy(&b, &bb, 2);
        unsigned exponent = (bb >> 7) & 0xff;
        if (exponent != 0xff) {
            float g = to_float(b);
            unsigned gb;
            memcpy(&gb, &g, 4);
            double e = to_double(b);
            unsigned long long eb;
            memcpy(&eb, &e, 8);
            hash = hash * 31 + gb + eb + hbits(to_half(b));
            hash = hash * 31 + to_bool(b);
            if (exponent < 127 + 50) {
                hash = hash * 31 + (unsigned long long)(long long)(to_long_double(b) * 1024);
                hash = hash * 31 + (unsigned long long)(long long)(to_quad(b) * 1024);
            }
            if (exponent < 127 + 31)
                hash = hash * 31 + (unsigned)to_int(b);
            if (exponent < 127 + 63)
                hash = hash * 31 + (unsigned long)to_long(b);
            if (exponent < 127 + 127)
                hash = hash * 31 + (unsigned long long)to_int128(b);
        }
    }
    printf("conversions %016llx\n", hash);
    /* Ties and sticky bits that rounding through float loses. */
    printf("integers %04x %04x %04x %04x %04x\n", bits(from_int(0x1008001)),
           bits(from_unsigned(0x80800001u)), bits(from_long(0x4040000000000001l)),
           bits(from_unsigned_long(0x8080000000000001ul)),
           bits(from_int128((__int128)0x8080000000000001ul << 40)));
    float edges[] = {0x1.01p0f, 0x1.0101p0f, 0x1.fffp127f, 0x1p-133f, 0x1.8p-134f, 0x1p-134f, -0.0f};
    for (int i = 0; i < 7; i++)
        printf("%04x ", bits(from_float(edges[i])));
    printf("\nrounding %04x %04x %04x\n", bits(from_double(1.00390625000000001)),
           bits(from_double(0x1.0100000000001p0)), bits((B)1.0039062500000002));
    /* Folded casts round once from binary128 and the x87 format, as the
       run-time conversions do. */
    printf("folded %04x %04x %04x %04x %04x\n", bits((B)0x1.0100000000001p0Q),
           bits(from_quad(0x1.0100000000001p0Q)), bits((B)0x1.0100000000001p0L),
           hbits((_Float16)0x1.0020000000001p0Q), hbits((_Float16)0x1.0020000000001p0L));
    /* And from a 128-bit integer, whose last bit decides the tie. */
#define WIDE (((__int128)0x8080000000000000ull << 32) + 1)
    {
        static B wide = (B)WIDE;
        static float single = WIDE;
        printf("folded integers %04x %04x %04x %a\n", bits(wide), bits((B)WIDE),
               bits(from_int128(WIDE)), (double)single);
    }
    /* GCC folds a cast of a literal as it parses it, rounding a conversion
       to _Float16 it leaves unrounded for a negated or computed operand. */
    printf("parse folding %a %a %a %a\n", (double)(long double)(_Float16)(B)0x1p20Q,
           (double)(long double)(_Float16)(B)-0x1p20Q, (double)(long double)(_Float16)(B)(0x1p20Q * 1),
           (double)(long double)(one ? (B)-0x1p20Q : vh));
    /* Through the selections of _Generic and __builtin_choose_expr, and a
       cast of such a cast, which GCC folds as well. */
    printf("parse selections %a %a %a %a\n",
           (double)(long double)(_Float16)_Generic(0, int: (B)0x1p20Q),
           (double)(long double)(_Float16)__builtin_choose_expr(1, (B)0x1p20Q, (B)0),
           (double)(long double)(_Float16)(B)(B)0x1p20Q,
           (double)(long double)(_Float16)_Generic(0, int: (B)-0x1p20Q));
    /* GCC folds a cast whose operand is already a constant: the selected
       operand of a constant ?:, arithmetic converted to float, and
       __builtin_complex; not arithmetic itself. */
    printf("parse operands %a %a %a %a\n",
           (double)(long double)(_Float16)(B)(1 ? 0x1p20 : -0x1p20),
           (double)(long double)(_Float16)(B)(float)(0x1p19 + 0x1p19),
           (double)(long double)(_Float16)(B)(0x1p19 + 0x1p19),
           (double)(long double)(B)(_Float16)__builtin_complex((_Float16)0x1.004p0, (_Float16)0));
}

/* small * small is 2^-140: a float subnormal, but zero as a __bf16. */
volatile B zero = 0, tiny = 0x1p-133f, small = 0x1p-70f;

static void classification(void) {
    B nan = zero / zero, infinity = v1 / zero;
    printf("classes %d %d %d %d %d %d %d\n", __builtin_isnan(nan), __builtin_isinf(infinity),
           __builtin_isfinite(v1), __builtin_signbit(-v1) != 0, __builtin_isnormal(tiny),
           __builtin_isnormal(v1), __builtin_fpclassify(0, 1, 2, 3, 4, tiny));
    printf("sizes %zu %zu\n", sizeof(B), _Alignof(B));
}

/* GCC tests the excess of a product that rounds to zero and converts it to
   a complex type directly. */
static void excess_uses(void) {
    int a = (small * small) ? 1 : 0, b = 0, c = (small * small) && 1, d = (small * small) || 0;
    int e = !(small * small);
    if (small * small)
        b = 1;
    printf("truth %d %d %d %d %d\n", a, b, c, d, e);
    double _Complex z = (double _Complex)X;
    float _Complex w = X;
    printf("complex %a %a\n", __real__ z, (double)__real__ w);
    printf("statement %a %a\n", (double)(({ X; }) / vc), (double)(_Generic(0, int: X) / vc));
}

int main(void) {
    contexts();
    conversions();
    classification();
    excess_uses();
    return 0;
}
