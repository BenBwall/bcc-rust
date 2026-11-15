/* An error in a header counts where the header is included: after the first
   function, before the second. */
void first(void) { typedef int T; }
#include "unused-order-error.h"
void second(void) { typedef int U; }
