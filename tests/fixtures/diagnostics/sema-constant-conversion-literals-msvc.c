/* The MSVC identity keeps Clang's earlier model of constant conversions: a
   hexadecimal literal warns as a decimal one does, and nothing is reported
   in code that does not run or in an operand a selection drops. */
#define CAT(a, b) a##b
char hexadecimal = 0xdf;
char pasted_decimal = CAT(2, 00);
char braced[2] = {223, 0};
void f(char *p) {
  *p = 0xdf;
  *p = 223;
  if (0)
    *p = 223;
  (void)_Generic(0, int: 0, default: (char){223});
  (void)__builtin_choose_expr(1, 0, *p = 223);
  return;
  *p = 200;
}
