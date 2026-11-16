// Compile only: which conversions GCC and Clang warn about, by default and
// with -Wall, -Wextra or -Wconversion (claims-audit.md, the Cast note).
int f(void) {
    char a = 300;
    char b = (char)300;
    char small = 100;
    char narrow = small + small;
    int n = -1; unsigned u = 1;
    int below = n < u;
    return a + b + narrow + below;
}
