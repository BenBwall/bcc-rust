/* Include stacks: headers nested two deep with quotes and with angle
   brackets, a header read twice, and a function a header defines. */
#include "context-headers/outer.h"
#include <angle-outer.h>
int *in_main = 7;
void reads_twice(void) {
#include "context-headers/twice.h"
#include "context-headers/twice.h"
}
#include "context-headers/function.h"
int main(void) { int *q = 8; return *q + header_function(); }
