/* The operations on _BitInt values of more than 128 bits that lowering does
   not expand inline, written in C and compiled by bcc itself into every
   module that calls them (wide_bit_int/runtime.rs). A value is an array of
   64-bit limbs, least significant first, ceil(N / 64) of them: the x86-64
   psABI's memory image of a _BitInt(N). A result is stored with its padding
   bits extended from bit N by its signedness, as GCC and Clang store them;
   an operand's padding bits are never read, so a value another compiler
   stored unextended computes correctly. Destinations never overlap
   operands. Each routine takes the width N and whether the type is
   signed. Division by zero, a quotient that overflows and an out-of-range
   shift count or floating value are undefined (C23 6.5.5p6, 6.5.7p4,
   6.3.1.4p1), and do whatever falls out of the code. */

typedef unsigned long long limb;
typedef unsigned __int128 wide;

#define LIMBS(bits) (((bits) + 63u) / 64u)

/* Limb i of a, its top limb extended from bit `bits` by signedness, and
   limbs beyond the top its sign. */
static limb __bcc_bitint_limb(const limb *a, unsigned i, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    unsigned spare = n * 64u - bits;
    limb top = a[n - 1];
    if (spare) {
        top = is_signed ? (limb)((long long)(top << spare) >> spare) : (top << spare) >> spare;
    }
    if (i + 1 < n)
        return a[i];
    if (i + 1 == n)
        return top;
    return is_signed && (long long)top < 0 ? ~0ull : 0;
}

/* Whether a is negative. */
static int __bcc_bitint_negative(const limb *a, unsigned bits, int is_signed) {
    return is_signed && (long long)__bcc_bitint_limb(a, LIMBS(bits) - 1, bits, 1) < 0;
}

/* Extends the top limb of d from bit `bits`. */
static void __bcc_bitint_extend(limb *d, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    d[n - 1] = __bcc_bitint_limb(d, n - 1, bits, is_signed);
}

