struct S { int x; };
struct B { unsigned x:3; };
int f(void) { return 7; }
int main(void) {
    int a[3] = { 2, 3, 4 };
    const volatile int q = 5;
    struct S s = { 6 };
    struct B b = { 7 };
    int *p = ({ a; });
    int (*fp)(void) = ({ f; });
    int v = ({ q; });
    struct S copy = ({ s; });
    unsigned bits = ({ b.x; });
    ({ });
    ({ (void)0; });
    ({ int unused; });
    s.x = 9;
    return sizeof(({ a; })) != sizeof(int *)
        || sizeof(({ f; })) != sizeof(fp)
        || sizeof(({ b.x; })) != sizeof(unsigned)
        || p != a || p[2] != 4 || fp() != 7 || v != 5
        || copy.x != 6 || bits != 7;
}
