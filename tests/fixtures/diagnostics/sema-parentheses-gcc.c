int f(int a, int b, int c, _Bool t) {
    int r = a && b || c;
    r += a + b << c;
    r += a - b >> c;
    r += a & b == c;
    r += a + b & c;
    r += a & b | c;
    r += a ^ b | c == 1;
    r += a + b ^ c;
    r += a < b < c;
    r += a < b == c;
    r += !a == b;
    r += !t == b;
    r += !a == 0;
    r += !a & b;
    r += !a | b;
    if (a = b)
        r++;
    while (a = b)
        r++;
    if ((a = b))
        r++;
    if (a)
        if (b)
            r++;
        else
            r--;
    if (a)
        l: if (b)
            r++;
        else
            r--;
    return r;
}

#define NEGATIVE(x) (((x) & 1) != 0)
enum { ONE = 1 };

/* GCC leaves `!x & y` and `!x | y` alone when y is already a truth value. */
int truth_values(int a, int b, _Bool t) {
    int r = !NEGATIVE(a) & NEGATIVE(b);
    r += !a & (b != 0);
    r += !(a != 0) & (b != 0);
    r += !a | !b;
    r += !a & (a && b);
    r += !a & t;
    r += !a & (_Bool)b;
    r += !a & 0;
    r += !a & 1;
    r += !a & ONE;
    r += !(a != 0) & b;
    r += !a & 2;
    r += !a & -1;
    r += !a & (b ? 1 : 0);
    r += !a | (t + 0);
    return r;
}
