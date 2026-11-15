/* Under -Werror, neither a #warning nor an unused variable it makes errors
   counts, nor does the error after the functions. */
#warning promoted
void first(void) { int unused; typedef int T; }
void second(void) { typedef int U; } int on_its_line = ;
void third(void) { typedef int V; }
