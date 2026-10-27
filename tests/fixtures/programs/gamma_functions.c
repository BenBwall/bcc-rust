/* The gamma functions in each precision: lgamma, lgammaf and lgammal with
   the sign they leave in signgam, the reentrant lgamma_r forms, glibc's
   gamma names, and tgamma, with errno. long double results print as
   their bits. float and double come from the host's libm, and glibc
   replaced those functions with CORE-MATH's after 2.39 (tgammaf in
   392b3f0971 and lgammaf in d846f4c12d, glibc 2.41; lgamma in d67d2f4688
   and tgamma in 1cae0550e8, glibc 2.43), so they print to fewer digits
   than differ between 2.39 and 2.43, without a NaN's sign (glibc's
   wrappers and its static library choose differently), and without the
   sign at a pole, which the new lgamma gives differently; the long double
   functions are unchanged. */

#define _GNU_SOURCE 1
#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

static void show_double(const char *label, double value, int digits) {
    if (isnan(value))
        printf("%s nan", label);
    else
        printf("%s %.*g", label, digits, value);
}

/* The sign of a float or double lgamma, except at a pole. */
static int pole_free(long double x, int sign) {
    return x == floorl(x) && x <= 0 ? 0 : sign;
}

static void show_long(const char *label, long double value) {
    unsigned char bytes[sizeof value];
    unsigned long long significand;
    unsigned exponent;
    memcpy(bytes, &value, sizeof value);
    memcpy(&significand, bytes, 8);
    exponent = bytes[8] | (unsigned)bytes[9] << 8;
    printf("%s %04x:%016llx", label, exponent, significand);
}

static const long double arguments[] = {
    0.0L, -0.0L, 1.0L, 2.0L, 0.5L, -0.5L, 1e-30L, -1e-30L, 0x1p-70L, -0x1p-70L,
    1.4616321449683623L, 0.9L, 1.25L, 1.75L, 2.5L, 3.7L, 7.99L, 8.5L,
    -1.5L, -2.0L, -2.5L, -2.0625L, -2.4570247382208005860L, -2.75L, -3.0L,
    -3.1L, -4.5L, -10.3L, -17.9999L, -32.9L, -33.0L, -33.5L, -100.25L,
    -170.5L, -1754.5L, -1800.5L, 12.5L, 33.25L, 100.25L, 171.7L, 1000.5L,
    1755.0L, 1756.0L, 1e10L, 1e30L, 1e4000L,
};

int main(void) {
    for (unsigned i = 0; i < sizeof arguments / sizeof arguments[0]; i++) {
        long double x = arguments[i];
        int sign;
        printf("%u:", i);

        errno = 0;
        signgam = 7;
        show_double(" lgamma", lgamma((double)x), 12);
        printf(" %d %d", pole_free(x, signgam), errno);
        errno = 0;
        signgam = 7;
        show_double(" lgammaf", lgammaf((float)x), 5);
        printf(" %d %d", pole_free(x, signgam), errno);
        errno = 0;
        signgam = 7;
        show_long(" lgammal", lgammal(x));
        printf(" %d %d\n", signgam, errno);

        sign = 7;
        show_double("  lgamma_r", lgamma_r((double)x, &sign), 12);
        printf(" %d", pole_free(x, sign));
        sign = 7;
        show_double(" lgammaf_r", lgammaf_r((float)x, &sign), 5);
        printf(" %d", pole_free(x, sign));
        sign = 7;
        show_long(" lgammal_r", lgammal_r(x, &sign));
        printf(" %d", sign);
        signgam = 7;
        show_long(" gammal", gammal(x));
        printf(" %d", signgam);
        signgam = 7;
        show_double(" gamma", gamma((double)x), 12);
        printf(" %d\n", pole_free(x, signgam));

        errno = 0;
        show_double("  tgamma", tgamma((double)x), 12);
        printf(" %d", errno);
        errno = 0;
        show_double(" tgammaf", tgammaf((float)x), 5);
        printf(" %d", errno);
        errno = 0;
        show_long(" tgammal", tgammal(x));
        printf(" %d\n", errno);
    }
    /* Infinities and a NaN, computed at run time. */
    volatile long double zero = 0.0L;
    long double specials[] = {1.0L / zero, -1.0L / zero, zero / zero, -(zero / zero)};
    for (unsigned i = 0; i < 4; i++) {
        long double x = specials[i];
        errno = 0;
        signgam = 7;
        show_long("special lgammal", lgammal(x));
        printf(" %d %d", signgam, errno);
        errno = 0;
        show_long(" tgammal", tgammal(x));
        printf(" %d\n", errno);
    }
    return 0;
}
