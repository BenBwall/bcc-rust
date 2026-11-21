// `-Werror=extra` makes errors of the options Clang's `-Wextra` holds, and
// a later `-Wno-error=` of one takes that back.
struct P { int x, y; };
struct P p = { 1 };
int f(int a, int u, unsigned v) {
    return u < v;
}
