/* Strict GCC still reads #warning, a GCC extension before C23, and leaves
   the other C23 directives unrecognized: an error in a group it reads, and
   nothing in a skipped one. GCC 14 has no #embed. */
#warning reached
#define YES
#if 1
#elifdef YES
#endif
#if 0
#elifdef YES
int skipped;
#endif
int values[] = { 0,
#embed "pp-c23-directives-strict-gcc.c" limit(1)
};
