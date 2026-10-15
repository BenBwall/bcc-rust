/* Constraint violations that Clang and GCC accept with a warning (-Wpointer-sign,
   -Wcompare-distinct-pointer-types); bcc reports an error. Csmith emits them in ~83% of its default programs. */
int main(void) {
    int x = 0;
    int *s = &x;
    unsigned *u = (unsigned *)&x;
    int cmp = (s == u);   /* "invalid operand types for comparison" */
    u = s;                /* "incompatible assignment conversion" */
    return cmp + (u != 0) - 2;
}
