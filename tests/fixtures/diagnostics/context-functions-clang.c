/* Clang names no function: its diagnostics in two functions and at file
   scope have no line between them. */
int first(void) {
  int *p = 1;
  return *p;
}
int second(void) {
  int *q = 2;
  return *q;
}
int *file_scope = 3;
