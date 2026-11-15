/* Include stacks of headers read more than once: a header two deep with a
   warning in its first reading and an error in its second, and a header
   with an error in each of its readings. */
#include "context-headers/repeat-outer.h"
#define REPEAT_INNER_ERROR
#include "context-headers/repeat-outer.h"
#include "context-headers/every-reading.h"
int between;
#include "context-headers/every-reading.h"
