// %d given a pointer: what prints, and which compilers warn.
#include <stdio.h>
int main(void) {
    int x = 5;
    printf("%d\n", &x);
    return 0;
}
