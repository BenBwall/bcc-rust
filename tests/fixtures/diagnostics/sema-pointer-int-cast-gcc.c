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
int following_valid;
