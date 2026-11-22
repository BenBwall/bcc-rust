/* tccpp.c's warnings, in TCC 0.9.27's words. */
#define A 1
#define A 2
#foo
char s[] = "\q\\é";
int m = 'ab';
int w = L'ab';
int n = '\xff\x01';
#if 'ab' == 0x6162
_Static_assert(L'ab' == 'b' && '\xff\x01' == (int)0xffffff01, "values");
#endif
