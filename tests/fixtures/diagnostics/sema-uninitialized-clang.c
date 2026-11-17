/* Clang's -Wuninitialized and -Wsometimes-uninitialized under -Wall, from
   its uninitialized-values analysis: a use no store reaches, and the
   branches after which one is uninitialized, each noted where the use is,
   with the branch to remove and the initializer to add. */
int g(int);
void read_only(const int *);
void ignore(const int *p) {}
_Noreturn void stop(void);
struct pair { int a, b; };
enum color { red, green };

int always(void) { int x; return x; }
int if_false(int a) { int x; if (a) x = 1; return x; }
int if_true(int a) {
  int x;
  if (a > 2) goto out;
  x = 1;
out:
  return x;
}
int and_or(int a, int b) { int x, y; if (a && b) x = 1; if (a || b) y = 1; return x + y; }
int or_true(int a, int b) { int x; if (a || b) goto out; x = 1; out: return x; }
int conditional(int a) { int x; a ? (x = 1) : 0; return x; }
int loop_exit(int n) {
  int x;
  for (int i = 0; i < n; i++)
    if (g(i)) {
      x = i;
      break;
    }
  return x;
}
int do_exit(int n) {
  int x;
  do {
    if (g(n)) {
      x = 1;
      break;
    }
  } while (n--);
  return x;
}
int while_exit(int n) { int x; while (n) { if (g(n)) { x = 1; break; } n--; } return x; }
int entered(int n, int m) {
  int x;
  if (m) { x = 1; goto body; }
  while (n) {
  body:
    return x;
  }
  return 0;
}
int cases(int a) {
  int x;
  switch (a) {
  case 1: x = 1; break;
  case 2: break;
  default: x = 3; break;
  }
  return x;
}
int no_default(int a) { int x; switch (a) { case 1: x = 1; break; default: break; } return x; }
int called(void) { int x; for (;;) { g(x); x = 1; } }
void declared(int n) {
  for (;;) {
    int x;
  again:
    g(x);
    x = 1;
    if (n) goto again;
  }
}
int itself(void) { int x = x + 1; return x; }
int deliberate(int a) { int x = x; if (a) x = 2; return x; }
int deliberate_read(void) { int x = x; return x; }
int pointer(void) { int x; read_only(&x); ignore(&x); return 0; }
int record(void) { struct pair p; struct pair q = p; return q.a; }
int enumeration(void) { enum color c; return c; }
int order(int a) { int x, y; g(x); if (a) g(1); return y; }
int quiet(int a) {
  int x, y, z;
  if (a) x = 1; else stop();
  int *p = &y;
  (void)z;
  return x + *p;
}
