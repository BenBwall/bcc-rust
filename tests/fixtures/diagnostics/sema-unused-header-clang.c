/* Clang reports an unused internal object only in the main file, and an
   unused internal function wherever it is. An object a header declares is
   reported at its first declaration in the main file. */
#include "sema-unused-header-clang.h"
static char *main_buffer;
/* Declared again in the main file, which is where Clang reports it. */
static int redeclared;
static const int main_constant = 1;
/* Only set, first declared in the header: Clang classifies it there and
   stays silent. */
static int set_only = 0;
void set_once(void) { set_only = 1; }
int main(void) { return 0; }
