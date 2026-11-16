/* Clang leaves a same-width constant conversion unreported when the literal
   is written in hexadecimal, octal or binary, and reports it at once, in dead
   code too. A conversion that loses bits waits for reachable code. */
#define BITS 0xdf
void same_width(char *p, signed char *q, short *s) {
  *p = 0xdf;
  *p = 0337;
  *p = 0b11011111;
  *p = (0xdf);
  *p = BITS;
  *q = 0xdf;
  *s = 0xdfff;
  *p = 0x80;
  *p = 223;
  *p = 0xdf + 0;
  *s = 57343;
  if (0)
    *p = 128;
  return;
  *p = 200;
}
void narrowing(char *p, int *i) {
  *p = 0x1df;
  *p = -0xdf;
  *i = 0xdfffffffff;
  if (0)
    *p = 479;
}
char decimal = 223;
char hexadecimal = 0xdf;
/* A literal pasted from pieces is read whole: the decimal 200 and 40000 warn,
   the hexadecimal 0xdf does not. */
#define CAT(a, b) a##b
char pasted_decimal = CAT(2, 00);
short pasted_short = CAT(40, 000);
char pasted_hexadecimal = CAT(0x, df);
/* Clang checks only the operand a selection takes, but the statements of a
   statement expression in any operand, which are full expressions of their
   own. A character type's element of a list in its own braces is an array's
   initialization to Clang, unless brace elision reached it. */
void selections(char *p, short *s) {
  (void)_Generic(0, int: 0, default: (char){223});
  (void)__builtin_choose_expr(1, 0, (char){223});
  (void)_Generic(0, int: 0, default: (short){57343});
  (void)_Generic(0, int: (short){57343}, default: 0);
  (void)__builtin_choose_expr(1, 0, *p = 223);
  (void)__builtin_choose_expr(0, 0, *p = 223);
  (void)_Generic(0, int: __builtin_choose_expr(0, *s = 57343, 0), default: *p = 223);
  (void)_Generic(0, int: 0, default: ({ *p = 200; 0; }));
  (void)__builtin_choose_expr(1, 0, ({ *s = 57343; 0; }));
  if (0)
    (void)_Generic(0, int: (short){57343}, default: (char){223});
  char a[2] = {223, 0};
  char b[2][1] = {223, 0};
}
