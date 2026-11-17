/* Clang's -Wdeprecated-non-prototype and its argument counts for calls
   to a definition without a prototype. */
int defined(a, b) int a; char *b; { return a + !b; }
int declared();
int later(a) int a; { return declared(a) + later(a); }
int declared(a) int a; { return a; }
int with_prototype(int);
int with_prototype();
int prototype_then(int);
int prototype_then(x) int x; { return x; }
int empty_then();
int empty_then(int x) { return x; }
int twice(int); int twice(); int twice(c) int c; { return c; }
typedef int Untyped();
Untyped from_typedef;
int from_typedef(t) int t; { return t; }
static int internal();
static int internal(s) int s; { return s; }
int no_parameters() { return 0; }
int (*pointer)();
struct holder { int (*member)(); } holder;
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-non-prototype"
int quiet();
#pragma clang diagnostic pop
long labs();
int uses(void) {
  int (*local)() = 0;
  return defined(1, "x") + defined(1) + defined(1, "x", 2)
         + no_parameters(1) + no_parameters()
         + pointer(1) + (*pointer)(2) + local(3) + holder.member(4)
         + quiet(5) + (int)labs(6L) + declared(declared(7));
}
void block(void) {
  extern int inner();
  inner(8);
}
int inner(i) int i; { return i; }
int keeps(void) { return prototype_then(9) + twice(10) + inner(11); }
typedef int Handler(int);
typedef int Untyped_handler();
Handler *get_handler(char *);
Handler *get_handler(n) char *n; { return 0; }
Untyped_handler *get_untyped(char *);
Untyped_handler *get_untyped(n) char *n; { return 0; }
