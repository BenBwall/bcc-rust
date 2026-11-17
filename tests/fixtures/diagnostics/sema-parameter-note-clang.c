/* Clang's note at the parameter an argument converts to, after each
   diagnostic of the conversion of an argument to a function declared with
   a prototype. */
void named(int *p);
void unnamed(int * , const char *);
void later(char *);
void later(char *s);
void defined(char *d) { (void)d; }
void (*pointer)(int *);
typedef void Typed(int *);
Typed from_typedef;
void array(int a[]);
void take(float f);
void variadic(int n, ...);
void f(const int *c, long l, double *x) {
  named(c);
  unnamed(c, c);
  later(c);
  defined(c);
  pointer(c);
  from_typedef(c);
  named(l);
  array(c);
  take(1);
  variadic(1, c);
}
void g(const int *c) {
  void named(int *q);
  named(c);
}
