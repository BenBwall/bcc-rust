/* A header read three times whose `#if` reads an invalid `\x` escape each
   time: three errors, each under the `#include` of its reading. */
#include "context-headers/every-escape.h"
#include "context-headers/every-escape.h"
#include "context-headers/every-escape.h"
