// The preprocessor rewrites this file before the compiler proper reads it.
#include <limits.h>

#define VERBOSE 0
/* parenthesized, so 1 + 2 works */
#define SQUARE(x) ((x) * (x))
/* # makes its argument a string */
#define STR(x) #x
/* ## pastes two tokens into one */
#define GLUE(a, b) a ## b
#define SWAP(a, b) do {   \
        int t = (a);      \
        (a) = (b);        \
        (b) = t;          \
    } while (0)

int GLUE(count, er) = 0;

int main(void) {
    int low = SQUARE(1 + 2), high = INT_MAX / 2;
    const char *name = STR(SQUARE(3));
    if (low > high) {
        SWAP(low, high);
    }
#if VERBOSE
    counter = counter + 100;
#else
    counter = counter + 1;
#endif
    return low + counter + (name[0] == 'S');
}
