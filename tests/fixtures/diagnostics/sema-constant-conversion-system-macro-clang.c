#include <conversion-macros.h>
#define USER_SELECT(value, selector) SELECT(value, selector)
struct pair { char low, high; };
struct holder { struct pair pair; char last; };
int calls(int v, char *p) {
    v = SELECT(v, 255);
    v = IMMEDIATE(v, 128);
    v = USER_SELECT(v, 300);
    v = SELECT(v, 2.5);
    v = immediate(v, WIDE);
    STORE(p, 302);
    *p = WIDE;
    return v;
}
char returned(void) { RETURN(303); }
char also_returned(void) { return WIDE; }
char listed[] = PAIR(304);
char declared = WIDE;
struct pair elided[] = { WIDE, 305, 306 };
struct holder designated = { INNER, 308 };
struct holder written = { .pair.low = 1, 310 };
void formats(void) {
    printf("%d\n", 1, EXTRA_ARGUMENT);
    PRINT_EXTRA("%d\n");
    printf("%d\n", 1, 2);
}
int following_valid;
