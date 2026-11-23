/* GCC 14 files a qualifier lost from a pointed-to array's elements under
   -Wdiscarded-array-qualifiers, in -Wdiscarded-qualifiers' words, at every
   site; a pointed-to qualified void stays -Wdiscarded-qualifiers, and a
   qualified array target there counts as unqualified. */
typedef const int CA[3];
void take(int (*)[3]);
void takev(void *);
const int (*cpa)[3];
const volatile int (*cvpa)[3];
const int (*cpb)[2][3];
int *restrict (*rpa)[3];
CA *tpa;
const void *cv;
const volatile void *cvv;
int (*ret(void))[3] { return cpa; }
void f(void) {
  int (*a)[3] = cpa;
  a = cvpa;
  take(cpa);
  int (*b)[2][3] = cpb;
  int *(*c)[3] = rpa;
  int (*d)[3] = tpa;
  void *e = cpa;
  takev(cvpa);
  int (*g)[3] = cv;
  const int (*h)[3] = cvv;
  (void)a; (void)b; (void)c; (void)d; (void)e; (void)g; (void)h;
}
