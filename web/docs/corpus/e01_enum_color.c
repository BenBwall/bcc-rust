#include <stdio.h>

enum color { RED, GREEN = 5, BLUE };

int main(void) {
    enum color c = BLUE;
    printf("%d %d %d\n", RED, GREEN, c);
    return c - GREEN;
}
