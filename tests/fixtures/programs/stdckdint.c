/* C23 7.20 checked arithmetic through <stdckdint.h>: each macro stores the
   wrapped result and yields whether it overflowed, for operands and results
   of different types. */
#include <limits.h>
#include <stdckdint.h>
#include <stdio.h>

int main(void) {
  int i;
  unsigned u;
  long l;
  long long ll;
  unsigned char uc;
  _Bool o1 = ckd_add(&i, INT_MAX, 1);
  _Bool o2 = ckd_sub(&u, 0, 1);
  _Bool o3 = ckd_mul(&l, 3000000000L, 3);
  _Bool o4 = ckd_mul(&ll, LLONG_MAX, 2);
  _Bool o5 = ckd_add(&uc, 200, 55);
  _Bool o6 = ckd_add(&uc, 200, 56);
  _Bool o7 = ckd_sub(&i, -5, 7u);
  printf("%d %d\n%d %u\n%d %ld\n%d %lld\n%d\n%d %u\n%d %d\n", o1, INT_MIN == -2147483647 - 1,
         o2, u, o3, l, o4, ll, o5, o6, uc, o7, i);
  printf("%d %zu\n", __STDC_VERSION_STDCKDINT_H__ == 202311L, sizeof ckd_add(&i, 1, 1));
  return 0;
}
