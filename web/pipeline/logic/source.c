// && and || stop as soon as they know the answer; ?: runs only one side.
#include <stdio.h>

int main(void) {
    int scores[3] = {7, 9, 4};
    int i = 0;
    while (i < 3 && scores[i] != 5) {   // at i == 3, scores[3] is never read
        i++;
    }
    int *found = i < 3 ? &scores[i] : 0;   // no 5, so a null pointer
    if (found != 0 && *found == 5) {       // the null test guards the *
        puts("found a 5");
    }
    printf("%s after %d tries\n", found ? "found" : "not found", i);
    return 0;
}
