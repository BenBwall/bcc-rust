/* Clang reports an unused internal object only in the main file, and an
   unused internal function wherever it is. */
#include "sema-unused-header-clang.h"
static char *main_buffer;
static const int main_constant = 1;
int main(void) { return 0; }
