/* clang-cl writes Clang's include stacks and macro notes, with cl's
   locations after them, and names no function. */
#include "context-headers/outer.h"
#include <angle-outer.h>
#include "context-headers/function.h"
#include "context-headers/macro.h"
int in_function(void) {
  int *p = 7;
  return *p + HEADER_SHIFT + header_function();
}
