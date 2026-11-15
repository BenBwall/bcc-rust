/* GCC's gnu modes read the C23 directives and report each under -pedantic
   where an older compiler would read the file otherwise. */
#warning reached
#define YES
#if 1
#elifdef YES
#endif
#if 0
#if 1
#elifdef YES
#endif
#elifdef NO
int skipped;
#elifdef YES
int taken;
#endif
#ifdef YES
#elifndef NO
#endif
int after;
