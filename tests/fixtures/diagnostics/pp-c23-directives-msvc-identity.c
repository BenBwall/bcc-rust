/* cl's identity keeps the gnu modes' silence about an #elifdef after a
   group it took, and reports one it takes at the directive's name. */
#define YES
#if 1
int first;
#elifdef YES
#endif
#if 0
#elifdef YES
int taken;
#endif
int after;
