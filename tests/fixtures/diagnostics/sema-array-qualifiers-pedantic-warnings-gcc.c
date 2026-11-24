/* GCC 14's pre-C23 pedantic reports of pointers to arrays whose elements
   differ in qualifiers, and of a qualified void * converted to a pointer to
   such an array, are warnings under plain -pedantic, and the translation
   unit compiles. The atomic builtins' operands get neither. */
int (*pa)[3];
const int (*cpa)[3];
const int (*retv(void))[3] { return (const void *)0; }
int f(void) {
  const int (*b)[3] = pa;
  const int (*v)[3] = (const void *)0;
  __atomic_store_n(&cpa, pa, 0);
  __atomic_store_n(&cpa, (const void *)0, 0);
  (void)b; (void)v;
  return (pa == cpa) + (int)(pa - cpa);
}
