/* Floating constants converted out of an integer type's range in static
 * initializers. The behavior is undefined; GCC and Clang both fold the
 * conversion to the nearest bound of the type (0 for a NaN). GCC saturates a
 * bit-field to its own width; Clang saturates to the declared type, whose
 * low bits the bit-field keeps. Clang's code generation refuses such a
 * conversion as a whole scalar initializer ("cannot compile this static
 * initializer yet") and rejects one stored into a bit-field, so only GCC
 * is the reference for those. */
#include <stdio.h>

int ints[] = {-1e10, 1e10, (int)1e10, (int)-1e10 / 2, 2147483647.5};
unsigned unsigneds[] = {-1.0, 1e10, -0.5, 4294967296.0, 4294967295.9};
short shorts[] = {1e5, -1e5};
signed char signed_chars[] = {300.0, -300.0};
unsigned char unsigned_chars[] = {-1.5, 256.0, (unsigned char)-1.0};
long long longs[] = {1e19, -1e19};
unsigned long long unsigned_longs[] = {1e20, -1.0};
int specials[] = {__builtin_nan(""), -__builtin_nan(""), __builtin_inf(), -__builtin_inf()};
int from_float[] = {3e9f, -3e9f};
int from_long_double[] = {1e10L, -1e30L};
unsigned __int128 unsigned_wide[] = {1e40, -1.0};
__int128 wide[] = {-1e40, 1e40};
struct record { int i; unsigned char c; } record = {1e10, 300.0};
struct fields { unsigned a : 4; int b : 3; _Bool c : 1; } fields = {100.0, -100.0, 5.0};
_Bool truth[] = {1e300, 0.5, -0.0};
#ifndef __clang__
/* Clang rejects an out-of-range value stored into a bit-field. */
struct casts { unsigned a : 4; int b : 3; } casts = {(unsigned)1e10, (int)-1e10};
struct overflowing { unsigned a : 4; long long b : 40; } overflowing = {1e10, 1e30};
static int scalar = 1e10;
unsigned scalar_unsigned = -1.0;
int scalar_cast = (int)1e10;
#endif

int main(void) {
    static int local[] = {1e10, -1e10};
    printf("ints: %d %d %d %d %d\n", ints[0], ints[1], ints[2], ints[3], ints[4]);
    printf("unsigneds: %u %u %u %u %u\n", unsigneds[0], unsigneds[1], unsigneds[2],
           unsigneds[3], unsigneds[4]);
    printf("shorts: %d %d\n", shorts[0], shorts[1]);
    printf("signed chars: %d %d\n", signed_chars[0], signed_chars[1]);
    printf("unsigned chars: %u %u %u\n", unsigned_chars[0], unsigned_chars[1],
           unsigned_chars[2]);
    printf("longs: %lld %lld\n", longs[0], longs[1]);
    printf("unsigned longs: %llu %llu\n", unsigned_longs[0], unsigned_longs[1]);
    printf("specials: %d %d %d %d\n", specials[0], specials[1], specials[2], specials[3]);
    printf("from float: %d %d\n", from_float[0], from_float[1]);
    printf("from long double: %d %d\n", from_long_double[0], from_long_double[1]);
    printf("unsigned wide: %016llx%016llx %016llx%016llx\n",
           (unsigned long long)(unsigned_wide[0] >> 64), (unsigned long long)unsigned_wide[0],
           (unsigned long long)(unsigned_wide[1] >> 64), (unsigned long long)unsigned_wide[1]);
    printf("wide: %016llx%016llx %016llx%016llx\n",
           (unsigned long long)((unsigned __int128)wide[0] >> 64), (unsigned long long)wide[0],
           (unsigned long long)((unsigned __int128)wide[1] >> 64), (unsigned long long)wide[1]);
    printf("record: %d %u\n", record.i, record.c);
    printf("fields: %u %d %d\n", fields.a, fields.b, fields.c);
    printf("truth: %d %d %d\n", truth[0], truth[1], truth[2]);
    printf("local: %d %d\n", local[0], local[1]);
#ifndef __clang__
    printf("casts: %u %d\n", casts.a, casts.b);
    printf("overflowing: %u %lld\n", overflowing.a, (long long)overflowing.b);
    printf("scalars: %d %u %d\n", scalar, scalar_unsigned, scalar_cast);
#endif
    return 0;
}
