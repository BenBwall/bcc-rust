/* strtof, strtod and strtold for wasm32-wasip1, and the nan functions.
   C99: §7.20.1.3, pp. 307-309; §7.12.11.2, p. 249.

   Every conversion is correctly rounded (to nearest, ties to even) to its
   own format, binary128 for long double included, for subject sequences
   of any length, as glibc's and musl's are. End pointers, NaN payloads and
   errno follow glibc: ERANGE on overflow, and on a result that is tiny
   after rounding and inexact (glibc's rule on x86).

   A decimal significand is read by musl's decfloat
   (src/internal/floatscan.c), translated with its binary128 parameters:
   the digits fill base-10^9 words, which are scaled by powers of two until
   their integer part is a 113-bit significand, and the words below it
   decide the rounding. musl then assembles the result in long double
   arithmetic; here integer code rounds the significand once, straight to
   the format asked for, so float and double are never rounded twice. A
   hexadecimal significand is read into an integer directly.

   musl libc: Copyright (c) 2005-2020 Rich Felker, et al. Under the MIT
   license:

   Permission is hereby granted, free of charge, to any person obtaining a
   copy of this software and associated documentation files (the
   "Software"), to deal in the Software without restriction, including
   without limitation the rights to use, copy, modify, merge, publish,
   distribute, sublicense, and/or sell copies of the Software, and to
   permit persons to whom the Software is furnished to do so, subject to
   the following conditions:

   The above copyright notice and this permission notice shall be included
   in all copies or substantial portions of the Software.

   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
   OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
   MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
   IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
   CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
   TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
   SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE. */
#include <ctype.h>
#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdlib.h>
#include <strings.h>

typedef unsigned __int128 u128;

/* A binary interchange format: its precision, the exponent of its least
   subnormal's bit, and the width of its exponent field. */
struct format {
    int bits;
    int emin;
    int exponent_bits;
};

static const struct format binary32 = {24, -149, 8};
static const struct format binary64 = {53, -1074, 11};
static const struct format binary128 = {113, -16494, 15};

/* A subject sequence (C99 7.20.1.3p3), read for one format. */
struct number {
    enum {
        NONE,     /* no conversion */
        ZERO,     /* a zero significand */
        FINITE,   /* significand, tail, exponent */
        HUGE,     /* overflows the format */
        TINY,     /* underflows the format to zero */
        INFINITE, /* INF or INFINITY */
        NAN_TAG,  /* NAN or NAN(n-char-sequence), with payload */
    } kind;
    int negative;
    /* A FINITE value is (significand + tail / 4) * 2^exponent, where the
       significand is in [2^112, 2^113) and the tail says what follows its
       lowest bit: 0 nothing, 1 less than a half, 2 exactly a half, 3 more
       than a half. */
    u128 significand;
    int tail;
    long long exponent;
    unsigned long long payload;
};

static const char *scan(const char *s, const struct format *f, struct number *n);
static u128 magnitude(const struct number *n, const struct format *f);
static double nan_double(int negative, unsigned long long payload);
static float nan_float(int negative, unsigned long long payload);
static long double nan_quad(int negative, unsigned long long payload);

double strtod(const char *s, char **end) {
    struct number n;
    const char *last = scan(s, &binary64, &n);
    if (end)
        *end = (char *)last;
    if (n.kind == NAN_TAG)
        return nan_double(n.negative, n.payload);
    union {
        unsigned long long u;
        double d;
    } bits = {(unsigned long long)magnitude(&n, &binary64) | (unsigned long long)n.negative << 63};
    return bits.d;
}

float strtof(const char *s, char **end) {
    struct number n;
    const char *last = scan(s, &binary32, &n);
    if (end)
        *end = (char *)last;
    if (n.kind == NAN_TAG)
        return nan_float(n.negative, n.payload);
    union {
        unsigned u;
        float f;
    } bits = {(unsigned)magnitude(&n, &binary32) | (unsigned)n.negative << 31};
    return bits.f;
}

