/* Clang's fix-its for a conversion it refuses: `&` or `*` before the
   value, with parentheses around one that needs them, the value's own `&`
   or `*` removed, and none for a null pointer, a value that is no lvalue,
   or a bit-field or vector element, whose address cannot be taken; a
   `register` object still gets its `&`. */
typedef int V __attribute__((vector_size(16)));
struct S { int a; int b : 3; };
int g(void);
int *h(void);
void f(int y, int *p, struct S s, struct S *ps, int arr[3], V v) {
  int x;
  register int r = 0;
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
  p = s.b;
  p = ps->b;
  p = (s.b);
  p = v[1];
  p = r;
}
