/* Clang's -Wconditional-uninitialized, which -Wall leaves off: a use the
   analysis cannot tie to a branch, and every use of a variable initialized
   with itself, unless one is always uninitialized. */
int g(int);
int loop(int n) { int x; while (n--) x = g(n); return x; }
int deliberate(int a) { int x = x; if (a) x = 2; return x; }
int deliberate_always(void) { int x = x; return g(x); }
int tied(int a) { int x; if (a) x = 1; return x; }
