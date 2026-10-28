/* The compiler-rt routines the WebAssembly back end calls: 128-bit integer
   multiplication, division and shifts, and conversions between __int128
   and the floating types, written on 64-bit halves so that none of them
   calls itself. Their names and signatures are compiler-rt's, as Clang
   calls them on wasm32 (an __int128 result is returned through a pointer
   passed first). */
#include <stdint.h>

typedef __int128 ti_int;
typedef unsigned __int128 tu_int;

typedef union {
    tu_int all;
    struct {
        uint64_t low, high;
    } s;
} twords;

static tu_int make(uint64_t high, uint64_t low) {
    twords r;
    r.s.low = low;
    r.s.high = high;
    return r.all;
}

static uint64_t low_of(tu_int a) {
    twords w;
    w.all = a;
    return w.s.low;
}

static uint64_t high_of(tu_int a) {
    twords w;
    w.all = a;
    return w.s.high;
}

/* The 128-bit product of two 64-bit values. */
static void multiply64(uint64_t a, uint64_t b, uint64_t *high, uint64_t *low) {
    uint64_t a0 = (uint32_t)a, a1 = a >> 32, b0 = (uint32_t)b, b1 = b >> 32;
    uint64_t p00 = a0 * b0, p01 = a0 * b1, p10 = a1 * b0, p11 = a1 * b1;
    uint64_t middle = (p00 >> 32) + (uint32_t)p01 + (uint32_t)p10;
    *low = (middle << 32) | (uint32_t)p00;
    *high = p11 + (p01 >> 32) + (p10 >> 32) + (middle >> 32);
}

ti_int __multi3(ti_int a, ti_int b) {
    uint64_t al = low_of((tu_int)a), ah = high_of((tu_int)a);
    uint64_t bl = low_of((tu_int)b), bh = high_of((tu_int)b);
    uint64_t high, low;
    multiply64(al, bl, &high, &low);
    high += al * bh + ah * bl;
    return (ti_int)make(high, low);
}

ti_int __ashlti3(ti_int a, int b) {
    uint64_t low = low_of((tu_int)a), high = high_of((tu_int)a);
    b &= 127;
    if (b == 0)
        return a;
    if (b >= 64)
        return (ti_int)make(low << (b - 64), 0);
    return (ti_int)make((high << b) | (low >> (64 - b)), low << b);
}

tu_int __lshrti3(tu_int a, int b) {
    uint64_t low = low_of(a), high = high_of(a);
    b &= 127;
    if (b == 0)
        return a;
    if (b >= 64)
        return make(0, high >> (b - 64));
    return make(high >> b, (low >> b) | (high << (64 - b)));
}

ti_int __ashrti3(ti_int a, int b) {
    uint64_t low = low_of((tu_int)a), high = high_of((tu_int)a);
    int64_t signed_high = (int64_t)high;
    b &= 127;
    if (b == 0)
        return a;
    if (b >= 64)
        return (ti_int)make((uint64_t)(signed_high >> 63), (uint64_t)(signed_high >> (b - 64)));
    return (ti_int)make((uint64_t)(signed_high >> b), (low >> b) | (high << (64 - b)));
}

/* Unsigned division with remainder by shift and subtract. */
static tu_int divide(tu_int n, tu_int d, tu_int *remainder) {
    uint64_t nh = high_of(n), nl = low_of(n), dh = high_of(d), dl = low_of(d);
    if (dh == 0 && nh == 0) {
        if (remainder)
            *remainder = make(0, nl % dl);
        return make(0, nl / dl);
    }
    uint64_t qh = 0, ql = 0, rh = 0, rl = 0;
    for (int i = 127; i >= 0; i--) {
        /* r = (r << 1) | bit i of n */
        rh = (rh << 1) | (rl >> 63);
        rl = (rl << 1) | ((i >= 64 ? nh >> (i - 64) : nl >> i) & 1);
        if (rh > dh || (rh == dh && rl >= dl)) {
            uint64_t borrow = rl < dl;
            rl -= dl;
            rh -= dh + borrow;
            if (i >= 64)
                qh |= 1ull << (i - 64);
            else
                ql |= 1ull << i;
        }
    }
    if (remainder)
        *remainder = make(rh, rl);
    return make(qh, ql);
}

