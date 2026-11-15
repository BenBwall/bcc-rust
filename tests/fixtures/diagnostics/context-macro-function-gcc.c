/* A function a macro writes whole is read at the invocation, which GCC
   names as the function's place. */
#define MAKE int made(void) { int *p = 1; return *p; }
MAKE
int written(void) { int *q = 2; return *q; }
