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
