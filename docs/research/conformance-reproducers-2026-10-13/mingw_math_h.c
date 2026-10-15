/* On the Windows target the MinGW-w64 <math.h> alone is refused: its inline fabsf/fabsl bodies use
   __builtin_fabsf ("construct not supported by code generation: its type was not analyzed") and x87 inline
   assembly ("not yet supported by lowering: inline assembly"). The conformance harness defines __CRT__NO_INLINE
   (which drops those bodies) for Clang and bcc alike to get past it.
   bcc --target=x86_64-w64-windows-gnu -idirafter <mingw>/x86_64-w64-mingw32/include */
#include <math.h>
int main(void) { return 0; }
