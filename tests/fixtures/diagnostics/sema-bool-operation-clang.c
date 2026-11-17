/* Clang's -Wbool-operation: ~ of an operand known to be 0 or 1, with the
   range of the operand its walk meets, always true when that converts to
   _Bool. Clang has no ++ or -- form in C. */
#define NOT(x) ~(x)
_Bool g(_Bool v) { return ~v; }
int k(_Bool v);
void f(_Bool b, int x, int y, int *r) {
  b = ~b;
  *r = ~(x > 0);
  *r = ~!x;
  *r = ~(x && y);
  *r = ~((x == 2) | (y == 1));
  *r = ~(x ? x > 1 : y < 2);
  *r = ~(_Bool) x;
  *r = ~(int) (x > 1);
  *r = ~+(x > 1);
  *r = ~(b = x);
  *r = (~b);
  b = (~b);
  _Bool c = ~!x;
  int d = ~(x != 1);
  *r = NOT(x > 1);
  *r = k(~b) + d + c;
  *r = sizeof ~b;
  b++;
  ~b;
}
