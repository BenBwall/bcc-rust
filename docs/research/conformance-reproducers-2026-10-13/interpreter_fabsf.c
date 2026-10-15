/* Host functions of the interpreter: fabsf has none ("call to @fabsf, which has no definition").
   Native arms pass. Affects half of the Csmith --float programs. */
#include <math.h>
#include <stdio.h>
int main(void) { volatile float f = -1.5f; printf("%f\n", fabsf(f)); return 0; }
