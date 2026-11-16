#define Min(x, y) ((x) < (y) ? (x) : (y))
#define ID(x) x
#define LIM 10u
#define BIG(a) ((a) + 1u)
#define SUM(a, b) a + b
#define MASK 0x10
int precedence(int a, int c) {
    int r = SUM(a, c) << c;
    r += a & MASK == c;
    return r;
}
int compared(int a, unsigned b, int c) {
    int r = 0;
    r += Min(a, b);
    r += a < ID(b);
    r += c < LIM + 1;
    r += BIG(c) > a;
    r += a < BIG(c);
    r += BIG(
        c) > a;
    r += (a < b) == ID(c);
    return r;
}
