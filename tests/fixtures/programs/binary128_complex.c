/* Complex binary128 multiplication and division (libgcc's __multc3 and
   __divtc3) on special values and random encodings: zeros, infinities,
   NaNs with payloads, the extremes, subnormals and values near the
   division's scaling thresholds. Prints every result's bits. */

int printf(const char *, ...);

typedef __float128 quad;
/* GCC spells the complex type only through a mode attribute. */
typedef __typeof__(0.0Qi) complex_quad;

typedef unsigned long long u64;

static u64 state = 0x9E3779B97F4A7C15ull;

static u64 next(void) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return state;
}

static quad from_bits(u64 high, u64 low) {
    union {
        quad value;
        u64 words[2];
    } u;
    u.words[0] = low;
    u.words[1] = high;
    return u.value;
}

static void show(const char *label, int index, quad value) {
    union {
        quad value;
        u64 words[2];
    } u;
    u.value = value;
    printf("%s%d %016llx%016llx\n", label, index, u.words[1], u.words[0]);
}

/* Encodings that reach each branch of the helpers. */
static const u64 specials[][2] = {
    {0x0000000000000000ull, 0},                     /* +0 */
    {0x8000000000000000ull, 0},                     /* -0 */
    {0x3FFF000000000000ull, 0},                     /* 1 */
    {0xBFFF800000000000ull, 0},                     /* -1.5 */
    {0x7FFF000000000000ull, 0},                     /* +inf */
    {0xFFFF000000000000ull, 0},                     /* -inf */
    {0x7FFF800000000000ull, 0},                     /* quiet NaN */
    {0xFFFF800000000000ull, 0x1234},                /* negative NaN with payload */
    {0x7FFF400000000000ull, 0x5},                   /* signaling NaN */
    {0x7FFEFFFFFFFFFFFFull, 0xFFFFFFFFFFFFFFFFull}, /* max */
    {0xFFFDFFFFFFFFFFFFull, 0xFFFFFFFFFFFFFFFFull}, /* -RBIG */
    {0x7FFE000000000000ull, 0},                     /* RBIG's neighbour above */
    {0x0001000000000000ull, 0},                     /* min normal */
    {0x0000800000000000ull, 0},                     /* subnormal */
    {0x8000000000000000ull, 1},                     /* -true min */
    {0x3F8F000000000000ull, 0},                     /* RMIN2 */
    {0x3F8EFFFFFFFFFFFFull, 0xFFFFFFFFFFFFFFFFull}, /* below RMIN2 */
    {0x7F8DFFFFFFFFFFFFull, 0xFFFFFFFFFFFFFFFFull}, /* RMAX2 */
    {0x4004400000000000ull, 0},                     /* 40 */
    {0xC3FF123456789ABCull, 0xDEF0123456789ABCull}, /* large */
    {0x3C00FEDCBA987654ull, 0x3210FEDCBA987654ull}, /* small */
};

#define SPECIALS ((int)(sizeof specials / sizeof specials[0]))

/* A part: a special encoding, or a random one whose exponent clusters
   near the middle, the extremes or the subnormals. */
static quad part(void) {
    u64 choice = next() % 8;
    if (choice < 3) {
        u64 index = next() % SPECIALS;
        return from_bits(specials[index][0], specials[index][1]);
    }
    u64 high = next(), low = next();
    u64 sign = high & 0x8000000000000000ull;
    u64 fraction = high & 0x0000FFFFFFFFFFFFull;
    u64 exponent;
    switch (choice) {
    case 3: exponent = 0x3FFF - 40 + next() % 80; break;
    case 4: exponent = 0x7FFE - next() % 130; break;
    case 5: exponent = next() % 130; break;
    case 6: exponent = 0x3F8F - 4 + next() % 8; break;
    default: exponent = next() % 0x8000; break;
    }
    return from_bits(sign | exponent << 48 | fraction, low);
}

int main(void) {
    /* Every pair of specials as the divisor, with a fixed dividend. */
    volatile complex_quad z, w;
    int index = 0;
    for (int i = 0; i < SPECIALS; i++) {
        for (int j = 0; j < SPECIALS; j++, index++) {
            __real__ z = from_bits(0x3FFF000000000000ull, 0);
            __imag__ z = from_bits(specials[(i + j) % SPECIALS][0], specials[(i + j) % SPECIALS][1]);
            __real__ w = from_bits(specials[i][0], specials[i][1]);
            __imag__ w = from_bits(specials[j][0], specials[j][1]);
            complex_quad product = z * w, quotient = z / w;
            show("mr", index, __real__ product);
            show("mi", index, __imag__ product);
            show("dr", index, __real__ quotient);
            show("di", index, __imag__ quotient);
        }
    }
    for (int i = 0; i < 2000; i++, index++) {
        __real__ z = part();
        __imag__ z = part();
        __real__ w = part();
        __imag__ w = part();
        complex_quad product = z * w, quotient = z / w;
        show("mr", index, __real__ product);
        show("mi", index, __imag__ product);
        show("dr", index, __real__ quotient);
        show("di", index, __imag__ quotient);
    }
    return 0;
}
