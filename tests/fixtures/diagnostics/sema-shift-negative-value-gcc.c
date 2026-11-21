/* GCC 14 under -Wextra from C99: a negative constant shifted left. */
enum { MINUS = -2 };
int shifts(int x) {
    int a = -1 << 1;
    int b = -2 << x;
    int c = MINUS << 3;
    int d = (char)-1 << 1;
    long e = -1L << 40;
    int f = __extension__ (-1 << 1);
    int g = -1 << 40;
    return a + b + c + d + (int)e + f + g;
}
int silent(int x) {
    int a = (0, -1) << 1;
    int b = 0 ? -1 << 1 : 1;
    int c = (int)sizeof(-1 << 1);
    unsigned d = -1u << 1;
    int e = -1 >> 1;
    int f = x << 1;
    return a + b + c + (int)d + e + f;
}
