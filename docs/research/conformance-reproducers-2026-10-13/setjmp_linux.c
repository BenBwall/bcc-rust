/* glibc's setjmp is `_setjmp`; on the Linux target bcc refuses it ("not yet supported by lowering: setjmp").
   (On the Windows target setjmp works natively but the interpreter has no host function for mingw_getsp.) */
#include <setjmp.h>
int main(void) { jmp_buf jb; if (setjmp(jb) == 0) longjmp(jb, 1); return 0; }
