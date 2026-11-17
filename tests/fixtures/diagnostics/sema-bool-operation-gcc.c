/* GCC's -Wbool-operation: ++ and -- of a _Bool, and ~ of a _Bool or of a
   truth value its build_unary_op sees past commas and casts to the
   operand's own type. */
#define NOT(x) ~(x)
void f(_Bool b, int x, int y, _Bool *pb, int *r) {
  b++; ++b; b--; --b;
  (*pb)++;
  pb[0]--;
  b = ~b;
  *r = ~(x > 0);
  *r = ~!x;
  *r = ~(x && y);
  *r = ~(x, x > 1);
  *r = ~(_Bool) x;
  *r = ~(int) (x > 1);
  *r = ~(b = x);
  *r = (~b);
  *r = ~((x == 2) | (y == 1));
  *r = ~(x ? x > 1 : y < 2);
  *r = ~+(x > 1);
  *r = ~(1 > 0);
  *r = -b;
  b += 1;
  *r = sizeof ~b;
  ~b;
}
