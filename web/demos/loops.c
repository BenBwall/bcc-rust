// One sum three ways: each loop
// adds 1 and 3, and skips 2.
#include <stdio.h>

int main(void) {
    // while tests before each pass;
    // continue goes to the test.
    int a = 0;
    int i = 0;
    while (i < 3) {
        i++;
        if (i == 2)
            continue;
        a += i;
    }

    // do tests after each pass;
    // continue goes to the test.
    int b = 0;
    int j = 0;
    do {
        j++;
        if (j == 2)
            continue;
        b += j;
    } while (j < 3);

    // for tests before each pass;
    // continue goes to k++ first.
    int c = 0;
    for (int k = 1; k <= 3; k++) {
        if (k == 2)
            continue;
        c += k;
    }

    printf("%d %d %d\n", a, b, c);
    return 0;
}