static tu_int negate(tu_int a) {
    uint64_t low = low_of(a), high = high_of(a);
    return make(~high + (low == 0), ~low + 1);
}

static int negative(ti_int a) { return (int64_t)high_of((tu_int)a) < 0; }

tu_int __udivti3(tu_int a, tu_int b) { return divide(a, b, 0); }

tu_int __umodti3(tu_int a, tu_int b) {
    tu_int r;
    divide(a, b, &r);
    return r;
}

ti_int __divti3(ti_int a, ti_int b) {
    int sign = negative(a) != negative(b);
    tu_int ua = negative(a) ? negate((tu_int)a) : (tu_int)a;
    tu_int ub = negative(b) ? negate((tu_int)b) : (tu_int)b;
    tu_int q = divide(ua, ub, 0);
    return (ti_int)(sign ? negate(q) : q);
}

ti_int __modti3(ti_int a, ti_int b) {
    tu_int ua = negative(a) ? negate((tu_int)a) : (tu_int)a;
    tu_int ub = negative(b) ? negate((tu_int)b) : (tu_int)b;
    tu_int r;
    divide(ua, ub, &r);
    return (ti_int)(negative(a) ? negate(r) : r);
}

static int leading_zeros64(uint64_t x) {
    int n = 0;
    if (!x)
        return 64;
    while (!(x >> 63)) {
        x <<= 1;
        n++;
    }
    return n;
}

/* An unsigned 128-bit value as a double, correctly rounded: the top 64
   bits with the rest folded into the last bit, which only breaks ties. */
static double unsigned_to_double(uint64_t high, uint64_t low) {
    if (high == 0)
        return (double)low;
    int shift = leading_zeros64(high);
    uint64_t top = shift ? (high << shift) | (low >> (64 - shift)) : high;
    uint64_t rest = shift ? low << shift : low;
    top |= rest != 0;
    double scale = 1.0;
    for (int i = 0; i < 64 - shift; i++)
        scale *= 2.0;
    return (double)top * scale;
}

static float unsigned_to_float(uint64_t high, uint64_t low) {
    if (high == 0)
        return (float)low;
    int shift = leading_zeros64(high);
    uint64_t top = shift ? (high << shift) | (low >> (64 - shift)) : high;
    uint64_t rest = shift ? low << shift : low;
    top |= rest != 0;
    float scale = 1.0f;
    for (int i = 0; i < 64 - shift; i++)
        scale *= 2.0f;
    return (float)top * scale;
}

double __floatuntidf(tu_int a) { return unsigned_to_double(high_of(a), low_of(a)); }
float __floatuntisf(tu_int a) { return unsigned_to_float(high_of(a), low_of(a)); }

double __floattidf(ti_int a) {
    if (negative(a)) {
        tu_int m = negate((tu_int)a);
        return -unsigned_to_double(high_of(m), low_of(m));
    }
    return unsigned_to_double(high_of((tu_int)a), low_of((tu_int)a));
}

float __floattisf(ti_int a) {
    if (negative(a)) {
        tu_int m = negate((tu_int)a);
        return -unsigned_to_float(high_of(m), low_of(m));
    }
    return unsigned_to_float(high_of((tu_int)a), low_of((tu_int)a));
}

/* A non-negative double's integer part as an unsigned 128-bit value;
   out-of-range values saturate. */
static tu_int double_to_unsigned(double x) {
    if (!(x >= 1.0))
        return 0;
    union {
        double d;
        uint64_t u;
    } v = {x};
    int exponent = (int)((v.u >> 52) & 0x7FF) - 1075;
    uint64_t m = (v.u & ((1ull << 52) - 1)) | (1ull << 52);
    if (exponent >= 128 - 53)
        return make(~0ull, ~0ull);
    if (exponent <= 0)
        return make(0, m >> -exponent);
    if (exponent >= 64)
        return make(m << (exponent - 64), 0);
    return make(exponent > 11 ? m >> (64 - exponent) : 0, m << exponent);
}

