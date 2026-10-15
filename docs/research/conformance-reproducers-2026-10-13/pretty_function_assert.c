/* With glibc's <assert.h> every assert() expands to __PRETTY_FUNCTION__, which bcc does not declare:
   "identifier has no visible declaration" (--target=x86_64-unknown-linux-gnu with glibc headers). */
#include <assert.h>
int main(void) { assert(1 + 1 == 2); return 0; }
