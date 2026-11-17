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
/* Clang's walk steps through parentheses, __extension__ and a selection to
   the operand selected, which gives the range; it never meets a ~ in a
   _Generic's other operands, its controlling one included, or in the
   operand __builtin_choose_expr does not select. A ~ in a statement
   expression's statement is a full expression of its own, and a call's
   argument converts when the call is done. */
void two(_Bool v, int w);
int sel(_Bool b, int x) {
  int r = _Generic(b, int: ~b, default: 0);
  r += __builtin_choose_expr(1, 0, ~b);
  r += _Generic(b, _Bool: ~b, default: 0);
  r += __builtin_choose_expr(0, 0, ~b);
  r += _Generic(~b, default: 0) + _Generic((~b) + 1, default: 0);
  r += _Generic(b, int: (~b) + 1, default: ~(x > 0));
  r += _Generic(b, _Bool: _Generic(b, int: ~b, default: 1), default: ~b);
  r += _Generic(b, int: ({ ~b; }), default: 0);
  r += _Generic(b, int: ~b, default: ({ 0; })) + ~b;
  r += 0 ? ~b : 1;
  typeof(~b) t = __builtin_constant_p(~b);
  two(~b, 0);
  return r + t;
}
_Bool sel_bool(_Bool b) { return __extension__ __builtin_choose_expr(1, (~b), 0); }
_Bool sel_generic(_Bool b) { return (_Generic(b, _Bool: ~b, default: 0)); }
int sel_file = _Generic((_Bool)0, int: ~(_Bool)0, default: 0)
    + __builtin_choose_expr(1, 0, ~(_Bool)0);
