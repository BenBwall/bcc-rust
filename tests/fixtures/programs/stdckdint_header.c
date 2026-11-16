#include <stdckdint.h>
int f(int a, int b) {
  int r;
  return ckd_add(&r, a, b) + ckd_sub(&r, a, b) + ckd_mul(&r, a, b);
}
