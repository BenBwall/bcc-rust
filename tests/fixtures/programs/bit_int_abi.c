/* Bit-precise integers passed between separately compiled halves: the
 * definitions (CALLEE_ONLY) and the calls that check them (CALLER_ONLY),
 * each built by bcc, Clang or GCC. Without either macro the program is
 * whole. The values set every padding bit's neighbour so that a callee or
 * caller assuming extended padding bits, or writing unextended ones where
 * the other reads them, prints a wrong value. */
#include <stdarg.h>
#include <stdio.h>

typedef unsigned __int128 u128;

struct mixed {
    char tag;
    _BitInt(65) wide;
    unsigned _BitInt(7) small;
    _BitInt(33) middle;
};

struct pair {
    _BitInt(63) a;
    unsigned _BitInt(15) b;
};

_BitInt(2) negate2(_BitInt(2) x);
unsigned _BitInt(7) add7(unsigned _BitInt(7) a, unsigned _BitInt(7) b);
_BitInt(8) twice8(_BitInt(8) x);
_BitInt(15) sub15(_BitInt(15) a, _BitInt(15) b);
unsigned _BitInt(17) mask17(unsigned _BitInt(17) x);
_BitInt(33) add33(_BitInt(33) a, _BitInt(33) b);
unsigned _BitInt(63) xor63(unsigned _BitInt(63) a, unsigned _BitInt(63) b);
_BitInt(64) neg64(_BitInt(64) x);
_BitInt(65) add65(_BitInt(65) a, _BitInt(65) b);
unsigned _BitInt(127) shl127(unsigned _BitInt(127) x, int n);
_BitInt(128) mul128(_BitInt(128) a, _BitInt(128) b);
_BitInt(65) spill(int a, int b, int c, int d, int e, _BitInt(65) x, int f, _BitInt(128) y);
_BitInt(33) many(_BitInt(33) a, _BitInt(33) b, _BitInt(33) c, _BitInt(33) d, _BitInt(33) e,
                 _BitInt(33) f, _BitInt(33) g, _BitInt(33) h);
struct mixed make_mixed(_BitInt(65) wide, unsigned _BitInt(7) small, _BitInt(33) middle);
_BitInt(65) sum_mixed(struct mixed m);
struct pair make_pair(_BitInt(63) a, unsigned _BitInt(15) b);
_BitInt(128) sum_varargs(int count, ...);
_BitInt(65) apply(_BitInt(65) (*f)(_BitInt(65), _BitInt(65)), _BitInt(65) a, _BitInt(65) b);
_BitInt(65) callback_sum(_BitInt(65) a, _BitInt(65) b);

static void p(const char *tag, u128 v) {
    printf("%s %016llx%016llx\n", tag, (unsigned long long)(v >> 64), (unsigned long long)v);
}

#ifndef CALLER_ONLY
_BitInt(2) negate2(_BitInt(2) x) { return -x; }
unsigned _BitInt(7) add7(unsigned _BitInt(7) a, unsigned _BitInt(7) b) { return a + b; }
_BitInt(8) twice8(_BitInt(8) x) { return x * 2; }
_BitInt(15) sub15(_BitInt(15) a, _BitInt(15) b) { return a - b; }
unsigned _BitInt(17) mask17(unsigned _BitInt(17) x) { return ~x; }
_BitInt(33) add33(_BitInt(33) a, _BitInt(33) b) { return a + b; }
unsigned _BitInt(63) xor63(unsigned _BitInt(63) a, unsigned _BitInt(63) b) { return a ^ b; }
_BitInt(64) neg64(_BitInt(64) x) { return -x; }
_BitInt(65) add65(_BitInt(65) a, _BitInt(65) b) { return a + b; }
unsigned _BitInt(127) shl127(unsigned _BitInt(127) x, int n) { return x << n; }
_BitInt(128) mul128(_BitInt(128) a, _BitInt(128) b) { return a * b; }

_BitInt(65) spill(int a, int b, int c, int d, int e, _BitInt(65) x, int f, _BitInt(128) y) {
    return x + a + b + c + d + e + f + (_BitInt(65))(y >> 60);
}