tu_int __fixunsdfti(double x) { return double_to_unsigned(x); }
tu_int __fixunssfti(float x) { return double_to_unsigned(x); }

ti_int __fixdfti(double x) {
    if (x < 0)
        return (ti_int)negate(double_to_unsigned(-x));
    return (ti_int)double_to_unsigned(x);
}

ti_int __fixsfti(float x) { return __fixdfti(x); }

/* IEEE binary128, which long double is on wasm32: arithmetic, comparisons
   and conversions as compiler-rt's soft-float routines compute them,
   rounding to nearest with ties to even. The significand has 112 stored
   bits, the exponent 15 with a bias of 16383. */

#define Q_SIGNIFICAND_BITS 112
#define Q_EXPONENT_MAX 0x7FFF
#define Q_BIAS 16383

typedef union {
    long double f;
    tu_int u;
} qbits;

static tu_int q_bits(long double x) {
    qbits v;
    v.f = x;
    return v.u;
}

static long double q_value(tu_int u) {
    qbits v;
    v.u = u;
    return v.f;
}

static tu_int q_sign_bit(void) { return make(1ull << 63, 0); }
static tu_int q_implicit(void) { return make(1ull << 48, 0); }
static tu_int q_significand_mask(void) { return make((1ull << 48) - 1, ~0ull); }
static tu_int q_infinity(void) { return make(0x7FFFull << 48, 0); }
static tu_int q_quiet_bit(void) { return make(1ull << 47, 0); }

static int leading_zeros128(tu_int a) {
    uint64_t high = high_of(a);
    return high ? leading_zeros64(high) : 64 + leading_zeros64(low_of(a));
}

/* Normalizes a subnormal significand, returning its exponent. */
static int q_normalize(tu_int *significand) {
    int shift = leading_zeros128(*significand) - leading_zeros128(q_implicit());
    *significand <<= shift;
    return 1 - shift;
}

/* Rounds and packs a result whose significand carries three extra low
   bits (guard, round, sticky) with the implicit bit at 115. */
static long double q_round_pack(tu_int sign, int exponent, tu_int significand) {
    if (exponent >= Q_EXPONENT_MAX)
        return q_value(q_infinity() | sign);
    if (exponent <= 0) {
        int shift = 1 - exponent;
        if (shift >= 128) {
            significand = significand != 0;
        } else {
            tu_int lost = significand << (128 - shift);
            significand = (significand >> shift) | (lost != 0);
        }
        exponent = 0;
    }
    int round = (int)(low_of(significand) & 7);
    tu_int result = (significand >> 3) & q_significand_mask();
    result |= (tu_int)exponent << Q_SIGNIFICAND_BITS;
    if (round > 4)
        result++;
    else if (round == 4)
        result += result & 1;
    return q_value(result | sign);
}

long double __addtf3(long double a, long double b) {
    tu_int A = q_bits(a), B = q_bits(b);
    tu_int absA = A & ~q_sign_bit(), absB = B & ~q_sign_bit();
    tu_int infinity = q_infinity();
    if (absA - 1 >= infinity - 1 || absB - 1 >= infinity - 1) {
        if (absA > infinity)
            return q_value(A | q_quiet_bit());
        if (absB > infinity)
            return q_value(B | q_quiet_bit());
        if (absA == infinity)
            return (A ^ B) == q_sign_bit() ? q_value(infinity | q_quiet_bit()) : a;
        if (absB == infinity)
            return b;
        if (absA == 0)
            return absB == 0 ? q_value(A & B) : b;
        if (absB == 0)
            return a;
    }
    if (absB > absA) {
        tu_int t = A;
        A = B;
        B = t;
        t = absA;
        absA = absB;
        absB = t;
    }
    int aExp = (int)(absA >> Q_SIGNIFICAND_BITS), bExp = (int)(absB >> Q_SIGNIFICAND_BITS);
    tu_int aSig = A & q_significand_mask(), bSig = B & q_significand_mask();
    if (aExp == 0)
        aExp = q_normalize(&aSig);
    if (bExp == 0)
        bExp = q_normalize(&bSig);
    tu_int sign = A & q_sign_bit();
    int subtract = ((A ^ B) & q_sign_bit()) != 0;
    aSig = (aSig | q_implicit()) << 3;
    bSig = (bSig | q_implicit()) << 3;
    int align = aExp - bExp;
    if (align) {
        if (align < 128) {
            int sticky = (bSig << (128 - align)) != 0;
            bSig = (bSig >> align) | (tu_int)sticky;
        } else {
            bSig = 1;
        }
    }
    if (subtract) {
        aSig -= bSig;
        if (aSig == 0)
            return q_value(0);
        if (aSig < q_implicit() << 3) {
            int shift = leading_zeros128(aSig) - leading_zeros128(q_implicit() << 3);
            aSig <<= shift;
            aExp -= shift;
        }
    } else {
        aSig += bSig;
        if (aSig & (q_implicit() << 4)) {
            int sticky = (int)(low_of(aSig) & 1);
            aSig = (aSig >> 1) | (tu_int)sticky;
            aExp++;
        }
    }
    return q_round_pack(sign, aExp, aSig);
}

