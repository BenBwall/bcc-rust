/* Clang's words for casts between pointers and smaller integers. */
enum E { A };
void g(void);
void f(int *p, void *v, char *s, int i, short sh, char c) {
  (void)(unsigned)p; (void)(int)v; (void)(enum E)p; (void)(enum E)v;
  (void)(int)g; (void)(short)s;
  (void)(int *)i; (void)(void *)i; (void)(char *)sh; (void)(void (*)(void))c;
  (void)(long)p; (void)(int *)(long)i;
}
