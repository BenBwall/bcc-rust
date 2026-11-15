/* The end of the input inside a function is an error GCC has read by the end
   of that function, but not by the end of the one before. */
void closed(void) { typedef int T; }
void left_open(void) { typedef int U;
