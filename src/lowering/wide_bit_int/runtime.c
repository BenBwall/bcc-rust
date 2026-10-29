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

/* A routine a module calls. Each loops over its operands' limbs, so it is
   kept out of line: expanding one at every wide operation costs code size
   and, at 65535 bits, most of LLVM's optimization time, for the saving of
   a call. */
#define BCC_ROUTINE __attribute__((noinline))

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

BCC_ROUTINE void __bcc_bitint_add(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    limb carry = 0;
    for (unsigned i = 0; i < n; i++) {
        wide sum = (wide)a[i] + b[i] + carry;
        d[i] = (limb)sum;
        carry = (limb)(sum >> 64);
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

BCC_ROUTINE void __bcc_bitint_sub(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
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
BCC_ROUTINE void __bcc_bitint_mul(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
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

BCC_ROUTINE void __bcc_bitint_and(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        d[i] = a[i] & b[i];
    __bcc_bitint_extend(d, bits, is_signed);
}

BCC_ROUTINE void __bcc_bitint_or(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        d[i] = a[i] | b[i];
    __bcc_bitint_extend(d, bits, is_signed);
}

BCC_ROUTINE void __bcc_bitint_xor(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        d[i] = a[i] ^ b[i];
    __bcc_bitint_extend(d, bits, is_signed);
}

BCC_ROUTINE void __bcc_bitint_not(limb *d, const limb *a, unsigned bits, int is_signed) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        d[i] = ~a[i];
    __bcc_bitint_extend(d, bits, is_signed);
}

BCC_ROUTINE void __bcc_bitint_neg(limb *d, const limb *a, unsigned bits, int is_signed) {
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
BCC_ROUTINE void __bcc_bitint_shl(limb *d, const limb *a, limb count, unsigned bits, int is_signed) {
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
BCC_ROUTINE void __bcc_bitint_shr(limb *d, const limb *a, limb count, unsigned bits, int is_signed) {
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
BCC_ROUTINE int __bcc_bitint_cmp(const limb *a, const limb *b, unsigned bits, int is_signed) {
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
BCC_ROUTINE int __bcc_bitint_test(const limb *a, unsigned bits) {
    for (unsigned i = 0; i < LIMBS(bits); i++)
        if (__bcc_bitint_limb(a, i, bits, 0))
            return 1;
    return 0;
}

/* a converted to a _BitInt(dbits): truncated, or extended by a's
   signedness (C23 6.3.1.3). */
BCC_ROUTINE void __bcc_bitint_convert(limb *d, unsigned dbits, int dsigned, const limb *a, unsigned abits,
                          int asigned) {
    for (unsigned i = 0; i < LIMBS(dbits); i++)
        d[i] = __bcc_bitint_limb(a, i, abits, asigned);
    __bcc_bitint_extend(d, dbits, dsigned);
}

/* The `width`-bit field at bit `shift` of the bytes at p, read as a
   _BitInt(bits) of the field's signedness, which is the type's. */
BCC_ROUTINE void __bcc_bitint_field_get(limb *d, unsigned bits, int is_signed, const unsigned char *p,
                            unsigned shift, unsigned width) {
    unsigned n = LIMBS(bits);
    for (unsigned i = 0; i < n; i++)
        d[i] = 0;
    for (unsigned i = 0; i < width; i++) {
        unsigned at = shift + i;
        d[i / 64] |= (limb)(p[at / 8] >> at % 8 & 1) << i % 64;
    }
    if (is_signed && d[(width - 1) / 64] >> (width - 1) % 64 & 1) {
        for (unsigned i = width; i < n * 64; i++)
            d[i / 64] |= 1ull << i % 64;
    }
    __bcc_bitint_extend(d, bits, is_signed);
}

/* Stores the low `width` bits of a into the field at bit `shift` of the
   bytes at p, leaving the bits around it: a byte the field covers is
   written whole, without reading it, and only the partly covered first and
   last bytes are read (which lowering freezes). */
BCC_ROUTINE void __bcc_bitint_field_set(unsigned char *p, unsigned shift, unsigned width, const limb *a) {
    unsigned end = shift + width;
    for (unsigned byte = shift / 8; byte * 8 < end; byte++) {
        unsigned low = byte * 8 < shift ? shift : byte * 8;
        unsigned high = byte * 8 + 8 < end ? byte * 8 + 8 : end;
        unsigned char value = 0, mask = 0;
        for (unsigned at = low; at < high; at++) {
            unsigned i = at - shift;
            mask |= (unsigned char)(1u << at % 8);
            if (a[i / 64] >> i % 64 & 1)
                value |= (unsigned char)(1u << at % 8);
        }
        p[byte] = mask == 0xff ? value : (unsigned char)((p[byte] & ~mask) | value);
    }
}

/* The 128-bit value low | high << 64, signed if `value_signed`, converted
   to a _BitInt(bits). */
BCC_ROUTINE void __bcc_bitint_from_int(limb *d, unsigned bits, int is_signed, limb low, limb high,
                           int value_signed) {
    unsigned n = LIMBS(bits);
    limb fill = value_signed && (long long)high < 0 ? ~0ull : 0;
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

/* Knuth's algorithm D (TAOCP vol. 2, 4.3.1) on 64-bit digits, in place:
   u, m + 1 limbs whose top one is zero, is divided by v, n limbs with a
   nonzero top one, for m >= n >= 2. Both are normalized in place. The
   quotient digit of step j goes to u[j + n], which the step leaves zero
   and no later step reads, so u ends holding the quotient in u[n..m] and
   the remainder, unnormalized again, in u[0..n-1]. */
static void __bcc_bitint_divide(limb *u, limb *v, unsigned m, unsigned n) {
    int s = __builtin_clzll(v[n - 1]);
    if (s) {
        for (unsigned i = n - 1; i > 0; i--)
            v[i] = v[i] << s | v[i - 1] >> (64 - s);
        v[0] <<= s;
        for (unsigned i = m; i > 0; i--)
            u[i] = u[i] << s | u[i - 1] >> (64 - s);
        u[0] <<= s;
    }
    for (unsigned j = m - n + 1; j-- > 0;) {
        wide numerator = (wide)u[j + n] << 64 | u[j + n - 1];
        wide qhat = numerator / v[n - 1];
        wide rhat = numerator - qhat * v[n - 1];
        while (qhat >> 64 || qhat * v[n - 2] > (rhat << 64 | u[j + n - 2])) {
            qhat--;
            rhat += v[n - 1];
            if (rhat >> 64)
                break;
        }
        limb borrow = 0, carry = 0;
        for (unsigned i = 0; i < n; i++) {
            wide product = qhat * v[i] + carry;
            carry = (limb)(product >> 64);
            limb low = (limb)product, x = u[i + j];
            u[i + j] = x - low - borrow;
            borrow = x < low || x - low < borrow;
        }
        limb x = u[j + n];
        limb digit = (limb)qhat;
        if (x < carry || x - carry < borrow) {
            digit--;
            limb add = 0;
            for (unsigned i = 0; i < n; i++) {
                wide sum = (wide)u[i + j] + v[i] + add;
                u[i + j] = (limb)sum;
                add = (limb)(sum >> 64);
            }
        }
        u[j + n] = digit;
    }
    if (s) {
        for (unsigned i = 0; i < n; i++)
            u[i] = u[i] >> s | (i + 1 < n ? u[i + 1] << (64 - s) : 0);
    }
}

/* The quotient (truncated toward zero) into quotient and the remainder into
   remainder, either of which may be null (C23 6.5.5p6). The work takes two
   arrays of the width's limbs, 2 MB at Clang's 8388608 bits. */
static void __bcc_bitint_divmod(limb *quotient, limb *remainder, const limb *a, const limb *b,
                                unsigned bits, int is_signed) {
    unsigned n = LIMBS(bits);
    int na = __bcc_bitint_negative(a, bits, is_signed);
    int nb = __bcc_bitint_negative(b, bits, is_signed);
    limb u[n + 1], v[n];
    unsigned m = __bcc_bitint_absolute(u, a, bits, is_signed, na);
    unsigned k = __bcc_bitint_absolute(v, b, bits, is_signed, nb);
    u[m] = 0;
    /* The quotient's and the remainder's limbs within u, and their counts. */
    const limb *q = u, *r = u;
    unsigned qn = 0, rn = 0;
    limb rest = 0;
    if (k == 0) {
        /* Division by zero is undefined. */
    } else if (m < k) {
        rn = m;
    } else if (k == 1) {
        wide carried = 0;
        for (unsigned j = m; j-- > 0;) {
            wide numerator = carried << 64 | u[j];
            u[j] = (limb)(numerator / v[0]);
            carried = numerator % v[0];
        }
        rest = (limb)carried;
        qn = m;
        r = &rest;
        rn = 1;
    } else {
        __bcc_bitint_divide(u, v, m, k);
        q = u + k;
        qn = m - k + 1;
        rn = k;
    }
    if (quotient)
        __bcc_bitint_store(quotient, q, qn, na != nb, bits, is_signed);
    if (remainder)
        __bcc_bitint_store(remainder, r, rn, na, bits, is_signed);
}

BCC_ROUTINE void __bcc_bitint_div(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
    __bcc_bitint_divmod(d, 0, a, b, bits, is_signed);
}

BCC_ROUTINE void __bcc_bitint_rem(limb *d, const limb *a, const limb *b, unsigned bits, int is_signed) {
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

BCC_ROUTINE void __bcc_bitint_from_f64(limb *d, unsigned bits, int is_signed, double value) {
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

BCC_ROUTINE void __bcc_bitint_from_f32(limb *d, unsigned bits, int is_signed, float value) {
    __bcc_bitint_from_f64(d, bits, is_signed, value);
}

BCC_ROUTINE void __bcc_bitint_from_f16(limb *d, unsigned bits, int is_signed, _Float16 value) {
    __bcc_bitint_from_f64(d, bits, is_signed, value);
}

BCC_ROUTINE void __bcc_bitint_from_bf16(limb *d, unsigned bits, int is_signed, __bf16 value) {
    __bcc_bitint_from_f64(d, bits, is_signed, value);
}

#if __LDBL_MANT_DIG__ == 64
BCC_ROUTINE void __bcc_bitint_from_f80(limb *d, unsigned bits, int is_signed, long double value) {
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
BCC_ROUTINE void __bcc_bitint_from_f128(limb *d, unsigned bits, int is_signed, __float128 value) {
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
    BCC_ROUTINE T name(const limb *a, unsigned bits, int is_signed) {                \
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

/* a rounded once to __bf16's 8 significant bits, to nearest even, as a
   float, which holds that value exactly: GCC's conversion. Clang rounds
   through float instead, which lowering does with __bcc_bitint_to_f32. */
BCC_ROUTINE float __bcc_bitint_to_bf16(const limb *a, unsigned bits, int is_signed) {
    int negative = __bcc_bitint_negative(a, bits, is_signed);
    limb m[LIMBS(bits)];
    unsigned scale;
    wide top = __bcc_bitint_top(m, __bcc_bitint_absolute(m, a, bits, is_signed, negative), &scale);
    limb high = (limb)(top >> 64);
    int leading = high ? 127 - __builtin_clzll(high) : top ? 63 - __builtin_clzll((limb)top) : 0;
    if (leading > 7) {
        unsigned shift = (unsigned)leading - 7;
        wide rest = top & (((wide)1 << shift) - 1), half = (wide)1 << (shift - 1);
        top >>= shift;
        if (rest > half || (rest == half && (top & 1)))
            top++;
        scale += shift;
    }
    float result = (float)top;
    for (; scale >= 32; scale -= 32)
        result *= 4294967296.0f;
    result *= (float)(1ull << scale);
    return negative ? -result : result;
}

/* The type-generic bit builtins and <stdbit.h> (GCC manual, "Bit Operation
   Builtins"; C23 7.18), on a's low `bits` bits: the counts lowering derives
   the other operations from, and the operations whose result is a value of
   a's type. A count is `bits` where every bit is the other one. */

/* The number of one bits. */
BCC_ROUTINE unsigned __bcc_bitint_popcount(const limb *a, unsigned bits) {
    unsigned n = LIMBS(bits), count = 0;
    for (unsigned i = 0; i < n; i++)
        count += (unsigned)__builtin_popcountll(__bcc_bitint_limb(a, i, bits, 0));
    return count;
}

/* The number of bits equal to `one`, from bit bits - 1 down. */
BCC_ROUTINE unsigned __bcc_bitint_leading(const limb *a, unsigned bits, int one) {
    unsigned n = LIMBS(bits), spare = n * 64u - bits;
    limb flip = one ? ~0ull : 0;
    for (unsigned i = n; i-- > 0;) {
        limb x = __bcc_bitint_limb(a, i, bits, 0) ^ flip;
        if (i == n - 1)
            x &= ~0ull >> spare;
        if (x)
            return (n - 1 - i) * 64u + (unsigned)__builtin_clzll(x) - spare;
    }
    return bits;
}

/* The number of bits equal to `one`, from bit 0 up. */
BCC_ROUTINE unsigned __bcc_bitint_trailing(const limb *a, unsigned bits, int one) {
    unsigned n = LIMBS(bits);
    limb flip = one ? ~0ull : 0;
    for (unsigned i = 0; i < n; i++) {
        limb x = __bcc_bitint_limb(a, i, bits, 0) ^ flip;
        if (x) {
            unsigned count = i * 64u + (unsigned)__builtin_ctzll(x);
            return count < bits ? count : bits;
        }
    }
    return bits;
}

/* The number of bits after the sign bit equal to it (__builtin_clrsbg). */
BCC_ROUTINE unsigned __bcc_bitint_clrsb(const limb *a, unsigned bits) {
    return __bcc_bitint_leading(a, bits, __bcc_bitint_negative(a, bits, 1)) - 1;
}

/* The largest power of two not above a, or 0 for 0 (stdc_bit_floor). */
BCC_ROUTINE void __bcc_bitint_bit_floor(limb *d, const limb *a, unsigned bits) {
    unsigned n = LIMBS(bits), length = bits - __bcc_bitint_leading(a, bits, 0);
    for (unsigned i = 0; i < n; i++)
        d[i] = 0;
    if (length)
        d[(length - 1) / 64] = 1ull << (length - 1) % 64;
}

/* The smallest power of two not below a, or 0 where that needs more than
   `bits` bits (stdc_bit_ceil). */
BCC_ROUTINE void __bcc_bitint_bit_ceil(limb *d, const limb *a, unsigned bits) {
    unsigned n = LIMBS(bits), length = bits - __bcc_bitint_leading(a, bits, 0);
    unsigned bit = length;
    if (length == 0 || __bcc_bitint_popcount(a, bits) == 1)
        bit = length ? length - 1 : 0;
    for (unsigned i = 0; i < n; i++)
        d[i] = 0;
    if (bit < bits)
        d[bit / 64] = 1ull << bit % 64;
}

/* a rotated left by `count`, below `bits` (stdc_rotate_left; a rotation
   right is one left by bits - count). */
BCC_ROUTINE void __bcc_bitint_rotate(limb *d, const limb *a, unsigned bits, limb count) {
    unsigned n = LIMBS(bits);
    /* Bit i of the result is bit `from` of a. */
    limb from = count ? bits - count : 0;
    for (unsigned i = 0; i < n; i++) {
        limb out = 0;
        for (unsigned k = 0; k < 64 && i * 64u + k < bits; k++) {
            out |= (__bcc_bitint_limb(a, (unsigned)(from / 64), bits, 0) >> from % 64 & 1) << k;
            if (++from == bits)
                from = 0;
        }
        d[i] = out;
    }
}

/* a modulo m, from 0 to m - 1 also for a negative a: a rotation count
   reduced to the width m. The limbs of a, extended to whole limbs, are
   2^(64n) + a for a negative a, so subtracting 2^(64n) mod m corrects
   their residue. */
BCC_ROUTINE limb __bcc_bitint_residue(const limb *a, unsigned bits, int is_signed, limb m) {
    unsigned n = LIMBS(bits);
    limb residue = 0, power = 1 % m;
    for (unsigned i = n; i-- > 0;) {
        residue = (limb)(((wide)residue << 64 | __bcc_bitint_limb(a, i, bits, is_signed)) % m);
        power = (limb)(((wide)power << 64) % m);
    }
    if (__bcc_bitint_negative(a, bits, is_signed))
        residue = (residue + m - power) % m;
    return residue;
}
