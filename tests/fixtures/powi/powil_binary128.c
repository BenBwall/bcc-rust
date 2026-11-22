/* __builtin_powil on a binary128 long double (wasm32): the bits of each
   result. powil_binary128.expected is what GCC 14 prints for libgcc's
   algorithm (__powitf2) on __float128 on x86-64 Linux. */
#include <stdio.h>
#include <string.h>
#include <limits.h>
#include <math.h>
typedef long double Q;
#define POW(b, e) __builtin_powil((Q)(b), e)
static double bases[] = {0.0, -0.0, 1.0, -1.0, 2.0, -2.0, 0.5, -0.5, 3.0, 1.1, -1.1, 1.0000001, 0.999999,
    10.0, 1e-300, 1e300, 4.9e-324, 1.7976931348623157e308, 3.14159265358979, -2.718281828459045,
    INFINITY, -INFINITY, NAN, -NAN, 123456.789, 1.5, -7.25};
static int exps[] = {0, 1, -1, 2, -2, 3, -3, 4, 5, 7, 10, -10, 15, 31, 32, 33, 63, 64, 100, -100, 1023, 1024,
    -1074, -1075, 16383, 16384, -16494, 1000000, -1000000, INT_MAX, INT_MIN, INT_MIN + 1, 0x55555555};
static volatile double vb; static volatile int ve;
int main(void) {
    for (unsigned i = 0; i < sizeof bases / sizeof *bases; i++)
        for (unsigned j = 0; j < sizeof exps / sizeof *exps; j++) {
            vb = bases[i]; ve = exps[j];
            Q l = POW(vb, ve);
            unsigned char b[16]; memcpy(b, &l, 16);
            printf("%u %d ", i, exps[j]);
            for (int k = 15; k >= 0; k--) printf("%02x", b[k]);
            printf("\n");
        }
    return 0;
}
