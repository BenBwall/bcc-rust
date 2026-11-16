// Whether the optimizer deletes x + 1 > x.
#include <stdio.h>
#include <limits.h>
int will_not_overflow(int x) { return x + 1 > x; }
int main(void) {
    volatile int m = INT_MAX;
    printf("INT_MAX + 1 > INT_MAX: %d\n", will_not_overflow(m));
    return 0;
}
