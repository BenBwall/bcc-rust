/* `cl` warns of a constant shift count only for `<<` and `>>`, not for
   `<<=` and `>>=`. */
int s(int x, unsigned u)
{
    int r = 0;
    x <<= 32;
    x >>= -2;
    u <<= 40;
    r += x << 32;
    r += x >> -1;
    return r;
}
