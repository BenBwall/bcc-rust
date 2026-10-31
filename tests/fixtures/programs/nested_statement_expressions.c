/* GNU nested functions defined inside statement expressions run only when
   called: the statements around the definition and the tail run once, as
   statements, as values, in conditions, loops, initializers and arguments,
   with labels, cleanups, jumps out of the block, VLAs and static locals
   beside the definition, and not at all in `sizeof` and `typeof`. Every
   line traces the digits `f` was called with. Takes no nested function's
   address, so it runs on wasm32 too. Prints what GCC prints. */
#include <stdio.h>
static char trace[64];
static int count;
static int f(int k) { trace[count++] = '0' + k % 10; return k; }
static void show(const char *what) { trace[count] = 0; printf("%s:%s\n", what, trace); count = 0; }
static void done(int *p) { f(*p); }
static int early(int c) {
  return ({ int g(int x) { return f(x); } if (c) return g(7); g(8); });
}
struct pair { int a, b; };
static int add(int a, int b) { return a + b; }
int main(void) {
  int base = 5, r = 0;
  ({ int g(int x) { return f(x) + base; } g(2); }); show("statement");
  r += ({ int g(int x) { return f(x); } g(3); }); show("value");
  r += ({ int g(int x) { return f(x); } 0; }); show("uncalled");
  r += ({ f(1); int g(int x) { return f(x); } f(3); int h(int x) { return g(x) + 1; } f(5); h(4); });
  show("around");
  ({ f(6); int g(void) { return f(7); } }); show("definition last");
  r += r ? ({ int g(int x) { return f(x) + base; } g(1); }) : 0; show("conditional");
  r += ({ int g(int x) { return f(x); } g(0); }) || ({ int h(int x) { return f(x); } h(2); });
  show("logical");
  for (int i = ({ int g(int x) { return f(x); } g(1); }); i < ({ int g(int x) { return f(x); } g(3); });
       i += ({ int g(int x) { return f(x); } g(1); }))
    r += ({ int g(int x) { return f(x) + i; } g(i); });
  show("for");
  for (int i = 0; i < 4; i++)
    r += ({ int g(int x) { return f(x); } if (i == 1) continue; if (i == 3) break; g(i); });
  show("break continue");
  r += early(1) + early(0); show("return");
  r += sizeof(({ int g(int x) { return f(x); } g(9); })); show("sizeof");
  typeof(({ int g(int x) { return f(x); } g(9); })) t = 1; show("typeof");
  struct pair p = { ({ int g(int x) { return f(x); } g(1); }), ({ int g(int x) { return f(x) + base; } g(2); }) };
  show("initializer");
  r += add(({ int g(int x) { return f(x); } g(3); }), 4); show("argument");
  r += ({ int g(int x) { return ({ int h(int y) { return f(y) + base; } h(x); }); } g(5); }); show("nested");
  r += ({ int g(int x) { return f(x); } goto skip; skip: g(6); }); show("label");
  r += ({ __attribute__((cleanup(done))) int k = 1; int g(int x) { return f(x); } g(2); }); show("cleanup");
  r += ({ int n = f(3); int g(int x) { return f(x); } int vla[g(n)]; (int)sizeof vla; }); show("vla");
  for (int i = 0; i < 2; i++) r += ({ int g(int x) { return f(x); } static int s; s += g(4); s; });
  show("static");
  r += ({ int g(int x) { return f(x); } int *q = (int[]){ g(5), g(6) }; q[1]; }); show("compound literal");
  printf("%d %d %d %d\n", r, t, p.a, p.b);
  return 0;
}
