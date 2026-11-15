/* A header first read without an error: the error of its second reading
   counts there, after the first function, not where it was first read. */
#include "context-headers/repeat-error.h"
void first(void) { typedef int T; }
#define REPEAT_ERROR
#include "context-headers/repeat-error.h"
void second(void) { typedef int U; }
