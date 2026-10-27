/* _Float16 on every back end: excess precision in each context where GCC
   and Clang keep or drop it, conversions to and from every arithmetic type,
   constants, classification and folded initializers. The program prints
   bits and values; the test compares them with the same program built by
   the compiler bcc's identity names. Nothing here is undefined: no
   out-of-range conversion to an integer, no contraction-sensitive form. */
#include <stdio.h>
#include <string.h>

typedef _Float16 H;

static unsigned bits(H h) {
    unsigned short u;
    memcpy(&u, &h, 2);
    return u;
}

H va = 60000.0f16, vb = 2.0f16, vc = 4.0f16, ve = 0.1f16, v1 = 1.0f16;
volatile int one = 1;
static float takef(float f) { return f; }
static H takeh(H h) { return h; }
#define X (va * vb)

static void contexts(void) {
    printf("compare %d %d %d\n", X / vc < 40000.0f16, X < 70000.0f, X - X == 0);
    printf("truth %d %d\n", (X - X) && 1, (X - X) ? 1 : 0);
    printf("casts %g %g %g\n", (double)(float)X, (double)X, (double)((H)X / vc));
    {
        float f = X;
        printf("assign %g\n", (double)f);
    }
    printf("mixed %g %g\n", (double)(X + 1.0f), X + 1.0);
    printf("arguments %g %g\n", (double)takef(X), (double)takeh(X / vc));
    {
        int i = X / vc;
        printf("integer %d\n", i);
    }
    printf("conditional %g\n", (double)((one ? X : vc) / vc));
    printf("comma %g\n", (double)((0, X) / vc));
    printf("sign %g %g\n", (double)(-X / vc), (double)(+X / vc));
    {
        H h = vc;
        h += X - X;
        printf("compound %g", (double)h);
        float f = 1;
        f += X;
        printf(" %g\n", (double)f);
    }
    printf("operands %g %g %g\n", (double)(va / 70000), (double)(va * 2 / vc),
           (double)(70000 / vc / vc));
    {
        float f = 0.1f16, g = 1.0f16 / 3.0f16, h = ve / 3;
        printf("literals %a %a %a %g\n", f, g, h, (double)(65504.0f16 / 2));
    }
    printf("products %a %a\n", (double)(ve * ve), (double)(float)(ve * ve));
    {
        static H s = 60000.0f16 * 2.0f16 / 4.0f16;
        static float t = 0.1f16 * 0.1f16;
        static H u = 0.1f16 * 3 * 1000;
        printf("statics %g %a %g\n", (double)s, t, (double)u);
    }
    {
        H h = va;
        h++;
        H k = vb;
        --k;
        printf("steps %04x %04x\n", bits(h), bits(k));
    }
    printf("generic %d %zu\n", _Generic(X, H: 1, float: 2, default: 3), sizeof(X));
}

static unsigned long long state = 0x9e3779b97f4a7c15ull;
static unsigned long long next(void) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return state;
}

__attribute__((noinline)) static H from_float(float x) { return x; }
__attribute__((noinline)) static H from_double(double x) { return x; }
__attribute__((noinline)) static H from_long_double(long double x) { return x; }
__attribute__((noinline)) static H from_quad(__float128 x) { return x; }
__attribute__((noinline)) static H from_int(int x) { return x; }
__attribute__((noinline)) static H from_unsigned(unsigned x) { return x; }
__attribute__((noinline)) static H from_long(long x) { return x; }
__attribute__((noinline)) static H from_unsigned_long(unsigned long x) { return x; }
__attribute__((noinline)) static H from_int128(__int128 x) { return x; }
__attribute__((noinline)) static H from_bool(_Bool x) { return x; }
__attribute__((noinline)) static float to_float(H x) { return x; }
__attribute__((noinline)) static double to_double(H x) { return x; }
__attribute__((noinline)) static long double to_long_double(H x) { return x; }
__attribute__((noinline)) static __float128 to_quad(H x) { return x; }
__attribute__((noinline)) static int to_int(H x) { return x; }
__attribute__((noinline)) static long to_long(H x) { return x; }
__attribute__((noinline)) static __int128 to_int128(H x) { return x; }
__attribute__((noinline)) static _Bool to_bool(H x) { return x; }