long double __subtf3(long double a, long double b) {
    return __addtf3(a, q_value(q_bits(b) ^ q_sign_bit()));
}

/* The 256-bit product of two 128-bit values, as high and low halves. */
static void multiply128(tu_int a, tu_int b, tu_int *high, tu_int *low) {
    uint64_t a0 = low_of(a), a1 = high_of(a), b0 = low_of(b), b1 = high_of(b);
    uint64_t p00h, p00l, p01h, p01l, p10h, p10l, p11h, p11l;
    multiply64(a0, b0, &p00h, &p00l);
    multiply64(a0, b1, &p01h, &p01l);
    multiply64(a1, b0, &p10h, &p10l);
    multiply64(a1, b1, &p11h, &p11l);
    /* word 1 = p00h + p01l + p10l, with carries into word 2 */
    tu_int word1 = (tu_int)p00h + p01l + p10l;
    tu_int word2 = (tu_int)p01h + p10h + p11l + high_of(word1);
    uint64_t word3 = p11h + high_of(word2);
    *low = make(low_of(word1), p00l);
    *high = make(word3, low_of(word2));
}

long double __multf3(long double a, long double b) {
    tu_int A = q_bits(a), B = q_bits(b);
    int aExp = (int)((A >> Q_SIGNIFICAND_BITS) & Q_EXPONENT_MAX);
    int bExp = (int)((B >> Q_SIGNIFICAND_BITS) & Q_EXPONENT_MAX);
    tu_int sign = (A ^ B) & q_sign_bit();
    tu_int aSig = A & q_significand_mask(), bSig = B & q_significand_mask();
    int scale = 0;
    tu_int infinity = q_infinity();
    if ((unsigned)aExp - 1 >= Q_EXPONENT_MAX - 1 || (unsigned)bExp - 1 >= Q_EXPONENT_MAX - 1) {
        tu_int absA = A & ~q_sign_bit(), absB = B & ~q_sign_bit();
        if (absA > infinity)
            return q_value(A | q_quiet_bit());
        if (absB > infinity)
            return q_value(B | q_quiet_bit());
        if (absA == infinity)
            return absB ? q_value(absA | sign) : q_value(infinity | q_quiet_bit());
        if (absB == infinity)
            return absA ? q_value(absB | sign) : q_value(infinity | q_quiet_bit());
        if (!absA || !absB)
            return q_value(sign);
        if (absA < q_implicit())
            scale += q_normalize(&aSig);
        if (absB < q_implicit())
            scale += q_normalize(&bSig);
    }
    aSig |= q_implicit();
    bSig |= q_implicit();
    tu_int high, low;
    multiply128(aSig, bSig << 15, &high, &low);
    int exponent = aExp + bExp - Q_BIAS + scale;
    if (high & q_implicit()) {
        exponent++;
    } else {
        high = (high << 1) | (low >> 127);
        low <<= 1;
    }
    if (exponent >= Q_EXPONENT_MAX)
        return q_value(infinity | sign);
    if (exponent <= 0) {
        int shift = 1 - exponent;
        if (shift >= 128)
            return q_value(sign);
        int sticky = (low << (128 - shift)) != 0;
        low = (high << (128 - shift)) | (low >> shift) | (tu_int)sticky;
        high >>= shift;
    } else {
        high &= q_significand_mask();
        high |= (tu_int)exponent << Q_SIGNIFICAND_BITS;
    }
    high |= sign;
    if (low > q_sign_bit())
        high++;
    else if (low == q_sign_bit())
        high += high & 1;
    return q_value(high);
}

