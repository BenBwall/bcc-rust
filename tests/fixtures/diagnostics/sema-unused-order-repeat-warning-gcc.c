/* A header with a `#warning` read three times under -Werror=cpp: each
   reading's error stands under its own include, and none is counted
   before a function ends, as GCC does not count what -Werror promotes. */
void first(void) { typedef int T; }
#include "context-headers/every-warning.h"
void second(void) { typedef int U; }
#include "context-headers/every-warning.h"
void third(void) { typedef int V; }
#include "context-headers/every-warning.h"
