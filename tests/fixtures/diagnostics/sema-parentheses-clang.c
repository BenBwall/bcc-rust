int f(int a, int b, int c, _Bool t) {
    int r = a && b || c;
    r += a || b && "why";
    r += a + b << c;
    r += a - b >> c;
    r += a & b == c;
    r += a & b | c;
    r += a ^ b | c;
    r += a | b ^ c == 1;
    r += !a == b;
    r += !t == b;
    r += !a == 0;
    r += !a & b;
    r += !a | b;
    r += a + (b < c) ? 1 : 2;
    r += a & (b < c) ? 1 : 2;
    r += (a < b) < c;
    if (a = b)
        r++;
    if (a |= b)
        r++;
    if ((a == b))
        r++;
    while ((a == b))
        r++;
    if ((a = b))
        r++;
    if (a)
        if (b)
            r++;
        else
            r--;
    if (a)
        while (c)
            if (b)
                r++;
            else
                r--;
    switch (a == b) {
    default:
        break;
    }
    return r;
}
