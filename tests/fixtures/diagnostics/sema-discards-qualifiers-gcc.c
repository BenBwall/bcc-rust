/* GCC words -Wdiscarded-qualifiers by where the conversion happens and names
   the qualifiers the pointed-to type loses; a deeper level that differs makes
   the pointers incompatible, an error in GCC 14. A literal zero character is
   a null pointer constant without a word. */
void take(char *);
void take2(char **);
void take3(void *);
const char *cs;
const char **css;
volatile int *vi;
const volatile int *cvi;
int *restrict *ri;
char *ret(void) { return cs; }
void f(void) {
  char *a = cs;
  a = cs;
  take(cs);
  char **b = css;
  b = css;
  take2(css);
  int *c = vi;
  const char *const *d = css;
  take3(cvi);
  int **e = ri;
  __builtin___clear_cache(cs, a);
  take3('\0');
  take3(1 - 1);
  (void)a; (void)b; (void)c; (void)d; (void)e;
}
