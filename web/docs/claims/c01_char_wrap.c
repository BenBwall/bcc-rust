// Out-of-range conversions to signed types: (char)300, a store of 200 into a char.
#include <stdio.h>
int main(void) {
    volatile int big = 300, two_hundred = 200;
    char a = (char)300;            /* constant cast */
    char b = (char)big;            /* run-time cast */
    char small = 100;
    char narrow = small + small;   /* store of 200 */
    char c = (char)two_hundred;
    signed char d = (signed char)-129;
    short e = (short)70000;
    printf("%d %d %d %d %d %d\n", a, b, narrow, c, d, e);
    return 0;
}
