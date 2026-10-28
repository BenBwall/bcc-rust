/* The bytes of bit-precise objects, padding bits included: GCC and Clang
 * store a value extended from its width, by its signedness, both in static
 * initializers and at run time. Atomic _BitInts are GCC's only. */
#include <stdio.h>
#include <string.h>

static void bytes(const char *tag, const void *object, size_t size) {
    const unsigned char *b = object;
    printf("%s ", tag);
    for (size_t i = size; i-- > 0;) printf("%02x", b[i]);
    printf("\n");
}
#define SHOW(x) bytes(#x, &(x), sizeof(x))

static unsigned _BitInt(7) u7 = -1;
static _BitInt(7) s7 = -2;
static unsigned _BitInt(9) u9 = -1;
static _BitInt(9) s9 = -200;
static unsigned _BitInt(33) u33 = -1;
static _BitInt(33) s33 = -5;
static unsigned _BitInt(65) u65 = -1;
static _BitInt(65) s65 = -3;
static unsigned _BitInt(100) u100 = -1;
static _BitInt(100) s100 = -7;
static unsigned _BitInt(127) u127 = -1;
static _BitInt(127) s127 = -1;
static unsigned _BitInt(65) u65_float = 1.5e19;
static _BitInt(33) s33_float = -3.75;
static unsigned _BitInt(9) u9_bool = (_Bool)7;

struct members {
    char c;
    unsigned _BitInt(65) a;
    _BitInt(9) b;
    unsigned _BitInt(33) d;
};
static struct members members = {1, -1, -1, -2};
static unsigned _BitInt(7) array7[3] = {-1, 5, -3};
static _BitInt(100) array100[2] = {-1, 1};
static unsigned _BitInt(65) *literal = &(unsigned _BitInt(65)){-1};

#if __BITINT_MAXWIDTH__ > 128
/* Wide unsigned values with bit 127 of their pattern set, 2^N - k,
 * converted to every floating type, rounded once. */
static _Float16 w_half = (unsigned _BitInt(129))-5;
static float w_float = (unsigned _BitInt(200))-1;
static double w_double = (unsigned _BitInt(200))-1;
static double w_double2 = (unsigned _BitInt(150))(-(__int128)(((__int128)1 << 64) + 3));
static long double w_long = (unsigned _BitInt(193))-7;
static __float128 w_quad = (unsigned _BitInt(200))-1;
static __float128 w_quad2 = (unsigned _BitInt(130))(-(__int128)((__int128)1 << 100));
static _Complex double w_complex = (unsigned _BitInt(200))-1;
static double w_huge = (unsigned _BitInt(4000))-1;
#endif

#if defined(__GNUC__) && !defined(__clang__)
static _Atomic unsigned _BitInt(7) q7 = -1;
static _Atomic _BitInt(33) q33 = -9;
static _Atomic unsigned _BitInt(65) q65 = -1;
#endif

int main(void) {
    SHOW(u7); SHOW(s7); SHOW(u9); SHOW(s9); SHOW(u33); SHOW(s33);
    SHOW(u65); SHOW(s65); SHOW(u100); SHOW(s100); SHOW(u127); SHOW(s127);
    SHOW(u65_float); SHOW(s33_float); SHOW(u9_bool);
    SHOW(members.a); SHOW(members.b); SHOW(members.d);
    SHOW(array7); SHOW(array100);
    bytes("literal", literal, sizeof *literal);

    volatile int k = 1;
    unsigned _BitInt(7) r7 = (unsigned _BitInt(7))-k;
    _BitInt(9) r9 = -k * 3;
    unsigned _BitInt(33) r33 = (unsigned _BitInt(33))(-k) + 5;
    unsigned _BitInt(65) r65 = (unsigned _BitInt(65))-k;
    _BitInt(100) r100 = -k;
    unsigned _BitInt(127) r127 = (unsigned _BitInt(127))-k << 3;
    SHOW(r7); SHOW(r9); SHOW(r33); SHOW(r65); SHOW(r100); SHOW(r127);
    u7 += 100; u65 += 2; s100 = s100 * 3;
    SHOW(u7); SHOW(u65); SHOW(s100);

#if __BITINT_MAXWIDTH__ > 128
    bytes("w_half", &w_half, 2);
    SHOW(w_float); SHOW(w_double); SHOW(w_double2);
    bytes("w_long", &w_long, 10);
    SHOW(w_quad); SHOW(w_quad2); SHOW(w_complex); SHOW(w_huge);
#endif

#if defined(__GNUC__) && !defined(__clang__)
    printf("atomic %d %lld\n", (int)q7, (long long)q33);
    SHOW(q7); SHOW(q33); SHOW(q65);
    q7 += 50; q33 -= 4; q65 += 2;
    printf("atomic %d %lld\n", (int)q7, (long long)q33);
    SHOW(q65);
    printf("%d\n", (int)(q65 == 1));
#endif
    return 0;
}
