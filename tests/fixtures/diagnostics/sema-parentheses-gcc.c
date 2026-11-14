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

/* An explicit conversion keeps a truth value where GCC's folding keeps one. */
int truth_conversions(int a, int b, _Bool t) {
    int r = !a & (int)(b != 0);
    r += !a & (unsigned)(b != 0);
    r += !a & __extension__ (b != 0);
    r += !a & (int)(b || a);
    r += !a & (char)(b < 0);
    r += !a & (int)(long)(b || a);
    r += !a & (int)(double)(b < a);
    r += !a & (int)(long)(char *)(long)(b != 0);
    r += !a & (int)(_Bool)b;
    r += !a & (long)(0, b != 0);
    r += !a & (long)!b;
    r += !a & !(b, a);
    r += !a & (unsigned)(b || a);
    r += !a & (int)(char)(b || a);
    r += !a & (long)(double)(b < a);
    r += !a & (unsigned long)(char *)(long)(b != 0);
    r += !a & (int)(_Bool)(b && a);
    r += !a & (int)t;
    r += !a & (0, b != 0);
    r += !a & (b ? a != 0 : b < 1);
    r += !a & !(b ? a : t);
    r += !a & (long)!(b || a);
    return r;
}

/* Folding a conditional, a comma or a constant through conversions decides
   whether GCC still sees a truth value. */
int folded_conversions(int a, int b, _Bool t, double d, double e) {
    int r = !a & (int)(_Bool)(b ? 0 : 1);
    r += !a & (int)(_Bool)(double)(b || a);
    r += !a & (int)(double)(b ? 0 : 1);
    r += !a & (unsigned)(_Bool)(b ? 0 : 2);
    r += !a & !(b ? 1 : 0);
    r += !a & !(_Bool)(b ? 1 : 0);
    r += !a & !(int)(double)(b ? 0 : 0);
    r += !a & (int)(_Bool)(b ? 2 : 0);
    r += !a & (unsigned)(b ? !a : 0);
    r += !a & (char)(b ? 0 : !a);
    r += !a & (int)(double)(b ? b != 0 : 1);
    r += !a & !(_Bool)(b ? t : 0);
    r += !a & (long)(char *)0;
    r += !a & (int)(unsigned)((b ? 1 : 0) ? 1 : 0);
    r += !a & (int)(_Bool)((b ? 0 : 1) ? 1 : 0);
    r += !a & (int)(unsigned)(1 || b);
    r += !a & (int)(_Bool)(char *)(long)(a++, b != 0);
    r += !a & (int)(_Bool)__extension__ (b ? 0 : 1);
    r += !a & (int)(double)(b ? 1.0 : 0.0);
    r += !a & (unsigned)(b ? (d < e) : 0);
    r += !a & !(_Bool)(b ? !a : 0);
    return r;
}
