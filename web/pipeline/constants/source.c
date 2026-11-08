// Names for values: a macro is replaced before the program runs, a const
// object may not change, a cast converts a value, and += updates in place.
#include <stdio.h>

#define LIMIT 4
#define LABEL "total"

int main(void) {
    const int step = 3;
    int total = 0;
    for (int i = 0; i < LIMIT; i++) {
        total += step;
    }
    char small = (char)300;  // 300 does not fit in a char
    total += small;
    printf(LABEL " is %.3d\n", total);
    return 0;
}
