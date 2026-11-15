/* A header with an error read three times: each reading reports it under
   its own include, and only the function before the first counts no error. */
void first(void) { typedef int T; }
#include "context-headers/every-reading.h"
void second(void) { typedef int U; }
#include "context-headers/every-reading.h"
void third(void) { typedef int V; }
#include "context-headers/every-reading.h"
void fourth(void) { typedef int W; }
