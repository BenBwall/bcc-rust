/* cl's numbered warnings at its level 4, as `cl /W4` writes them. */
void unused(int p, int q)
{
    int a;
    {
        int b = q;
    }
    {
        static int s;
        int c;
        c = 1;
    }
}

int checks(int i, unsigned u, short s)
{
    int r = i < u;
    r += i == u;
    r += s < u;
    r += -1 < u;
    if (r = i)
        r++;
    if ((r = i))
        r++;
    return r + (i << 32) + (1 << 31);
}

#pragma warning(push, 3)
void quiet(int p)
{
    int d;
}
#pragma warning(pop)
