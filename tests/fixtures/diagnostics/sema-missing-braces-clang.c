/* Clang's -Wmissing-braces: a warning at the first element of each array,
   structure or union whose braces brace elision left out, inner ones
   first, with the elements underlined and the braces suggested. `{0}`
   alone, the only member of a structure, a string, a scalar and a
   subobject a designator names warn of none. */
#include <brace-macros.h>
struct P { int x, y; };
struct S { struct P p; int z; };
struct S s1 = {1, 2, 3};
struct S s0 = {0};
struct S s00 = {0, 0};
struct S szero = {(0), 1};
int a[2][2] = {1, 2, 3, 4};
int b[2][2] = {0};
int mixed[2][2] = {{1, 2}, 3, 4};
struct S s2 = {.p = 1, 2};
struct S s3 = {.p.x = 1, 2, 3};
struct S s4 = {1, 2, .z = 3};
struct S s5 = {1, .p.y = 2};
union U { struct P p; int i; };
union U u = {1, 2};
struct Q { char s[4]; int k; };
struct Q q = {"abc", 1};
struct Q q2 = {'a', 'b', 'c', 0, 1};
struct Q q3 = {{"ab"}, 2};
int scalar = {1};
struct S braced = { {1}, 2 };
struct W { int w[2]; };
struct W w = {1, 2};
struct R { struct P p[2]; };
struct R r = {1, 2, 3, 4};
struct R r2 = {{1, 2, 3, 4}};
struct S s6[2] = {1, 2, 3, 4, 5, 6};
struct S partial = {1};
struct S long_list = {
  1,
  2, 3 };
#define INIT {1, 2, 3}
struct S from_macro = INIT;
#define ONE 1, 2
struct S from_list = {ONE, 3};
#define FIRST 1
struct S from_first = {FIRST, 2, 3};
#define LAST 2, 3
struct S from_last = {1, LAST};
struct S from_system = SYS_INIT;
struct S from_system_list = {SYS_PAIR, 3};
void f(void) {
  struct S local = {1, 2, 3};
  int cells[2][3] = {1, 2, 3, 4, 5, 6};
  (void)local;
  (void)cells;
}
