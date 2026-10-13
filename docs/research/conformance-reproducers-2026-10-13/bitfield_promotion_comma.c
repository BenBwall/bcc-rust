/* A bit-field narrower than int keeps its integer promotion (C99 6.3.1.1p2) when it is the right operand
   of a comma or the result of an assignment in GCC and Clang, so the unsigned 17-bit field below becomes int.
   bcc gives the comma and assignment results the declared type unsigned int instead, so mixed comparisons and
   arithmetic switch to unsigned: GCC and Clang print "1 1 1 1" / "1", bcc prints "0 1 0 1" / "0".
   On the Linux target (64-bit long) only the last line differs.
   (Csmith seeds 100167 and 300681, the latter a wrong checksum in all five arms.) */
#include <stdio.h>
struct B { unsigned f : 17; } b = {308};
int main(void) {
    int one = 1;
    printf("%d %d %d %d\n", ((0, b.f) & 1L) >= -7L, ((one ? b.f : b.f) & 1L) >= -7L,
           ((b.f = 308) & 1L) >= -7L, ((b.f) & 1L) >= -7L);
    printf("%d\n", (0, b.f) - 400 < 0);
    return 0;
}
