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
