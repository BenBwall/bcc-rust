/* __builtin_powi, __builtin_powif and __builtin_powil over bases and
   exponents at their edges, with volatile operands so that no compiler
   folds them: the bits of each result. powi.x86_64.expected is what GCC 14
   and Clang 23 print on x86-64 Linux at -O0 and -O2 (the long double is
   x87's, its ten bytes). */
#include <stdio.h>
#include <string.h>
#include <limits.h>
#include <math.h>

static double bases[] = {0.0, -0.0, 1.0, -1.0, 2.0, -2.0, 0.5, -0.5, 3.0, 1.1, -1.1, 1.0000001, 0.999999,
    10.0, 1e-300, 1e300, 4.9e-324, 1.7976931348623157e308, 3.14159265358979, -2.718281828459045,
    INFINITY, -INFINITY, NAN, -NAN, 123456.789, 1.5, -7.25};
static int exps[] = {0, 1, -1, 2, -2, 3, -3, 4, 5, 7, 10, -10, 15, 31, 32, 33, 63, 64, 100, -100, 1023, 1024,
    -1074, -1075, 1000000, -1000000, INT_MAX, INT_MIN, INT_MIN + 1, 0x55555555, -0x55555555};

/* volatile operands keep the compilers from folding. */
static volatile double vb;
static volatile int ve;

int main(void) {
    unsigned long long h;
    for (unsigned i = 0; i < sizeof bases / sizeof *bases; i++)
        for (unsigned j = 0; j < sizeof exps / sizeof *exps; j++) {
            vb = bases[i];
            ve = exps[j];
            double d = __builtin_powi(vb, ve);
            float f = __builtin_powif((float)vb, ve);
            long double l = __builtin_powil((long double)vb, ve);
            unsigned int fb;
            unsigned char lb[16] = {0};
            memcpy(&h, &d, 8);
            memcpy(&fb, &f, 4);
            memcpy(lb, &l, sizeof l < 10 ? sizeof l : 10);
            printf("%u %d d=%016llx f=%08x l=", i, exps[j], h, fb);
            for (int k = (sizeof l < 10 ? (int)sizeof l : 10) - 1; k >= 0; k--) printf("%02x", lb[k]);
            printf("\n");
        }
    return 0;
}
