/* GCC's <stdckdint.h> calls the overflow builtins and checks them as it
   checks a direct call: any integer operand, and a result that points to a
   modifiable integer that is not bool or an enumeration. */
#include <stdckdint.h>
enum E { A };
void checked(void) {
  char c;
  _Bool b;
  enum E e = A;
  _BitInt(9) w;
  unsigned _BitInt(9) u;
  short s;
  signed char sc;
  unsigned char uc;
  const int k = 0;
  int r, *p;
  ckd_add(&c, 127, 1);
  ckd_add(&b, 1, 1);
  ckd_sub(&e, 1, 1);
  ckd_mul(&w, 255, 1);
  ckd_add(&u, 255, 1);
  ckd_add(&k, 1, 1);
  ckd_add(r, 1, 1);
  ckd_add(&p, 1, 1);
  ckd_add(&r, (char)1, 1);
  ckd_add(&r, (_Bool)1, 1);
  ckd_add(&r, (_BitInt(9))255, 1);
  ckd_sub(&r, 1, e);
  ckd_add(&r, 1.0, 1);
  ckd_mul(&c, (char)1, 1);
  ckd_add(&s, 1, 1);
  ckd_mul(&sc, A, 1);
  ckd_mul(&uc, (signed char)1, (unsigned char)1);
}
#define ADD(R, X, Y) ckd_add(R, X, Y)
void nested(char *c) { ADD(c, 1, 1); }
void direct(char *c, int *r, const int *k) {
  __builtin_add_overflow(1, 1, c);
  __builtin_add_overflow((char)1, (_BitInt(9))1, r);
  __builtin_sub_overflow(1.0, 1, r);
  __builtin_mul_overflow(1, 1, k);
}
