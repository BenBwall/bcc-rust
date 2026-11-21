/* `cl`'s `#pragma warning(suppress: N)`, which withholds the warning on the
   pragma's line and the next, and `once`. */
void a(void)
{
#pragma warning(suppress: 4101)
    int x;
    int y;
}
#pragma warning(suppress: 4100)
void b(int p,
       int q)
{
}
void c(void)
{
    int d; __pragma(warning(suppress: 4101)) int e;
    int f;
    int g;
}
void h(void)
{
#pragma warning(suppress: 4101)
#pragma warning(suppress: 4189)
    int i; int j = 1;
    int k = 2;
}
void l(void)
{
#pragma warning(suppress: 4101)

    int blank;
}
#pragma warning(once: 4101)
void m(void) { int n; }
void o(void) { int p; }
