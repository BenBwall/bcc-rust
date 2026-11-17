/* Clang takes a discarded statement expression's value from its last
   statement, after its labels; its value statement itself warns nothing. */
int h(void);
void f(int a) {
  ({ a + 1; });
  ({ h(); a; });
  ({ goto l; l: a == 1; });
  ({ h(); });
  ({ (void)a; });
  ({ int v = a; v; });
}
/* Clang drops the warning when a macro's body or argument placed it, as
   for a function-like macro used as a statement, but not when parentheses
   or __extension__ surround the statement expression, and only as far as
   its value goes. */
#include <statement-expression-macros.h>
#define VALUE(x) ((x) + 1)
#define SAME(x) x
#define PLUS +
#define EQUAL(x) ((x) == 1)
void g(int a) {
  ({ VALUE(a); });
  ({ SAME(a + 1); });
  ({ SAME((a + 1)); });
  ({ SAME(a); });
  ({ a PLUS 1; });
  ({ EQUAL(a); });
  ({ h(); l: VALUE(a); });
  ({ ({ VALUE(a); }); });
  ({ SYSTEM_VALUE(a); });
  ({ SYSTEM_SAME(a + 1); });
  ({ VALUE(a); }), h();
  ({ SAME(a) + 1; });
  ({ VALUE(a) + 1; });
  (({ VALUE(a); }));
  __extension__ ({ VALUE(a); });
  SAME(a + 1);
  SYSTEM_SAME(a + 1);
  VALUE(a);
}
