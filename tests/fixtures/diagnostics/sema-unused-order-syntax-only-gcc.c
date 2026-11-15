/* Under -fsyntax-only GCC lowers no function and checks no symbol: only the
   front end's warnings remain. */
static int defined_later(void);
static int object;
static void never_defined(void);
static int first_function(void) { return 0; }
static int initialized = 1;
static int defined_later(void) { return 1; }
static int object = 2;
int reaches(int a) {
    if (a)
        return 1;
}
static const int constant = 3;
static void also_never_defined(void);
int block(unsigned u, int a) {
    int k;
    return a < u;
}
