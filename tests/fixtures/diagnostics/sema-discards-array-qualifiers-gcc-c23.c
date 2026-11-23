/* From C23 on, where an array has its elements' qualifiers, GCC 14's
   -Wdiscarded-array-qualifiers is a pedantic warning that -pedantic-errors
   makes an error, at every site. */
void take(int (*)[3]);
const int (*cpa)[3];
int (*ret(void))[3] { return cpa; }
void f(void) {
  int (*a)[3] = cpa;
  a = cpa;
  take(cpa);
  (void)a;
}
