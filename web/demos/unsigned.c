// Unsigned arithmetic wraps around; signed arithmetic must not overflow.
#include <stdio.h>

int main(void) {
    unsigned int small = 2;
    small = small - 3;             // defined: 2 - 3 wraps to 4294967295
    printf("2u - 3 is %u\n", small);

    unsigned int i = 2;
    while (i < 3) {                // stands in for i >= 0, which never fails
        i--;                       // 2, 1, 0, then 4294967295
    }
    printf("the countdown ended at %u\n", i);

    int values[3] = {4, 5, 6};
    unsigned int sum = 0;
    for (unsigned int k = 3; k-- > 0;) {   // the countdown that works: 2, 1, 0
        sum += values[k];
    }
    size_t size = sizeof values;   // 12 bytes, a size_t
    int below = -1 < 1u;           // -1 converts to unsigned: 0
    unsigned int top = 1u << 31;   // the highest bit
    unsigned int lost = top << 1;  // shifted out: 0
    printf("%zu bytes, sum %u, -1 < 1u is %d, %u, %u\n", size, sum, below, top, lost);
    return 0;
}
