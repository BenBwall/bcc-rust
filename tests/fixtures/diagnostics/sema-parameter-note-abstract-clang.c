/* Clang's note at an unnamed parameter is where its parser places the
   name the abstract declarator leaves out: past the pointers, their
   qualifiers and attributes, at an array's `[`, after a function's `(`,
   inside parentheses where the declarator there places it, and otherwise
   after the declarator. */
void plain(int);
void pointer(int *);
void qualified(int *const, int);
void attributed(int *__attribute__((unused)));
void array(int [3]);
void static_array(int [static 3]);
void qualified_array(int [const 3]);
void pointers(int *[3]);
void arrays(int [2][3]);
void function(int (double));
void empty_function(int ());
void function_pointer(int (*)(double));
void constant_function_pointer(int (*const)(void));
void array_of_function_pointers(int (*[2])(void));
void nested(int (*(*)(int))(char));
void array_pointer(int (*)[3]);
void grouped_attributes(int (__attribute__((unused)) *)[3]);
void returns_pointer(int *(*)(void));
void second(char, int [4], char);
void f(const int *c, int *p, int (*a)[4], void *v) {
  plain(v);
  pointer(c);
  qualified(c, 0);
  attributed(c);
  array(c);
  static_array(c);
  qualified_array(c);
  pointers(c);
  arrays(c);
  function(p);
  empty_function(p);
  function_pointer(p);
  constant_function_pointer(p);
  array_of_function_pointers(p);
  nested(p);
  array_pointer(a);
  grouped_attributes(a);
  returns_pointer(p);
  second(0, c, 0);
}
