// && and || stop once they know
// the answer; ?: runs one side.
#include <stdio.h>

int main(void) {
    int scores[3] = {7, 9, 4};
    int i = 0;
    // Never reads scores[3]:
    while (i < 3 && scores[i] != 5) {
        i++;
    }
    // No 5, so found is null,
    int *found = i < 3 ? &scores[i] : 0;
    // and the test guards *found.
    if (found != 0 && *found == 5) {
        puts("found a 5");
    }
    printf("%s after %d tries\n",
           found ? "found" : "not found", i);
    return 0;
}
