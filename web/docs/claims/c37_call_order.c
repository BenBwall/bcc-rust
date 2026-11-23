/* A call and another operand that reads what the call changes: C99
   leaves their order unspecified (6.5p3, 6.5.2.2p10). */
#include <stdio.h>
int g;
int f(void) { g = 10; return 1; }
int main(void) {
    printf("%d\n", g + f());
    printf("%d %d\n", printf("a"), printf("b"));
    return 0;
}
