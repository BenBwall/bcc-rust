enum E { A };
int x;
void g(int i, char c, _Bool b, enum E e, __int128 q, void *p, char *cp) {
    (void)(void *)i;
    (void)(char *)c;
    (void)(void *)(int){0};
    (void)(char *)(1 ? 2 : i);
    (void)(char *)q;
    (void)(char *)b;
    (void)(char *)e;
    (void)(char *)(char)300;
    (void)(char *)(int)(long)&x;
    (void)(int)p;
    (void)(char)cp;
    (void)(__int128)p;
    (void)(enum E)p;
    (void)(_Bool)p;
    (void)(long)p;
}
struct bits { _BitInt(16) narrow; _BitInt(64) wide; } bits;
void bit_ints(void) {
    (void)(char *)bits.narrow;
    (void)(char *)bits.wide;
    (void)(char *)(_BitInt(32))3;
}
int following_valid;
