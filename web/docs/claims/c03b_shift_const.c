// A shift by the width with a count the optimizer can see.
#include <stdio.h>
int shl(int x, int n) { return x << n; }
int main(void) {
    printf("shl(1, 32) = %d\n", shl(1, 32));
    return 0;
}
