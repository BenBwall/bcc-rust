/* GCC 14 under -pedantic: conversions between function and object pointers. */
typedef void (*fp)(void);
void fn(void);
int object;
void *address = (void *)fn;
fp function = (fp)&object;
fp null = (fp)(void *)0;
void *returned(void) { return fn; }
void conversions(void *vp, int *ip, fp f, int c) {
    fp a = (fp)vp;
    int *b = (int *)f;
    fp d = (fp)ip;
    fp e = (fp)(int *)0;
    void *g = f;
    vp = f;
    int h = vp == f;
    int i = f == (void *)0;
    void *j = c ? vp : f;
    (void)a; (void)b; (void)d; (void)e; (void)g; (void)h; (void)i; (void)j;
}
void marked(void *vp, fp f) {
    fp a = __extension__ (fp)vp;
    __extension__ void *b = (void *)f;
    int c = __extension__ (vp == f);
    (void)a; (void)b; (void)c;
}
