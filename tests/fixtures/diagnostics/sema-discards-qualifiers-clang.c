/* Clang words -Wincompatible-pointer-types-discards-qualifiers by where the
   conversion happens, and says "in nested pointer types" when only a deeper
   level loses a qualifier. */
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
