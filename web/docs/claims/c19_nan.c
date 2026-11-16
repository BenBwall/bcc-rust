// What printf writes for an infinity and for 0.0 / 0.0 at run time.
#include <stdio.h>
int main(void) {
    volatile double zero = 0.0, one = 1.0;
    printf("%f %f %f\n", one / zero, -one / zero, zero / zero);
    return 0;
}
