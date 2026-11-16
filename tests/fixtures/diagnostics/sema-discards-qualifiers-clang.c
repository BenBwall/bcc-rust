/* Clang words -Wincompatible-pointer-types-discards-qualifiers by where the
   conversion happens, and says "in nested pointer types" when a deeper level
   differs, even if the pointed-to type also loses a qualifier. */
void take(char *);
void take2(char **);
const char *cs;
const char **css;
volatile int *vi;
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
  (void)a; (void)b; (void)c; (void)d;
}
void take3(int **);
const int *const *cc;
int *const *ic;
int (*arr)[2];
const int (*carr)[2];
int **mixed(void) { return cc; }
void g(void) {
  int **e = cc;
  e = cc;
  take3(cc);
  const int **h = ic;
  arr = carr;
  (void)e; (void)h;
}
