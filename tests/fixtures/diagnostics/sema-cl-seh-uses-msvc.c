/* Names that only `__try`, `__except` and `__finally` use are used, and
   what those blocks hold draws `cl`'s other warnings. */
int work(int);
int f(int p, int q)
{
    int rc;
    int local = 3;
    __try {
        rc = work(p + local);
    }
    __except (q) {
        rc = 0;
    }
    return rc;
}
int g(int p)
{
    int r = 0;
    __try {
        r = work(p);
    }
    __finally {
        r++;
    }
    return r;
}
int h(int p, unsigned u)
{
    int x;
    __try {
        if (p < u)
            return 1;
        if (p = 2)
            return 2;
    }
    __except (1) {
        return p << 40;
    }
    return 0;
}
