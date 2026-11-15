/* Clang's #warning before C23 is a -Wpedantic extension, which
   -pedantic-errors makes an error; -Wno-c23-extensions silences its
   #elifdef. The message keeps its place without a second snippet. */
#warning reached
#if 0
#elifdef NO
#endif
int after;
