/* `__extension__` silences GCC's -pedantic reports about its operand, as
   glibc's assert() relies on: `__extension__ __PRETTY_FUNCTION__`. */
const char *pretty(void) { return __PRETTY_FUNCTION__; }
const char *quiet_pretty(void) { return __extension__ __PRETTY_FUNCTION__; }
const char *name(void) { return __FUNCTION__; }
const char *quiet_name(void) { return __extension__ __FUNCTION__; }
const char *nested(int c) { return __extension__ (c ? __FUNCTION__ : __PRETTY_FUNCTION__); }
const char *outside(int c) { return c ? __extension__ __FUNCTION__ : __FUNCTION__; }
/* A declaration or function definition marked `__extension__` silences them
   in its initializers and body. */
__extension__ const char *quiet_definition(void) { return __FUNCTION__; }
const char *quiet_declaration(void) { __extension__ const char *p = __FUNCTION__; return p; }
const char *loud_after(void) { __extension__ const char *p = __FUNCTION__; (void)p; return __FUNCTION__; }
/* A marked declaration silences folded enumerators, case labels and bit-field
   widths, but not a folded array bound, which GCC makes variable length and
   then judges by the array's storage and uses. */
__extension__ enum { quiet_enumerator = (int)(1.0 + 1) };
__extension__ struct quiet_width { int b : (int)(1.5 * 2); };
__extension__ int quiet_case(int i) { switch (i) { case (int)(1.0 + 1): return 1; } return 0; }
__extension__ static int loud_bound[(int)(1.0 + 1)];
__extension__ struct loud_member { int a[(int)(1.0 + 1)]; };
void loud_static(void) { __extension__ static int a[(int)(1.0 + 1)]; (void)a; }
void loud_automatic(void) { __extension__ int a[(int)(1.0 + 1)]; (void)a; }
/* A marked declaration also silences pointers to arrays whose elements differ
   in qualifiers and a static initializer folded from an undefined shift; an
   operand around that initializer does not, as GCC reports it after. */
__extension__ void quiet_arrays(int (*p)[2], const int (*q)[2], int c) { (void)(c ? p : q); }
void loud_arrays(int (*p)[2], const int (*q)[2], int c) { (void)(c ? p : q); }
__extension__ static int quiet_shift = 1 << 40;
static int loud_shift = 1 << 40;
static int loud_operand_shift = __extension__ (1 << 40);
/* A cast to a union type initializes a static object; a marked declaration
   silences the cast. */
union cast_union { int i; float f; };
__extension__ union cast_union quiet_cast = (union cast_union)1;
union cast_union loud_cast = (union cast_union)2.5f;
