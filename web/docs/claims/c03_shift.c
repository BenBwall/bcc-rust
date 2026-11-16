// A shift by the width at run time, and 1u << 31 shifted out.
#include <stdio.h>
int main(void) {
    volatile int count = 32;
    int one = 1;
    unsigned top = 1u << 31;
    printf("1 << 32 (run time) = %d\n", one << count);
    printf("1u << 31 = %u, << 1 = %u\n", top, top << 1);
    return 0;
}