/* long double is binary128 on wasm32. */
long double strtold(const char *s, char **end) {
    struct number n;
    const char *last = scan(s, &binary128, &n);
    if (end)
        *end = (char *)last;
    if (n.kind == NAN_TAG)
        return nan_quad(n.negative, n.payload);
    u128 value = magnitude(&n, &binary128);
    union {
        unsigned long long words[2];
        long double f;
    } bits = {{(unsigned long long)value, (unsigned long long)(value >> 64) | (unsigned long long)n.negative << 63}};
    return bits.f;
}

double atof(const char *s) { return strtod(s, 0); }

static const char *nan_sequence(const char *p, unsigned long long *payload);
static const char *decimal(const char *p, const struct format *f, struct number *n);
static const char *hexadecimal(const char *p, struct number *n);

/* Reads the subject sequence at the start of s for format f (C99
   7.20.1.3p3-p6) and returns its end, or s when there is none. */
static const char *scan(const char *s, const struct format *f, struct number *n) {
    const char *p = s;
    n->kind = NONE;
    n->negative = 0;
    while (isspace((unsigned char)*p))
        p++;
    int negative = 0;
    if (*p == '+' || *p == '-')
        negative = *p++ == '-';
    const char *last;
    if (!strncasecmp(p, "inf", 3)) {
        last = p + 3;
        if (!strncasecmp(last, "inity", 5))
            last += 5;
        n->kind = INFINITE;
    } else if (!strncasecmp(p, "nan", 3)) {
        last = nan_sequence(p + 3, &n->payload);
        n->kind = NAN_TAG;
    } else if (p[0] == '0' && (p[1] | 32) == 'x') {
        last = hexadecimal(p + 2, n);
        if (!last) {
            /* "0x" without a hexadecimal digit is the constant 0. */
            n->kind = ZERO;
            last = p + 1;
        }
    } else {
        last = decimal(p, f, n);
        if (!last)
            return s;
    }
    n->negative = negative;
    return last;
}

/* The bits of n's magnitude in format f, rounded to nearest with ties to
   even. Sets errno to ERANGE on overflow and when the result is tiny
   after rounding (as if the exponent were unbounded) and inexact. */
static u128 magnitude(const struct number *n, const struct format *f) {
    u128 infinity = (((u128)1 << f->exponent_bits) - 1) << (f->bits - 1);
    switch (n->kind) {
    case INFINITE:
        return infinity;
    case HUGE:
        errno = ERANGE;
        return infinity;
    case TINY:
        errno = ERANGE;
        return 0;
    case FINITE:
        break;
    default:
        return 0;
    }
    /* m * 2^(exponent - 2) is the value, with the tail as m's two lowest
       bits: rounding m compares them as it would the exact tail. */
    u128 m = n->significand << 2 | (unsigned)n->tail;
    /* The number of significand bits at or above the least subnormal. */
    long long available = 113 + n->exponent - f->emin;
    if (available < 0) {
        /* Below half the least subnormal. */
        errno = ERANGE;
        return 0;
    }
    int bits = available < f->bits ? (int)available : f->bits;
    int drop = 115 - bits;
    u128 rest = m & (((u128)1 << drop) - 1), half = (u128)1 << (drop - 1);
    u128 kept = m >> drop;
    kept += rest > half || (rest == half && (kept & 1));
    /* kept's lowest bit is 2^(emin + scale), so the encoding is
       scale << (bits - 1) plus kept, whose carry into the exponent field
       is the hidden bit (or a subnormal's promotion to normal). */
    long long scale = n->exponent - 2 + drop - f->emin;
    if (scale >= 1 << f->exponent_bits) {
        errno = ERANGE;
        return infinity;
    }
    u128 result = ((u128)scale << (f->bits - 1)) + kept;
    if (result >= infinity) {
        errno = ERANGE;
        return infinity;
    }
    if (rest && bits < f->bits) {
        /* Tiny unless rounding to the full precision would reach the
           least normal. */
        int tiny = 1;
        if (bits == f->bits - 1) {
            int full = 115 - f->bits;
            u128 low = m & (((u128)1 << full) - 1), mid = (u128)1 << (full - 1);
            u128 wide = (m >> full) + (low > mid || (low == mid && ((m >> full) & 1)));
            tiny = !(wide >> f->bits);
        }
        if (tiny)
            errno = ERANGE;
    }
    return result;
}