long double __divtf3(long double a, long double b) {
    tu_int A = q_bits(a), B = q_bits(b);
    int aExp = (int)((A >> Q_SIGNIFICAND_BITS) & Q_EXPONENT_MAX);
    int bExp = (int)((B >> Q_SIGNIFICAND_BITS) & Q_EXPONENT_MAX);
    tu_int sign = (A ^ B) & q_sign_bit();
    tu_int aSig = A & q_significand_mask(), bSig = B & q_significand_mask();
    tu_int infinity = q_infinity();
    tu_int absA = A & ~q_sign_bit(), absB = B & ~q_sign_bit();
    if (absA > infinity)
        return q_value(A | q_quiet_bit());
    if (absB > infinity)
        return q_value(B | q_quiet_bit());
    if (absA == infinity)
        return absB == infinity ? q_value(infinity | q_quiet_bit()) : q_value(absA | sign);
    if (absB == infinity)
        return q_value(sign);
    if (!absA)
        return absB ? q_value(sign) : q_value(infinity | q_quiet_bit());
    if (!absB)
        return q_value(infinity | sign);
    if (aExp == 0)
        aExp = q_normalize(&aSig);
    if (bExp == 0)
        bExp = q_normalize(&bSig);
    aSig |= q_implicit();
    bSig |= q_implicit();
    int exponent = aExp - bExp + Q_BIAS;
    if (aSig < bSig) {
        aSig <<= 1;
        exponent--;
    }
    /* 116 quotient bits: the leading one, 112 more and three guard bits. */
    tu_int quotient = 0, remainder = aSig;
    for (int i = 0; i < 116; i++) {
        quotient <<= 1;
        if (remainder >= bSig) {
            remainder -= bSig;
            quotient |= 1;
        }
        remainder <<= 1;
    }
    quotient |= (tu_int)(remainder != 0);
    return q_round_pack(sign, exponent, quotient);
}

/* Comparisons: __letf2 returns -1, 0 or 1 for less, equal and greater, and
   1 when unordered; __getf2 returns -1 when unordered. */
static int q_compare(long double a, long double b, int unordered) {
    tu_int A = q_bits(a), B = q_bits(b);
    tu_int absA = A & ~q_sign_bit(), absB = B & ~q_sign_bit();
    if (absA > q_infinity() || absB > q_infinity())
        return unordered;
    if ((absA | absB) == 0)
        return 0;
    ti_int sa = (ti_int)A, sb = (ti_int)B;
    if ((sa & sb) >= 0) {
        if (sa < sb)
            return -1;
        return sa == sb ? 0 : 1;
    }
    if (sa > sb)
        return -1;
    return sa == sb ? 0 : 1;
}

int __letf2(long double a, long double b) { return q_compare(a, b, 1); }
int __lttf2(long double a, long double b) { return q_compare(a, b, 1); }
int __eqtf2(long double a, long double b) { return q_compare(a, b, 1); }
int __netf2(long double a, long double b) { return q_compare(a, b, 1); }
int __getf2(long double a, long double b) { return q_compare(a, b, -1); }
int __gttf2(long double a, long double b) { return q_compare(a, b, -1); }

int __unordtf2(long double a, long double b) {
    return (q_bits(a) & ~q_sign_bit()) > q_infinity() || (q_bits(b) & ~q_sign_bit()) > q_infinity();
}

