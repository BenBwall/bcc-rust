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
#define CAT(a, b) a##b
int commas(int v) {
    v = SELECT(v, (0, 311));
    v = SELECT(v, SYSTEM_COMMA(312));
    v = immediate(v, SYSTEM_COMMA(313));
    v = SELECT(v, (0, v ? 314 : 315));
    return SYSTEM_NAME(v, 316) + SYSTEM_CAT(immedi, ate)(v, 317);
}
char comma_returned(void) { RETURN((0, 318)); }
char comma_listed[] = PAIR((0, 319));
struct holder revisited = { .pair.low = 1, HIGH };
struct holder reversed = { INNER, .pair.high = 320 };
struct holder elided_once = { 1, HIGH };
#define DECLARE_CHAR(name, value) char name = value
DECLARE_CHAR(SYSTEM_CAT(paste, d), 322);
DECLARE_CHAR(CAT(paste, e), 323);
int following_valid;