_BitInt(33) many(_BitInt(33) a, _BitInt(33) b, _BitInt(33) c, _BitInt(33) d, _BitInt(33) e,
                 _BitInt(33) f, _BitInt(33) g, _BitInt(33) h) {
    return a - b + c - d + e - f + g - h;
}

struct mixed make_mixed(_BitInt(65) wide, unsigned _BitInt(7) small, _BitInt(33) middle) {
    struct mixed m = {'m', wide, small, middle};
    return m;
}

_BitInt(65) sum_mixed(struct mixed m) { return m.wide + m.small + m.middle + m.tag; }

struct pair make_pair(_BitInt(63) a, unsigned _BitInt(15) b) {
    struct pair r = {a, b};
    return r;
}

_BitInt(128) sum_varargs(int count, ...) {
    va_list ap;
    va_start(ap, count);
    _BitInt(128) sum = 0;
    for (int i = 0; i < count; i++) {
        switch (i % 4) {
        case 0: sum += va_arg(ap, _BitInt(65)); break;
        case 1: sum += va_arg(ap, _BitInt(33)); break;
        case 2: sum += va_arg(ap, unsigned _BitInt(128)); break;
        default: sum += va_arg(ap, _BitInt(15)); break;
        }
    }
    va_end(ap);
    return sum;
}

_BitInt(65) apply(_BitInt(65) (*f)(_BitInt(65), _BitInt(65)), _BitInt(65) a, _BitInt(65) b) {
    return f(a, b) * 3;
}
#endif

#ifndef CALLEE_ONLY
_BitInt(65) callback_sum(_BitInt(65) a, _BitInt(65) b) { return a - b; }

int main(void) {
    volatile int k = 3;
    p("negate2", (u128)negate2((_BitInt(2))k - 2));
    p("add7", (u128)add7(100, 50));
    p("twice8", (u128)twice8(-60));
    p("sub15", (u128)sub15(-16000, 300));
    p("mask17", (u128)mask17(5));
    p("add33", (u128)add33(-4294967290LL, -k));
    p("xor63", (u128)xor63(0x7fffffffffffffffULL, 0x5555555555555555ULL));
    p("neg64", (u128)neg64(-0x7fffffffffffffffLL));
    p("add65", (u128)add65(-((_BitInt(65))1 << 63) * 2 + 1, -k + 2));
    p("shl127", (u128)shl127(3, 125));
    p("mul128", (u128)mul128(-(_BitInt(128))0x1234567890abcdefLL, (_BitInt(128))1 << 64));
    p("spill", (u128)spill(1, 2, 3, 4, 5, -((_BitInt(65))1 << 62), 6, -((_BitInt(128))1 << 100)));
    p("many", (u128)many(1, -2, 3, -4, 5, -6, 7, (_BitInt(33))-4294967000LL));
    struct mixed m = make_mixed(-((_BitInt(65))1 << 63) * 2, 127, -k);
    p("mixed.tag", (u128)m.tag);
    p("mixed.wide", (u128)m.wide);
    p("mixed.small", (u128)m.small);
    p("mixed.middle", (u128)m.middle);
    m.wide = (_BitInt(65))1 << 63;
    p("sum_mixed", (u128)sum_mixed(m));
    struct pair pr = make_pair(-((_BitInt(63))1 << 61), 32767);
    p("pair.a", (u128)pr.a);
    p("pair.b", (u128)pr.b);
    p("varargs", (u128)sum_varargs(8, (_BitInt(65))-1, (_BitInt(33))-2, (unsigned _BitInt(128))3 << 100,
                                   (_BitInt(15))-4, (_BitInt(65))1 << 63, (_BitInt(33))4294967295LL,
                                   (unsigned _BitInt(128))-1, (_BitInt(15))-16384));
    p("apply", (u128)apply(callback_sum, (_BitInt(65))1 << 62, -k));
    p("apply_callee", (u128)apply(add65, -((_BitInt(65))1 << 62), -k));
    return 0;
}
#endif
