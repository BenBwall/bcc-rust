// Right shift of a negative int.
#include <stdio.h>
int main(void) {
    volatile int n = -8;
    printf("%d\n", n >> 1);
    return 0;
}
