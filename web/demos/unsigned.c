// Unsigned arithmetic wraps around; signed arithmetic must not overflow.
#include <stdio.h>

int main(void) {
    unsigned int small = 2;
    // defined: 2 - 3 wraps around
    // to 4294967295
    small = small - 3;
    printf("2u - 3 is %u\n", small);

    unsigned int i = 2;
    // stands in for i >= 0, which
    // never fails
    while (i < 3) {
        // 2, 1, 0, then 4294967295
        i--;
    }
    printf("the countdown ended at %u\n", i);

    int values[3] = {4, 5, 6};
    unsigned int sum = 0;
    // the countdown that works:
    // 2, 1, 0
    for (unsigned int k = 3; k-- > 0;) {
        sum += values[k];
    }
    // 12 bytes, a size_t
    size_t size = sizeof values;
    // -1 converts to unsigned: 0
    int below = -1 < 1u;
    // the highest bit
    unsigned int top = 1u << 31;
    // shifted out: 0
    unsigned int lost = top << 1;
    printf("%zu bytes, sum %u, -1 < 1u is %d, %u, %u\n", size, sum, below, top, lost);
    return 0;
}
