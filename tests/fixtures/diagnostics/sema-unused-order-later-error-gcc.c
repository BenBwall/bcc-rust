/* GCC counts an error once it has read it: an error further on, even on the
   line of a function's closing brace, leaves the unused local typedefs of the
   functions before it reported. */
void first(void) { typedef int T; }
void second(void) { typedef int U; } int on_its_line = ;

/* After the error GCC reports none. */
void third(void) { typedef int V; }
