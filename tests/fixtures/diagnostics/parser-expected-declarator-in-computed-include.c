/* A header that a macro names is read on its own: its tokens are spelled
   in the header, not at the `#include` line (C99 6.10.2p4). */
#define HEADER "included-error.h"
#include HEADER
