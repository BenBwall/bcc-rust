/* Clang's fix-its for a conversion it refuses: `&` or `*` before the
   value, with parentheses around one that needs them, the value's own `&`
   or `*` removed, and none for a null pointer or a value that is no
   lvalue. */
struct S { int a; };
int g(void);
int *h(void);
void f(int y, int *p, struct S s, struct S *ps, int arr[3]) {
  int x;
  p = y;
  p = s.a;
  p = ps->a;
  p = arr[1];
  p = (y);
  p = *p;
  x = p;
  x = &y;
  x = h();
  x = p + 1;
  x = (int *)0;
  p = y ? y : y;
  x = (p);
  p = x = 1;
  x = *&p;
}
