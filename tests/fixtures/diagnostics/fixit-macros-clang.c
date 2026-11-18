/* Clang's fix-it for a token a macro expansion produced: one an argument
   wrote stays where the invocation spells it, and one a macro's body wrote
   is dropped with every other hint of the diagnostic. A tab before the
   hint is expanded as in the source line. */
#define ID(x) x
#define P(x) (x)
#define NB ~b
#define TWO(x, y) x + y
int f(_Bool b) {
  int r = ID(~b);
  r += P(~b);
  r += NB;
  r += TWO(~b, 1);
  r += TWO(1, ~b);
	r += ~b;
  return r;
}
