// `-Werror=extra` enables `-Wextra` and makes errors of the options it
// holds, and a later `-Wno-error=` of one takes that back.
int f(int u, unsigned v) {
    if (u < v)
        return 1;
    if (u)
        ;
    return 0;
}
int g(unsigned char c) { return c < 0; }