static void conversions(void) {
    unsigned long long hash = 0;
    for (int i = 0; i < 4000; i++) {
        unsigned long long r = next();
        unsigned fb = (unsigned)r;
        if (i & 1)
            fb = (fb & 0x807fffffu) | (unsigned)(100 + (r >> 40) % 60) << 23;
        float f;
        memcpy(&f, &fb, 4);
        unsigned long long db = next();
        if (i & 1)
            db = (db & 0x800fffffffffffffull) | (1023ull - 30 + (r >> 33) % 50) << 52;
        double d;
        memcpy(&d, &db, 8);
        long double ld = (long double)d * (1 + (long double)(r & 0xffff) / 0x1p40L);
        __float128 q = (__float128)d * (1 + (__float128)(r & 0xffffff) / 0x1p80Q);
        int iv = (int)(r >> 32) >> (r % 31);
        long lv = (long)next() >> (r % 63);
        hash = hash * 31 + bits(from_float(f));
        hash = hash * 31 + bits(from_double(d));
        hash = hash * 31 + bits(from_long_double(ld));
        hash = hash * 31 + bits(from_quad(q));
        hash = hash * 31 + bits(from_int(iv)) + bits(from_unsigned((unsigned)iv)) * 3;
        hash = hash * 31 + bits(from_long(lv)) + bits(from_unsigned_long((unsigned long)lv)) * 5;
        hash = hash * 31 + bits(from_int128((__int128)lv * 3)) + bits(from_bool(iv & 1));
        unsigned short hb = (unsigned short)r;
        H h;
        memcpy(&h, &hb, 2);
        if ((hb & 0x7c00) != 0x7c00) {
            float g = to_float(h);
            unsigned gb;
            memcpy(&gb, &g, 4);
            double e = to_double(h);
            unsigned long long eb;
            memcpy(&eb, &e, 8);
            hash = hash * 31 + gb + eb;
            hash = hash * 31 + (unsigned long long)(long long)(to_long_double(h) * 1024);
            hash = hash * 31 + (unsigned long long)(long long)(to_quad(h) * 1024);
            hash = hash * 31 + (unsigned)to_int(h) + to_bool(h);
            hash = hash * 31 + (unsigned long)to_long(h) + (unsigned long long)to_int128(h);
        }
    }
    printf("conversions %016llx\n", hash);
    float edges[] = {0x1.002p0f, 0x1.0021p0f, 65519.0f, 65520.0f, 0x1p-24f, 0x1.8p-25f, 0x1p-25f, -0.0f};
    for (int i = 0; i < 8; i++)
        printf("%04x ", bits(from_float(edges[i])));
    printf("\nrounding %04x %04x %04x\n", bits(from_double(1.00048828125000001)),
           bits(from_double(0x1.0020000000001p0)), bits(1.0004882821813226f16));
}

volatile H zero = 0, tiny = 0x1p-24f16;

static void classification(void) {
    H nan = zero / zero, infinity = v1 / zero;
    printf("classes %d %d %d %d %d %d %d\n", __builtin_isnan(nan), __builtin_isinf(infinity),
           __builtin_isfinite(v1), __builtin_signbit(-v1) != 0, __builtin_isnormal(tiny),
           __builtin_isnormal(v1), __builtin_fpclassify(0, 1, 2, 3, 4, tiny));
    printf("limits %d %d %g %zu %zu\n", __FLT16_MANT_DIG__, __FLT16_MAX_EXP__,
           (double)__FLT16_MAX__, sizeof(H), _Alignof(H));
}

/* GCC tests the excess of a product that rounds to zero, converts it to a
   complex type directly, classifies a folded constant as a binary16 value
   and reads an underflowing constant as zero. */
static void excess_uses(void) {
    int a = (tiny * tiny) ? 1 : 0, b = 0, c = (tiny * tiny) && 1, d = (tiny * tiny) || 0;
    int e = !(tiny * tiny), f = 0;
    if (tiny * tiny)
        b = 1;
    while (tiny * tiny) {
        f = 1;
        break;
    }
    H g = (tiny * tiny) ?: vc;
    printf("truth %d %d %d %d %d %d %04x\n", a, b, c, d, e, f, bits(g));
    double _Complex z = (double _Complex)X;
    float _Complex w = X;
    printf("complex %g %g\n", __real__ z, (double)__real__ w);
    printf("folded %d %d %d\n", __builtin_isnormal(0x1p-24f16), __builtin_isnormal(0x1p-14f16),
           __builtin_fpclassify(0, 1, 2, 3, 4, 0x1p-15f16));
    H u = 1e-99999f16, v = -1e-99999f16, o = 1e99999f16;
    printf("underflow %04x %04x %04x\n", bits(u), bits(v), bits(o));
}

int main(void) {
    contexts();
    conversions();
    classification();
    excess_uses();
    return 0;
}
