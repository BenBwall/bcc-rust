/* cl's C4242, which /Wall shows in place of C4244 for a narrowing of a
   value cl has not computed at int, or of a 64-bit one. */
int narrowing(int i, long long ll, char *p, double d)
{
    char c = i;
    short s;
    c = (char)(i + 1);
    s = i;
    s = c + s;
    i = ll;
    i = (int)ll + 1;
    *p = i;
    i = d;
#pragma warning(disable: 4244)
    c = i;
    c = c + i;
#pragma warning(default: 4244)
    c += i;
    c += s;
    return c + s;
}