/* Reads an exponent's optionally signed digits at p; LLONG_MIN when
   there are none. A huge exponent saturates, as in musl. */
static long long exponent_part(const char *p, const char **end) {
    int negative = 0;
    if (*p == '+' || *p == '-')
        negative = *p++ == '-';
    if ((unsigned)(*p - '0') >= 10)
        return LLONG_MIN;
    long long y = 0;
    for (; (unsigned)(*p - '0') < 10; p++)
        if (y < LLONG_MAX / 100)
            y = 10 * y + (*p - '0');
    *end = p;
    return negative ? -y : y;
}

/* musl's decfloat for binary128: the significand's base-10^9 words, at
   most KMAX of them, the number of them that hold the 113-bit
   significand, and 2^113 - 1 in those words. */
#define KMAX 2048
#define MASK (KMAX - 1)
#define B1B_DIG 4

/* Reads a decimal floating constant's significand and exponent at p
   (C99 7.20.1.3p3) and returns its end, or a null pointer when it has no
   digit. */
static const char *decimal(const char *p, const struct format *f, struct number *n) {
    uint32_t x[KMAX];
    static const uint32_t th[] = {10384593, 717069655, 257060992, 658440191};
    static const int p10s[] = {10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000};
    int i, j = 0, k = 0, a, z;
    long long lrp = 0, dc = 0;
    int lnz = 0;
    int gotdig = 0, gotrad = 0;
    int rp;
    int e2;
    int c = (unsigned char)*p;

    /* Don't let leading zeros consume buffer space */
    for (; c == '0'; c = (unsigned char)*++p)
        gotdig = 1;
    if (c == '.') {
        gotrad = 1;
        for (c = (unsigned char)*++p; c == '0'; c = (unsigned char)*++p)
            gotdig = 1, lrp--;
    }

    x[0] = 0;
    for (; (unsigned)(c - '0') < 10 || c == '.'; c = (unsigned char)*++p) {
        if (c == '.') {
            if (gotrad)
                break;
            gotrad = 1;
            lrp = dc;
        } else if (k < KMAX - 3) {
            dc++;
            if (c != '0')
                lnz = dc;
            if (j)
                x[k] = x[k] * 10 + c - '0';
            else
                x[k] = c - '0';
            if (++j == 9) {
                k++;
                j = 0;
            }
            gotdig = 1;
        } else {
            dc++;
            if (c != '0') {
                lnz = (KMAX - 4) * 9;
                x[KMAX - 4] |= 1;
            }
        }
    }
    if (!gotrad)
        lrp = dc;
    if (!gotdig)
        return 0;
    if ((c | 32) == 'e') {
        const char *after;
        long long e10 = exponent_part(p + 1, &after);
        if (e10 != LLONG_MIN) {
            lrp += e10;
            p = after;
        }
    }

    /* Handle zero specially to avoid nasty special cases later */
    if (!x[0]) {
        n->kind = ZERO;
        return p;
    }
    /* Over- and underflow: the value is at least 10^(lrp - 1) and below
       10^lrp. */
    if (lrp > -f->emin / 2) {
        n->kind = HUGE;
        return p;
    }
    if (lrp < f->emin - 2 * 113) {
        n->kind = TINY;
        return p;
    }

    /* Align incomplete final B1B digit */
    if (j) {
        for (; j < 9; j++)
            x[k] *= 10;
        k++;
        j = 0;
    }

    a = 0;
    z = k;
    e2 = 0;
    rp = lrp;

    /* Drop trailing zeros */
    for (; !x[z - 1]; z--)
        ;

    /* Align radix point to B1B digit boundary */
    if (rp % 9) {
        int rpm9 = rp >= 0 ? rp % 9 : rp % 9 + 9;
        int p10 = p10s[8 - rpm9];
        uint32_t carry = 0;
        for (k = a; k != z; k++) {
            uint32_t tmp = x[k] % p10;
            x[k] = x[k] / p10 + carry;
            carry = 1000000000 / p10 * tmp;
            if (k == a && !x[k]) {
                a = (a + 1 & MASK);
                rp -= 9;
            }
        }
        if (carry)
            x[z++] = carry;
        rp += 9 - rpm9;
    }

    /* Upscale until desired number of bits are left of radix point */
    while (rp < 9 * B1B_DIG || (rp == 9 * B1B_DIG && x[a] < th[0])) {
        uint32_t carry = 0;
        e2 -= 29;
        for (k = (z - 1 & MASK);; k = (k - 1 & MASK)) {
            uint64_t tmp = ((uint64_t)x[k] << 29) + carry;
            /* musl compares with >, which can leave a word of exactly
               10^9; >= keeps every word a base-10^9 digit. */
            if (tmp >= 1000000000) {
                carry = tmp / 1000000000;
                x[k] = tmp % 1000000000;
            } else {
                carry = 0;
                x[k] = tmp;
            }
            if (k == (z - 1 & MASK) && k != a && !x[k])
                z = k;
            if (k == a)
                break;
        }
        if (carry) {
            rp += 9;
            a = (a - 1 & MASK);
            if (a == z) {
                z = (z - 1 & MASK);
                x[z - 1 & MASK] |= x[z];
            }
            x[a] = carry;
        }
    }

    /* Downscale until exactly number of bits are left of radix point */
    for (;;) {
        uint32_t carry = 0;
        int sh = 1;
        for (i = 0; i < B1B_DIG; i++) {
            k = (a + i & MASK);
            if (k == z || x[k] < th[i]) {
                i = B1B_DIG;
                break;
            }
            if (x[a + i & MASK] > th[i])
                break;
        }
        if (i == B1B_DIG && rp == 9 * B1B_DIG)
            break;
        /* FIXME: find a way to compute optimal sh */
        if (rp > 9 + 9 * B1B_DIG)
            sh = 9;
        e2 += sh;
        for (k = a; k != z; k = (k + 1 & MASK)) {
            uint32_t tmp = x[k] & (1 << sh) - 1;
            x[k] = (x[k] >> sh) + carry;
            carry = (1000000000 >> sh) * tmp;
            if (k == a && !x[k]) {
                a = (a + 1 & MASK);
                i--;
                rp -= 9;
            }
        }
        if (carry) {
            if ((z + 1 & MASK) != a) {
                x[z] = carry;
                z = (z + 1 & MASK);
            } else
                x[z - 1 & MASK] |= 1;
        }
    }

    /* The integer part is the significand, exactly; the first word below
       it and whether any later one is nonzero give the tail. */
    u128 y = 0;
    for (i = 0; i < B1B_DIG; i++) {
        if ((a + i & MASK) == z)
            x[z] = 0, z = z + 1 & MASK;
        y = 1000000000 * y + x[a + i & MASK];
    }
    int tail = 0;
    if ((a + i & MASK) != z) {
        uint32_t t = x[a + i & MASK];
        int more = 0;
        for (k = (a + i + 1 & MASK); k != z; k = (k + 1 & MASK))
            more |= x[k] != 0;
        if (t < 500000000)
            tail = t || more;
        else
            tail = t > 500000000 || more ? 3 : 2;
    }
    n->kind = FINITE;
    n->significand = y;
    n->tail = tail;
    n->exponent = e2;
    return p;
}

