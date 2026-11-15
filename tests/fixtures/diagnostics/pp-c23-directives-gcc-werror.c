/* -Werror makes GCC's pedantic reports of the C23 directives errors that
   name it, as -pedantic-errors does not; an #elifdef without an #if or
   after an #else is an error first. */
#warning reached
#elifdef NONE
#define YES
#if 1
#elifdef YES
#endif
#if 0
#elifndef NO
int taken;
#endif
#if 0
#else
#elifdef YES
#endif
int after;
