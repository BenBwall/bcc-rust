/* TCC keeps the first declaration's linkage and warns of a later one that
   says otherwise (patch_type in tccgen.c). */
int f(void);
static int f(void) { return 3; }
int k(void) { return 1; }
static int k(void);
static int g(void);
int g(void) { return 1; }
int x;
static int x;
static int z;
int z;
extern int y;
static int y = 2;
static int v;
extern int v;
static int s;
int use(void) { extern int s; return s + v + x + y + z + f() + g() + k(); }
int i(void);
static inline int i(void) { return 0; }
