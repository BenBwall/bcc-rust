/* Clang's -Wc23-extensions reports each C23 directive it meets before C23,
   in a skipped group too, by default; its #warning is a -Wpedantic
   extension. */
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
#embed "pp-c23-directives-clang.c" limit(1)
};
