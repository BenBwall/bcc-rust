/* Bit-precise integers of more than 128 bits passed between separately
 * compiled halves: the definitions (CALLEE_ONLY) and the calls that check
 * them (CALLER_ONLY), each built by bcc, Clang or GCC. Without either
 * macro the program is whole. Such a value travels in memory: System V
 * copies it to the stack, 8-aligned, and returns it through a hidden
 * pointer; it is not split between registers and the stack even when
 * registers remain. The values set the bits next to the padding so that a
 * side reading padding bits as value bits prints a wrong value. */
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

typedef unsigned long long limb;
typedef _BitInt(129) b129;
typedef unsigned _BitInt(192) u192;
typedef _BitInt(256) b256;
typedef unsigned _BitInt(257) u257;
typedef _BitInt(1000) b1000;

struct holder {
    char tag;
    b129 value;
    int after;
};

b129 add129(b129 a, b129 b);
u192 mul192(u192 a, u192 b);
b256 sub256(int a, b256 x, double d, b256 y, int b);
u257 shift257(u257 x, int n);
b1000 div1000(b1000 a, b1000 b);
b129 spill(int a, int b, int c, int d, int e, int f, b129 x, long g, b256 y, char h);
b256 floats(double a, b256 x, float b, long double c, b256 y);
struct holder make_holder(b129 value, int after);
b129 sum_holder(struct holder h);
b256 sum_varargs(int count, ...);
b1000 apply(b1000 (*f)(b1000, b1000), b1000 a, b1000 b);
b1000 callback_sub(b1000 a, b1000 b);
int compare(b256 a, b256 b);
long double to_long_double(u257 x);
b1000 from_double(double d);

static void show(const char *tag, const void *p, int bits) {
    const limb *l = p;
    int n = (bits + 63) / 64;
    limb top = l[n - 1];
    if (bits % 64)
        top &= (1ULL << bits % 64) - 1;
    printf("%s %llx", tag, top);
    for (int i = n - 2; i >= 0 && i >= n - 5; i--)
        printf(" %016llx", l[i]);
    limb hash = 0xcbf29ce484222325ULL;
    for (int i = 0; i < n; i++)
        hash = (hash ^ (i == n - 1 ? top : l[i])) * 0x100000001b3ULL;
    printf(" #%016llx\n", hash);
}

#define SHOW(tag, type, bits, x) do { type value = (x); show(tag, &value, bits); } while (0)

#ifndef CALLER_ONLY
b129 add129(b129 a, b129 b) { return a + b; }
u192 mul192(u192 a, u192 b) { return a * b; }
b256 sub256(int a, b256 x, double d, b256 y, int b) { return x - y + a - b + (int)d; }
u257 shift257(u257 x, int n) { return x << n; }
b1000 div1000(b1000 a, b1000 b) { return a / b; }

b129 spill(int a, int b, int c, int d, int e, int f, b129 x, long g, b256 y, char h) {
    return x + a + b + c + d + e + f + g + h + (b129)(y >> 130);
}

b256 floats(double a, b256 x, float b, long double c, b256 y) {
    return x + y + (b256)a + (b256)b + (b256)c;
}

struct holder make_holder(b129 value, int after) {
    struct holder h = {'h', value, after};
    return h;
}

b129 sum_holder(struct holder h) { return h.value + h.after + h.tag; }

b256 sum_varargs(int count, ...) {
    va_list ap;
    va_start(ap, count);
    b256 sum = 0;
    for (int i = 0; i < count; i++) {
        sum += va_arg(ap, b256);
        sum += va_arg(ap, int);
    }
    va_end(ap);
    return sum;
}

b1000 apply(b1000 (*f)(b1000, b1000), b1000 a, b1000 b) { return f(a, b) + 1; }
b1000 callback_sub(b1000 a, b1000 b) { return a - b; }
int compare(b256 a, b256 b) { return (a < b) - (a > b) + 4 * (a == b); }
long double to_long_double(u257 x) { return x; }
b1000 from_double(double d) { return d; }
#endif

#ifndef CALLEE_ONLY
int main(void) {
    volatile b129 big129 = ((b129)1 << 127) + ((b129)0x123456789abcdefLL << 64) + 0x5555;
    volatile b129 neg129 = (b129)(((unsigned _BitInt(129))1 << 128) + 77);
    volatile u192 big192 = ~(u192)0 / 3;
    volatile b256 big256 = ((b256)0x7edcba9876543210LL << 190) | 0xabcdef;
    volatile b256 neg256 = -((b256)1 << 254) - 12345;
    volatile u257 top257 = (u257)1 << 256 | 0x1234;
    /* Clang 23 converts an unsigned value with its top bit set to long
     * double as if it were negative, so this one has it clear. */
    volatile u257 high257 = (u257)1 << 255 | 0x1234;
    volatile b1000 big1000 = ((b1000)0x76543210fedcbaLL << 900) + ((b1000)0xfedcba98LL << 400) + 7;
    volatile b1000 divisor = ((b1000)0x123456789LL << 300) - 1;

    SHOW("add129", b129, 129, add129(big129, neg129));
    SHOW("mul192", u192, 192, mul192(big192, big192));
    SHOW("sub256", b256, 256, sub256(5, big256, 2.5, neg256, 9));
    SHOW("shift257", u257, 257, shift257(top257 | 1, 3));
    SHOW("div1000", b1000, 1000, div1000(big1000, divisor));
    SHOW("div1000n", b1000, 1000, div1000(-big1000, divisor));
    SHOW("spill", b129, 129, spill(1, 2, 3, 4, 5, 6, big129, -7L, big256, 'c'));
    SHOW("floats", b256, 256, floats(1e30, big256, 2.5f, -3.0e40L, neg256));
    struct holder h = make_holder(neg129, 41);
    printf("holder %c %d\n", h.tag, h.after);
    SHOW("holder", b129, 129, h.value);
    SHOW("sum_holder", b129, 129, sum_holder(h));
    SHOW("varargs", b256, 256, sum_varargs(3, big256, 1, neg256, 2, (b256)-1, 3));
    SHOW("apply", b1000, 1000, apply(callback_sub, big1000, divisor));
    printf("compare %d %d %d\n", compare(big256, neg256), compare(neg256, big256), compare(big256, big256));
    long double ld = to_long_double(high257);
    unsigned char bytes[10];
    memcpy(bytes, &ld, 10);
    printf("long double");
    for (int i = 9; i >= 0; i--)
        printf(" %02x", bytes[i]);
    printf("\n");
    SHOW("from_double", b1000, 1000, from_double(-0x1.23456789abcdep900));
    return 0;
}
#endif
