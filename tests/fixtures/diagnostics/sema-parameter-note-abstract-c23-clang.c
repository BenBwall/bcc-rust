/* Clang's note at an unnamed parameter in C23, where `()` is a function of
   no parameters and `(...)` one with only variable arguments: after the
   `(`, at the `)` or the `...`. */
void empty_function(int ());
void variadic_function(int (...));
void f(int *p) {
  empty_function(p);
  variadic_function(p);
}
