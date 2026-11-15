/* -pedantic-errors makes Clang's C23 directive warnings errors, and
   -Wno-pedantic leaves those of -Wc23-extensions errors. */
#warning reached
#define YES
#if 1
#elifdef YES
#endif
#if 0
#if 1
#elifndef YES
#endif
#elifdef YES
int taken;
#endif
const unsigned char first[] = {
#embed "pp-c23-directives-clang-pedantic-errors.c" limit(1)
};
