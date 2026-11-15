/* A header first read without an error: the invalid `\u` escape of its
   second reading counts there, after the first function, and stands under
   the second `#include`. */
#include "context-headers/repeat-ucn.h"
void first(void) { typedef int T; }
#define REPEAT_ESCAPE
#include "context-headers/repeat-ucn.h"
void second(void) { typedef int U; }
