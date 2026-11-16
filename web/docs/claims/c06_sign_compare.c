// -1 < 1u, and which warning options catch a signed/unsigned comparison.
#include <stdio.h>
int main(void) {
    int below = -1 < 1u;
    int n = -1;
    unsigned u = 1;
    int below2 = n < u;
    int k = 3;
    int below3 = k < sizeof(int);
    printf("%d %d %d\n", below, below2, below3);
    return 0;
}
