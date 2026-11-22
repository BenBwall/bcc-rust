#include <stdio.h>
int counter = 0;
int next(void) { return ++counter; }
int main(void) {
    printf("%d %d\n", next(), next());
    return 0;
}