/* Reads a hexadecimal floating constant after its 0x (C99 7.20.1.3p3)
   and returns its end, or a null pointer when it has no hexadecimal
   digit. Digits past the first 117 significant bits only count as
   nonzero or not. */
static const char *hexadecimal(const char *p, struct number *n) {
    u128 m = 0;
    long long exponent = 0;
    int sticky = 0, point = 0, any = 0;
    for (;; p++) {
        int c = (unsigned char)*p, d;
        if (c == '.' && !point) {
            point = 1;
            continue;
        }
        if ((unsigned)(c - '0') < 10)
            d = c - '0';
        else if ((unsigned)((c | 32) - 'a') < 6)
            d = (c | 32) - 'a' + 10;
        else
            break;
        any = 1;
        if (m >> 116) {
            sticky |= d;
            if (!point)
                exponent += 4;
        } else {
            m = m * 16 + (unsigned)d;
            if (point)
                exponent -= 4;
        }
    }
    if (!any)
        return 0;
    if ((*p | 32) == 'p') {
        const char *after;
        long long e = exponent_part(p + 1, &after);
        if (e != LLONG_MIN) {
            exponent += e;
            p = after;
        }
    }
    if (!m) {
        n->kind = ZERO;
        return p;
    }
    int top = 127;
    while (!(m >> top))
        top--;
    if (top > 112) {
        int s = top - 112;
        u128 low = m & (((u128)1 << s) - 1), half = (u128)1 << (s - 1);
        if (low < half)
            n->tail = low || sticky;
        else
            n->tail = low > half || sticky ? 3 : 2;
        m >>= s;
        exponent += s;
    } else {
        m <<= 112 - top;
        exponent -= 112 - top;
        n->tail = 0;
    }
    /* Keep the exponent small: these are past every format's range. */
    n->kind = exponent > 1 << 20 ? HUGE : exponent < -(1 << 20) ? TINY : FINITE;
    n->significand = m;
    n->exponent = exponent;
    return p;
}

