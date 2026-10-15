/* NOT a bcc defect: the bundled Clang 23.1.1 (x86_64-w64-windows-gnu) reads
   back wrong values from a #pragma pack(1) record with a zero-width bit-field
   (its fields overlap). MinGW GCC and bcc print the initialized values. This is
   why Csmith seeds 1000061 and 1000312 disagree with the reference. */
#include <stdio.h>
#pragma pack(push)
#pragma pack(1)
struct S0 { signed char f0; unsigned f1 : 18; unsigned : 0; signed f2 : 4; signed f3 : 27; unsigned f4 : 29; signed f5 : 12; signed f6 : 22; unsigned f7 : 13; signed f8 : 31; };
#pragma pack(pop)
static struct S0 g = {0x0F, 43, 3, 5396, 4178, 8, -1457, 84, 437};
int main(void) {
    struct S0 s = {0x4F, 186, -1, -10539, 17499, 48, -1348, 41, 40855};
    printf("%d %d %d %d %d %d %d %d %d %d\n", (int)sizeof g, g.f0, g.f1, g.f2, g.f3, g.f4, g.f5, g.f6, g.f7, g.f8);
    printf("%d %d %d %d %d %d %d %d %d\n", s.f0, s.f1, s.f2, s.f3, s.f4, s.f5, s.f6, s.f7, s.f8);
    s.f2 = 5; s.f1 = 1000; s.f3 = -7;
    printf("%d %d %d %d\n", s.f1, s.f2, s.f3, s.f4);
    return 0;
}
