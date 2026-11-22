#include <stdio.h>

enum shape { CIRCLE, SQUARE, TRIANGLE };

int sides(enum shape s) {
    int n = 0;
    switch (s) {
    case CIRCLE:
        n = 0;
        break;
    case SQUARE:
        n = 4;
        break;
    default:
        n = 3;
    }
    return n;
}

int main(void) {
    printf("%d\n", sides(SQUARE));
    return sides(TRIANGLE);
}