void __bcc_bitint_add(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    limb carry = 0;
    for (unsigned i = 0; i < n; i++) {
        wide sum = (wide)a[i] + b[i] + carry;
        d[i] = (limb)sum;
        carry = (limb)(sum >> 64);
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

void __bcc_bitint_sub(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    limb borrow = 0;
    for (unsigned i = 0; i < n; i++) {
        limb x = a[i], y = b[i];
        d[i] = x - y - borrow;
        borrow = x < y || x - y < borrow;
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

/* The low N bits of the product, which only the operands' low N bits
   affect. */
void __bcc_bitint_mul(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    for (unsigned i = 0; i < n; i++)
        d[i] = 0;
    for (unsigned i = 0; i < n; i++) {
        limb x = a[i];
        if (x == 0)
            continue;
        limb carry = 0;
        for (unsigned j = 0; i + j < n; j++) {
            wide t = (wide)x * b[j] + d[i + j] + carry;
            d[i + j] = (limb)t;
            carry = (limb)(t >> 64);
        }
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

void __bcc_bitint_and(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        d[i] = a[i] & b[i];
    __bcc_bitint_extend(d, bits, is_signed);
}

void __bcc_bitint_or(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        d[i] = a[i] | b[i];
    __bcc_bitint_extend(d, bits, is_signed);
}

void __bcc_bitint_xor(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        d[i] = a[i] ^ b[i];
    __bcc_bitint_extend(d, bits, is_signed);
}

void __bcc_bitint_not(limb *d, const limb *a, unsigned bits, int is_signed) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        d[i] = ~a[i];
    __bcc_bitint_extend(d, bits, is_signed);
}

void __bcc_bitint_neg(limb *d, const limb *a, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    limb borrow = 0;
    for (unsigned i = 0; i < n; i++) {
        limb x = a[i];
        d[i] = 0 - x - borrow;
        borrow = x != 0 || borrow;
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

/* a << count, for count < N. */
void __bcc_bitint_shl(limb *d, const limb *a, limb count, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    unsigned whole = (unsigned)(count / 64), part = (unsigned)(count % 64);
    for (unsigned k = n; k-- > 0;) {
        limb high = k >= whole ? a[k - whole] : 0;
        limb low = k >= whole + 1 ? a[k - whole - 1] : 0;
        d[k] = part ? high << part | low >> (64 - part) : high;
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

/* a >> count, for count < N: arithmetic for a signed type. */
void __bcc_bitint_shr(limb *d, const limb *a, limb count, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    unsigned whole = (unsigned)(count / 64), part = (unsigned)(count % 64);
    for (unsigned k = 0; k < n; k++) {
        limb low = __bcc_bitint_limb(a, k + whole, bits, is_signed);
        limb high = __bcc_bitint_limb(a, k + whole + 1, bits, is_signed);
        d[k] = part ? low >> part | high << (64 - part) : low;
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

/* -1, 0 or 1 as a is below, equal to or above b. */
int __bcc_bitint_cmp(const limb *a, const limb *b, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    int na = __bcc_bitint_negative(a, bits, is_signed);
    int nb = __bcc_bitint_negative(b, bits, is_signed);
    if (na != nb)
        return na ? -1 : 1;
    for (unsigned k = n; k-- > 0;) {
        limb x = __bcc_bitint_limb(a, k, bits, is_signed);
        limb y = __bcc_bitint_limb(b, k, bits, is_signed);
        if (x != y)
            return x < y ? -1 : 1;
    }
    return 0;
}

/* Whether a is nonzero. */
int __bcc_bitint_test(const limb *a, unsigned bits) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        if (__bcc_bitint_limb(a, i, bits, 0))
            return 1;
    return 0;
}

/* a converted to a _BitInt(dbits): truncated, or extended by a's
   signedness (C23 6.3.1.3). */
void __bcc_bitint_convert(limb *d, unsigned dbits, int dsigned, const limb *a, unsigned abits,
                          int asigned) {
    for (unsigned i = 0; i < LIMBS(dbits); i++)
        d[i] = __bcc_bitint_limb(a, i, abits, asigned);
    __bcc_bitint_extend(d, dbits, dsigned);
}

/* The 128-bit value low | high << 64, already extended by its own
   signedness, converted to a _BitInt(bits). */
void __bcc_bitint_from_int(limb *d, unsigned bits, int is_signed, limb low, limb high) {
    unsigned n = LIMBS(bits);
    limb fill = (long long)high < 0 ? ~0ull : 0;
    for (unsigned i = 0; i < n; i++)
        d[i] = i == 0 ? low : i == 1 ? high : fill;
    __bcc_bitint_extend(d, bits, is_signed);
}

/* |a| into m, and the number of its limbs up to the highest nonzero one. */
static unsigned __bcc_bitint_absolute(limb *m, const limb *a, unsigned bits, int is_signed,
                                      int negative) {
    unsigned n = LIMBS(bits), used = 0;
    limb borrow = negative;
    for (unsigned i = 0; i < n; i++) {
        limb x = __bcc_bitint_limb(a, i, bits, is_signed);
        if (negative) {
            limb y = ~x + borrow;
            borrow = borrow && y == 0;
            x = y;
        }
        m[i] = x;
        if (x)
            used = i + 1;
    }
    return used;
}

/* Stores the n-limb magnitude m, negated if `negative`, as a _BitInt(bits). */
static void __bcc_bitint_store(limb *d, const limb *m, unsigned n, int negative, unsigned bits,
                               int is_signed) {
    limb borrow = 0;
    for (unsigned i = 0; i < LIMBS(bits); i++) {
        limb x = i < n ? m[i] : 0;
        if (negative) {
            d[i] = 0 - x - borrow;
            borrow = x != 0 || borrow;
        } else {
            d[i] = x;
        }
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

/* Knuth's algorithm D (TAOCP vol. 2, 4.3.1) on 64-bit digits: the quotient
   of the m-limb u by the n-limb v into q (m - n + 1 limbs) and the
   remainder into r (n limbs), for m >= n >= 2 and a nonzero top limb of v.
   un and vn are scratch of m + 1 and n limbs. */
static void __bcc_bitint_divide(limb *q, limb *r, const limb *u, const limb *v, unsigned m,
                                unsigned n, limb *un, limb *vn) {
    int s = __builtin_clzll(v[n - 1]);
    for (unsigned i = n - 1; i > 0; i--)
        vn[i] = v[i] << s | (s ? v[i - 1] >> (64 - s) : 0);
    vn[0] = v[0] << s;
    un[m] = s ? u[m - 1] >> (64 - s) : 0;
    for (unsigned i = m - 1; i > 0; i--)
        un[i] = u[i] << s | (s ? u[i - 1] >> (64 - s) : 0);
    un[0] = u[0] << s;
    for (unsigned j = m - n + 1; j-- > 0;) {
        wide numerator = (wide)un[j + n] << 64 | un[j + n - 1];
        wide qhat = numerator / vn[n - 1];
        wide rhat = numerator - qhat * vn[n - 1];
        while (qhat >> 64 || qhat * vn[n - 2] > (rhat << 64 | un[j + n - 2])) {
            qhat--;
            rhat += vn[n - 1];
            if (rhat >> 64)
                break;
        }
        limb borrow = 0, carry = 0;
        for (unsigned i = 0; i < n; i++) {
            wide product = qhat * vn[i] + carry;
            carry = (limb)(product >> 64);
            limb low = (limb)product, x = un[i + j];
            un[i + j] = x - low - borrow;
            borrow = x < low || x - low < borrow;
        }
        limb x = un[j + n];
        un[j + n] = x - carry - borrow;
        q[j] = (limb)qhat;
        if (x < carry || x - carry < borrow) {
            q[j]--;
            limb add = 0;
            for (unsigned i = 0; i < n; i++) {
                wide sum = (wide)un[i + j] + vn[i] + add;
                un[i + j] = (limb)sum;
                add = (limb)(sum >> 64);
            }
            un[j + n] += add;
        }
    }
    for (unsigned i = 0; i < n; i++)
        r[i] = un[i] >> s | (s ? un[i + 1] << (64 - s) : 0);
}

/* The quotient (truncated toward zero) into quotient and the remainder into
   remainder, either of which may be null (C23 6.5.5p6). */
static void __bcc_bitint_divmod(limb *quotient, limb *remainder, const limb *a, const limb *b,
                                unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    int na = __bcc_bitint_negative(a, bits, is_signed);
    int nb = __bcc_bitint_negative(b, bits, is_signed);
    limb u[n], v[n], q[n], r[n], un[n + 1], vn[n];
    unsigned m = __bcc_bitint_absolute(u, a, bits, is_signed, na);
    unsigned k = __bcc_bitint_absolute(v, b, bits, is_signed, nb);
    for (unsigned i = 0; i < n; i++)
        q[i] = r[i] = 0;
    if (k == 0) {
        /* Division by zero is undefined. */
    } else if (m < k) {
        for (unsigned i = 0; i < m; i++)
            r[i] = u[i];
    } else if (k == 1) {
        wide rest = 0;
        for (unsigned j = m; j-- > 0;) {
            wide numerator = rest << 64 | u[j];
            q[j] = (limb)(numerator / v[0]);
            rest = numerator % v[0];
        }
        r[0] = (limb)rest;
    } else {
        __bcc_bitint_divide(q, r, u, v, m, k, un, vn);
    }
    if (quotient)
        __bcc_bitint_store(quotient, q, n, na != nb, bits, is_signed);
    if (remainder)
        __bcc_bitint_store(remainder, r, n, na, bits, is_signed);
}

void __bcc_bitint_div(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    __bcc_bitint_divmod(d, 0, a, b, bits, is_signed);
}

void __bcc_bitint_rem(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    __bcc_bitint_divmod(0, d, a, b, bits, is_signed);
}

/* The value mantissa * 2^exponent, truncated toward zero and negated if
   `negative`, as a _BitInt(bits) (C23 6.3.1.4p1). */
static void __bcc_bitint_from_parts(limb *d, unsigned bits, int is_signed, int negative,
                                    wide mantissa, int exponent) {
    unsigned n = LIMBS(bits);
    for (unsigned i = 0; i < n; i++)
        d[i] = 0;
    if (exponent < 0) {
        mantissa = -exponent >= 128 ? 0 : mantissa >> -exponent;
        exponent = 0;
    }
    unsigned whole = (unsigned)exponent / 64, part = (unsigned)exponent % 64;
    limb low = (limb)mantissa, high = (limb)(mantissa >> 64);
    limb parts[3] = {low << part, part ? high << part | low >> (64 - part) : high,
                     part ? high >> (64 - part) : 0};
    for (unsigned i = 0; i < 3; i++)
        if (whole < n && i < n - whole)
            d[whole + i] = parts[i];
    if (negative) {
        limb borrow = 0;
        for (unsigned i = 0; i < n; i++) {
            limb x = d[i];
            d[i] = 0 - x - borrow;
            borrow = x != 0 || borrow;
        }
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

void __bcc_bitint_from_f64(limb *d, unsigned bits, int is_signed, double value) {
    union {
        double value;
        limb bits;
    } u = {value};
    int exponent = (int)(u.bits >> 52 & 0x7ff);
    wide mantissa = u.bits & ((1ull << 52) - 1);
    if (exponent)
        mantissa |= 1ull << 52;
    __bcc_bitint_from_parts(d, bits, is_signed, (int)(u.bits >> 63), mantissa,
                            (exponent ? exponent : 1) - 1075);
}

void __bcc_bitint_from_f32(limb *d, unsigned bits, int is_signed, float value) {
    __bcc_bitint_from_f64(d, bits, is_signed, value);
}

void __bcc_bitint_from_f16(limb *d, unsigned bits, int is_signed, _Float16 value) {
    __bcc_bitint_from_f64(d, bits, is_signed, value);
}

void __bcc_bitint_from_bf16(limb *d, unsigned bits, int is_signed, __bf16 value) {
    __bcc_bitint_from_f64(d, bits, is_signed, value);
}

#if __LDBL_MANT_DIG__ == 64
void __bcc_bitint_from_f80(limb *d, unsigned bits, int is_signed, long double value) {
    union {
        long double value;
        struct {
            limb mantissa;
            unsigned short exponent;
        } parts;
    } u;
    u.value = value;
    int exponent = u.parts.exponent & 0x7fff;
    __bcc_bitint_from_parts(d, bits, is_signed, u.parts.exponent >> 15, u.parts.mantissa,
                            (exponent ? exponent : 1) - 16383 - 63);
}
#endif

#ifdef __SIZEOF_FLOAT128__
void __bcc_bitint_from_f128(limb *d, unsigned bits, int is_signed, __float128 value) {
    union {
        __float128 value;
        struct {
            limb low, high;
        } parts;
    } u;
    u.value = value;
    int exponent = (int)(u.parts.high >> 48 & 0x7fff);
    wide mantissa = (wide)(u.parts.high & ((1ull << 48) - 1)) << 64 | u.parts.low;
    if (exponent)
        mantissa |= (wide)1 << 112;
    __bcc_bitint_from_parts(d, bits, is_signed, (int)(u.parts.high >> 63), mantissa,
                            (exponent ? exponent : 1) - 16383 - 112);
}
#endif

/* The top 128 bits of the used-limb magnitude m, its lowest bit ORed with
   every bit below them, and in *scale the number of bits below them: a
   value that rounds to any floating format as m does, once scaled. */
static wide __bcc_bitint_top(const limb *m, unsigned used, unsigned *scale) {
    *scale = 0;
    if (used <= 2)
        return used == 0 ? 0 : used == 1 ? m[0] : (wide)m[1] << 64 | m[0];
    unsigned shift = (used - 1) * 64 - (unsigned)__builtin_clzll(m[used - 1]) - 64;
    unsigned whole = shift / 64, part = shift % 64;
    limb l0 = m[whole], l1 = whole + 1 < used ? m[whole + 1] : 0,
         l2 = whole + 2 < used ? m[whole + 2] : 0;
    wide low = (wide)l1 << 64 | l0;
    wide top = part ? low >> part | (wide)l2 << (128 - part) : low;
    int sticky = part && (l0 & ((1ull << part) - 1)) != 0;
    for (unsigned i = 0; i < whole && !sticky; i++)
        sticky = m[i] != 0;
    *scale = shift;
    return top | (wide)sticky;
}

#define BCC_BITINT_TO_FLOAT(name, T)                                                 \
    T name(const limb *a, unsigned bits, int is_signed) {                            \
        int negative = __bcc_bitint_negative(a, bits, is_signed);                    \
        limb m[LIMBS(bits)];                                                         \
        unsigned scale;                                                              \
        wide top = __bcc_bitint_top(m, __bcc_bitint_absolute(m, a, bits, is_signed, \
                                                             negative),              \
                                    &scale);                                         \
        T result = (T)top;                                                           \
        for (; scale >= 32; scale -= 32)                                             \
            result *= (T)4294967296.0f;                                              \
        result *= (T)(1ull << scale);                                                \
        return negative ? -result : result;                                          \
    }

BCC_BITINT_TO_FLOAT(__bcc_bitint_to_f32, float)
BCC_BITINT_TO_FLOAT(__bcc_bitint_to_f64, double)
#if __LDBL_MANT_DIG__ == 64
BCC_BITINT_TO_FLOAT(__bcc_bitint_to_f80, long double)
#endif
#ifdef __SIZEOF_FLOAT128__
BCC_BITINT_TO_FLOAT(__bcc_bitint_to_f128, __float128)
#endif
