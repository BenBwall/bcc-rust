// && and || stop once they know the
// answer; ?: runs only the side it picks.
#include <stdio.h>

int main(void) {
    int scores[3] = {7, 9, 4};
    int i = 0;
    // At i == 3, scores[3] is never read.
    while (i < 3 && scores[i] != 5) {
        i++;
    }
    // No 5, so found is a null pointer,
    int *found = i < 3 ? &scores[i] : 0;
    // and the null test guards the *.
    if (found != 0 && *found == 5) {
        puts("found a 5");
    }
    printf("%s after %d tries\n",
           found ? "found" : "not found", i);
    return 0;
}
