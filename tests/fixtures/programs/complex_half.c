/* _Complex _Float16 on every back end. GCC and Clang compute it in
   float _Complex, multiplying and dividing with __mulsc3 and __divsc3, and
   keep or drop the excess where they do for a real _Float16: GCC keeps it
   until a conversion to the complex half type, Clang only along chains of
   + - * /. The program prints bits and values; the test compares them with
   the same program built by the compiler bcc's identity names. */
#include <stdio.h>
#include <string.h>

typedef _Float16 H;
typedef _Complex _Float16 C;

static unsigned cb(C c) {
    unsigned u;
    memcpy(&u, &c, 4);
    return u;
}

static C mk(float r, float i) {
    C c;
    __real__ c = r;
    __imag__ c = i;
    return c;
}

volatile int one = 1;
/* (1 + 2^-10)^2 needs 21 bits: its excess and its binary16 value differ. */
C a, b;
H h = 1.0009765625f;
static C takec(C c) { return c; }
static _Complex float takef(_Complex float c) { return c; }
#define X (a * b)

static void contexts(void) {
    printf("product %08x %08x %08x\n", cb(X), cb(a * a), cb(a / b));
    _Complex float f = X;
    printf("to float %a %a\n", (double)__real__ f, (double)__imag__ f);
    _Complex double d = X * a;
    printf("chain %a %a\n", (double)__real__ d, (double)__imag__ d);
    printf("parts %a %a\n", (double)__real__ (a * a), (double)__imag__ (a * a));
    printf("compare %d %d %d\n", X == mk(2.001953125f, 0), X != X, a * a == a * a);
    printf("truth %d %d\n", (a * a - a * a) ? 1 : 0, !(X - mk(2.001953125f, 0)));
    printf("conditional %08x\n", cb((one ? X : a) * a));
    {
        /* GCC's ?: keeps the excess of a complex half arm. */
        _Complex float z = one ? a * a : a;
        _Complex float w = !one ? a : a * a;
        printf("conditional excess %a %a %a\n", (double)__real__ z, (double)__imag__ z,
               (double)__real__ w);
        /* And of a real half arm converted to the complex type. */
        volatile __bf16 bf = 1.0078125f;
        _Complex float r = one ? h * h : a, s = one ? bf * bf : a;
        printf("conditional real %a %a\n", (double)__real__ r, (double)__real__ s);
        /* GCC follows the selected operand of _Generic and
           __builtin_choose_expr to see an excess arm; it rounds a statement
           expression's value. */
        volatile __bf16 wide = 0x1p20f;
        _Complex float g = one ? wide : _Generic(0, int: a + a),
                       k = one ? wide : __builtin_choose_expr(1, a + a, a),
                       t = one ? wide : ({ a + a; });
        printf("conditional selected %a %a %a\n", (double)__real__ g, (double)__real__ k,
               (double)__real__ t);
    }
    printf("comma %08x\n", cb((0, X) * a));
    printf("real operands %08x %08x %08x\n", cb(a * h * h), cb(h * h * a), cb(a + h * h));
    printf("integer %08x %08x\n", cb(a + 2049), cb(2049 * a));
    printf("division %08x %08x\n", cb(a / h), cb(h / a));
    printf("sign %08x %08x\n", cb(-X * a), cb(+X * a));
    printf("arguments %08x %a\n", cb(takec(X * a)), (double)__real__ takef(X * a));
    printf("generic %d %zu %zu\n", _Generic(X, C: 1, _Complex float: 2, default: 3), sizeof(X),
           _Alignof(C));
    {
        C s = a;
        s *= b;
        s += h * h;
        s /= mk(0.5f, 0.25f);
        printf("compound %08x\n", cb(s));
    }
    {
        static C st = (C)1.0009765625f16 * (C)1.0009765625f16 * (C)1.0009765625f16;
        static _Complex float sf = (C)1.0009765625f16 * (C)1.0009765625f16;
        C local = (C)1.0009765625f16 * (C)1.0009765625f16 * (C)1.0009765625f16;
        printf("constants %08x %a %08x\n", cb(st), (double)__real__ sf, cb(local));
    }
    {
        _Complex double mixed = (a * a) + (_Complex double)1;
        float real = __real__ (a * a);
        printf("conversions %a %a %08x\n", __real__ mixed, (double)real, cb((C)(a * a)));
    }
    printf("statement %08x\n", cb(({ X; }) * a));
}

/* Infinities and NaNs recover as Annex G's multiplication and division
   recover them. */
static void special(void) {
    volatile H zero = 0;
    H inf = 1 / zero, nan = zero / zero;
    C x = mk(inf, nan), y = mk(1, 1), z = mk(0, 0);
    printf("special %08x %08x %08x\n", cb(x * y) & 0x7fff7fffu, cb(y / z) & 0x7fff7fffu,
           cb(mk(nan, nan) * y) & 0x7c007c00u);
}

/* The parts convert and narrow as real _Float16 values do. */
static void parts(void) {
    C c = mk(65520.0f, 0x1p-25f);
    _Complex double d = 0.1 + 0.2 * 1.0i;
    C e = d;
    printf("rounding %08x %08x\n", cb(c), cb(e));
    printf("integers %d %d\n", (int)mk(3.75f, 9), (_Bool)mk(0, 0x1p-24f));
}

int main(void) {
    a = mk(1.0009765625f, 1.0f);
    b = mk(1.0009765625f, -1.0f);
    contexts();
    special();
    parts();
    return 0;
}
