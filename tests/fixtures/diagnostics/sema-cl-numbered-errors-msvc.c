/* Errors cl numbers, in its words. */
struct S { int a; };
int g(int a);
int f(void) { return 0; }
int f(void) { return 1; }
int k(struct S s, int i, int x)
{
    int r = s.b + *i;
    r += g(1, 2) + g() + x(1);
    1 += i;
    return r;
}