long double __extenddftf2(double x) {
    union {
        double d;
        uint64_t u;
    } v = {x};
    tu_int sign = make(v.u & (1ull << 63), 0);
    int exponent = (int)((v.u >> 52) & 0x7FF);
    tu_int significand = make(0, v.u & ((1ull << 52) - 1));
    if (exponent == 0x7FF)
        return q_value(sign | q_infinity() | (significand << 60));
    if (exponent == 0) {
        if (significand == 0)
            return q_value(sign);
        /* Subnormal: shift its leading one to the implicit position. */
        int shift = leading_zeros128(significand) - leading_zeros128(make(0, 1ull << 52));
        significand <<= shift;
        exponent = 1 - shift;
        significand &= make(0, (1ull << 52) - 1);
    }
    return q_value(sign | ((tu_int)(exponent - 1023 + Q_BIAS) << Q_SIGNIFICAND_BITS) | (significand << 60));
}

long double __extendsftf2(float x) { return __extenddftf2((double)x); }

/* Rounds a binary128 value to a format with `bits` stored significand bits
   and an exponent of `exponent_bits`, returning its bits. */
static uint64_t q_truncate(long double a, int bits, int exponent_bits) {
    tu_int A = q_bits(a);
    uint64_t sign = (uint64_t)(high_of(A) >> 63) << (bits + exponent_bits);
    tu_int absA = A & ~q_sign_bit();
    int max = (1 << exponent_bits) - 1;
    int bias = max >> 1;
    uint64_t infinity = (uint64_t)max << bits;
    if (absA > q_infinity()) {
        uint64_t payload = (uint64_t)((absA & q_significand_mask()) >> (Q_SIGNIFICAND_BITS - bits));
        return sign | infinity | (1ull << (bits - 1)) | payload;
    }
    int exponent = (int)(absA >> Q_SIGNIFICAND_BITS) - Q_BIAS + bias;
    tu_int significand = (absA & q_significand_mask()) | (absA >> Q_SIGNIFICAND_BITS ? q_implicit() : 0);
    if (exponent >= max)
        return sign | infinity;
    int drop = Q_SIGNIFICAND_BITS - bits;
    if (exponent <= 0) {
        drop += 1 - exponent;
        exponent = 0;
        if (drop > 120)
            return sign;
    }
    tu_int kept = significand >> drop;
    tu_int rest = significand & (((tu_int)1 << drop) - 1);
    tu_int half = (tu_int)1 << (drop - 1);
    uint64_t result = (uint64_t)kept & ((1ull << bits) - 1);
    result |= (uint64_t)exponent << bits;
    if (exponent == 0 && ((uint64_t)kept >> bits))
        result |= 1ull << bits;
    if (rest > half || (rest == half && (result & 1)))
        result++;
    return sign | result;
}

double __trunctfdf2(long double a) {
    union {
        uint64_t u;
        double d;
    } v = {q_truncate(a, 52, 11)};
    return v.d;
}

float __trunctfsf2(long double a) {
    union {
        uint32_t u;
        float f;
    } v = {(uint32_t)q_truncate(a, 23, 8)};
    return v.f;
}

/* The integer part of a binary128 value's magnitude, saturating at the
   `bits`-bit maximum, and its sign. */
static tu_int q_integer(long double a, int *negative) {
    tu_int A = q_bits(a);
    *negative = (int)(high_of(A) >> 63);
    tu_int absA = A & ~q_sign_bit();
    if (absA > q_infinity())
        return 0;
    int exponent = (int)(absA >> Q_SIGNIFICAND_BITS) - Q_BIAS;
    if (exponent < 0)
        return 0;
    if (exponent >= 127)
        return make(~0ull, ~0ull);
    tu_int significand = (absA & q_significand_mask()) | q_implicit();
    return exponent >= Q_SIGNIFICAND_BITS ? significand << (exponent - Q_SIGNIFICAND_BITS)
                                          : significand >> (Q_SIGNIFICAND_BITS - exponent);
}

ti_int __fixtfti(long double a) {
    int negative;
    tu_int m = q_integer(a, &negative);
    if (m > make(1ull << 63, 0) - (negative ? 0 : 1))
        m = make(1ull << 63, 0) - (negative ? 0 : 1);
    return (ti_int)(negative ? negate(m) : m);
}

tu_int __fixunstfti(long double a) {
    int negative;
    tu_int m = q_integer(a, &negative);
    return negative ? 0 : m;
}

