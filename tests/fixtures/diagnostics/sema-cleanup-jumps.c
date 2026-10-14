/* GNU cleanup functions and the jumps Clang rejects around them. */
void release(int *p);
void two(int *p, int extra);
void unsigned_release(unsigned *p);
int object;
void checks(void) {
    int a __attribute__((cleanup(two)));
    int b __attribute__((cleanup(unsigned_release)));
    int c __attribute__((cleanup(object)));
    static int d __attribute__((cleanup(release)));
}
void jumps(int n) {
    goto inside;
    {
        int a __attribute__((cleanup(release))) = 1;
inside:
        n++;
    }
    switch (n) {
        int b __attribute__((cleanup(release)));
    case 1:
        break;
    }
    void *target = &&done;
    {
        int c __attribute__((cleanup(release))) = 2;
        goto *target;
    }
done:;
}
int after;