/* The payload of a NaN whose n-char-sequence is [first, last): the
   sequence read as an integer in base 0 when all of it is one, and 0
   otherwise, as in glibc (an integer too large gives all ones). errno is
   unchanged. C99 7.20.1.3p4. */
static unsigned long long nan_payload(const char *first, const char *last) {
    int saved = errno;
    char *digits;
    unsigned long long v = strtoull(first, &digits, 0);
    errno = saved;
    return digits == last ? v : 0;
}

/* Reads what follows "nan": an optional (n-char-sequence), whose payload
   it stores, 0 without one. Returns the end of what it read. */
static const char *nan_sequence(const char *p, unsigned long long *payload) {
    *payload = 0;
    if (*p != '(')
        return p;
    const char *q = p + 1;
    while (isalnum((unsigned char)*q) || *q == '_')
        q++;
    if (*q != ')')
        return p;
    *payload = nan_payload(p + 1, q);
    return q + 1;
}

/* The quiet NaN with a sign and the low bits of a payload. */
static double nan_double(int negative, unsigned long long payload) {
    union {
        unsigned long long u;
        double d;
    } bits = {(negative ? 1ull << 63 : 0) | 0x7ff8000000000000ull | (payload & 0x7ffffffffffffull)};
    return bits.d;
}

static float nan_float(int negative, unsigned long long payload) {
    union {
        unsigned u;
        float f;
    } bits = {(negative ? 1u << 31 : 0) | 0x7fc00000u | (unsigned)(payload & 0x3fffffu)};
    return bits.f;
}

/* The quiet binary128 NaN (long double on wasm32) with a sign and a
   payload, which its 111 bits below the quiet bit hold whole. */
static long double nan_quad(int negative, unsigned long long payload) {
    union {
        unsigned long long words[2];
        long double f;
    } bits = {{payload, (negative ? 1ull << 63 : 0) | 0x7FFF800000000000ull}};
    return bits.f;
}

/* nan("n-char-sequence") is strtod("NAN(n-char-sequence)") (C99
   7.12.11.2): a tag that is no n-char-sequence gives payload 0. */
static unsigned long long tag_payload(const char *tag) {
    const char *last = tag;
    while (isalnum((unsigned char)*last) || *last == '_')
        last++;
    return *last ? 0 : nan_payload(tag, last);
}

double nan(const char *tag) { return nan_double(0, tag_payload(tag)); }
float nanf(const char *tag) { return nan_float(0, tag_payload(tag)); }
long double nanl(const char *tag) { return nan_quad(0, tag_payload(tag)); }
