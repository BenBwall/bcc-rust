/* GCC 14 reports pointers to arrays whose elements differ only in
   qualifiers under -Wpedantic before C23, wherever it compares the targets:
   conversion by assignment, a comparison, a subtraction and the arms of ?:,
   ahead of any -Wdiscarded-array-qualifiers warning at the same place. A
   qualified void * converted to a pointer to an array whose elements carry
   its qualifiers loses them all under -pedantic. __extension__ silences the
   first; a pragma naming -Wpedantic does not reach the second. */
const int carr[3];
int (*pa)[3];
const int (*cpa)[3];
void take(const int (*)[3]);
int (*ret(void))[3] { return cpa; }
const int (*retv(void))[3] { return (const void *)0; }
int f(int c) {
  int r = 0;
  int (*a)[3] = &carr;
  const int (*b)[3] = pa;
  a = cpa;
  take(pa);
  r += pa == cpa;
  r += pa < cpa;
  r += (int)(pa - cpa);
  r += (c ? pa : cpa) != 0;
  r += __extension__ (pa != cpa);
  const int (*v)[3] = (const void *)0;
  const volatile int (*w)[3] = (const volatile void *)0;
  take((const void *)0);
  (void)a; (void)b; (void)v; (void)w;
  return r;
}
#pragma GCC diagnostic ignored "-Wpedantic"
const int (*quiet)[3] = (const void *)0;
