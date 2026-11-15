/* A header two deep read twice, with a warning in its first reading and
   an error in its second. */
#include "context-headers/repeat-outer.h"
void first(void) { typedef int T; }
#define REPEAT_INNER_ERROR
#include "context-headers/repeat-outer.h"
void second(void) { typedef int U; }
