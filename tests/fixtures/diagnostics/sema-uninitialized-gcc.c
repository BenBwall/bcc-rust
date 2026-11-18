/* GCC's -Wuninitialized and -Wmaybe-uninitialized come from its optimizer
   and depend on the optimization level; bcc does not model them, so GCC's
   rules report none of Clang's analysis, only the unused variable. */
int g(int);
int always(void) { int x; return x; }
int sometimes(int a) { int x; if (a) x = 1; return x; }
int itself(void) { int x = x + 1; return x; }
int unused(void) { int y; return 0; }
