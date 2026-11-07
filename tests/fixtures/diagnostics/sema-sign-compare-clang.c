int g(int a, unsigned b, unsigned long c, int d) {
    int r = a < b;
    r += a == b;
    r += a < c;
    r += (a & 0xff) < b;
    r += 1 < b;
    r += (unsigned)a < b;
    return d ? a : b;
}