long long __fixtfdi(long double a) {
    int negative;
    tu_int m = q_integer(a, &negative);
    tu_int limit = make(0, 1ull << 63) - (negative ? 0 : 1);
    if (m > limit)
        m = limit;
    return negative ? (long long)(0 - low_of(m)) : (long long)low_of(m);
}

unsigned long long __fixunstfdi(long double a) {
    int negative;
    tu_int m = q_integer(a, &negative);
    if (negative)
        return 0;
    return high_of(m) ? ~0ull : low_of(m);
}

int __fixtfsi(long double a) {
    long long v = __fixtfdi(a);
    return v > 2147483647 ? 2147483647 : v < -2147483647 - 1 ? -2147483647 - 1 : (int)v;
}

unsigned __fixunstfsi(long double a) {
    unsigned long long v = __fixunstfdi(a);
    return v > 0xFFFFFFFFu ? 0xFFFFFFFFu : (unsigned)v;
}

/* An unsigned 128-bit integer as binary128, rounded to nearest even. */
static long double q_from_unsigned(tu_int m, int negative) {
    if (m == 0)
        return q_value(0);
    tu_int sign = negative ? q_sign_bit() : 0;
    int top = 127 - leading_zeros128(m);
    int exponent = top + Q_BIAS;
    tu_int significand;
    if (top <= Q_SIGNIFICAND_BITS) {
        significand = m << (Q_SIGNIFICAND_BITS - top);
        return q_value(sign | ((tu_int)exponent << Q_SIGNIFICAND_BITS) | (significand & q_significand_mask()));
    }
    /* Keep three extra bits for rounding, folding the rest into sticky. */
    int drop = top - Q_SIGNIFICAND_BITS - 3;
    tu_int sticky = drop > 0 ? (m << (128 - drop)) != 0 : 0;
    significand = (drop > 0 ? m >> drop : m << -drop) | sticky;
    return q_round_pack(sign, exponent, significand);
}

long double __floatsitf(int a) { return q_from_unsigned(a < 0 ? 0u - (unsigned)a : (unsigned)a, a < 0); }
long double __floatunsitf(unsigned a) { return q_from_unsigned(a, 0); }
long double __floatditf(long long a) {
    return q_from_unsigned(a < 0 ? 0ull - (unsigned long long)a : (unsigned long long)a, a < 0);
}
long double __floatunditf(unsigned long long a) { return q_from_unsigned(a, 0); }
long double __floattitf(ti_int a) {
    return negative(a) ? q_from_unsigned(negate((tu_int)a), 1) : q_from_unsigned((tu_int)a, 0);
}
long double __floatuntitf(tu_int a) { return q_from_unsigned(a, 0); }

/* fmodl, exactly, by aligned subtraction of the significands. */
long double fmodl(long double x, long double y) {
    tu_int X = q_bits(x), Y = q_bits(y);
    tu_int sign = X & q_sign_bit();
    tu_int absX = X & ~q_sign_bit(), absY = Y & ~q_sign_bit();
    if (absY == 0 || absX >= q_infinity() || absY > q_infinity())
        return q_value(q_infinity() | q_quiet_bit());
    if (absX < absY)
        return x;
    int ex = (int)(absX >> Q_SIGNIFICAND_BITS), ey = (int)(absY >> Q_SIGNIFICAND_BITS);
    tu_int mx = absX & q_significand_mask(), my = absY & q_significand_mask();
    if (ex == 0)
        ex = q_normalize(&mx);
    if (ey == 0)
        ey = q_normalize(&my);
    mx |= q_implicit();
    my |= q_implicit();
    for (; ex > ey; ex--) {
        if (mx >= my)
            mx -= my;
        mx <<= 1;
    }
    if (mx >= my)
        mx -= my;
    if (mx == 0)
        return q_value(sign);
    while (mx < q_implicit()) {
        mx <<= 1;
        ey--;
    }
    if (ey <= 0) {
        mx >>= 1 - ey;
        ey = 0;
        return q_value(sign | mx);
    }
    return q_value(sign | ((tu_int)ey << Q_SIGNIFICAND_BITS) | (mx & q_significand_mask()));
}
